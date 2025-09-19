use detached_shell::{NdsError, Result, SessionManager};
use std::io::Write;
use std::os::unix::net::UnixStream;

/// List all clients connected to a session
pub fn handle_list_clients(session_id_or_name: &str) -> Result<()> {
    // Find the session
    let sessions = SessionManager::list_sessions()?;

    let matching_sessions: Vec<_> = sessions
        .iter()
        .filter(|s| {
            s.id.starts_with(session_id_or_name)
                || s.name
                    .as_ref()
                    .map_or(false, |n| n.starts_with(session_id_or_name))
        })
        .collect();

    match matching_sessions.len() {
        0 => {
            eprintln!("No session found matching: {}", session_id_or_name);
            Err(NdsError::SessionNotFound(session_id_or_name.to_string()))
        }
        1 => {
            let session = matching_sessions[0];
            println!("Clients connected to session {}:", session.display_name());
            println!();

            // Send a command to the session to list clients
            match send_client_command(&session.socket_path, "list_clients") {
                Ok(response) => {
                    println!("{}", response);
                }
                Err(e) => {
                    eprintln!("Failed to get client list: {}", e);
                    eprintln!("Session may not support client management yet.");
                }
            }
            Ok(())
        }
        _ => {
            eprintln!(
                "Multiple sessions match '{}'. Please be more specific:",
                session_id_or_name
            );
            for session in matching_sessions {
                eprintln!("  - {}", session.display_name());
            }
            Err(NdsError::InvalidSessionId(session_id_or_name.to_string()))
        }
    }
}

/// Disconnect a specific client from a session
pub fn handle_disconnect_client(session_id_or_name: &str, client_id: &str) -> Result<()> {
    // Find the session
    let sessions = SessionManager::list_sessions()?;

    let matching_sessions: Vec<_> = sessions
        .iter()
        .filter(|s| {
            s.id.starts_with(session_id_or_name)
                || s.name
                    .as_ref()
                    .map_or(false, |n| n.starts_with(session_id_or_name))
        })
        .collect();

    match matching_sessions.len() {
        0 => {
            eprintln!("No session found matching: {}", session_id_or_name);
            Err(NdsError::SessionNotFound(session_id_or_name.to_string()))
        }
        1 => {
            let session = matching_sessions[0];
            println!(
                "Disconnecting client {} from session {}...",
                client_id,
                session.display_name()
            );

            // Send disconnect command to the session
            let command = format!("disconnect_client:{}", client_id);
            match send_client_command(&session.socket_path, &command) {
                Ok(response) => {
                    println!("{}", response);
                }
                Err(e) => {
                    eprintln!("Failed to disconnect client: {}", e);
                }
            }
            Ok(())
        }
        _ => {
            eprintln!(
                "Multiple sessions match '{}'. Please be more specific:",
                session_id_or_name
            );
            for session in matching_sessions {
                eprintln!("  - {}", session.display_name());
            }
            Err(NdsError::InvalidSessionId(session_id_or_name.to_string()))
        }
    }
}

/// Send a command to the session for client management
fn send_client_command(socket_path: &std::path::Path, command: &str) -> Result<String> {
    use std::time::Duration;

    let mut socket = UnixStream::connect(socket_path)
        .map_err(|e| NdsError::SocketError(format!("Failed to connect to session: {}", e)))?;

    // Set timeout
    socket.set_read_timeout(Some(Duration::from_secs(2)))?;
    socket.set_write_timeout(Some(Duration::from_secs(2)))?;

    // Send command with special NDS prefix
    let cmd = format!("\x1b]NDS:{};\x07", command);
    socket.write_all(cmd.as_bytes())?;
    socket.flush()?;

    // Read response
    let mut response = Vec::new();
    let mut buffer = [0u8; 1024];

    // Read for a short time to get the response
    match socket.read(&mut buffer) {
        Ok(n) if n > 0 => {
            response.extend_from_slice(&buffer[..n]);
        }
        _ => {
            return Ok(
                "No response from session (client management may not be implemented)".to_string(),
            );
        }
    }

    Ok(String::from_utf8_lossy(&response).to_string())
}

// Re-export for easier imports
use std::io::Read;
