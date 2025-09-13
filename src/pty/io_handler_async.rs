use std::fs::File;
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::RwLock;
use tokio::time::sleep;

use crate::pty::socket_async::DEFAULT_BUFFER_SIZE;
use crate::pty_buffer::PtyBuffer;

/// Handle reading from PTY master and broadcasting to clients (async version)
pub struct AsyncPtyIoHandler {
    master_fd: RawFd,
    buffer_size: usize,
}

impl AsyncPtyIoHandler {
    pub fn new(master_fd: RawFd) -> Self {
        Self {
            master_fd,
            buffer_size: DEFAULT_BUFFER_SIZE, // Use 16KB buffer for better performance
        }
    }

    /// Read from PTY master file descriptor asynchronously
    pub async fn read_from_pty(&self, buffer: &mut [u8]) -> tokio::io::Result<usize> {
        // Create async file from raw fd
        let master_file = unsafe { File::from_raw_fd(self.master_fd) };
        let master_fd_clone = master_file.as_raw_fd();
        std::mem::forget(master_file); // Don't close the fd

        // Use tokio's async file operations
        let mut async_file =
            tokio::fs::File::from_std(unsafe { std::fs::File::from_raw_fd(master_fd_clone) });

        let result = async_file.read(buffer).await;
        std::mem::forget(async_file); // Don't close the fd
        result
    }

    /// Write to PTY master file descriptor asynchronously
    pub async fn write_to_pty(&self, data: &[u8]) -> tokio::io::Result<()> {
        let master_file = unsafe { File::from_raw_fd(self.master_fd) };
        let master_fd_clone = master_file.as_raw_fd();
        std::mem::forget(master_file); // Don't close the fd

        let mut async_file =
            tokio::fs::File::from_std(unsafe { std::fs::File::from_raw_fd(master_fd_clone) });

        let result = async_file.write_all(data).await;
        std::mem::forget(async_file); // Don't close the fd
        result
    }

    /// Send a control character to the PTY
    pub async fn send_control_char(&self, ch: u8) -> tokio::io::Result<()> {
        self.write_to_pty(&[ch]).await
    }

    /// Send Ctrl+L to refresh the display
    pub async fn send_refresh(&self) -> tokio::io::Result<()> {
        self.send_control_char(0x0c).await // Ctrl+L
    }
}

/// Handle scrollback buffer management with thread-safe async operations
pub struct AsyncScrollbackHandler {
    buffer: Arc<RwLock<Vec<u8>>>,
    max_size: usize,
}

impl AsyncScrollbackHandler {
    pub fn new(max_size: usize) -> Self {
        Self {
            buffer: Arc::new(RwLock::new(Vec::with_capacity(max_size / 4))),
            max_size,
        }
    }

    /// Add data to the scrollback buffer (async)
    pub async fn add_data(&self, data: &[u8]) {
        let mut buffer = self.buffer.write().await;
        buffer.extend_from_slice(data);

        // Trim if too large
        if buffer.len() > self.max_size {
            let remove = buffer.len() - self.max_size;
            buffer.drain(..remove);
        }
    }

    /// Get a clone of the scrollback buffer (async)
    pub async fn get_buffer(&self) -> Vec<u8> {
        self.buffer.read().await.clone()
    }

    /// Get a reference to the shared buffer
    pub fn get_shared_buffer(&self) -> Arc<RwLock<Vec<u8>>> {
        Arc::clone(&self.buffer)
    }
}

/// Async task that reads from socket and writes to stdout
pub async fn socket_to_stdout_task(
    mut socket: UnixStream,
    running: Arc<AtomicBool>,
    scrollback: Arc<RwLock<Vec<u8>>>,
) -> tokio::io::Result<()> {
    let mut buffer = vec![0u8; DEFAULT_BUFFER_SIZE];
    let mut stdout = tokio::io::stdout();

    while running.load(Ordering::SeqCst) {
        tokio::select! {
            result = socket.read(&mut buffer) => {
                match result {
                    Ok(0) => break, // Socket closed
                    Ok(n) => {
                        // Write to stdout
                        if stdout.write_all(&buffer[..n]).await.is_err() {
                            break;
                        }
                        stdout.flush().await?;

                        // Add to scrollback buffer
                        let mut scrollback_guard = scrollback.write().await;
                        scrollback_guard.extend_from_slice(&buffer[..n]);

                        // Trim if too large
                        let scrollback_max = 10 * 1024 * 1024; // 10MB
                        if scrollback_guard.len() > scrollback_max {
                            let remove = scrollback_guard.len() - scrollback_max;
                            scrollback_guard.drain(..remove);
                        }
                    }
                    Err(e) if e.kind() == tokio::io::ErrorKind::WouldBlock => {
                        sleep(Duration::from_millis(10)).await;
                    }
                    Err(e) if e.kind() == tokio::io::ErrorKind::BrokenPipe => {
                        // Expected when socket is closed, just exit cleanly
                        break;
                    }
                    Err(_) => break,
                }
            }
            _ = sleep(Duration::from_millis(100)) => {
                // Periodic check if we should continue
                if !running.load(Ordering::SeqCst) {
                    break;
                }
            }
        }
    }

    Ok(())
}

