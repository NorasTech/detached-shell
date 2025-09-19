use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use crate::error::{NdsError, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub name: Option<String>,
    pub pid: i32,
    pub created_at: DateTime<Utc>,
    pub attached: bool,
    pub socket_path: PathBuf,
    pub shell: String,
    pub working_dir: String,
}

impl Session {
    pub fn new(id: String, pid: i32, socket_path: PathBuf) -> Self {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        let working_dir = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| "/".to_string());

        Session {
            id,
            name: None,
            pid,
            created_at: Utc::now(),
            attached: false, // Sessions start detached
            socket_path,
            shell,
            working_dir,
        }
    }

    pub fn with_name(id: String, name: Option<String>, pid: i32, socket_path: PathBuf) -> Self {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        let working_dir = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| "/".to_string());

        Session {
            id,
            name,
            pid,
            created_at: Utc::now(),
            attached: false, // Sessions start detached
            socket_path,
            shell,
            working_dir,
        }
    }

    pub fn display_name(&self) -> String {
        match &self.name {
            Some(name) => format!("{} [{}]", name, self.id),
            None => self.id.clone(),
        }
    }

    pub fn session_dir() -> Result<PathBuf> {
        let dir = if let Ok(nds_home) = std::env::var("NDS_HOME") {
            PathBuf::from(nds_home).join("sessions")
        } else {
            directories::BaseDirs::new()
                .ok_or_else(|| {
                    NdsError::DirectoryCreationError("Could not find home directory".to_string())
                })?
                .home_dir()
                .join(".nds")
                .join("sessions")
        };

        if !dir.exists() {
            fs::create_dir_all(&dir)
                .map_err(|e| NdsError::DirectoryCreationError(e.to_string()))?;
        }

        Ok(dir)
    }

    pub fn socket_dir() -> Result<PathBuf> {
        let dir = if let Ok(nds_home) = std::env::var("NDS_HOME") {
            PathBuf::from(nds_home).join("sockets")
        } else {
            directories::BaseDirs::new()
                .ok_or_else(|| {
                    NdsError::DirectoryCreationError("Could not find home directory".to_string())
                })?
                .home_dir()
                .join(".nds")
                .join("sockets")
        };

        if !dir.exists() {
            fs::create_dir_all(&dir)
                .map_err(|e| NdsError::DirectoryCreationError(e.to_string()))?;
        }

        Ok(dir)
    }

    pub fn metadata_path(&self) -> Result<PathBuf> {
        Ok(Self::session_dir()?.join(format!("{}.json", self.id)))
    }

    pub fn save(&self) -> Result<()> {
        let path = self.metadata_path()?;
        let json = serde_json::to_string_pretty(self)?;
        fs::write(path, json)?;
        Ok(())
    }

    pub fn load(id: &str) -> Result<Self> {
        let path = Self::session_dir()?.join(format!("{}.json", id));

        if !path.exists() {
            return Err(NdsError::SessionNotFound(id.to_string()));
        }

        let content = fs::read_to_string(path)?;
        let session: Session = serde_json::from_str(&content)?;

        // Verify the process is still alive
        if !Self::is_process_alive(session.pid) {
            // Clean up dead session
            Self::cleanup(&session.id)?;
            return Err(NdsError::SessionNotFound(id.to_string()));
        }

        Ok(session)
    }

    pub fn list_all() -> Result<Vec<Session>> {
        let dir = Self::session_dir()?;
        let mut sessions = Vec::new();
        let mut cleaned_count = 0;

        if dir.exists() {
            for entry in fs::read_dir(dir)? {
                let entry = entry?;
                let path = entry.path();

                if path.extension().and_then(|s| s.to_str()) == Some("json") {
                    let content = fs::read_to_string(&path)?;
                    if let Ok(session) = serde_json::from_str::<Session>(&content) {
                        // Check both process and socket health
                        let process_alive = Self::is_process_alive(session.pid);
                        let socket_healthy = session.socket_path.exists()
                            && Self::is_socket_healthy(&session.socket_path);

                        if process_alive && socket_healthy {
                            sessions.push(session);
                        } else {
                            // Clean up dead session completely
                            let _ = Self::cleanup(&session.id);
                            cleaned_count += 1;
                        }
                    }
                }
            }
        }

        if cleaned_count > 0 {
            eprintln!("Auto-cleaned {} dead session(s)", cleaned_count);
        }

        // Sort by creation time
        sessions.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        Ok(sessions)
    }

    /// Check if a socket is healthy by attempting to connect
    fn is_socket_healthy(socket_path: &PathBuf) -> bool {
        use std::time::Duration;

        match UnixStream::connect(socket_path) {
            Ok(socket) => {
                // Set a very short timeout for the health check
                let _ = socket.set_read_timeout(Some(Duration::from_millis(50)));
                let _ = socket.set_write_timeout(Some(Duration::from_millis(50)));
                true
            }
            Err(_) => false,
        }
    }

    pub fn cleanup(id: &str) -> Result<()> {
        let metadata_path = Self::session_dir()?.join(format!("{}.json", id));
        if metadata_path.exists() {
            fs::remove_file(metadata_path)?;
        }

        let socket_path = Self::socket_dir()?.join(format!("{}.sock", id));
        if socket_path.exists() {
            fs::remove_file(socket_path)?;
        }

        let status_path = Self::session_dir()?.join(format!("{}.status", id));
        if status_path.exists() {
            fs::remove_file(status_path)?;
        }

        Ok(())
    }

    pub fn is_process_alive(pid: i32) -> bool {
        // Check if process exists by sending signal 0
        unsafe { libc::kill(pid, 0) == 0 }
    }

    pub fn mark_attached(&mut self) -> Result<()> {
        self.attached = true;
        self.save()
    }

    pub fn mark_detached(&mut self) -> Result<()> {
        self.attached = false;
        self.save()
    }

    pub fn connect_socket(&self) -> Result<UnixStream> {
        use std::time::Duration;

        // Check if socket file exists first
        if !self.socket_path.exists() {
            return Err(NdsError::SocketError(format!(
                "Session socket does not exist: {}",
                self.socket_path.display()
            )));
        }

        // Try to connect with a timeout
        match UnixStream::connect(&self.socket_path) {
            Ok(socket) => {
                // Set socket timeout to prevent hanging
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .map_err(|e| {
                        NdsError::SocketError(format!("Failed to set socket timeout: {}", e))
                    })?;
                socket
                    .set_write_timeout(Some(Duration::from_secs(5)))
                    .map_err(|e| {
                        NdsError::SocketError(format!("Failed to set socket timeout: {}", e))
                    })?;
                Ok(socket)
            }
            Err(e) => {
                // Check if it's a connection refused or broken pipe
                match e.kind() {
                    std::io::ErrorKind::ConnectionRefused
                    | std::io::ErrorKind::BrokenPipe
                    | std::io::ErrorKind::NotFound => {
                        // Session might be dead, try to clean up
                        Err(NdsError::SessionNotFound(format!(
                            "Session {} appears to be dead or unreachable: {}",
                            self.id, e
                        )))
                    }
                    _ => Err(NdsError::SocketError(format!(
                        "Failed to connect to session socket: {}",
                        e
                    ))),
                }
            }
        }
    }

    pub fn get_client_count(&self) -> usize {
        // Read client count from a status file instead of connecting to the socket
        // This avoids disrupting active sessions
        let status_path = Self::session_dir()
            .ok()
            .and_then(|dir| Some(dir.join(format!("{}.status", self.id))));

        if let Some(path) = status_path {
            if let Ok(content) = fs::read_to_string(path) {
                if let Ok(count) = content.trim().parse::<usize>() {
                    return count;
                }
            }
        }

        // Fallback: assume 0 if detached, 1 if attached
        if self.attached {
            1
        } else {
            0
        }
    }

    pub fn update_client_count(session_id: &str, count: usize) -> Result<()> {
        let status_path = Self::session_dir()?.join(format!("{}.status", session_id));
        fs::write(status_path, count.to_string())?;
        Ok(())
    }
}
