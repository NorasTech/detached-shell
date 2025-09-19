use std::io::{self, Write};
use std::os::unix::io::{BorrowedFd, RawFd};
use std::thread;
use std::time::Duration;

use crossterm::terminal;
use nix::sys::termios::{tcflush, tcgetattr, tcsetattr, FlushArg, SetArg, Termios};
use nix::sys::termios::{
    ControlFlags, InputFlags, LocalFlags, OutputFlags, SpecialCharacterIndices,
};

use crate::error::{NdsError, Result};
use crate::terminal_state::TerminalState;

/// Save the current terminal state
pub fn save_terminal_state(stdin_fd: RawFd) -> Result<Termios> {
    let stdin = unsafe { BorrowedFd::borrow_raw(stdin_fd) };
    tcgetattr(&stdin)
        .map_err(|e| NdsError::TerminalError(format!("Failed to get terminal attributes: {}", e)))
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

    // Send comprehensive terminal reset sequences to stdout (not socket)
    let reset_sequences = [
        "\x1b[m",      // Reset all attributes
        "\x1b[?1000l", // Disable mouse tracking
        "\x1b[?1002l", // Disable cell motion mouse tracking
        "\x1b[?1003l", // Disable all motion mouse tracking
        "\x1b[?1006l", // Disable SGR mouse mode
        "\x1b[?1015l", // Disable urxvt mouse mode
        "\x1b[?1049l", // Exit alternate screen buffer
        "\x1b[?47l",   // Exit alternate screen buffer (legacy)
        "\x1b[?1l",    // Return to normal cursor key mode
        "\x1b[?7h",    // Enable auto-wrap
        "\x1b[?25h",   // Ensure cursor is visible
        "\x1b[0;0r",   // Reset scroll region
    ]
    .join("");
    let _ = io::stdout().write_all(reset_sequences.as_bytes());
    let _ = io::stdout().flush();

    // Clear any remaining input after terminal restore
    tcflush(&stdin, FlushArg::TCIFLUSH).map_err(|e| {
        NdsError::TerminalError(format!("Failed to flush stdin after restore: {}", e))
    })?;

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
            return Err(NdsError::PtyError(
                "Failed to set terminal size".to_string(),
            ));
        }
    }
    Ok(())
}

/// Set stdin to non-blocking mode
#[allow(dead_code)]
pub fn set_stdin_nonblocking(stdin_fd: RawFd) -> Result<()> {
    unsafe {
        let flags = libc::fcntl(stdin_fd, libc::F_GETFL);
        if libc::fcntl(stdin_fd, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
            return Err(NdsError::TerminalError(
                "Failed to set stdin non-blocking".to_string(),
            ));
        }
    }
    Ok(())
}

/// Reset stdin to blocking mode
pub fn set_stdin_blocking(stdin_fd: RawFd) -> Result<()> {
    unsafe {
        let flags = libc::fcntl(stdin_fd, libc::F_GETFL);
        if flags < 0 {
            return Err(NdsError::TerminalError(
                "Failed to get stdin flags".to_string(),
            ));
        }
        // Clear the O_NONBLOCK flag
        let new_flags = flags & !libc::O_NONBLOCK;
        if libc::fcntl(stdin_fd, libc::F_SETFL, new_flags) < 0 {
            return Err(NdsError::TerminalError(
                "Failed to set stdin blocking".to_string(),
            ));
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
    // Send minimal refresh sequences without modifying cursor
    let refresh_sequences = [
        "\x1b[m", // Reset all attributes
        "\x0c",   // Form feed (clear screen)
    ]
    .join("");

    stream.write_all(refresh_sequences.as_bytes())?;
    stream.flush()
}

/// Capture current terminal state for restoration
pub fn capture_terminal_state(stdin_fd: RawFd) -> Result<TerminalState> {
    TerminalState::capture(stdin_fd)
}
