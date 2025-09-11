use crate::manager::SessionManager;
use crate::session::Session;
use std::path::PathBuf;
use tempfile::TempDir;
use uuid::Uuid;

#[test]
fn test_session_creation() {
    let id = Uuid::new_v4().to_string()[..8].to_string();
    let socket_path = PathBuf::from("/tmp/test.sock");
    let session = Session::new(id.clone(), 12345, socket_path.clone());
    assert_eq!(session.id, id);
    assert_eq!(session.pid, 12345);
    assert_eq!(session.socket_path, socket_path);
}

#[test]
fn test_session_different_ids() {
    let id1 = Uuid::new_v4().to_string()[..8].to_string();
    let id2 = Uuid::new_v4().to_string()[..8].to_string();
    let socket_path = PathBuf::from("/tmp/test.sock");
    let session1 = Session::new(id1.clone(), 12345, socket_path.clone());
    let session2 = Session::new(id2.clone(), 12346, socket_path);
    assert_ne!(session1.id, session2.id);
}

#[test]
fn test_session_manager_list_empty() {
    let temp_dir = TempDir::new().unwrap();
    std::env::set_var("NDS_HOME", temp_dir.path());

    // Initialize directories
    let sessions_dir = temp_dir.path().join("sessions");
    let sockets_dir = temp_dir.path().join("sockets");
    std::fs::create_dir_all(&sessions_dir).unwrap();
    std::fs::create_dir_all(&sockets_dir).unwrap();

    let sessions = SessionManager::list_sessions().unwrap();
    assert_eq!(sessions.len(), 0);
}

#[test]
fn test_session_serialization() {
    let id = Uuid::new_v4().to_string()[..8].to_string();
    let socket_path = PathBuf::from("/tmp/test.sock");
    let session = Session::new(id.clone(), 12345, socket_path);
    let json = serde_json::to_string(&session).unwrap();
    let deserialized: Session = serde_json::from_str(&json).unwrap();
    assert_eq!(session.id, deserialized.id);
    assert_eq!(session.pid, deserialized.pid);
}

#[test]
#[ignore] // This test requires exclusive access to NDS_HOME env var
fn test_session_creation_and_cleanup() {
    let temp_dir = TempDir::new().unwrap();
    std::env::set_var("NDS_HOME", temp_dir.path());

    // Initialize directories
    let sessions_dir = temp_dir.path().join("sessions");
    let sockets_dir = temp_dir.path().join("sockets");
    std::fs::create_dir_all(&sessions_dir).unwrap();
    std::fs::create_dir_all(&sockets_dir).unwrap();

    // Create a fake dead session file
    let dead_session = Session::new(
        "deadbeef".to_string(),
        99999999, // Non-existent PID
        sockets_dir.join("deadbeef.sock"),
    );
    let session_file = sessions_dir.join("deadbeef.json");
    std::fs::write(&session_file, serde_json::to_string(&dead_session).unwrap()).unwrap();

    // Cleanup should remove dead sessions
    SessionManager::cleanup_dead_sessions().unwrap();

    // File should be gone
    assert!(!session_file.exists());
}
