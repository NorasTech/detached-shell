use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
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

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionHistory {
    pub entries: Vec<HistoryEntry>,
}

impl SessionHistory {
    pub fn new() -> Self {
        SessionHistory {
            entries: Vec::new(),
        }
    }

    pub fn history_file() -> Result<PathBuf> {
        let dir = directories::BaseDirs::new()
            .ok_or_else(|| {
                NdsError::DirectoryCreationError("Could not find home directory".to_string())
            })?
            .home_dir()
            .join(".nds");

        if !dir.exists() {
            fs::create_dir_all(&dir)
                .map_err(|e| NdsError::DirectoryCreationError(e.to_string()))?;
        }

        Ok(dir.join("history.json"))
    }

    pub fn load() -> Result<Self> {
        let path = Self::history_file()?;

        if !path.exists() {
            // Create empty history if file doesn't exist
            let history = Self::new();
            history.save()?;
            return Ok(history);
        }

        let content = fs::read_to_string(&path)?;
        let history: SessionHistory =
            serde_json::from_str(&content).unwrap_or_else(|_| Self::new());

        Ok(history)
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::history_file()?;
        let json = serde_json::to_string_pretty(self)?;
        fs::write(path, json)?;
        Ok(())
    }

    pub fn add_entry(&mut self, entry: HistoryEntry) -> Result<()> {
        self.entries.push(entry);
        self.save()
    }

    pub fn record_session_created(session: &Session) -> Result<()> {
        let mut history = Self::load()?;
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
        history.add_entry(entry)?;
        Ok(())
    }

    pub fn record_session_attached(session: &Session) -> Result<()> {
        let mut history = Self::load()?;
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
        history.add_entry(entry)?;
        Ok(())
    }

    pub fn record_session_detached(session: &Session) -> Result<()> {
        let mut history = Self::load()?;
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
        history.add_entry(entry)?;
        Ok(())
    }

    pub fn record_session_killed(session: &Session) -> Result<()> {
        let mut history = Self::load()?;
        let duration = (Utc::now() - session.created_at).num_seconds();
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
        history.add_entry(entry)?;
        Ok(())
    }

    pub fn record_session_crashed(session: &Session) -> Result<()> {
        let mut history = Self::load()?;
        let duration = (Utc::now() - session.created_at).num_seconds();
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
        history.add_entry(entry)?;
        Ok(())
    }

    pub fn record_session_renamed(
        session: &Session,
        old_name: Option<String>,
        new_name: String,
    ) -> Result<()> {
        let mut history = Self::load()?;
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
        history.add_entry(entry)?;
        Ok(())
    }

    pub fn get_session_history(&self, session_id: &str) -> Vec<&HistoryEntry> {
        self.entries
            .iter()
            .filter(|e| e.session_id.starts_with(session_id))
            .collect()
    }

    pub fn get_all_sessions(&self) -> Vec<String> {
        let mut sessions = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for entry in &self.entries {
            if seen.insert(entry.session_id.clone()) {
                sessions.push(entry.session_id.clone());
            }
        }

        sessions
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
}
