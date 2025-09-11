use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

#[test]
fn test_cli_version() {
    let mut cmd = Command::cargo_bin("nds").unwrap();
    cmd.arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("nds 0.1.0"));
}

#[test]
fn test_cli_help() {
    let mut cmd = Command::cargo_bin("nds").unwrap();
    cmd.arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Noras Detached Shell"));
}

#[test]
fn test_list_empty() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("nds").unwrap();
    cmd.env("NDS_HOME", temp_dir.path())
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("No active sessions"));
}

#[test]
fn test_clean_command() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("nds").unwrap();
    cmd.env("NDS_HOME", temp_dir.path())
        .arg("clean")
        .assert()
        .success()
        .stdout(predicate::str::contains("Cleanup complete"));
}

#[test]
fn test_kill_nonexistent() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("nds").unwrap();
    cmd.env("NDS_HOME", temp_dir.path())
        .arg("kill")
        .arg("nonexistent")
        .assert()
        .failure()
        .stderr(predicate::str::contains("SessionNotFound"));
}

#[test]
fn test_attach_nonexistent() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("nds").unwrap();
    cmd.env("NDS_HOME", temp_dir.path())
        .arg("attach")
        .arg("nonexistent")
        .assert()
        .failure()
        .stderr(predicate::str::contains("SessionNotFound"));
}

#[test]
fn test_info_nonexistent() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("nds").unwrap();
    cmd.env("NDS_HOME", temp_dir.path())
        .arg("info")
        .arg("nonexistent")
        .assert()
        .failure()
        .stderr(predicate::str::contains("SessionNotFound"));
}
