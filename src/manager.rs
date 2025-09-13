use chrono::{DateTime, Local, Timelike, Utc};
use std::fmt;

use crate::error::Result;
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

            // Attach to the session
            let switch_to = PtyProcess::attach_to_session(&session)?;

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

        for session in sessions {
            if !Session::is_process_alive(session.pid) {
                // Record crash event in history before cleanup
                let _ = SessionHistory::record_session_crashed(&session);
                Session::cleanup(&session.id)?;
            }
        }

        Ok(())
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

        // Sleek header
        println!("SESSIONS\n");

        // Print sessions with cleaner format
        for session in &self.sessions {
            let is_current = self.current_session_id.as_ref() == Some(&session.id);
            println!("{}", SessionDisplay::with_current(session, is_current));
        }

        // Footer
        println!("\n{} sessions", self.sessions.len());
    }
}
