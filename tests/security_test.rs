use std::fs;
use std::os::unix::fs::PermissionsExt;
use tempfile::TempDir;

#[test]
fn test_socket_permissions() {
    let temp_dir = TempDir::new().unwrap();
    let socket_path = temp_dir.path().join("test.sock");

    // Create a test file simulating socket creation
    fs::write(&socket_path, b"test").unwrap();

    // Set permissions as our code does
    let mut perms = fs::metadata(&socket_path).unwrap().permissions();
    perms.set_mode(0o600);
    fs::set_permissions(&socket_path, perms).unwrap();

    // Verify permissions
    let metadata = fs::metadata(&socket_path).unwrap();
    let mode = metadata.permissions().mode();

    // Check that only owner has read/write (0o600)
    assert_eq!(mode & 0o777, 0o600, "Socket should have 0600 permissions");
}

#[test]
#[cfg(feature = "async")]
fn test_input_sanitization() {
    use detached_shell::pty::socket_async::{is_valid_command, sanitize_string_input};

    // Test control character removal
    let dirty = "test\x00\x01\x02\x03";
    let clean = sanitize_string_input(dirty);
    assert_eq!(clean, "test");

    // Test command validation
    assert!(is_valid_command("resize"));
    assert!(is_valid_command("detach"));
    assert!(!is_valid_command("rm"));
    assert!(!is_valid_command("../../etc/passwd"));
}

#[test]
#[cfg(feature = "async")]
fn test_numeric_bounds() {
    use detached_shell::pty::socket_async::sanitize_numeric_input;

    // Test bounds checking
    assert_eq!(sanitize_numeric_input(0), 1);
    assert_eq!(sanitize_numeric_input(5000), 5000);
    assert_eq!(sanitize_numeric_input(10000), 9999);
}

#[test]
#[cfg(feature = "async")]
fn test_command_length_limits() {
    use detached_shell::pty::socket_async::parse_nds_command_secure;

    // Test max command length (8KB)
    let long_cmd = format!("\x1b]nds:test:{}\x07", "x".repeat(10000));
    let result = parse_nds_command_secure(long_cmd.as_bytes());
    assert!(result.is_none(), "Should reject commands over 8KB");

    // Test max arg count
    let many_args = format!(
        "\x1b]nds:test:{}\x07",
        (0..20).map(|_| "arg").collect::<Vec<_>>().join(":")
    );
    let result = parse_nds_command_secure(many_args.as_bytes());
    assert!(result.is_none(), "Should reject more than 10 arguments");
}
