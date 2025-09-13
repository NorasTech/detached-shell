use detached_shell::{NdsError, Result, Session, SessionManager};
use std::thread;
use std::time::Duration;

/// Creates a new detached shell session with optional name
pub fn handle_new_session(name: Option<String>, attach: bool) -> Result<()> {
    if let Some(ref session_name) = name {
        println!("Creating new session '{}'...", session_name);
    } else {
        println!("Creating new session...");
    }

    match SessionManager::create_session_with_name(name) {
        Ok(session) => {
            println!("Created session: {}", session.id);
            println!("PID: {}", session.pid);
            println!("Socket: {}", session.socket_path.display());

            if attach {
                println!("\nAttaching to session...");
                // Give the session a moment to fully initialize
                thread::sleep(Duration::from_millis(100));
                handle_attach_session(&session.id)?;
            } else {
                println!("\nTo attach to this session, run:");
                println!("  nds attach {}", session.id);
            }
            Ok(())
        }
        Err(e) => {
            eprintln!("Failed to create session: {}", e);
            Err(e)
        }
    }
}

/// Attaches to an existing session by ID or name (supports partial matching)
pub fn handle_attach_session(session_id_or_name: &str) -> Result<()> {
    // Allow partial ID or name matching
    let sessions = SessionManager::list_sessions()?;

    // First try to match by ID
    let mut matching_sessions: Vec<_> = sessions
        .iter()
        .filter(|s| s.id.starts_with(session_id_or_name))
        .collect();

    // If no ID matches, try matching by name
    if matching_sessions.is_empty() {
        matching_sessions = sessions
            .iter()
            .filter(|s| {
                if let Some(ref name) = s.name {
                    name == session_id_or_name
                        || name.starts_with(session_id_or_name)
                        || name
                            .to_lowercase()
                            .starts_with(&session_id_or_name.to_lowercase())
                } else {
                    false
                }
            })
            .collect();
    }

    match matching_sessions.len() {
        0 => {
            eprintln!(
                "No session found matching ID or name: {}",
                session_id_or_name
            );
            Err(NdsError::SessionNotFound(session_id_or_name.to_string()))
        }
        1 => {
            let session = matching_sessions[0];
            SessionManager::attach_session(&session.id)?;
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

/// Kills one or more sessions by ID or name
pub fn handle_kill_sessions(session_ids: &[String]) -> Result<()> {
    if session_ids.is_empty() {
        eprintln!("No session IDs provided");
        return Err(NdsError::SessionNotFound(
            "No session IDs provided".to_string(),
        ));
    }

    let sessions = SessionManager::list_sessions()?;
    let mut killed_count = 0;
    let mut errors = Vec::new();

    for session_id in session_ids {
        match kill_single_session(session_id, &sessions) {
            Ok(killed_id) => {
                println!("Killed session: {}", killed_id);
                killed_count += 1;
            }
            Err(e) => {
                eprintln!("Error killing session '{}': {}", session_id, e);
                errors.push(format!("{}: {}", session_id, e));
            }
        }
    }

    if killed_count > 0 {
        println!("Successfully killed {} session(s)", killed_count);
    }

    if !errors.is_empty() && killed_count == 0 {
        Err(NdsError::SessionNotFound(errors.join(", ")))
    } else {
        Ok(())
    }
}

/// Helper function to kill a single session with partial matching support
fn kill_single_session(session_id_or_name: &str, sessions: &[Session]) -> Result<String> {
    // Allow partial ID or name matching
    let mut matching_sessions: Vec<_> = sessions
        .iter()
        .filter(|s| s.id.starts_with(session_id_or_name))
        .collect();

    // If no ID matches, try matching by name
    if matching_sessions.is_empty() {
        matching_sessions = sessions
            .iter()
            .filter(|s| {
                if let Some(ref name) = s.name {
                    name == session_id_or_name
                        || name.starts_with(session_id_or_name)
                        || name
                            .to_lowercase()
                            .starts_with(&session_id_or_name.to_lowercase())
                } else {
                    false
                }
            })
            .collect();
    }

    match matching_sessions.len() {
        0 => Err(NdsError::SessionNotFound(format!(
            "No session found matching ID or name: {}",
            session_id_or_name
        ))),
        1 => {
            let session = matching_sessions[0];
            SessionManager::kill_session(&session.id)?;
            Ok(session.id.clone())
        }
        _ => {
            let matches: Vec<String> = matching_sessions.iter().map(|s| s.display_name()).collect();
            Err(NdsError::SessionNotFound(format!(
                "Multiple sessions match '{}': {}. Please be more specific",
                session_id_or_name,
                matches.join(", ")
            )))
        }
    }
}

/// Renames a session
pub fn handle_rename_session(session_id_or_name: &str, new_name: &str) -> Result<()> {
    // Allow partial ID or name matching
    let sessions = SessionManager::list_sessions()?;

    // First try to match by ID
    let mut matching_sessions: Vec<_> = sessions
        .iter()
        .filter(|s| s.id.starts_with(session_id_or_name))
        .collect();

    // If no ID matches, try matching by name
    if matching_sessions.is_empty() {
        matching_sessions = sessions
            .iter()
            .filter(|s| {
                if let Some(ref name) = s.name {
                    name == session_id_or_name
                        || name.starts_with(session_id_or_name)
                        || name
                            .to_lowercase()
                            .starts_with(&session_id_or_name.to_lowercase())
                } else {
                    false
                }
            })
            .collect();
    }

    match matching_sessions.len() {
        0 => {
            eprintln!(
                "No session found matching ID or name: {}",
                session_id_or_name
            );
            Err(NdsError::SessionNotFound(session_id_or_name.to_string()))
        }
        1 => {
            let session = matching_sessions[0];
            let old_display_name = session.display_name();
            SessionManager::rename_session(&session.id, new_name)?;
            println!("Renamed session {} to '{}'", old_display_name, new_name);
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

/// Cleans up dead sessions
pub fn handle_clean_sessions() -> Result<()> {
    println!("Cleaning up dead sessions...");
    SessionManager::cleanup_dead_sessions()?;
    println!("Cleanup complete.");
    Ok(())
}
