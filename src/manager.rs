use chrono::{DateTime, Local, Timelike, Utc};
use std::fmt;

use crate::error::{NdsError, Result};
use crate::history_v2::SessionHistory;
use crate::pty::PtyProcess;
use crate::session::Session;

pub struct SessionManager;

impl SessionManager {
    pub fn create_session() -> Result<Session> {
        Self::create_session_with_name(None)
    }

    pub fn create_session_with_name(name: Option<String>) -> Result<Session> {
        // Generate session ID
        let session_id = uuid::Uuid::new_v4().to_string()[..8].to_string();

        // Spawn new PTY process with optional name
        let session = PtyProcess::spawn_new_detached_with_name(&session_id, name)?;

        // Record session creation in history
        let _ = SessionHistory::record_session_created(&session);

        Ok(session)
    }

    pub fn attach_session(session_id: &str) -> Result<()> {
        let mut current_session_id = session_id.to_string();

        loop {
            // Load session metadata
            let mut session = Session::load(&current_session_id)?;

            // Validate session is still alive before attempting to attach
            if !Self::validate_session_health(&session) {
                eprintln!("Session {} appears to be dead.", session.id);
                eprintln!("The process (PID {}) is no longer running.", session.pid);
                eprintln!("");
                eprintln!("Would you like to:");
                eprintln!("  1. Clean up the dead session");
                eprintln!("  2. Try to attach anyway (will likely fail)");
                eprintln!("");

                // For now, attempt cleanup and return error
                eprintln!("Cleaning up dead session...");
                let _ = Session::cleanup(&session.id);
                let _ = SessionHistory::record_session_crashed(&session);

                return Err(NdsError::SessionNotFound(format!(
                    "Session {} was dead and has been cleaned up. Create a new session with 'nds new'.",
                    session.id
                )));
            }

            if session.attached {
                // Session appears to be attached, but allow override
                // This handles cases where terminal closed without proper detach
                eprintln!(
                    "Warning: Session {} appears to be already attached.",
                    current_session_id
                );
                eprintln!("Attempting to attach anyway (previous connection may have been lost).");

                // Force mark as detached first to clean up stale state
                session.attached = false;
                session.save()?;
            }

            // Mark as attached
            session.mark_attached()?;

            // Record attach event in history
            let _ = SessionHistory::record_session_attached(&session);

            // Attach to the session with better error handling
            let switch_to = match PtyProcess::attach_to_session(&session) {
                Ok(result) => result,
                Err(e) => {
                    // If we get a broken pipe or connection refused, the session is dead
                    if matches!(e, NdsError::Io(ref io_err) if
                        io_err.kind() == std::io::ErrorKind::BrokenPipe ||
                        io_err.kind() == std::io::ErrorKind::ConnectionRefused)
                    {
                        eprintln!(
                            "\nSession {} is dead (broken pipe/connection refused).",
                            session.id
                        );
                        eprintln!("Cleaning up dead session...");

                        // Mark as detached and cleanup
                        let _ = session.mark_detached();
                        let _ = Session::cleanup(&session.id);
                        let _ = SessionHistory::record_session_crashed(&session);

                        return Err(NdsError::SessionNotFound(format!(
                            "Session {} was dead and has been cleaned up. Create a new session with 'nds new'.",
                            session.id
                        )));
                    }
                    return Err(e);
                }
            };

            // Mark as detached when done
            let _ = session.mark_detached();

            // Record detach event in history
            let _ = SessionHistory::record_session_detached(&session);

            // If switching to another session, continue the loop
            if let Some(new_session_id) = switch_to {
                // Update current session ID and continue
                current_session_id = new_session_id;
            } else {
                // Normal detach
                return Ok(());
            }
        }
    }

    pub fn list_sessions() -> Result<Vec<Session>> {
        Session::list_all()
    }

    pub fn kill_session(session_id: &str) -> Result<()> {
        // Load session for history recording
        if let Ok(session) = Session::load(session_id) {
            // Record kill event in history
            let _ = SessionHistory::record_session_killed(&session);
        }

        PtyProcess::kill_session(session_id)
    }

    pub fn get_session(session_id: &str) -> Result<Session> {
        Session::load(session_id)
    }

    pub fn rename_session(session_id: &str, new_name: &str) -> Result<()> {
        let mut session = Session::load(session_id)?;
        let old_name = session.name.clone();

        session.name = if new_name.trim().is_empty() {
            None
        } else {
            Some(new_name.to_string())
        };

        // Record rename event in history
        if let Some(ref name) = session.name {
            let _ = SessionHistory::record_session_renamed(&session, old_name, name.clone());
        }

        session.save()
    }

    pub fn cleanup_dead_sessions() -> Result<()> {
        let sessions = Session::list_all()?;
        let mut cleaned = 0;

        for session in sessions {
            if !Self::validate_session_health(&session) {
                // Record crash event in history before cleanup
                let _ = SessionHistory::record_session_crashed(&session);
                Session::cleanup(&session.id)?;
                cleaned += 1;
                println!("Cleaned up dead session: {}", session.display_name());
            }
        }

        if cleaned > 0 {
            println!("Cleaned up {} dead session(s)", cleaned);
        } else {
            println!("No dead sessions found");
        }

        Ok(())
    }

