use std::io::{self, Write, BufRead};
use std::os::unix::io::{BorrowedFd, RawFd};

use nix::sys::termios::{tcgetattr, tcsetattr, SetArg, Termios};

use crate::error::{NdsError, Result};
use crate::session::Session;
use crate::manager::SessionManager;

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
        println!("\r\n[Session Switcher]\r");
        
        // Get list of other sessions
        let sessions = SessionManager::list_sessions()?;
        let other_sessions: Vec<_> = sessions
            .iter()
            .filter(|s| s.id != self.current_session.id)
            .collect();
        
        // Show available sessions
        println!("\r\nAvailable options:\r");
        
        // Show existing sessions
        if !other_sessions.is_empty() {
            for (i, s) in other_sessions.iter().enumerate() {
                println!(
                    "\r  {}. {} (PID: {})\r",
                    i + 1,
                    s.display_name(),
                    s.pid
                );
            }
        }
        
        // Add new session option
        let new_option_num = other_sessions.len() + 1;
        println!("\r  {}. [New Session]\r", new_option_num);
        println!("\r  0. Cancel\r");
        println!("\r\nSelect option (0-{}): ", new_option_num);
        let _ = io::stdout().flush();
        
        // Read user selection with temporary cooked mode
        let selection = self.read_user_input()?;
        
        if let Ok(num) = selection.trim().parse::<usize>() {
            if num > 0 && num <= other_sessions.len() {
                // Switch to selected session
                let target_session = other_sessions[num - 1];
                println!("\r\n[Switching to session {}]\r", target_session.id);
                return Ok(SwitchResult::SwitchTo(target_session.id.clone()));
            } else if num == new_option_num {
                // Create new session
                return self.handle_new_session();
            }
        }
        
        // Cancelled or invalid selection
        println!("\r\n[Continuing current session]\r");
        Ok(SwitchResult::Continue)
    }
    
    /// Handle creating a new session
    fn handle_new_session(&self) -> Result<SwitchResult> {
        println!("\r\nEnter name for new session (or press Enter for no name): ");
        let _ = io::stdout().flush();
        
        let session_name = self.read_user_input()?;
        let session_name = session_name.trim();
        
        let name = if session_name.is_empty() {
            None
        } else {
            Some(session_name.to_string())
        };
        
        // Create new session
        match SessionManager::create_session_with_name(name.clone()) {
            Ok(new_session) => {
                if let Some(ref n) = name {
                    println!(
                        "\r\n[Created and switching to new session '{}' ({})]",
                        n, new_session.id
                    );
                } else {
                    println!(
                        "\r\n[Created and switching to new session {}]",
                        new_session.id
                    );
                }
                Ok(SwitchResult::SwitchTo(new_session.id))
            }
            Err(e) => {
                eprintln!("\r\nError creating session: {}\r", e);
                Ok(SwitchResult::Continue)
            }
        }
    }
    
    /// Read user input with temporary cooked mode
    fn read_user_input(&self) -> Result<String> {
        let stdin_borrowed = unsafe { BorrowedFd::borrow_raw(self.stdin_fd) };
        
        // Save current raw mode settings
        let current_termios = tcgetattr(&stdin_borrowed)?;
        
        // Restore to original (cooked) mode for line input
        tcsetattr(&stdin_borrowed, SetArg::TCSANOW, self.original_termios)?;
        
        // Read user input
        let stdin = io::stdin();
        let mut buffer = String::new();
        let read_result = stdin.lock().read_line(&mut buffer);
        
        // Restore raw mode
        tcsetattr(&stdin_borrowed, SetArg::TCSANOW, &current_termios)?;
        
        read_result.map_err(|e| NdsError::Io(e))?;
        Ok(buffer)
    }
}

/// Show the session help message
pub fn show_session_help() {
    println!("\r\n[Session Commands]\r");
    println!("\r  ~d - Detach from current session\r");
    println!("\r  ~s - Switch sessions\r");
    println!("\r  ~h - Show scrollback history\r");
    println!("\r  ~~ - Send literal tilde\r");
    println!("\r\n[Press any key to continue]\r");
}