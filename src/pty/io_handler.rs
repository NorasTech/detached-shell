use std::fs::File;
use std::io::{self, Read, Write};
use std::os::unix::io::{FromRawFd, RawFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use crate::pty_buffer::PtyBuffer;

/// Handle reading from PTY master and broadcasting to clients
pub struct PtyIoHandler {
    master_fd: RawFd,
    buffer_size: usize,
}

impl PtyIoHandler {
    pub fn new(master_fd: RawFd) -> Self {
        Self {
            master_fd,
            buffer_size: 4096,
        }
    }

    /// Read from PTY master file descriptor
    pub fn read_from_pty(&self, buffer: &mut [u8]) -> io::Result<usize> {
        let master_file = unsafe { File::from_raw_fd(self.master_fd) };
        let mut master_file_clone = master_file.try_clone()?;
        std::mem::forget(master_file); // Don't close the fd

        master_file_clone.read(buffer)
    }

    /// Write to PTY master file descriptor
    pub fn write_to_pty(&self, data: &[u8]) -> io::Result<()> {
        let mut master_file = unsafe { File::from_raw_fd(self.master_fd) };
        let result = master_file.write_all(data);
        std::mem::forget(master_file); // Don't close the fd
        result
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
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut stdout = io::stdout();
        let mut buffer = [0u8; 4096];

        while running.load(Ordering::SeqCst) {
            match socket.read(&mut buffer) {
                Ok(0) => break, // Socket closed
                Ok(n) => {
                    // Write to stdout
                    if stdout.write_all(&buffer[..n]).is_err() {
                        break;
                    }
                    let _ = stdout.flush();

                    // Add to scrollback buffer
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
    use crossterm::terminal;

    thread::spawn(move || {
        let mut last_size = initial_size;

        while running.load(Ordering::SeqCst) {
            if let Ok((new_cols, new_rows)) = terminal::size() {
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

/// Helper to send buffered output to a new client
pub fn send_buffered_output(
    stream: &mut std::os::unix::net::UnixStream,
    output_buffer: &PtyBuffer,
    io_handler: &PtyIoHandler,
) -> io::Result<()> {
    if !output_buffer.is_empty() {
        let mut buffered_data = Vec::new();
        output_buffer.drain_to(&mut buffered_data);

        // Save cursor position, clear screen, and reset
        let init_sequence = b"\x1b7\x1b[?47h\x1b[2J\x1b[H"; // Save cursor, alt screen, clear, home
        stream.write_all(init_sequence)?;
        stream.flush()?;

        // Send buffered data in chunks to avoid overwhelming the client
        for chunk in buffered_data.chunks(4096) {
            stream.write_all(chunk)?;
            stream.flush()?;
            thread::sleep(Duration::from_millis(1));
        }

        // Exit alt screen and restore cursor
        let restore_sequence = b"\x1b[?47l\x1b8"; // Exit alt screen, restore cursor
        stream.write_all(restore_sequence)?;
        stream.flush()?;

        // Small delay for terminal to process
        thread::sleep(Duration::from_millis(50));

        // Send a full redraw command to the shell
        io_handler.send_refresh()?;

        // Give time for the refresh to complete
        thread::sleep(Duration::from_millis(100));
    } else {
        // No buffer, just request a refresh to sync state
        io_handler.send_refresh()?;
    }

    Ok(())
}
