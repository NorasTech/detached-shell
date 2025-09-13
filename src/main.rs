use clap::{Parser, Subcommand};
use detached_shell::{
    NdsError, Result, Session, SessionEvent, SessionHistory, SessionManager, SessionTable,
};

#[derive(Parser)]
#[command(name = "nds")]
#[command(about = "Noras Detached Shell - A minimalist shell session manager", long_about = None)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a new detached shell session
    New {
        /// Optional session name
        name: Option<String>,
        /// Don't attach to the new session (default is to attach)
        #[arg(long = "no-attach")]
        no_attach: bool,
    },

    /// List all active sessions
    #[command(aliases = &["ls", "l"])]
    List {
        /// Interactive mode - select session to attach
        #[arg(short, long)]
        interactive: bool,
    },

    /// Attach to an existing session
    #[command(aliases = &["a", "at"])]
    Attach {
        /// Session ID or name to attach to (supports partial matching)
        id: String,
    },

    /// Kill one or more sessions
    #[command(aliases = &["k"])]
    Kill {
        /// Session IDs or names to kill (supports partial matching)
        ids: Vec<String>,
    },

    /// Show information about a specific session
    #[command(aliases = &["i"])]
    Info {
        /// Session ID or name to get info about (supports partial matching)
        id: String,
    },

    /// Rename a session
    #[command(aliases = &["rn"])]
    Rename {
        /// Session ID or name to rename (supports partial matching)
        id: String,
        /// New name for the session
        new_name: String,
    },

    /// Clean up dead sessions
    Clean,

    /// Show session history
    #[command(aliases = &["h", "hist"])]
    History {
        /// Show history for a specific session ID or name (supports partial matching)
        #[arg(short, long)]
        session: Option<String>,

        /// Show all history entries (including crashed/killed sessions)
        #[arg(short, long)]
        all: bool,

        /// Limit number of entries to show
        #[arg(short, long, default_value = "50")]
        limit: usize,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::New { name, no_attach }) => {
            handle_new_session(name, !no_attach)?;
        }
        Some(Commands::List { interactive }) => {
            handle_list_sessions(interactive)?;
        }
        Some(Commands::Attach { id }) => {
            handle_attach_session(&id)?;
        }
        Some(Commands::Kill { ids }) => {
            handle_kill_sessions(&ids)?;
        }
        Some(Commands::Info { id }) => {
            handle_session_info(&id)?;
        }
        Some(Commands::Rename { id, new_name }) => {
            handle_rename_session(&id, &new_name)?;
        }
        Some(Commands::Clean) => {
            handle_clean_sessions()?;
        }
        Some(Commands::History {
            session,
            all,
            limit,
        }) => {
            handle_session_history(session, all, limit)?;
        }
        None => {
            // Default action: interactive session picker
            handle_list_sessions(true)?;
        }
    }

    Ok(())
}

fn handle_new_session(name: Option<String>, attach: bool) -> Result<()> {
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
                std::thread::sleep(std::time::Duration::from_millis(100));
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

fn handle_list_sessions(interactive: bool) -> Result<()> {
    if interactive {
        // Interactive mode - let user select and attach
        use detached_shell::interactive::InteractivePicker;

        match InteractivePicker::new() {
            Ok(mut picker) => {
                match picker.run()? {
                    Some(session_id) => {
                        // User selected a session, attach to it
                        println!("Attaching to session {}...", session_id);
                        handle_attach_session(&session_id)?;
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

fn handle_attach_session(session_id_or_name: &str) -> Result<()> {
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

fn handle_kill_sessions(session_ids: &[String]) -> Result<()> {
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
            let matches: Vec<String> = matching_sessions
                .iter()
                .map(|s| s.display_name())
                .collect();
            Err(NdsError::SessionNotFound(format!(
                "Multiple sessions match '{}': {}. Please be more specific",
                session_id_or_name,
                matches.join(", ")
            )))
        }
    }
}

fn handle_session_info(session_id_or_name: &str) -> Result<()> {
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

fn handle_rename_session(session_id_or_name: &str, new_name: &str) -> Result<()> {
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

fn handle_clean_sessions() -> Result<()> {
    println!("Cleaning up dead sessions...");
    SessionManager::cleanup_dead_sessions()?;
    println!("Cleanup complete.");
    Ok(())
}

fn handle_session_history(session_id_or_name: Option<String>, all: bool, limit: usize) -> Result<()> {
    use chrono::{DateTime, Local};

    // Migrate old format if needed
    let _ = SessionHistory::migrate_from_single_file();

    if let Some(ref id_or_name) = session_id_or_name {
        // First try to resolve session name to ID
        let sessions = SessionManager::list_sessions()?;
        let resolved_id = if sessions.iter().any(|s| s.id == *id_or_name) {
            // It's already a session ID
            id_or_name.clone()
        } else {
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
                    return Ok(());
                }
                1 => matches[0].id.clone(),
                _ => {
                    println!("Multiple sessions match '{}'. Please be more specific:", id_or_name);
                    for session in matches {
                        println!("  {} [{}]", session.display_name(), session.id);
                    }
                    return Ok(());
                }
            }
        };

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

            let event_str = match &entry.event {
                SessionEvent::Created => "Created".to_string(),
                SessionEvent::Attached => "Attached".to_string(),
                SessionEvent::Detached => "Detached".to_string(),
                SessionEvent::Killed => format!(
                    "Killed (duration: {})",
                    entry
                        .duration_seconds
                        .map(SessionHistory::format_duration)
                        .unwrap_or_else(|| "unknown".to_string())
                ),
                SessionEvent::Crashed => format!(
                    "Crashed (duration: {})",
                    entry
                        .duration_seconds
                        .map(SessionHistory::format_duration)
                        .unwrap_or_else(|| "unknown".to_string())
                ),
                SessionEvent::Renamed { from, to } => match from {
                    Some(old) => format!("Renamed from '{}' to '{}'", old, to),
                    None => format!("Named as '{}'", to),
                },
            };

            println!(
                "{} | {:<20} | PID: {} | {}",
                time_str, event_str, entry.pid, entry.working_dir
            );
        }
    } else {
        // Show all history or active sessions only
        let entries = SessionHistory::load_all_history(all, Some(limit))?;

        let filtered_entries: Vec<_> = if !all {
            // Filter to show only entries for currently active sessions
            let active_sessions = SessionManager::list_sessions()?;
            let active_ids: std::collections::HashSet<_> =
                active_sessions.iter().map(|s| s.id.clone()).collect();

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

        println!(
            "{} Session History (showing {} entries)",
            if all { "All" } else { "Active" },
            filtered_entries.len()
        );
        println!("{:-<100}", "");
        println!(
            "{:<20} {:<12} {:<20} {:<8} {:<10} {:<30}",
            "Time", "Session", "Event", "PID", "Duration", "Working Dir"
        );
        println!("{:-<100}", "");

        for entry in filtered_entries {
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
                        .map(SessionHistory::format_duration)
                        .unwrap_or_else(|| "-".to_string()),
                ),
                SessionEvent::Crashed => (
                    "Crashed".to_string(),
                    entry
                        .duration_seconds
                        .map(SessionHistory::format_duration)
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

    Ok(())
}
