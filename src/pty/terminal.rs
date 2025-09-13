use std::os::unix::io::{BorrowedFd, RawFd};
use std::thread;
use std::time::Duration;
use std::io::{self, Write};

use crossterm::terminal;
use nix::sys::termios::{tcflush, tcgetattr, tcsetattr, FlushArg, SetArg, Termios};
use nix::sys::termios::{InputFlags, OutputFlags, ControlFlags, LocalFlags, SpecialCharacterIndices};

use crate::error::{NdsError, Result};
use crate::terminal_state::TerminalState;

/// Save the current terminal state
pub fn save_terminal_state(stdin_fd: RawFd) -> Result<Termios> {
    let stdin = unsafe { BorrowedFd::borrow_raw(stdin_fd) };
    tcgetattr(&stdin).map_err(|e| {
        NdsError::TerminalError(format!("Failed to get terminal attributes: {}", e))
    })
}

/// Set terminal to raw mode
pub fn set_raw_mode(stdin_fd: RawFd, original: &Termios) -> Result<()> {
    let stdin = unsafe { BorrowedFd::borrow_raw(stdin_fd) };
    
    let mut raw = original.clone();
    // Manually set raw mode flags
    raw.input_flags = InputFlags::empty();
    raw.output_flags = OutputFlags::empty();
    raw.control_flags |= ControlFlags::CS8;
    raw.local_flags = LocalFlags::empty();
    raw.control_chars[SpecialCharacterIndices::VMIN as usize] = 1;
    raw.control_chars[SpecialCharacterIndices::VTIME as usize] = 0;
    
    tcsetattr(&stdin, SetArg::TCSANOW, &raw)
        .map_err(|e| NdsError::TerminalError(format!("Failed to set raw mode: {}", e)))
}

/// Restore terminal to original state
pub fn restore_terminal(stdin_fd: RawFd, original: &Termios) -> Result<()> {
    let stdin = unsafe { BorrowedFd::borrow_raw(stdin_fd) };
    
    // First restore stdin to blocking mode
    unsafe {
        let flags = libc::fcntl(stdin_fd, libc::F_GETFL);
        libc::fcntl(stdin_fd, libc::F_SETFL, flags & !libc::O_NONBLOCK);
    }
    
    // Clear any pending input from stdin buffer
    tcflush(&stdin, FlushArg::TCIFLUSH)
        .map_err(|e| NdsError::TerminalError(format!("Failed to flush stdin: {}", e)))?;
    
    // Restore the terminal settings
    tcsetattr(&stdin, SetArg::TCSANOW, original)
        .map_err(|e| NdsError::TerminalError(format!("Failed to restore terminal: {}", e)))?;
    
    // Ensure we're back in cooked mode
    terminal::disable_raw_mode().ok();
    
    // Clear any remaining input after terminal restore
    tcflush(&stdin, FlushArg::TCIFLUSH)
        .map_err(|e| NdsError::TerminalError(format!("Failed to flush stdin after restore: {}", e)))?;
    
    // Add a small delay to ensure terminal is fully restored
    thread::sleep(Duration::from_millis(50));
    
    Ok(())
}

/// Get current terminal size
pub fn get_terminal_size() -> Result<(u16, u16)> {
    terminal::size().map_err(|e| NdsError::TerminalError(e.to_string()))
}

/// Set terminal size on a file descriptor
pub fn set_terminal_size(fd: RawFd, cols: u16, rows: u16) -> Result<()> {
    unsafe {
        let winsize = libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        if libc::ioctl(fd, libc::TIOCSWINSZ as u64, &winsize) < 0 {
            return Err(NdsError::PtyError("Failed to set terminal size".to_string()));
        }
    }
    Ok(())
}

/// Set stdin to non-blocking mode
pub fn set_stdin_nonblocking(stdin_fd: RawFd) -> Result<()> {
    unsafe {
        let flags = libc::fcntl(stdin_fd, libc::F_GETFL);
        if libc::fcntl(stdin_fd, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
            return Err(NdsError::TerminalError("Failed to set stdin non-blocking".to_string()));
        }
    }
    Ok(())
}

/// Send a refresh command to the terminal
pub fn send_refresh(stream: &mut impl Write) -> io::Result<()> {
    // Send Ctrl+L to refresh the display
    stream.write_all(b"\x0c")?;
    stream.flush()
}

/// Send terminal refresh sequences to restore normal state
pub fn send_terminal_refresh_sequences(stream: &mut impl Write) -> io::Result<()> {
    let refresh_sequences = [
        "\x1b[?25h",     // Show cursor
        "\x1b[?12h",     // Enable cursor blinking
        "\x1b[1 q",      // Blinking block cursor (default)
        "\x1b[m",        // Reset all attributes
        "\x1b[?1000l",   // Disable mouse tracking (if enabled)
        "\x1b[?1002l",   // Disable cell motion mouse tracking
        "\x1b[?1003l",   // Disable all motion mouse tracking
    ].join("");
    
    stream.write_all(refresh_sequences.as_bytes())?;
    stream.flush()
}

/// Capture current terminal state for restoration
pub fn capture_terminal_state(stdin_fd: RawFd) -> Result<TerminalState> {
    TerminalState::capture(stdin_fd)
}