/// Async task that monitors terminal size changes
pub async fn resize_monitor_task(
    mut socket: UnixStream,
    running: Arc<AtomicBool>,
    initial_size: (u16, u16),
) -> tokio::io::Result<()> {
    use crate::pty::socket_async::send_resize_command_async;
    use crossterm::terminal;

    let mut last_size = initial_size;

    while running.load(Ordering::SeqCst) {
        if let Ok((new_cols, new_rows)) = terminal::size() {
            if (new_cols, new_rows) != last_size {
                // Terminal size changed, send resize command
                send_resize_command_async(&mut socket, new_cols, new_rows).await?;
                last_size = (new_cols, new_rows);
            }
        }
        sleep(Duration::from_millis(250)).await;
    }

    Ok(())
}

/// Helper to send buffered output to a new client (async version)
pub async fn send_buffered_output_async(
    stream: &mut UnixStream,
    output_buffer: &PtyBuffer,
    io_handler: &AsyncPtyIoHandler,
) -> tokio::io::Result<()> {
    if !output_buffer.is_empty() {
        let mut buffered_data = Vec::new();
        output_buffer.drain_to(&mut buffered_data);

        // Save cursor position, clear screen, and reset
        let init_sequence = b"\x1b7\x1b[?47h\x1b[2J\x1b[H"; // Save cursor, alt screen, clear, home
        stream.write_all(init_sequence).await?;
        stream.flush().await?;

        // Send buffered data in chunks to avoid overwhelming the client
        for chunk in buffered_data.chunks(DEFAULT_BUFFER_SIZE) {
            stream.write_all(chunk).await?;
            stream.flush().await?;
            sleep(Duration::from_millis(1)).await;
        }

        // Exit alt screen and restore cursor
        let restore_sequence = b"\x1b[?47l\x1b8"; // Exit alt screen, restore cursor
        stream.write_all(restore_sequence).await?;
        stream.flush().await?;

        // Small delay for terminal to process
        sleep(Duration::from_millis(50)).await;

        // Send a full redraw command to the shell
        io_handler.send_refresh().await?;

        // Give time for the refresh to complete
        sleep(Duration::from_millis(100)).await;
    } else {
        // No buffer, just request a refresh to sync state
        io_handler.send_refresh().await?;
    }

    Ok(())
}

/// Session manager using Arc<RwLock> for multi-threaded access
pub struct AsyncSessionManager {
    sessions: Arc<RwLock<std::collections::HashMap<String, SessionData>>>,
}

#[derive(Clone)]
pub struct SessionData {
    pub id: String,
    pub master_fd: RawFd,
    pub pid: i32,
    pub socket_path: std::path::PathBuf,
    pub created_at: std::time::SystemTime,
}

impl AsyncSessionManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(std::collections::HashMap::new())),
        }
    }

    pub async fn add_session(&self, id: String, data: SessionData) {
        let mut sessions = self.sessions.write().await;
        sessions.insert(id, data);
    }

    pub async fn remove_session(&self, id: &str) -> Option<SessionData> {
        let mut sessions = self.sessions.write().await;
        sessions.remove(id)
    }

    pub async fn get_session(&self, id: &str) -> Option<SessionData> {
        let sessions = self.sessions.read().await;
        sessions.get(id).cloned()
    }

    pub async fn list_sessions(&self) -> Vec<SessionData> {
        let sessions = self.sessions.read().await;
        sessions.values().cloned().collect()
    }

    pub async fn session_count(&self) -> usize {
        let sessions = self.sessions.read().await;
        sessions.len()
    }
}
