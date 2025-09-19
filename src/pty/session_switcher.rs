use std::io::{self, BufRead, Write};
use std::os::unix::io::{BorrowedFd, RawFd};

use nix::sys::termios::{tcgetattr, tcsetattr, SetArg, Termios};

use crate::error::{NdsError, Result};
use crate::manager::SessionManager;
use crate::session::Session;

/// Result of a session switch operation
pub enum SwitchResult {
    /// Switch to an existing session with the given ID
    SwitchTo(String),
    /// Continue with the current session
    Continue,
}

/// Handle the session switcher interface
pub struct SessionSwitcher<'a> {
    current_session: &'a Session,
    stdin_fd: RawFd,
    original_termios: &'a Termios,
}

impl<'a> SessionSwitcher<'a> {
    pub fn new(
        current_session: &'a Session,
        stdin_fd: RawFd,
        original_termios: &'a Termios,
    ) -> Self {
        Self {
            current_session,
            stdin_fd,
            original_termios,
        }
    }

    /// Show the session switcher interface and handle user selection
    pub fn show_switcher(&self) -> Result<SwitchResult> {
        // Clear screen and show simple picker
        print!("\x1b[2J\x1b[H"); // Clear screen
        println!("\r\n╔══════════════════════════════════════╗\r");
        println!("\r║         SESSION SWITCHER             ║\r");
        println!("\r╚══════════════════════════════════════╝\r");

        // Get sessions excluding current
        let sessions = SessionManager::list_sessions()?;
        let other_sessions: Vec<_> = sessions
            .iter()
            .filter(|s| s.id != self.current_session.id)
            .collect();

        // Show current session
        println!(
            "\r\nCurrent: {} [{}]\r",
            self.current_session.display_name(),
            &self.current_session.id[..8]
        );
        println!("\r\n─────────────────────────────────────────\r");

        // Show available sessions
        if !other_sessions.is_empty() {
            println!("\r\nOther Sessions:\r");
            for (i, session) in other_sessions.iter().enumerate() {
                let client_count = session.get_client_count();
                let status = if client_count > 0 { "●" } else { "○" };
                println!(
                    "\r  [{}] {} {} {}\r",
                    i + 1,
                    status,
                    session.display_name(),
                    format!("[{}]", &session.id[..8])
                );
            }
        }

        // Add options
        let new_option = other_sessions.len() + 1;
        println!("\r  [{}] ➕ Create New Session\r", new_option);
        println!("\r  [0] Cancel\r");
        println!("\r\n─────────────────────────────────────────\r");
        print!("\r\nSelect [0-{}]: ", new_option);
        let _ = io::stdout().flush();

        // Read selection
        let selection = self.read_user_input()?;
        if let Ok(num) = selection.trim().parse::<usize>() {
            if num > 0 && num <= other_sessions.len() {
                let target = other_sessions[num - 1];
                println!("\r\n✓ Switching to: {}\r", target.display_name());
                return Ok(SwitchResult::SwitchTo(target.id.clone()));
            } else if num == new_option {
                return self.handle_new_session();
            }
        }

        println!("\r\n[Continuing current session]\r");
        Ok(SwitchResult::Continue)
    }

    /// Handle creating a new session
    fn handle_new_session(&self) -> Result<SwitchResult> {
        print!("\r\nEnter session name (or Enter for no name): ");
        let _ = io::stdout().flush();

        let name_input = self.read_user_input()?;
        let name = if name_input.trim().is_empty() {
            None
        } else {
            Some(name_input.trim().to_string())
        };

        match SessionManager::create_session_with_name(name.clone()) {
            Ok(new_session) => {
                println!("\r\n✓ Created session: {}\r", new_session.display_name());
                Ok(SwitchResult::SwitchTo(new_session.id))
            }
            Err(e) => {
                eprintln!("\r\nError creating session: {}\r", e);
                Ok(SwitchResult::Continue)
            }
        }
    }

    /// Read user input in cooked mode
    fn read_user_input(&self) -> Result<String> {
        let stdin_borrowed = unsafe { BorrowedFd::borrow_raw(self.stdin_fd) };

        // Save current settings
        let current_termios = tcgetattr(&stdin_borrowed)?;

        // Restore to cooked mode
        tcsetattr(&stdin_borrowed, SetArg::TCSAFLUSH, self.original_termios)?;

        // Set blocking mode
        unsafe {
            let flags = libc::fcntl(self.stdin_fd, libc::F_GETFL);
            if flags >= 0 {
                let _ = libc::fcntl(self.stdin_fd, libc::F_SETFL, flags & !libc::O_NONBLOCK);
            }
        }

        // Read line
        let stdin = io::stdin();
        let mut buffer = String::new();
        let mut stdin_lock = stdin.lock();
        let result = stdin_lock.read_line(&mut buffer);

        // Restore non-blocking if needed
        unsafe {
            let flags = libc::fcntl(self.stdin_fd, libc::F_GETFL);
            if flags >= 0 {
                let _ = libc::fcntl(self.stdin_fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
            }
        }

        // Restore raw mode
        tcsetattr(&stdin_borrowed, SetArg::TCSANOW, &current_termios)?;

        result.map_err(|e| NdsError::Io(e))?;
        Ok(buffer)
    }
}
