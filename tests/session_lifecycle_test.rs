use assert_cmd::Command;
use predicates::prelude::*;
use std::thread;
use std::time::Duration;

#[test]
fn test_session_lifecycle() {
    // Test creating a session
    let mut cmd = Command::cargo_bin("nds").unwrap();
    let output = cmd
        .arg("new")
        .arg("test-lifecycle")
        .arg("--no-attach")
        .output()
        .expect("Failed to create session");

    assert!(output.status.success());
    let output_str = String::from_utf8_lossy(&output.stdout);

    // Extract session ID from output
    let session_id = output_str
        .lines()
        .find(|line| line.starts_with("Created session:"))
        .and_then(|line| line.split_whitespace().last())
        .expect("Failed to extract session ID");

    // Test listing sessions
    let mut cmd = Command::cargo_bin("nds").unwrap();
    cmd.arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("test-lifecycle"));

    // Test getting session info
    let mut cmd = Command::cargo_bin("nds").unwrap();
    cmd.arg("info")
        .arg(&session_id)
        .assert()
        .success()
        .stdout(predicate::str::contains("Session ID:"))
        .stdout(predicate::str::contains("test-lifecycle"));

    // Test renaming session
    let mut cmd = Command::cargo_bin("nds").unwrap();
    cmd.arg("rename")
        .arg(&session_id)
        .arg("renamed-session")
        .assert()
        .success();

    // Verify rename worked
    let mut cmd = Command::cargo_bin("nds").unwrap();
    cmd.arg("info")
        .arg(&session_id)
        .assert()
        .success()
        .stdout(predicate::str::contains("renamed-session"));

    // Test killing session
    let mut cmd = Command::cargo_bin("nds").unwrap();
    cmd.arg("kill")
        .arg(&session_id)
        .assert()
        .success()
        .stdout(predicate::str::contains("Killed session"));

    // Verify session is gone
    let mut cmd = Command::cargo_bin("nds").unwrap();
    cmd.arg("info").arg(&session_id).assert().failure();
}

#[test]
fn test_session_name_matching() {
    // Create session with specific name
    let mut cmd = Command::cargo_bin("nds").unwrap();
    cmd.arg("new")
        .arg("unique-test-name")
        .arg("--no-attach")
        .assert()
        .success();

    // Small delay to ensure session is created
    thread::sleep(Duration::from_millis(100));

    // Test partial name matching for info
    let mut cmd = Command::cargo_bin("nds").unwrap();
    cmd.arg("info")
        .arg("unique")
        .assert()
        .success()
        .stdout(predicate::str::contains("unique-test-name"));

    // Test case-insensitive matching
    let mut cmd = Command::cargo_bin("nds").unwrap();
    cmd.arg("info")
        .arg("UNIQUE")
        .assert()
        .success()
        .stdout(predicate::str::contains("unique-test-name"));

    // Clean up
    let mut cmd = Command::cargo_bin("nds").unwrap();
    cmd.arg("kill").arg("unique").assert().success();
}

#[test]
fn test_multiple_sessions() {
    // Create multiple sessions
    for i in 1..=3 {
        let mut cmd = Command::cargo_bin("nds").unwrap();
        cmd.arg("new")
            .arg(format!("multi-test-{}", i))
            .arg("--no-attach")
            .assert()
            .success();
    }

    // Small delay
    thread::sleep(Duration::from_millis(200));

    // List should show all sessions
    let mut cmd = Command::cargo_bin("nds").unwrap();
    let output = cmd.arg("list").output().unwrap();
    let output_str = String::from_utf8_lossy(&output.stdout);

    assert!(output_str.contains("multi-test-1"));
    assert!(output_str.contains("multi-test-2"));
    assert!(output_str.contains("multi-test-3"));

    // Kill all multi-test sessions
    let mut cmd = Command::cargo_bin("nds").unwrap();
    cmd.arg("kill")
        .arg("multi-test-1")
        .arg("multi-test-2")
        .arg("multi-test-3")
        .assert()
        .success()
        .stdout(predicate::str::contains("Successfully killed 3 session(s)"));
}

#[test]
fn test_clean_command() {
    // The clean command should always succeed
    let mut cmd = Command::cargo_bin("nds").unwrap();
    cmd.arg("clean")
        .assert()
        .success()
        .stdout(predicate::str::contains("Cleanup complete"));
}

#[test]
fn test_history_command() {
    // Create a session for history
    let mut cmd = Command::cargo_bin("nds").unwrap();
    cmd.arg("new")
        .arg("history-test")
        .arg("--no-attach")
        .assert()
        .success();

    thread::sleep(Duration::from_millis(100));

    // Check history
    let mut cmd = Command::cargo_bin("nds").unwrap();
    cmd.arg("history").assert().success();

    // Check history with --all flag
    let mut cmd = Command::cargo_bin("nds").unwrap();
    cmd.arg("history").arg("--all").assert().success();

    // Clean up
    let mut cmd = Command::cargo_bin("nds").unwrap();
    cmd.arg("kill").arg("history-test").assert().success();
}
