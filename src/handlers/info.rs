use chrono::{DateTime, Local};
use detached_shell::{
    NdsError, Result, Session, SessionEvent, SessionHistory, SessionManager, SessionTable,
};
use std::collections::HashSet;

/// Lists all active sessions with optional interactive mode
pub fn handle_list_sessions(interactive: bool) -> Result<()> {
    if interactive {
        // Interactive mode - let user select and attach
        use detached_shell::interactive::InteractivePicker;

        match InteractivePicker::new() {
            Ok(mut picker) => {
                match picker.run()? {
                    Some(session_id) => {
                        // User selected a session, attach to it
                        println!("Attaching to session {}...", session_id);
                        crate::handlers::session::handle_attach_session(&session_id)?;
                    }
                    None => {
                        // User quit without selecting
                        println!("No session selected.");
                    }
                }
            }
            Err(e) => {
                eprintln!("Error: {}", e);
                if matches!(e, NdsError::SessionNotFound(_)) {
                    println!("No active sessions found.");
                }
            }
        }
    } else {
        // Normal list mode
        let sessions = SessionManager::list_sessions()?;
        let table = SessionTable::new(sessions);
        table.print();
    }
    Ok(())
}

/// Shows detailed information about a specific session
pub fn handle_session_info(session_id_or_name: &str) -> Result<()> {
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
                    name == session_id_or_name || 
                    name.starts_with(session_id_or_name) ||
                    name.to_lowercase().starts_with(&session_id_or_name.to_lowercase())
                } else {
                    false
                }
            })
            .collect();
    }

    match matching_sessions.len() {
        0 => {
            eprintln!("No session found matching ID or name: {}", session_id_or_name);
            Err(NdsError::SessionNotFound(session_id_or_name.to_string()))
        }
        1 => {
            let session = matching_sessions[0];
            let client_count = session.get_client_count();
            
            println!("Session ID: {}", session.id);
            if let Some(ref name) = session.name {
                println!("Session Name: {}", name);
            }
            println!("PID: {}", session.pid);
            println!("Created: {}", session.created_at);
            println!("Socket: {}", session.socket_path.display());
            println!("Shell: {}", session.shell);
            println!("Working Directory: {}", session.working_dir);
            println!(
                "Status: {}",
                if client_count > 0 {
                    format!("Attached ({} client(s))", client_count)
                } else {
                    "Detached".to_string()
                }
            );
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

/// Shows session history with various filtering options
pub fn handle_session_history(
    session_id_or_name: Option<String>,
    all: bool,
    limit: usize,
) -> Result<()> {
    // Migrate old format if needed
    let _ = SessionHistory::migrate_from_single_file();

    if let Some(ref id_or_name) = session_id_or_name {
        // Show history for specific session
        handle_specific_session_history(id_or_name, limit)
    } else {
        // Show all history or active sessions only
        handle_general_session_history(all, limit)
    }
}

/// Helper function to handle history for a specific session
fn handle_specific_session_history(id_or_name: &str, limit: usize) -> Result<()> {
    // First try to resolve session name to ID
    let sessions = SessionManager::list_sessions()?;
    let resolved_id = resolve_session_id(id_or_name, &sessions)?;

    // Show history for specific session
    let entries = SessionHistory::get_session_history(&resolved_id)?;

    if entries.is_empty() {
        println!("No history found for session: {}", resolved_id);
        return Ok(());
    }

    println!("History for session {}:", resolved_id);
    println!("{:-<80}", "");

    for entry in entries.iter().take(limit) {
        let local_time: DateTime<Local> = entry.timestamp.into();
        let time_str = local_time.format("%Y-%m-%d %H:%M:%S").to_string();

        let event_str = format_session_event(&entry.event, entry.duration_seconds);

        println!(
            "{} | {:<20} | PID: {} | {}",
            time_str, event_str, entry.pid, entry.working_dir
        );
    }

    Ok(())
}

/// Helper function to handle general session history
fn handle_general_session_history(all: bool, limit: usize) -> Result<()> {
    let entries = SessionHistory::load_all_history(all, Some(limit))?;

    let filtered_entries: Vec<_> = if !all {
        // Filter to show only entries for currently active sessions
        let active_sessions = SessionManager::list_sessions()?;
        let active_ids: HashSet<_> = active_sessions.iter().map(|s| s.id.clone()).collect();

        entries
            .into_iter()
            .filter(|e| active_ids.contains(&e.session_id))
            .collect()
    } else {
        entries
    };

    if filtered_entries.is_empty() {
        if all {
            println!("No session history found.");
        } else {
            println!("No history for active sessions. Use --all to see all history.");
        }
        return Ok(());
    }

    print_history_table(&filtered_entries, all);
    Ok(())
}

/// Helper function to resolve session name to ID
fn resolve_session_id(id_or_name: &str, sessions: &[Session]) -> Result<String> {
    if sessions.iter().any(|s| s.id == id_or_name) {
        // It's already a session ID
        return Ok(id_or_name.to_string());
    }

    // Try to find by name (case-insensitive partial matching)
    let matches: Vec<&Session> = sessions
        .iter()
        .filter(|s| {
            if let Some(ref name) = s.name {
                name.to_lowercase().contains(&id_or_name.to_lowercase())
            } else {
                false
            }
        })
        .collect();

    match matches.len() {
        0 => {
            println!("No session found with ID or name matching: {}", id_or_name);
            Err(NdsError::SessionNotFound(id_or_name.to_string()))
        }
        1 => Ok(matches[0].id.clone()),
        _ => {
            println!(
                "Multiple sessions match '{}'. Please be more specific:",
                id_or_name
            );
            for session in matches {
                println!("  {} [{}]", session.display_name(), session.id);
            }
            Err(NdsError::InvalidSessionId(id_or_name.to_string()))
        }
    }
}

/// Helper function to format session events
fn format_session_event(event: &SessionEvent, duration: Option<i64>) -> String {
    match event {
        SessionEvent::Created => "Created".to_string(),
        SessionEvent::Attached => "Attached".to_string(),
        SessionEvent::Detached => "Detached".to_string(),
        SessionEvent::Killed => format!(
            "Killed (duration: {})",
            duration
                .map(|d| SessionHistory::format_duration(d))
                .unwrap_or_else(|| "unknown".to_string())
        ),
        SessionEvent::Crashed => format!(
            "Crashed (duration: {})",
            duration
                .map(|d| SessionHistory::format_duration(d))
                .unwrap_or_else(|| "unknown".to_string())
        ),
        SessionEvent::Renamed { from, to } => match from {
            Some(old) => format!("Renamed from '{}' to '{}'", old, to),
            None => format!("Named as '{}'", to),
        },
    }
}

/// Helper function to print history table
fn print_history_table(entries: &[detached_shell::HistoryEntry], all: bool) {
    println!(
        "{} Session History (showing {} entries)",
        if all { "All" } else { "Active" },
        entries.len()
    );
    println!("{:-<100}", "");
    println!(
        "{:<20} {:<12} {:<20} {:<8} {:<10} {:<30}",
        "Time", "Session", "Event", "PID", "Duration", "Working Dir"
    );
    println!("{:-<100}", "");

    for entry in entries {
        let local_time: DateTime<Local> = entry.timestamp.into();
        let time_str = local_time.format("%Y-%m-%d %H:%M:%S").to_string();

        let session_display = if let Some(ref name) = entry.session_name {
            format!(
                "{} [{}]",
                name,
                &entry.session_id[..8.min(entry.session_id.len())]
            )
        } else {
            entry.session_id[..8.min(entry.session_id.len())].to_string()
        };

        let (event_str, duration_str) = match &entry.event {
            SessionEvent::Created => ("Created".to_string(), "-".to_string()),
            SessionEvent::Attached => ("Attached".to_string(), "-".to_string()),
            SessionEvent::Detached => ("Detached".to_string(), "-".to_string()),
            SessionEvent::Killed => (
                "Killed".to_string(),
                entry
                    .duration_seconds
                    .map(|d| SessionHistory::format_duration(d))
                    .unwrap_or_else(|| "-".to_string()),
            ),
            SessionEvent::Crashed => (
                "Crashed".to_string(),
                entry
                    .duration_seconds
                    .map(|d| SessionHistory::format_duration(d))
                    .unwrap_or_else(|| "-".to_string()),
            ),
            SessionEvent::Renamed { .. } => ("Renamed".to_string(), "-".to_string()),
        };

        let working_dir = if entry.working_dir.len() > 30 {
            format!("...{}", &entry.working_dir[entry.working_dir.len() - 27..])
        } else {
            entry.working_dir.clone()
        };

        println!(
            "{:<20} {:<12} {:<20} {:<8} {:<10} {:<30}",
            time_str, session_display, event_str, entry.pid, duration_str, working_dir
        );
    }
}