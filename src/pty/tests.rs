#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixStream;
    use tempfile::TempDir;

    mod client_tests {
        use super::*;
        use crate::pty::client::*;

        #[test]
        fn test_client_info_creation() {
            let temp_dir = TempDir::new().unwrap();
            let _socket_path = temp_dir.path().join("test.sock");

            // Create a mock socket pair
            let (stream1, _stream2) = UnixStream::pair().unwrap();

            let client = ClientInfo::new(stream1);
            assert!(client.rows > 0);
            assert!(client.cols > 0);
        }

        #[test]
        fn test_client_size_update() {
            let (stream1, _stream2) = UnixStream::pair().unwrap();
            let mut client = ClientInfo::new(stream1);

            client.update_size(30, 100);
            assert_eq!(client.rows, 30);
            assert_eq!(client.cols, 100);
        }

        #[test]
        fn test_terminal_size_detection() {
            let result = get_terminal_size();
            // Should either succeed with valid dimensions or fail
            if let Ok((rows, cols)) = result {
                assert!(rows > 0);
                assert!(cols > 0);
            }
        }
    }

    mod socket_tests {
        use super::*;
        use crate::pty::socket::*;

        #[test]
        fn test_parse_nds_command() {
            let resize_cmd = b"\x1b]nds:resize:80:24\x07";
            let result = parse_nds_command(resize_cmd);

            assert!(result.is_some());
            let (cmd, args) = result.unwrap();
            assert_eq!(cmd, "resize");
            assert_eq!(args, vec!["80", "24"]);
        }

        #[test]
        fn test_parse_nds_command_invalid() {
            let invalid_cmd = b"regular text";
            let result = parse_nds_command(invalid_cmd);
            assert!(result.is_none());
        }

        #[test]
        fn test_get_command_end() {
            let data = b"\x1b]nds:resize:80:24\x07more data";
            let end_idx = get_command_end(data);

            assert!(end_idx.is_some());
            assert_eq!(end_idx.unwrap(), 19); // Position after \x07 (index 18 + 1)
        }

        #[test]
        fn test_send_resize_command() {
            let (mut stream1, mut stream2) = UnixStream::pair().unwrap();

            // Send resize command
            let result = send_resize_command(&mut stream1, 100, 50);
            assert!(result.is_ok());

            // Read from other end
            let mut buffer = [0u8; 256];
            use std::io::Read;
            let n = stream2.read(&mut buffer).unwrap();

            let received = &buffer[..n];
            let expected = format!("\x1b]nds:resize:100:50\x07");
            assert_eq!(received, expected.as_bytes());
        }
    }

    mod terminal_tests {
        use crate::pty::terminal::*;

        #[test]
        fn test_terminal_size_operations() {
            // Test getting terminal size
            let result = get_terminal_size();
            if let Ok((cols, rows)) = result {
                assert!(cols > 0);
                assert!(rows > 0);
            }
        }

        #[test]
        fn test_terminal_refresh_sequences() {
            let mut buffer = Vec::new();
            let result = send_terminal_refresh_sequences(&mut buffer);

            assert!(result.is_ok());
            assert!(!buffer.is_empty());

            // Check for some expected escape sequences
            let output = String::from_utf8_lossy(&buffer);
            assert!(output.contains("\x1b[?25h")); // Show cursor
            assert!(output.contains("\x1b[m")); // Reset attributes
        }

        #[test]
        fn test_send_refresh() {
            let mut buffer = Vec::new();
            let result = send_refresh(&mut buffer);

            assert!(result.is_ok());
            assert_eq!(buffer, b"\x0c"); // Ctrl+L
        }
    }

    mod io_handler_tests {
        use crate::pty::io_handler::*;

        #[test]
        fn test_scrollback_handler() {
            let handler = ScrollbackHandler::new(1024);

            // Add some data
            handler.add_data(b"Hello, World!");

            // Get buffer back
            let buffer = handler.get_buffer();
            assert_eq!(buffer, b"Hello, World!");
        }

        #[test]
        fn test_scrollback_overflow() {
            let handler = ScrollbackHandler::new(10); // Very small buffer

            // Add data larger than buffer
            handler.add_data(b"This is a very long string that exceeds the buffer");

            // Buffer should be trimmed
            let buffer = handler.get_buffer();
            assert!(buffer.len() <= 10);
        }

        #[test]
        fn test_pty_io_handler_creation() {
            // We can't test actual PTY operations without a real PTY
            // but we can test creation
            let _handler = PtyIoHandler::new(0);
            // Can't access private field, just test creation
            // The handler is created successfully
        }
    }

    mod session_switcher_tests {
        use crate::pty::session_switcher::*;

        #[test]
        fn test_switch_result_variants() {
            // Test that enum variants work correctly
            let continue_result = SwitchResult::Continue;
            assert!(matches!(continue_result, SwitchResult::Continue));

            let switch_result = SwitchResult::SwitchTo("test123".to_string());
            if let SwitchResult::SwitchTo(id) = switch_result {
                assert_eq!(id, "test123");
            } else {
                panic!("Expected SwitchTo variant");
            }
        }
    }

    mod edge_case_tests {
        use super::*;
        use crate::pty::client::*;
        use crate::pty::io_handler::*;
        use crate::pty::socket::*;

        #[test]
        fn test_parse_nds_command_empty() {
            let empty_cmd = b"";
            let result = parse_nds_command(empty_cmd);
            assert!(result.is_none());
        }

        #[test]
        fn test_parse_nds_command_incomplete() {
            let incomplete_cmd = b"\x1b]nds:resize:80:24"; // Missing \x07
            let result = parse_nds_command(incomplete_cmd);
            assert!(result.is_none());
        }

        #[test]
        fn test_parse_nds_command_no_args() {
            // Use an allowed command instead of "ping"
            let cmd = b"\x1b]nds:detach\x07";
            let result = parse_nds_command(cmd);
            assert!(result.is_some());
            let (cmd_name, args) = result.unwrap();
            assert_eq!(cmd_name, "detach");
            assert_eq!(args.len(), 0);
        }

        #[test]
        fn test_get_command_end_no_terminator() {
            let data = b"\x1b]nds:resize:80:24"; // Missing \x07
            let end_idx = get_command_end(data);
            assert!(end_idx.is_none());
        }

        #[test]
        fn test_get_command_end_not_nds_command() {
            let data = b"regular text\x07";
            let end_idx = get_command_end(data);
            assert!(end_idx.is_none());
        }

        #[test]
        fn test_send_resize_command_zero_dimensions() {
            let (mut stream1, mut stream2) = UnixStream::pair().unwrap();

            // Send resize with zero dimensions (edge case)
            // Should be sanitized to 1:1 for security
            let result = send_resize_command(&mut stream1, 0, 0);
            assert!(result.is_ok());

            // Read from other end
            let mut buffer = [0u8; 256];
            use std::io::Read;
            let n = stream2.read(&mut buffer).unwrap();

            let received = &buffer[..n];
            let expected = format!("\x1b]nds:resize:1:1\x07"); // Sanitized to 1:1
            assert_eq!(received, expected.as_bytes());
        }

        #[test]
        fn test_send_resize_command_large_dimensions() {
            let (mut stream1, mut stream2) = UnixStream::pair().unwrap();

            // Send resize with very large dimensions
            let result = send_resize_command(&mut stream1, 9999, 9999);
            assert!(result.is_ok());

            // Read from other end
            let mut buffer = [0u8; 256];
            use std::io::Read;
            let n = stream2.read(&mut buffer).unwrap();

            let received = &buffer[..n];
            let expected = format!("\x1b]nds:resize:9999:9999\x07");
            assert_eq!(received, expected.as_bytes());
        }

        #[test]
        fn test_parse_nds_command_invalid_command() {
            // Test that invalid commands are rejected for security
            let cmd = b"\x1b]nds:rm:rf:/\x07"; // Dangerous command
            let result = parse_nds_command(cmd);
            assert!(result.is_none(), "Invalid command should be rejected");

            let cmd2 = b"\x1b]nds:unknown:command\x07"; // Unknown command
            let result2 = parse_nds_command(cmd2);
            assert!(result2.is_none(), "Unknown command should be rejected");
        }

        #[test]
        fn test_client_info_with_broken_pipe() {
            let (stream1, stream2) = UnixStream::pair().unwrap();
            // Drop one end to simulate broken pipe
            drop(stream2);

            let client = ClientInfo::new(stream1);
            // Should still create with default terminal size
            assert!(client.rows > 0);
            assert!(client.cols > 0);
        }

        #[test]
        fn test_scrollback_handler_empty() {
            let handler = ScrollbackHandler::new(1024);

            // Get empty buffer
            let buffer = handler.get_buffer();
            assert_eq!(buffer.len(), 0);
        }

        #[test]
        fn test_scrollback_handler_multiple_adds() {
            let handler = ScrollbackHandler::new(1024);

            handler.add_data(b"First ");
            handler.add_data(b"Second ");
            handler.add_data(b"Third");

            let buffer = handler.get_buffer();
            assert_eq!(buffer, b"First Second Third");
        }
    }
}
