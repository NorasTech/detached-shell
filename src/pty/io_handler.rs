use std::io::{self, Read, Write};
use std::os::unix::io::RawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use crate::pty_buffer::PtyBuffer;

/// Buffer size constants for improved performance
pub const DEFAULT_BUFFER_SIZE: usize = 16384; // 16KB for better throughput
#[allow(dead_code)]
pub const SMALL_BUFFER_SIZE: usize = 4096; // 4KB for control messages

/// Handle reading from PTY master and broadcasting to clients
pub struct PtyIoHandler {
    master_fd: RawFd,
    #[allow(dead_code)]
    buffer_size: usize,
}

impl PtyIoHandler {
    pub fn new(master_fd: RawFd) -> Self {
        Self {
            master_fd,
            buffer_size: DEFAULT_BUFFER_SIZE, // Use 16KB buffer for better performance
        }
    }

    /// Read from PTY master file descriptor
    pub fn read_from_pty(&self, buffer: &mut [u8]) -> io::Result<usize> {
        // Use direct syscall to avoid file descriptor issues
        unsafe {
            let result = libc::read(
                self.master_fd,
                buffer.as_mut_ptr() as *mut libc::c_void,
                buffer.len(),
            );

            if result < 0 {
                let err = io::Error::last_os_error();
                // Check if it's a recoverable error
                if err.kind() == io::ErrorKind::WouldBlock
                    || err.kind() == io::ErrorKind::Interrupted
                {
                    return Err(err);
                }
                // For other errors, still return them but log for debugging
                return Err(err);
            }

            Ok(result as usize)
        }
    }

    /// Write to PTY master file descriptor
    pub fn write_to_pty(&self, data: &[u8]) -> io::Result<()> {
        // Use direct syscall to avoid file descriptor issues
        let mut written = 0;
        while written < data.len() {
            unsafe {
                let result = libc::write(
                    self.master_fd,
                    data[written..].as_ptr() as *const libc::c_void,
                    data.len() - written,
                );

                if result < 0 {
                    let err = io::Error::last_os_error();
                    // Retry on interrupted system call
                    if err.kind() == io::ErrorKind::Interrupted {
                        continue;
                    }
                    // Return error for non-recoverable errors
                    return Err(err);
                }

                written += result as usize;
            }
        }
        Ok(())
    }

    /// Send a control character to the PTY
    pub fn send_control_char(&self, ch: u8) -> io::Result<()> {
        self.write_to_pty(&[ch])
    }

    /// Send Ctrl+L to refresh the display
    pub fn send_refresh(&self) -> io::Result<()> {
        self.send_control_char(0x0c) // Ctrl+L
    }
}

/// Handle scrollback buffer management
pub struct ScrollbackHandler {
    buffer: Arc<Mutex<Vec<u8>>>,
    #[allow(dead_code)]
    max_size: usize,
}

impl ScrollbackHandler {
    pub fn new(max_size: usize) -> Self {
        Self {
            buffer: Arc::new(Mutex::new(Vec::new())),
            max_size,
        }
    }

    /// Add data to the scrollback buffer
    #[allow(dead_code)]
    pub fn add_data(&self, data: &[u8]) {
        let mut buffer = self.buffer.lock().unwrap();
        buffer.extend_from_slice(data);

        // Trim if too large
        if buffer.len() > self.max_size {
            let remove = buffer.len() - self.max_size;
            buffer.drain(..remove);
        }
    }

    /// Get a clone of the scrollback buffer
    pub fn get_buffer(&self) -> Vec<u8> {
        self.buffer.lock().unwrap().clone()
    }

    /// Get a reference to the shared buffer
    pub fn get_shared_buffer(&self) -> Arc<Mutex<Vec<u8>>> {
        Arc::clone(&self.buffer)
    }
}

