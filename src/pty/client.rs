use chrono::{DateTime, Utc};
use nix::libc;
use std::io::{self, Write};
use std::os::unix::net::UnixStream;
use uuid::Uuid;

// Structure to track client information
#[allow(dead_code)]
#[derive(Debug)]
pub struct ClientInfo {
    pub id: String, // Unique client ID
    pub stream: UnixStream,
    pub rows: u16,
    pub cols: u16,
    pub connected_at: DateTime<Utc>,
    #[allow(dead_code)]
    pub remote_addr: Option<String>, // For future use with network connections
    #[allow(dead_code)]
    pub user_agent: Option<String>, // Client type/version info
    pub pending_output: Vec<u8>, // Bytes we still owe the client
}

impl ClientInfo {
    #[allow(dead_code)]
    pub fn new(stream: UnixStream) -> Self {
        // Get initial terminal size
        let (rows, cols) = get_terminal_size().unwrap_or((24, 80));

        Self {
            id: Uuid::new_v4().to_string()[..8].to_string(),
            stream,
            rows,
            cols,
            connected_at: Utc::now(),
            remote_addr: None,
            user_agent: None,
            pending_output: Vec::new(),
        }
    }

    /// Try to drain any queued output for this client. We tolerate WouldBlock
    /// by leaving remaining bytes in the queue for the next loop iteration.
    pub fn flush_pending(&mut self) -> io::Result<()> {
        while !self.pending_output.is_empty() {
            match self.stream.write(&self.pending_output) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "socket closed while flushing pending output",
                    ));
                }
                Ok(n) => {
                    self.pending_output.drain(..n);
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    break;
                }
                Err(ref e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// Attempt to send fresh data to the client, queueing any tail bytes that
    /// cannot be delivered immediately.
    pub fn send_data(&mut self, data: &[u8]) -> io::Result<()> {
        if data.is_empty() {
            return Ok(());
        }

        // If we already have queued data, append and try to flush once.
        if !self.pending_output.is_empty() {
            self.pending_output.extend_from_slice(data);
            return self.flush_pending();
        }

        let mut offset = 0;
        while offset < data.len() {
            match self.stream.write(&data[offset..]) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "socket closed while sending",
                    ));
                }
                Ok(n) => {
                    offset += n;
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    self.pending_output.extend_from_slice(&data[offset..]);
                    break;
                }
                Err(ref e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }

        Ok(())
    }

    /// Get a display string for this client
    #[allow(dead_code)]
    pub fn display_info(&self) -> String {
        let duration = Utc::now().signed_duration_since(self.connected_at);
        let hours = duration.num_hours();
        let minutes = (duration.num_minutes() % 60) as u32;
        let seconds = (duration.num_seconds() % 60) as u32;

        let duration_str = if hours > 0 {
            format!("{}h{}m", hours, minutes)
        } else if minutes > 0 {
            format!("{}m{}s", minutes, seconds)
        } else {
            format!("{}s", seconds)
        };

        format!(
            "Client {} | Size: {}x{} | Connected: {} | Duration: {}",
            self.id,
            self.cols,
            self.rows,
            self.connected_at.format("%H:%M:%S"),
            duration_str
        )
    }

    #[allow(dead_code)]
    pub fn update_size(&mut self, rows: u16, cols: u16) {
        self.rows = rows;
        self.cols = cols;
    }
}

#[allow(dead_code)]
pub fn get_terminal_size() -> Result<(u16, u16), std::io::Error> {
    unsafe {
        let mut size: libc::winsize = std::mem::zeroed();
        if libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut size) == -1 {
            return Err(std::io::Error::last_os_error());
        }
        Ok((size.ws_row, size.ws_col))
    }
}

#[cfg(test)]
mod tests {
    use super::ClientInfo;
    use std::io::{Read, Result};
    use std::os::unix::net::UnixStream;

    #[test]
    fn send_data_buffers_when_socket_would_block() -> Result<()> {
        let (mut writer, mut reader) = UnixStream::pair()?;
        writer.set_nonblocking(true)?;

        let mut client = ClientInfo::new(writer);
        let chunk = vec![0u8; 64 * 1024];
        let mut total_expected = 0usize;

        // Keep sending until the socket refuses more data or we hit a hard cap.
        for _ in 0..128 {
            client.send_data(&chunk)?;
            total_expected += chunk.len();
            if !client.pending_output.is_empty() {
                break;
            }
        }

        assert!(
            !client.pending_output.is_empty(),
            "socket never blocked during test"
        );

        // Drain the other end in chunks, flushing pending bytes as space frees up.
        let mut buf = [0u8; 32 * 1024];
        let mut received = 0usize;
        while received < total_expected {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    received += n;
                    client.flush_pending()?;
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    client.flush_pending()?;
                }
                Err(e) => return Err(e),
            }
        }

        assert_eq!(received, total_expected);
        assert!(client.pending_output.is_empty());

        Ok(())
    }
}
