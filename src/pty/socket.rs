use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

use crate::error::{NdsError, Result};
use crate::session::Session;

/// Creates a Unix socket listener for a session with secure permissions
pub fn create_listener(session_id: &str) -> Result<(UnixListener, PathBuf)> {
    let socket_path = Session::socket_dir()?.join(format!("{}.sock", session_id));

    // Remove socket if it exists
    if socket_path.exists() {
        std::fs::remove_file(&socket_path)?;
    }

    let listener = UnixListener::bind(&socket_path)
        .map_err(|e| NdsError::SocketError(format!("Failed to bind socket: {}", e)))?;

    // Set socket permissions to 0600 (owner read/write only) for security
    let metadata = std::fs::metadata(&socket_path)?;
    let mut permissions = metadata.permissions();
    permissions.set_mode(0o600);
    std::fs::set_permissions(&socket_path, permissions)?;

    Ok((listener, socket_path))
}

/// Send a resize command to the daemon through the socket
pub fn send_resize_command(socket: &mut UnixStream, cols: u16, rows: u16) -> io::Result<()> {
    // Sanitize input to prevent overflow
    let cols = cols.min(9999).max(1);
    let rows = rows.min(9999).max(1);

    // Format: \x1b]nds:resize:<cols>:<rows>\x07
    let resize_cmd = format!("\x1b]nds:resize:{}:{}\x07", cols, rows);
    socket.write_all(resize_cmd.as_bytes())?;
    socket.flush()
}

/// Parse NDS commands from socket data with input validation
/// Returns Some((command, args)) if a valid command is found, None otherwise
pub fn parse_nds_command(data: &[u8]) -> Option<(String, Vec<String>)> {
    // Check for special NDS commands with size limit for security
    // Format: \x1b]nds:<command>:<arg1>:<arg2>...\x07
    if data.len() > 10 && data.len() < 8192 && data.starts_with(b"\x1b]nds:") {
        if let Ok(cmd_str) = std::str::from_utf8(data) {
            if let Some(end_idx) = cmd_str.find('\x07') {
                let cmd = &cmd_str[6..end_idx]; // Skip \x1b]nds:

                // Validate command is in allowed list
                if !is_valid_command(cmd) {
                    return None;
                }

                let parts: Vec<String> = cmd.split(':').map(|s| sanitize_input(s)).collect();

                if !parts.is_empty() && parts.len() <= 10 {
                    // Limit number of arguments
                    return Some((parts[0].clone(), parts[1..].to_vec()));
                }
            }
        }
    }
    None
}

/// Get the end index of an NDS command in the data with bounds checking
pub fn get_command_end(data: &[u8]) -> Option<usize> {
    if data.len() < 8192 && data.starts_with(b"\x1b]nds:") {
        if let Ok(cmd_str) = std::str::from_utf8(data) {
            if let Some(end_idx) = cmd_str.find('\x07') {
                // Ensure the end index is within reasonable bounds
                if end_idx < 1024 {
                    return Some(end_idx + 1);
                }
            }
        }
    }
    None
}

/// Validate that a command string is safe
fn is_valid_command(cmd: &str) -> bool {
    // Whitelist of allowed commands
    const ALLOWED_COMMANDS: &[&str] = &[
        "resize",
        "detach",
        "attach",
        "list",
        "kill",
        "switch",
        "scrollback",
        "clear",
        "refresh",
    ];

    if let Some(command) = cmd.split(':').next() {
        ALLOWED_COMMANDS.contains(&command)
    } else {
        false
    }
}

/// Sanitize string input to prevent command injection
fn sanitize_input(input: &str) -> String {
    // Remove control characters and limit length
    input
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\r' || *c == '\t')
        .take(4096) // Limit input length
        .collect()
}