    /// Validate that a session is healthy and can be attached to
    fn validate_session_health(session: &Session) -> bool {
        // First check if the process is alive
        if !Session::is_process_alive(session.pid) {
            return false;
        }

        // Check if the socket file exists
        if !session.socket_path.exists() {
            return false;
        }

        // Try to connect to the socket to verify it's responsive
        // We use a very short timeout to avoid hanging
        use std::os::unix::net::UnixStream;
        use std::time::Duration;

        match UnixStream::connect(&session.socket_path) {
            Ok(socket) => {
                // Set a short timeout for the test
                let _ = socket.set_read_timeout(Some(Duration::from_millis(100)));
                let _ = socket.set_write_timeout(Some(Duration::from_millis(100)));

                // Socket is connectable, session is likely healthy
                drop(socket);
                true
            }
            Err(_) => {
                // Can't connect to socket, session is likely dead
                false
            }
        }
    }
}

// Helper for pretty-printing sessions
pub struct SessionDisplay<'a> {
    pub session: &'a Session,
    pub is_current: bool,
}

impl<'a> SessionDisplay<'a> {
    pub fn new(session: &'a Session) -> Self {
        SessionDisplay {
            session,
            is_current: false,
        }
    }

    pub fn with_current(session: &'a Session, is_current: bool) -> Self {
        SessionDisplay {
            session,
            is_current,
        }
    }

    fn format_duration(&self) -> String {
        let now = Utc::now();
        let duration = now - self.session.created_at;

        if duration.num_days() > 0 {
            format!("{}d", duration.num_days())
        } else if duration.num_hours() > 0 {
            format!("{}h", duration.num_hours())
        } else if duration.num_minutes() > 0 {
            format!("{}m", duration.num_minutes())
        } else {
            format!("{}s", duration.num_seconds())
        }
    }

    fn format_time(&self) -> String {
        let now = Local::now();
        let local_time: DateTime<Local> = self.session.created_at.into();
        let duration = now.signed_duration_since(local_time);

        if duration.num_days() > 0 {
            format!(
                "{}d, {:02}:{:02}",
                duration.num_days(),
                local_time.hour(),
                local_time.minute()
            )
        } else {
            local_time.format("%H:%M:%S").to_string()
        }
    }
}

impl<'a> fmt::Display for SessionDisplay<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Check if we're inside an nds session - use simpler output if so
        let in_nds_session = std::env::var("NDS_SESSION_ID").is_ok();

        if in_nds_session {
            // Simple format for PTY sessions
            let client_count = self.session.get_client_count();
            let status = if self.is_current {
                "CURRENT"
            } else if client_count > 0 {
                "attached"
            } else {
                "detached"
            };

            write!(
                f,
                "{} [{}] - PID {} - {}",
                self.session.display_name(),
                &self.session.id[..8],
                self.session.pid,
                status
            )
        } else {
            // Full formatted output for normal terminal
            // Get client count
            let client_count = self.session.get_client_count();

            // Status icon and color
            let (icon, status_text) = if self.is_current {
                (
                    "★",
                    format!(
                        "CURRENT · {} client{}",
                        client_count,
                        if client_count == 1 { "" } else { "s" }
                    ),
                )
            } else if client_count > 0 {
                (
                    "●",
                    format!(
                        "{} client{}",
                        client_count,
                        if client_count == 1 { "" } else { "s" }
                    ),
                )
            } else {
                ("○", "detached".to_string())
            };

            // Truncate working dir if too long
            let mut working_dir = self.session.working_dir.clone();
            if working_dir.len() > 30 {
                // Show last 27 chars with ellipsis
                working_dir = format!(
                    "...{}",
                    &self.session.working_dir[self.session.working_dir.len() - 27..]
                );
            }

            // Format with sleek layout including all info
            write!(
                f,
                " {} {:<25} │ PID {:<6} │ {:<8} │ {:<8} │ {:<30} │ {}",
                icon,
                self.session.display_name(),
                self.session.pid,
                self.format_duration(),
                self.format_time(),
                working_dir,
                status_text
            )
        }
    }
}

pub struct SessionTable {
    sessions: Vec<Session>,
    current_session_id: Option<String>,
}

impl SessionTable {
    pub fn new(sessions: Vec<Session>) -> Self {
        // Check if we're currently attached to a session
        let current_session_id = std::env::var("NDS_SESSION_ID").ok();
        SessionTable {
            sessions,
            current_session_id,
        }
    }

    pub fn print(&self) {
        if self.sessions.is_empty() {
            println!("No active sessions");
            return;
        }

        // Check if we're inside an nds session - use simpler output to avoid PTY corruption
        let in_nds_session = std::env::var("NDS_SESSION_ID").is_ok();

        if in_nds_session {
            // Simple output for use within PTY sessions to avoid display corruption
            self.print_simple();
        } else {
            // Full formatted output for normal terminal
            self.print_formatted();
        }
    }

    fn print_simple(&self) {
        // Simple, PTY-friendly output without complex formatting
        println!("Active sessions:");
        println!();

        for session in &self.sessions {
            let is_current = self.current_session_id.as_ref() == Some(&session.id);
            let client_count = session.get_client_count();

            // Simple format without complex columns or ANSI codes
            let status = if is_current {
                "[CURRENT]"
            } else if client_count > 0 {
                &format!("[{} clients]", client_count)
            } else {
                "[detached]"
            };

            println!(
                "  {} {} - PID {} {}",
                session.display_name(),
                &session.id[..8],
                session.pid,
                status
            );
        }

        println!();
        println!("Total: {} sessions", self.sessions.len());
    }

    fn print_formatted(&self) {
        // Sleek header
        println!("SESSIONS\n");

        // Print sessions with full formatting
        for session in &self.sessions {
            let is_current = self.current_session_id.as_ref() == Some(&session.id);
            println!("{}", SessionDisplay::with_current(session, is_current));
        }

        // Footer
        println!("\n{} sessions", self.sessions.len());
    }
}
