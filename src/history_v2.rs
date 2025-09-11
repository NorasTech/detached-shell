use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use crate::error::{NdsError, Result};
use crate::session::Session;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SessionEvent {
    Created,
    Attached,
    Detached,
    Killed,
    Crashed,
    Renamed { from: Option<String>, to: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub session_id: String,
    pub session_name: Option<String>,
    pub event: SessionEvent,
    pub timestamp: DateTime<Utc>,
    pub pid: i32,
    pub shell: String,
    pub working_dir: String,
    pub duration_seconds: Option<i64>, // For Killed/Crashed events
}

// Individual session history stored in separate files
#[derive(Debug, Serialize, Deserialize)]
pub struct SessionHistoryFile {
    pub session_id: String,
    pub created_at: DateTime<Utc>,
    pub entries: Vec<HistoryEntry>,
}

// Main history manager
pub struct SessionHistory;

impl SessionHistory {
    // Directory structure: ~/.nds/history/active/ and ~/.nds/history/archived/
    pub fn history_dir() -> Result<PathBuf> {
        let dir = if let Ok(nds_home) = std::env::var("NDS_HOME") {
            PathBuf::from(nds_home).join("history")
        } else {
            directories::BaseDirs::new()
                .ok_or_else(|| {
                    NdsError::DirectoryCreationError("Could not find home directory".to_string())
                })?
                .home_dir()
                .join(".nds")
                .join("history")
        };

        if !dir.exists() {
            fs::create_dir_all(&dir)
                .map_err(|e| NdsError::DirectoryCreationError(e.to_string()))?;
        }

        Ok(dir)
    }

    pub fn active_history_dir() -> Result<PathBuf> {
        let dir = Self::history_dir()?.join("active");
        if !dir.exists() {
            fs::create_dir_all(&dir)
                .map_err(|e| NdsError::DirectoryCreationError(e.to_string()))?;
        }
        Ok(dir)
    }

    pub fn archived_history_dir() -> Result<PathBuf> {
        let dir = Self::history_dir()?.join("archived");
        if !dir.exists() {
            fs::create_dir_all(&dir)
                .map_err(|e| NdsError::DirectoryCreationError(e.to_string()))?;
        }
        Ok(dir)
    }

    // Get history file path for a session
    fn session_history_path(session_id: &str, archived: bool) -> Result<PathBuf> {
        let dir = if archived {
            Self::archived_history_dir()?
        } else {
            Self::active_history_dir()?
        };
        Ok(dir.join(format!("{}.json", session_id)))
    }

    // Load history for a specific session
    pub fn load_session_history(session_id: &str) -> Result<SessionHistoryFile> {
        // Try active first, then archived
        let active_path = Self::session_history_path(session_id, false)?;
        let archived_path = Self::session_history_path(session_id, true)?;

        let path = if active_path.exists() {
            active_path
        } else if archived_path.exists() {
            archived_path
        } else {
            // Create new history file for this session
            let history = SessionHistoryFile {
                session_id: session_id.to_string(),
                created_at: Utc::now(),
                entries: Vec::new(),
            };
            let json = serde_json::to_string_pretty(&history)?;
            fs::write(&active_path, json)?;
            return Ok(history);
        };

        let content = fs::read_to_string(&path)?;
        let history: SessionHistoryFile = serde_json::from_str(&content)?;
        Ok(history)
    }

    // Save history for a specific session
    fn save_session_history(history: &SessionHistoryFile, archived: bool) -> Result<()> {
        let path = Self::session_history_path(&history.session_id, archived)?;
        let json = serde_json::to_string_pretty(history)?;
        fs::write(path, json)?;
        Ok(())
    }

    // Add an entry to a session's history
    fn add_entry_to_session(session_id: &str, entry: HistoryEntry) -> Result<()> {
        let mut history = Self::load_session_history(session_id)?;
        history.entries.push(entry);

        // Determine if this should be archived (session ended)
        let should_archive = history
            .entries
            .iter()
            .any(|e| matches!(e.event, SessionEvent::Killed | SessionEvent::Crashed));

        Self::save_session_history(&history, should_archive)?;

        // If archived, remove from active directory
        if should_archive {
            let active_path = Self::session_history_path(session_id, false)?;
            if active_path.exists() {
                let _ = fs::remove_file(active_path);
            }
        }

        Ok(())
    }

    // Record session events
    pub fn record_session_created(session: &Session) -> Result<()> {
        let entry = HistoryEntry {
            session_id: session.id.clone(),
            session_name: session.name.clone(),
            event: SessionEvent::Created,
            timestamp: session.created_at,
            pid: session.pid,
            shell: session.shell.clone(),
            working_dir: session.working_dir.clone(),
            duration_seconds: None,
        };
        Self::add_entry_to_session(&session.id, entry)
    }

    pub fn record_session_attached(session: &Session) -> Result<()> {
        let entry = HistoryEntry {
            session_id: session.id.clone(),
            session_name: session.name.clone(),
            event: SessionEvent::Attached,
            timestamp: Utc::now(),
            pid: session.pid,
            shell: session.shell.clone(),
            working_dir: session.working_dir.clone(),
            duration_seconds: None,
        };
        Self::add_entry_to_session(&session.id, entry)
    }

    pub fn record_session_detached(session: &Session) -> Result<()> {
        let entry = HistoryEntry {
            session_id: session.id.clone(),
            session_name: session.name.clone(),
            event: SessionEvent::Detached,
            timestamp: Utc::now(),
            pid: session.pid,
            shell: session.shell.clone(),
            working_dir: session.working_dir.clone(),
            duration_seconds: None,
        };
        Self::add_entry_to_session(&session.id, entry)
    }

    pub fn record_session_killed(session: &Session) -> Result<()> {
        let history = Self::load_session_history(&session.id)?;
        let duration = if let Some(first_entry) = history.entries.first() {
            (Utc::now() - first_entry.timestamp).num_seconds()
        } else {
            (Utc::now() - session.created_at).num_seconds()
        };

        let entry = HistoryEntry {
            session_id: session.id.clone(),
            session_name: session.name.clone(),
            event: SessionEvent::Killed,
            timestamp: Utc::now(),
            pid: session.pid,
            shell: session.shell.clone(),
            working_dir: session.working_dir.clone(),
            duration_seconds: Some(duration),
        };
        Self::add_entry_to_session(&session.id, entry)
    }

    pub fn record_session_crashed(session: &Session) -> Result<()> {
        let history = Self::load_session_history(&session.id)?;
        let duration = if let Some(first_entry) = history.entries.first() {
            (Utc::now() - first_entry.timestamp).num_seconds()
        } else {
            (Utc::now() - session.created_at).num_seconds()
        };

        let entry = HistoryEntry {
            session_id: session.id.clone(),
            session_name: session.name.clone(),
            event: SessionEvent::Crashed,
            timestamp: Utc::now(),
            pid: session.pid,
            shell: session.shell.clone(),
            working_dir: session.working_dir.clone(),
            duration_seconds: Some(duration),
        };
        Self::add_entry_to_session(&session.id, entry)
    }

    pub fn record_session_renamed(
        session: &Session,
        old_name: Option<String>,
        new_name: String,
    ) -> Result<()> {
        let entry = HistoryEntry {
            session_id: session.id.clone(),
            session_name: Some(new_name.clone()),
            event: SessionEvent::Renamed {
                from: old_name,
                to: new_name,
            },
            timestamp: Utc::now(),
            pid: session.pid,
            shell: session.shell.clone(),
            working_dir: session.working_dir.clone(),
            duration_seconds: None,
        };
        Self::add_entry_to_session(&session.id, entry)
    }

    // Get all history entries (from all sessions)
    pub fn load_all_history(
        include_archived: bool,
        limit: Option<usize>,
    ) -> Result<Vec<HistoryEntry>> {
        let mut all_entries = Vec::new();

        // Load from active sessions
        let active_dir = Self::active_history_dir()?;
        if active_dir.exists() {
            for entry in fs::read_dir(active_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("json") {
                    if let Ok(content) = fs::read_to_string(&path) {
                        if let Ok(history) = serde_json::from_str::<SessionHistoryFile>(&content) {
                            all_entries.extend(history.entries);
                        }
                    }
                }
            }
        }

        // Load from archived sessions if requested
        if include_archived {
            let archived_dir = Self::archived_history_dir()?;
            if archived_dir.exists() {
                for entry in fs::read_dir(archived_dir)? {
                    let entry = entry?;
                    let path = entry.path();
                    if path.extension().and_then(|s| s.to_str()) == Some("json") {
                        if let Ok(content) = fs::read_to_string(&path) {
                            if let Ok(history) =
                                serde_json::from_str::<SessionHistoryFile>(&content)
                            {
                                all_entries.extend(history.entries);
                            }
                        }
                    }
                }
            }
        }

        // Sort by timestamp (newest first)
        all_entries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

        // Apply limit if specified
        if let Some(limit) = limit {
            all_entries.truncate(limit);
        }

        Ok(all_entries)
    }

    // Get history for a specific session
    pub fn get_session_history(session_id: &str) -> Result<Vec<HistoryEntry>> {
        let history = Self::load_session_history(session_id)?;
        Ok(history.entries)
    }

    // Clean up old archived history (older than specified days)
    pub fn cleanup_old_history(days_to_keep: i64) -> Result<usize> {
        let archived_dir = Self::archived_history_dir()?;
        let cutoff = Utc::now() - Duration::days(days_to_keep);
        let mut removed_count = 0;

        if archived_dir.exists() {
            for entry in fs::read_dir(archived_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("json") {
                    if let Ok(content) = fs::read_to_string(&path) {
                        if let Ok(history) = serde_json::from_str::<SessionHistoryFile>(&content) {
                            // Check if all entries are older than cutoff
                            if history.entries.iter().all(|e| e.timestamp < cutoff) {
                                fs::remove_file(&path)?;
                                removed_count += 1;
                            }
                        }
                    }
                }
            }
        }

        Ok(removed_count)
    }

    pub fn format_duration(seconds: i64) -> String {
        let hours = seconds / 3600;
        let minutes = (seconds % 3600) / 60;
        let secs = seconds % 60;

        if hours > 0 {
            format!("{}h {}m {}s", hours, minutes, secs)
        } else if minutes > 0 {
            format!("{}m {}s", minutes, secs)
        } else {
            format!("{}s", secs)
        }
    }

    // Migrate from old single-file format to new per-session format
    pub fn migrate_from_single_file() -> Result<()> {
        let old_file = directories::BaseDirs::new()
            .ok_or_else(|| {
                NdsError::DirectoryCreationError("Could not find home directory".to_string())
            })?
            .home_dir()
            .join(".nds")
            .join("history.json");

        if !old_file.exists() {
            return Ok(()); // Nothing to migrate
        }

        // Read old format
        let content = fs::read_to_string(&old_file)?;
        if let Ok(old_history) = serde_json::from_str::<crate::history::SessionHistory>(&content) {
            // Group entries by session ID
            let mut sessions: HashMap<String, Vec<HistoryEntry>> = HashMap::new();

            for old_entry in old_history.entries {
                let entry = HistoryEntry {
                    session_id: old_entry.session_id.clone(),
                    session_name: old_entry.session_name,
                    event: match old_entry.event {
                        crate::history::SessionEvent::Created => SessionEvent::Created,
                        crate::history::SessionEvent::Attached => SessionEvent::Attached,
                        crate::history::SessionEvent::Detached => SessionEvent::Detached,
                        crate::history::SessionEvent::Killed => SessionEvent::Killed,
                        crate::history::SessionEvent::Crashed => SessionEvent::Crashed,
                        crate::history::SessionEvent::Renamed { from, to } => {
                            SessionEvent::Renamed { from, to }
                        }
                    },
                    timestamp: old_entry.timestamp,
                    pid: old_entry.pid,
                    shell: old_entry.shell,
                    working_dir: old_entry.working_dir,
                    duration_seconds: old_entry.duration_seconds,
                };

                sessions
                    .entry(old_entry.session_id.clone())
                    .or_insert_with(Vec::new)
                    .push(entry);
            }

            // Save each session to its own file
            for (session_id, entries) in sessions {
                let is_terminated = entries
                    .iter()
                    .any(|e| matches!(e.event, SessionEvent::Killed | SessionEvent::Crashed));

                let created_at = entries
                    .first()
                    .map(|e| e.timestamp)
                    .unwrap_or_else(Utc::now);

                let history_file = SessionHistoryFile {
                    session_id: session_id.clone(),
                    created_at,
                    entries,
                };

                Self::save_session_history(&history_file, is_terminated)?;
            }

            // Rename old file to .backup
            let backup_path = old_file.with_extension("json.backup");
            fs::rename(old_file, backup_path)?;
        }

        Ok(())
    }
}
