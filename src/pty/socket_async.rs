use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use tokio::io::AsyncWriteExt;
use tokio::net::{UnixListener, UnixStream};

use crate::error::{NdsError, Result};
use crate::session::Session;

/// Buffer size constants for improved performance
pub const DEFAULT_BUFFER_SIZE: usize = 16384; // 16KB for better throughput
pub const SMALL_BUFFER_SIZE: usize = 4096; // 4KB for control messages

/// Creates a Unix socket listener for a session with secure permissions
pub async fn create_listener_async(session_id: &str) -> Result<(UnixListener, PathBuf)> {
    let socket_path = Session::socket_dir()?.join(format!("{}.sock", session_id));

    // Remove socket if it exists
    if socket_path.exists() {
        tokio::fs::remove_file(&socket_path).await?;
    }

    // Create socket with restricted permissions
    let listener = UnixListener::bind(&socket_path)
        .map_err(|e| NdsError::SocketError(format!("Failed to bind socket: {}", e)))?;

    // Set socket permissions to 0600 (owner read/write only)
    let metadata = tokio::fs::metadata(&socket_path).await?;
    let mut permissions = metadata.permissions();
    permissions.set_mode(0o600);
    tokio::fs::set_permissions(&socket_path, permissions).await?;

    Ok((listener, socket_path))
}

/// Send a resize command to the daemon through the socket (async version)
pub async fn send_resize_command_async(
    socket: &mut UnixStream,
    cols: u16,
    rows: u16,
) -> tokio::io::Result<()> {
    // Sanitize input to prevent injection
    let cols = sanitize_numeric_input(cols);
    let rows = sanitize_numeric_input(rows);

    // Format: \x1b]nds:resize:<cols>:<rows>\x07
    let resize_cmd = format!("\x1b]nds:resize:{}:{}\x07", cols, rows);
    socket.write_all(resize_cmd.as_bytes()).await?;
    socket.flush().await
}

/// Sanitize numeric input to prevent overflow or injection
pub fn sanitize_numeric_input(value: u16) -> u16 {
    // Limit terminal size to reasonable values
    value.min(9999).max(1)
}

/// Sanitize string input to prevent command injection
pub fn sanitize_string_input(input: &str) -> String {
    // Remove control characters and limit length
    input
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\r' || *c == '\t')
        .take(4096) // Limit input length
        .collect()
}

/// Parse NDS commands from socket data with input validation
/// Returns Some((command, args)) if a valid command is found, None otherwise
pub fn parse_nds_command_secure(data: &[u8]) -> Option<(String, Vec<String>)> {
    // Check for special NDS commands
    // Format: \x1b]nds:<command>:<arg1>:<arg2>...\x07
    if data.len() > 10 && data.len() < 8192 && data.starts_with(b"\x1b]nds:") {
        if let Ok(cmd_str) = std::str::from_utf8(data) {
            if let Some(end_idx) = cmd_str.find('\x07') {
                let cmd = &cmd_str[6..end_idx]; // Skip \x1b]nds:

                // Validate command format
                if !is_valid_command(cmd) {
                    return None;
                }

                let parts: Vec<String> = cmd.split(':').map(|s| sanitize_string_input(s)).collect();

                if !parts.is_empty() && parts.len() <= 10 {
                    // Limit number of arguments
                    return Some((parts[0].clone(), parts[1..].to_vec()));
                }
            }
        }
    }
    None
}

/// Validate that a command string is safe
pub fn is_valid_command(cmd: &str) -> bool {
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

/// Get the end index of an NDS command in the data (with bounds checking)
pub fn get_command_end_secure(data: &[u8]) -> Option<usize> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_numeric_input() {
        assert_eq!(sanitize_numeric_input(100), 100);
        assert_eq!(sanitize_numeric_input(10000), 9999);
        assert_eq!(sanitize_numeric_input(0), 1);
    }

    #[test]
    fn test_sanitize_string_input() {
        assert_eq!(sanitize_string_input("hello"), "hello");
        assert_eq!(sanitize_string_input("hello\x00world"), "helloworld");
        assert_eq!(sanitize_string_input("hello\nworld"), "hello\nworld");
    }

    #[test]
    fn test_is_valid_command() {
        assert!(is_valid_command("resize:80:24"));
        assert!(is_valid_command("detach"));
        assert!(!is_valid_command("rm:rf:/"));
        assert!(!is_valid_command("unknown:command"));
    }
}
