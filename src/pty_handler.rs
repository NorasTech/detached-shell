use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::io::{FromRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};

use crate::error::{NdsError, Result};
use crate::pty_buffer::PtyBuffer;

/// Enhanced PTY handler that maintains state across connections
pub struct PtyHandler {
    master_fd: RawFd,
    output_buffer: PtyBuffer,
    last_prompt_position: Arc<Mutex<Option<usize>>>,
}

impl PtyHandler {
    pub fn new(master_fd: RawFd, buffer_size: usize) -> Self {
        PtyHandler {
            master_fd,
            output_buffer: PtyBuffer::new(buffer_size),
            last_prompt_position: Arc::new(Mutex::new(None)),
        }
    }

    /// Read from PTY and handle the data
    pub fn read_pty_data(&mut self, buffer: &mut [u8]) -> Result<Option<Vec<u8>>> {
        let master_file = unsafe { File::from_raw_fd(self.master_fd) };
        let mut master_file_clone = master_file.try_clone()?;
        std::mem::forget(master_file); // Don't close the fd

        match master_file_clone.read(buffer) {
            Ok(0) => Ok(None), // EOF
            Ok(n) => {
                let data = buffer[..n].to_vec();

                // Detect prompt patterns (simple heuristic)
                if Self::looks_like_prompt(&data) {
                    *self.last_prompt_position.lock().unwrap() = Some(n);
                }

                Ok(Some(data))
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                Ok(Some(Vec::new())) // No data available
            }
            Err(e) => Err(NdsError::Io(e)),
        }
    }

    /// Write data to PTY
    pub fn write_to_pty(&mut self, data: &[u8]) -> Result<()> {
        let mut master_file = unsafe { File::from_raw_fd(self.master_fd) };
        master_file.write_all(data)?;
        std::mem::forget(master_file); // Don't close the fd
        Ok(())
    }

    /// Process PTY data and distribute to client or buffer
    pub fn process_pty_output(
        &mut self,
        data: &[u8],
        client: &mut Option<UnixStream>,
    ) -> Result<()> {
        if let Some(ref mut stream) = client {
            // Try to send to connected client
            if let Err(e) = stream.write_all(data) {
                if e.kind() == std::io::ErrorKind::BrokenPipe {
                    // Client disconnected, buffer the data
                    self.output_buffer.push(data);
                    *client = None;
                }
                return Err(NdsError::Io(e));
            }
        } else {
            // No client connected, buffer the output
            self.output_buffer.push(data);
        }
        Ok(())
    }

    /// Send buffered data to a newly connected client
    pub fn send_buffered_data(&mut self, client: &mut UnixStream) -> Result<()> {
        if !self.output_buffer.is_empty() {
            let mut buffered_data = Vec::new();
            self.output_buffer.drain_to(&mut buffered_data);

            // Send in manageable chunks
            const CHUNK_SIZE: usize = 4096;
            for chunk in buffered_data.chunks(CHUNK_SIZE) {
                client.write_all(chunk)?;
                client.flush()?;
                // Small delay to avoid overwhelming the client
                std::thread::sleep(std::time::Duration::from_micros(100));
            }
        }
        Ok(())
    }

    /// Check if data looks like a shell prompt
    fn looks_like_prompt(data: &[u8]) -> bool {
        // Simple heuristic: check for common prompt endings
        // This could be made more sophisticated
        if data.len() < 2 {
            return false;
        }

        // Check for common prompt characters at the end
        let last_chars = &data[data.len().saturating_sub(10)..];
        last_chars.contains(&b'$')
            || last_chars.contains(&b'#')
            || last_chars.contains(&b'>')
            || last_chars.contains(&b'%')
    }

    /// Request terminal to refresh/redraw
    pub fn refresh_terminal(&mut self) -> Result<()> {
        // Send Ctrl+L to refresh the display
        self.write_to_pty(b"\x0c")?;
        Ok(())
    }
}
