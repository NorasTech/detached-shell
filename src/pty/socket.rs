use std::io::{self, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

use crate::error::{NdsError, Result};
use crate::session::Session;

/// Creates a Unix socket listener for a session
pub fn create_listener(session_id: &str) -> Result<(UnixListener, PathBuf)> {
    let socket_path = Session::socket_dir()?.join(format!("{}.sock", session_id));

    // Remove socket if it exists
    if socket_path.exists() {
        std::fs::remove_file(&socket_path)?;
    }

    let listener = UnixListener::bind(&socket_path)
        .map_err(|e| NdsError::SocketError(format!("Failed to bind socket: {}", e)))?;

    Ok((listener, socket_path))
}

/// Send a resize command to the daemon through the socket
pub fn send_resize_command(socket: &mut UnixStream, cols: u16, rows: u16) -> io::Result<()> {
    // Format: \x1b]nds:resize:<cols>:<rows>\x07
    let resize_cmd = format!("\x1b]nds:resize:{}:{}\x07", cols, rows);
    socket.write_all(resize_cmd.as_bytes())?;
    socket.flush()
}

/// Parse NDS commands from socket data
/// Returns Some((command, args)) if a command is found, None otherwise
pub fn parse_nds_command(data: &[u8]) -> Option<(String, Vec<String>)> {
    // Check for special NDS commands
    // Format: \x1b]nds:<command>:<arg1>:<arg2>...\x07
    if data.len() > 10 && data.starts_with(b"\x1b]nds:") {
        if let Ok(cmd_str) = std::str::from_utf8(data) {
            if let Some(end_idx) = cmd_str.find('\x07') {
                let cmd = &cmd_str[6..end_idx]; // Skip \x1b]nds:
                let parts: Vec<String> = cmd.split(':').map(String::from).collect();
                if !parts.is_empty() {
                    return Some((parts[0].clone(), parts[1..].to_vec()));
                }
            }
        }
    }
    None
}

/// Get the end index of an NDS command in the data
pub fn get_command_end(data: &[u8]) -> Option<usize> {
    if data.starts_with(b"\x1b]nds:") {
        if let Ok(cmd_str) = std::str::from_utf8(data) {
            if let Some(end_idx) = cmd_str.find('\x07') {
                return Some(end_idx + 1);
            }
        }
    }
    None
}
