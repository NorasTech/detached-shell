use nix::sys::termios::{tcgetattr, tcsetattr, SetArg, Termios};
use serde::{Deserialize, Serialize};
use std::os::unix::io::{BorrowedFd, RawFd};

use crate::error::{NdsError, Result};

/// Stores terminal state for restoration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalState {
    pub window_size: (u16, u16),             // cols, rows
    pub cursor_position: Option<(u16, u16)>, // x, y
    #[serde(skip)]
    pub termios: Option<Termios>,
}

impl TerminalState {
    pub fn capture(fd: RawFd) -> Result<Self> {
        // Get window size
        let (cols, rows) = Self::get_window_size(fd)?;

        // Get termios settings
        let borrowed_fd = unsafe { BorrowedFd::borrow_raw(fd) };
        let termios = tcgetattr(&borrowed_fd).ok();

        Ok(TerminalState {
            window_size: (cols, rows),
            cursor_position: None, // Could be enhanced to capture cursor position
            termios,
        })
    }

    pub fn restore(&self, fd: RawFd) -> Result<()> {
        // Restore window size
        Self::set_window_size(fd, self.window_size.0, self.window_size.1)?;

        // Restore termios if available
        if let Some(ref termios) = self.termios {
            let borrowed_fd = unsafe { BorrowedFd::borrow_raw(fd) };
            tcsetattr(&borrowed_fd, SetArg::TCSANOW, termios).map_err(|e| {
                NdsError::TerminalError(format!("Failed to restore termios: {}", e))
            })?;
        }

        Ok(())
    }

    fn get_window_size(fd: RawFd) -> Result<(u16, u16)> {
        unsafe {
            let mut winsize = libc::winsize {
                ws_row: 0,
                ws_col: 0,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };

            if libc::ioctl(fd, libc::TIOCGWINSZ as u64, &mut winsize) < 0 {
                return Err(NdsError::TerminalError(
                    "Failed to get window size".to_string(),
                ));
            }

            Ok((winsize.ws_col, winsize.ws_row))
        }
    }

    fn set_window_size(fd: RawFd, cols: u16, rows: u16) -> Result<()> {
        unsafe {
            let winsize = libc::winsize {
                ws_row: rows,
                ws_col: cols,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };

            if libc::ioctl(fd, libc::TIOCSWINSZ as u64, &winsize) < 0 {
                return Err(NdsError::TerminalError(
                    "Failed to set window size".to_string(),
                ));
            }

            Ok(())
        }
    }
}

/// Terminal commands for state management
pub struct TerminalCommands;

impl TerminalCommands {
    /// Send terminal refresh sequence
    pub fn refresh_display() -> &'static [u8] {
        // Ctrl+L to refresh display
        b"\x0c"
    }

    /// Request terminal to redraw
    pub fn redraw_prompt() -> &'static [u8] {
        // Send empty input to trigger prompt redraw
        b"\n\x1b[A" // Newline then cursor up
    }
}