/// Thread that reads from socket and writes to stdout
pub fn spawn_socket_to_stdout_thread(
    mut socket: std::os::unix::net::UnixStream,
    running: Arc<AtomicBool>,
    scrollback: Arc<Mutex<Vec<u8>>>,
    paused: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut stdout = io::stdout();
        let mut buffer = [0u8; DEFAULT_BUFFER_SIZE]; // Use 16KB buffer
        let mut held_buffer = Vec::new(); // Buffer to hold data while paused

        while running.load(Ordering::SeqCst) {
            // If paused, just sleep and continue
            if paused.load(Ordering::SeqCst) {
                // Still read from socket to prevent blocking, but buffer it
                match socket.read(&mut buffer) {
                    Ok(0) => break, // Socket closed
                    Ok(n) => {
                        // Hold the data while paused
                        held_buffer.extend_from_slice(&buffer[..n]);
                    }
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
                continue;
            }

            // If we have held data and we're no longer paused, flush it
            if !held_buffer.is_empty() {
                if stdout.write_all(&held_buffer).is_err() {
                    break;
                }
                let _ = stdout.flush();

                // Add to scrollback
                let mut scrollback = scrollback.lock().unwrap();
                scrollback.extend_from_slice(&held_buffer);
                held_buffer.clear();
            }

            match socket.read(&mut buffer) {
                Ok(0) => break, // Socket closed
                Ok(n) => {
                    // Write to stdout only if not paused
                    if !paused.load(Ordering::SeqCst) {
                        if stdout.write_all(&buffer[..n]).is_err() {
                            break;
                        }
                        let _ = stdout.flush();
                    } else {
                        // If paused mid-read, buffer it
                        held_buffer.extend_from_slice(&buffer[..n]);
                    }

                    // Always add to scrollback buffer
                    let mut scrollback = scrollback.lock().unwrap();
                    scrollback.extend_from_slice(&buffer[..n]);

                    // Trim if too large
                    let scrollback_max = 10 * 1024 * 1024; // 10MB
                    if scrollback.len() > scrollback_max {
                        let remove = scrollback.len() - scrollback_max;
                        scrollback.drain(..remove);
                    }
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(ref e) if e.kind() == io::ErrorKind::BrokenPipe => {
                    // Expected when socket is closed, just exit cleanly
                    break;
                }
                Err(_) => break,
            }
        }
    })
}

/// Thread that monitors terminal size changes
pub fn spawn_resize_monitor_thread(
    mut socket: std::os::unix::net::UnixStream,
    running: Arc<AtomicBool>,
    initial_size: (u16, u16),
) -> thread::JoinHandle<()> {
    use crate::pty::socket::send_resize_command;
    use crate::pty::terminal::get_terminal_size;

    thread::spawn(move || {
        let mut last_size = initial_size;

        while running.load(Ordering::SeqCst) {
            if let Ok((new_cols, new_rows)) = get_terminal_size() {
                if (new_cols, new_rows) != last_size {
                    // Terminal size changed, send resize command
                    let _ = send_resize_command(&mut socket, new_cols, new_rows);
                    last_size = (new_cols, new_rows);
                }
            }
            thread::sleep(Duration::from_millis(250));
        }
    })
}

#[allow(dead_code)]
/// Helper to send buffered output to a new client
pub fn send_buffered_output(
    stream: &mut std::os::unix::net::UnixStream,
    output_buffer: &PtyBuffer,
    io_handler: &PtyIoHandler,
) -> io::Result<()> {
    if !output_buffer.is_empty() {
        let mut buffered_data = Vec::new();
        output_buffer.drain_to(&mut buffered_data);

        // Don't use alternate screen or clear - it destroys the session state
        // Just send the buffered output directly
        for chunk in buffered_data.chunks(DEFAULT_BUFFER_SIZE) {
            stream.write_all(chunk)?;
            stream.flush()?;
            thread::sleep(Duration::from_millis(1));
        }

        // Send a refresh to sync the display
        io_handler.send_refresh()?;

        // Small delay for terminal to process
        thread::sleep(Duration::from_millis(50));
    } else {
        // No buffer, just request a refresh to sync state
        io_handler.send_refresh()?;
    }

    Ok(())
}
