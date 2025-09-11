use std::fs::File;
use std::io::{self, Read, Write};
use std::os::unix::io::{BorrowedFd, FromRawFd, RawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

use crossterm::terminal;
use nix::fcntl::{fcntl, FcntlArg, OFlag};
use nix::sys::signal::{kill, Signal};
use nix::sys::termios::{tcgetattr, tcsetattr, SetArg};
use nix::unistd::{close, dup2, execvp, fork, setsid, ForkResult, Pid};

use crate::error::{NdsError, Result};
use crate::manager::SessionManager;
use crate::pty_buffer::PtyBuffer;
use crate::scrollback::ScrollbackViewer;
use crate::session::Session;
use crate::terminal_state::TerminalState;

pub struct PtyProcess {
    pub master_fd: RawFd,
    pub pid: Pid,
    pub socket_path: PathBuf,
    listener: Option<UnixListener>,
    output_buffer: Option<PtyBuffer>,
}

impl PtyProcess {
    pub fn spawn_new_detached(session_id: &str) -> Result<Session> {
        Self::spawn_new_detached_with_name(session_id, None)
    }

    pub fn spawn_new_detached_with_name(session_id: &str, name: Option<String>) -> Result<Session> {
        // Capture terminal size BEFORE detaching
        let (cols, rows) = terminal::size().unwrap_or((80, 24));

        // First fork to create intermediate process
        match unsafe { fork() }
            .map_err(|e| NdsError::ForkError(format!("First fork failed: {}", e)))?
        {
            ForkResult::Parent { child: _ } => {
                // Wait for the intermediate process to complete
                std::thread::sleep(std::time::Duration::from_millis(200));

                // Load the session that was created by the daemon
                Session::load(session_id)
            }
            ForkResult::Child => {
                // We're in the intermediate process
                // Create a new session to detach from the terminal
                setsid().map_err(|e| NdsError::ProcessError(format!("setsid failed: {}", e)))?;

                // Second fork to ensure we can't acquire a controlling terminal
                match unsafe { fork() }
                    .map_err(|e| NdsError::ForkError(format!("Second fork failed: {}", e)))?
                {
                    ForkResult::Parent { child: _ } => {
                        // Intermediate process exits immediately
                        std::process::exit(0);
                    }
                    ForkResult::Child => {
                        // We're now in the daemon process
                        // Close standard file descriptors to fully detach
                        unsafe {
                            libc::close(0);
                            libc::close(1);
                            libc::close(2);

                            // Redirect to /dev/null
                            let dev_null = libc::open(
                                b"/dev/null\0".as_ptr() as *const libc::c_char,
                                libc::O_RDWR,
                            );
                            if dev_null >= 0 {
                                libc::dup2(dev_null, 0);
                                libc::dup2(dev_null, 1);
                                libc::dup2(dev_null, 2);
                                if dev_null > 2 {
                                    libc::close(dev_null);
                                }
                            }
                        }

                        // Continue with PTY setup, passing the captured terminal size
                        let (pty_process, _session) =
                            Self::spawn_new_internal_with_size(session_id, name, cols, rows)?;

                        // Run the PTY handler
                        if let Err(_e) = pty_process.run_detached() {
                            // Can't print errors anymore since stdout is closed
                        }

                        // Clean up when done
                        Session::cleanup(session_id).ok();
                        std::process::exit(0);
                    }
                }
            }
        }
    }

    fn spawn_new_internal_with_size(
        session_id: &str,
        name: Option<String>,
        cols: u16,
        rows: u16,
    ) -> Result<(Self, Session)> {
        // Open PTY using libc directly since nix 0.29 doesn't have pty module
        let (master_fd, slave_fd) = Self::open_pty()?;

        // Set terminal size on slave
        unsafe {
            let winsize = libc::winsize {
                ws_row: rows,
                ws_col: cols,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };
            if libc::ioctl(slave_fd, libc::TIOCSWINSZ as u64, &winsize) < 0 {
                return Err(NdsError::PtyError(
                    "Failed to set terminal size".to_string(),
                ));
            }
        }

        // Set non-blocking on master
        let flags = fcntl(master_fd, FcntlArg::F_GETFL)
            .map_err(|e| NdsError::PtyError(format!("Failed to get flags: {}", e)))?;
        fcntl(
            master_fd,
            FcntlArg::F_SETFL(OFlag::from_bits_truncate(flags) | OFlag::O_NONBLOCK),
        )
        .map_err(|e| NdsError::PtyError(format!("Failed to set non-blocking: {}", e)))?;

        // Create socket for IPC
        let socket_path = Session::socket_dir()?.join(format!("{}.sock", session_id));

        // Remove socket if it exists
        if socket_path.exists() {
            std::fs::remove_file(&socket_path)?;
        }

        let listener = UnixListener::bind(&socket_path)
            .map_err(|e| NdsError::SocketError(format!("Failed to bind socket: {}", e)))?;

        // Fork process
        match unsafe { fork() }.map_err(|e| NdsError::ForkError(e.to_string()))? {
            ForkResult::Parent { child } => {
                // Close slave in parent
                let _ = close(slave_fd);

                // Create session metadata
                let session = Session::with_name(
                    session_id.to_string(),
                    name,
                    child.as_raw(),
                    socket_path.clone(),
                );
                session.save().map_err(|e| {
                    eprintln!("Failed to save session: {}", e);
                    e
                })?;

                let pty_process = PtyProcess {
                    master_fd,
                    pid: child,
                    socket_path,
                    listener: Some(listener),
                    output_buffer: Some(PtyBuffer::new(1024 * 1024)), // 1MB buffer
                };

                Ok((pty_process, session))
            }
            ForkResult::Child => {
                // Close master in child
                let _ = close(master_fd);

                // Create new session
                setsid().map_err(|e| NdsError::ProcessError(format!("setsid failed: {}", e)))?;

                // Make slave the controlling terminal
                unsafe {
                    if libc::ioctl(slave_fd, libc::TIOCSCTTY as u64, 0) < 0 {
                        eprintln!("Failed to set controlling terminal");
                        std::process::exit(1);
                    }
                }

                // Duplicate slave to stdin/stdout/stderr
                dup2(slave_fd, 0)
                    .map_err(|e| NdsError::ProcessError(format!("dup2 stdin failed: {}", e)))?;
                dup2(slave_fd, 1)
                    .map_err(|e| NdsError::ProcessError(format!("dup2 stdout failed: {}", e)))?;
                dup2(slave_fd, 2)
                    .map_err(|e| NdsError::ProcessError(format!("dup2 stderr failed: {}", e)))?;

                // Close original slave
                if slave_fd > 2 {
                    let _ = close(slave_fd);
                }

                // Get shell
                let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());

                // Execute shell
                let shell_cstr = std::ffi::CString::new(shell.as_str()).unwrap();
                let args = vec![shell_cstr.clone()];

                execvp(&shell_cstr, &args)
                    .map_err(|e| NdsError::ProcessError(format!("execvp failed: {}", e)))?;

                // Should never reach here
                unreachable!()
            }
        }
    }

    fn open_pty() -> Result<(RawFd, RawFd)> {
        unsafe {
            // Open PTY master
            let master_fd = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY);
            if master_fd < 0 {
                return Err(NdsError::PtyError("Failed to open PTY master".to_string()));
            }

            // Grant access to slave
            if libc::grantpt(master_fd) < 0 {
                let _ = libc::close(master_fd);
                return Err(NdsError::PtyError("Failed to grant PTY access".to_string()));
            }

            // Unlock slave
            if libc::unlockpt(master_fd) < 0 {
                let _ = libc::close(master_fd);
                return Err(NdsError::PtyError("Failed to unlock PTY".to_string()));
            }

            // Get slave name
            let slave_name = libc::ptsname(master_fd);
            if slave_name.is_null() {
                let _ = libc::close(master_fd);
                return Err(NdsError::PtyError(
                    "Failed to get PTY slave name".to_string(),
                ));
            }

            // Open slave
            let slave_cstr = std::ffi::CStr::from_ptr(slave_name);
            let slave_fd = libc::open(slave_cstr.as_ptr(), libc::O_RDWR);
            if slave_fd < 0 {
                let _ = libc::close(master_fd);
                return Err(NdsError::PtyError("Failed to open PTY slave".to_string()));
            }

            Ok((master_fd, slave_fd))
        }
    }

    pub fn attach_to_session(session: &Session) -> Result<Option<String>> {
        // Save current terminal state
        let stdin_fd = 0;
        let stdin = unsafe { BorrowedFd::borrow_raw(stdin_fd) };
        let original_termios = tcgetattr(&stdin).map_err(|e| {
            NdsError::TerminalError(format!("Failed to get terminal attributes: {}", e))
        })?;

        // Capture current terminal state
        let _terminal_state = TerminalState::capture(stdin_fd)?;

        // Connect to session socket
        let mut socket = session.connect_socket()?;

        // Get current terminal size and send resize command
        let (cols, rows) = terminal::size().map_err(|e| NdsError::TerminalError(e.to_string()))?;

        // Send a special resize command to the daemon
        // Format: \x1b]nds:resize:<cols>:<rows>\x07
        let resize_cmd = format!("\x1b]nds:resize:{}:{}\x07", cols, rows);
        let _ = socket.write_all(resize_cmd.as_bytes());
        let _ = socket.flush();

        // Small delay to let the resize happen
        thread::sleep(std::time::Duration::from_millis(50));

        // Send a sequence to help restore the terminal state
        // First, send Ctrl+L to refresh the display
        let _ = socket.write_all(b"\x0c");
        let _ = socket.flush();

        // Small delay to let the refresh happen
        thread::sleep(std::time::Duration::from_millis(50));

        // Set terminal to raw mode
        let mut raw = original_termios.clone();
        // Manually set raw mode flags
        raw.input_flags = nix::sys::termios::InputFlags::empty();
        raw.output_flags = nix::sys::termios::OutputFlags::empty();
        raw.control_flags |= nix::sys::termios::ControlFlags::CS8;
        raw.local_flags = nix::sys::termios::LocalFlags::empty();
        raw.control_chars[nix::sys::termios::SpecialCharacterIndices::VMIN as usize] = 1;
        raw.control_chars[nix::sys::termios::SpecialCharacterIndices::VTIME as usize] = 0;
        tcsetattr(&stdin, SetArg::TCSANOW, &raw)
            .map_err(|e| NdsError::TerminalError(format!("Failed to set raw mode: {}", e)))?;

        // Create a flag for clean shutdown
        let running = Arc::new(AtomicBool::new(true));
        let r1 = running.clone();
        let r2 = running.clone();

        // Handle Ctrl+C
        ctrlc::set_handler(move || {
            r1.store(false, Ordering::SeqCst);
        })
        .map_err(|e| NdsError::SignalError(format!("Failed to set signal handler: {}", e)))?;

        println!("\r\n[Attached to session {}]\r", session.id);
        println!("[Press Enter then ~d to detach, ~s to switch, ~h for history]\r");

        // Scrollback buffer (max 10MB)
        let scrollback_buffer = Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));

        // Spawn thread to monitor terminal size changes
        let socket_for_resize = socket
            .try_clone()
            .map_err(|e| NdsError::SocketError(format!("Failed to clone socket: {}", e)))?;
        let resize_running = running.clone();
        let _resize_monitor = thread::spawn(move || {
            let mut last_size = (cols, rows);
            let mut socket = socket_for_resize;

            while resize_running.load(Ordering::SeqCst) {
                if let Ok((new_cols, new_rows)) = terminal::size() {
                    if (new_cols, new_rows) != last_size {
                        // Terminal size changed, send resize command
                        let resize_cmd = format!("\x1b]nds:resize:{}:{}\x07", new_cols, new_rows);
                        let _ = socket.write_all(resize_cmd.as_bytes());
                        let _ = socket.flush();
                        last_size = (new_cols, new_rows);
                    }
                }
                thread::sleep(std::time::Duration::from_millis(250));
            }
        });

        // Spawn thread to read from socket and write to stdout
        let socket_clone = socket
            .try_clone()
            .map_err(|e| NdsError::SocketError(format!("Failed to clone socket: {}", e)))?;
        let scrollback_clone = Arc::clone(&scrollback_buffer);
        let socket_to_stdout = thread::spawn(move || {
            let mut socket = socket_clone;
            let mut stdout = io::stdout();
            let mut buffer = [0u8; 4096];

            while r2.load(Ordering::SeqCst) {
                match socket.read(&mut buffer) {
                    Ok(0) => break, // Socket closed
                    Ok(n) => {
                        // Write to stdout
                        if let Err(_) = stdout.write_all(&buffer[..n]) {
                            break;
                        }
                        let _ = stdout.flush();

                        // Add to scrollback buffer
                        let mut scrollback = scrollback_clone.lock().unwrap();
                        scrollback.extend_from_slice(&buffer[..n]);

                        // Trim if too large
                        let scrollback_max = 10 * 1024 * 1024;
                        if scrollback.len() > scrollback_max {
                            let remove = scrollback.len() - scrollback_max;
                            scrollback.drain(..remove);
                        }
                    }
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(ref e) if e.kind() == io::ErrorKind::BrokenPipe => {
                        // Expected when socket is closed, just exit cleanly
                        break;
                    }
                    Err(_) => break,
                }
            }
        });

        // Read from stdin and write to socket
        let mut stdin = io::stdin();
        let mut buffer = [0u8; 1024];

        // SSH-style escape sequence: Enter ~d
        // We'll track if we're at the beginning of a line
        let mut at_line_start = true;
        let mut escape_state = 0; // 0=normal, 1=saw tilde at line start
        let mut escape_time = std::time::Instant::now();

        loop {
            if !running.load(Ordering::SeqCst) {
                break;
            }

            match stdin.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    let mut should_detach = false;
                    let mut should_switch = false;
                    let mut should_scroll = false;
                    let mut data_to_forward = Vec::new();

                    // Check for escape timeout (reset after 1 second)
                    if escape_state == 1
                        && escape_time.elapsed() > std::time::Duration::from_secs(1)
                    {
                        // Timeout - forward the held tilde and reset
                        data_to_forward.push(b'~');
                        escape_state = 0;
                    }

                    // Process each byte for escape sequence
                    for i in 0..n {
                        let byte = buffer[i];

                        match escape_state {
                            0 => {
                                // Normal state
                                if at_line_start && byte == b'~' {
                                    // Start of potential escape sequence
                                    escape_state = 1;
                                    escape_time = std::time::Instant::now();
                                    // Don't forward the tilde yet
                                } else {
                                    // Regular character
                                    data_to_forward.push(byte);
                                    // Update line start tracking
                                    at_line_start =
                                        byte == b'\r' || byte == b'\n' || byte == 10 || byte == 13;
                                }
                            }
                            1 => {
                                // We saw ~ at the beginning of a line
                                if byte == b'd' {
                                    // Detach command ~d
                                    should_detach = true;
                                    break;
                                } else if byte == b's' {
                                    // Switch sessions command ~s
                                    should_switch = true;
                                    break;
                                } else if byte == b'h' {
                                    // History/scrollback command ~h
                                    should_scroll = true;
                                    break;
                                } else if byte == b'~' {
                                    // ~~ means literal tilde
                                    data_to_forward.push(b'~');
                                    escape_state = 0;
                                    at_line_start = false;
                                } else {
                                    // Not an escape sequence, forward tilde and this char
                                    data_to_forward.push(b'~');
                                    data_to_forward.push(byte);
                                    escape_state = 0;
                                    at_line_start =
                                        byte == b'\r' || byte == b'\n' || byte == 10 || byte == 13;
                                }
                            }
                            _ => {
                                escape_state = 0;
                            }
                        }
                    }

                    if should_detach {
                        println!("\r\n[Detaching from session {}]\r", session.id);
                        break;
                    }

                    if should_switch {
                        // Show session switcher
                        println!("\r\n[Session Switcher]\r");

                        // Get list of other sessions
                        match SessionManager::list_sessions() {
                            Ok(sessions) => {
                                let other_sessions: Vec<_> =
                                    sessions.iter().filter(|s| s.id != session.id).collect();

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

                                // Read user selection
                                let mut selection = String::new();
                                if let Ok(_) = io::stdin().read_line(&mut selection) {
                                    if let Ok(num) = selection.trim().parse::<usize>() {
                                        if num > 0 && num <= other_sessions.len() {
                                            // Switch to selected session
                                            let target_session = other_sessions[num - 1];
                                            println!(
                                                "\r\n[Switching to session {}]\r",
                                                target_session.id
                                            );

                                            // Store the target session ID for return
                                            return Ok(Some(target_session.id.clone()));
                                        } else if num == new_option_num {
                                            // Create new session
                                            println!("\r\nEnter name for new session (or press Enter for no name): ");
                                            let _ = io::stdout().flush();

                                            let mut session_name = String::new();
                                            if let Ok(_) = io::stdin().read_line(&mut session_name)
                                            {
                                                let session_name = session_name.trim();
                                                let name = if session_name.is_empty() {
                                                    None
                                                } else {
                                                    Some(session_name.to_string())
                                                };

                                                // Create new session
                                                match SessionManager::create_session_with_name(
                                                    name.clone(),
                                                ) {
                                                    Ok(new_session) => {
                                                        if let Some(ref n) = name {
                                                            println!("\r\n[Created and switching to new session '{}' ({})]", n, new_session.id);
                                                        } else {
                                                            println!("\r\n[Created and switching to new session {}]", new_session.id);
                                                        }

                                                        // Return the new session ID to switch to it
                                                        return Ok(Some(new_session.id.clone()));
                                                    }
                                                    Err(e) => {
                                                        eprintln!(
                                                            "\r\nError creating session: {}\r",
                                                            e
                                                        );
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }

                                // Cancelled or invalid selection
                                println!("\r\n[Continuing current session]\r");
                                escape_state = 0;
                                at_line_start = true;
                            }
                            Err(e) => {
                                eprintln!("\r\nError listing sessions: {}\r", e);
                                escape_state = 0;
                                at_line_start = true;
                            }
                        }
                    }

                    if should_scroll {
                        // Show scrollback viewer
                        println!("\r\n[Opening scrollback viewer...]\r");

                        // Get scrollback content
                        let content = scrollback_buffer.lock().unwrap().clone();

                        // Temporarily restore terminal for viewer
                        let stdin_fd = 0;
                        let stdin = unsafe { BorrowedFd::borrow_raw(stdin_fd) };
                        tcsetattr(&stdin, SetArg::TCSANOW, &original_termios).map_err(|e| {
                            NdsError::TerminalError(format!("Failed to restore terminal: {}", e))
                        })?;

                        // Show scrollback viewer
                        let mut viewer = ScrollbackViewer::new(&content);
                        let _ = viewer.run(); // Ignore errors, just return to session

                        // Re-enter raw mode
                        tcsetattr(&stdin, SetArg::TCSANOW, &raw).map_err(|e| {
                            NdsError::TerminalError(format!("Failed to set raw mode: {}", e))
                        })?;

                        // Refresh display
                        let _ = socket.write_all(b"\x0c"); // Ctrl+L
                        let _ = socket.flush();

                        println!("\r\n[Returned to session]\r");

                        // Reset state
                        escape_state = 0;
                        at_line_start = true;
                    }

                    // Forward the processed data
                    if !data_to_forward.is_empty() {
                        if let Err(e) = socket.write_all(&data_to_forward) {
                            // Check if it's a broken pipe (expected on detach)
                            if e.kind() == io::ErrorKind::BrokenPipe {
                                // This is expected when detaching, just break
                                break;
                            } else {
                                eprintln!("\r\nError writing to socket: {}\r", e);
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("\r\nError reading stdin: {}\r", e);
                    break;
                }
            }
        }

        // Stop the socket reader thread
        running.store(false, Ordering::SeqCst);

        // Close the socket to unblock the reader thread
        drop(socket);

        // Wait for the thread with a timeout
        thread::sleep(std::time::Duration::from_millis(100));
        let _ = socket_to_stdout.join();

        // Restore terminal - do this BEFORE any output
        let stdin_fd = 0;
        let stdin = unsafe { BorrowedFd::borrow_raw(stdin_fd) };

        // First restore the terminal settings
        tcsetattr(&stdin, SetArg::TCSANOW, &original_termios)
            .map_err(|e| NdsError::TerminalError(format!("Failed to restore terminal: {}", e)))?;

        // Ensure we're back in cooked mode
        terminal::disable_raw_mode().ok();

        // Add a small delay to ensure terminal is fully restored
        thread::sleep(std::time::Duration::from_millis(50));

        // Now it's safe to print the detach message
        println!("\n[Detached from session {}]", session.id);

        // Flush stdout to ensure message is displayed
        let _ = io::stdout().flush();

        Ok(None)
    }

    pub fn run_detached(mut self) -> Result<()> {
        let listener = self
            .listener
            .take()
            .ok_or_else(|| NdsError::PtyError("No listener available".to_string()))?;

        // Set listener to non-blocking
        listener.set_nonblocking(true)?;

        let running = Arc::new(AtomicBool::new(true));
        let r = running.clone();

        // Handle cleanup on exit
        ctrlc::set_handler(move || {
            r.store(false, Ordering::SeqCst);
        })
        .map_err(|e| NdsError::SignalError(format!("Failed to set signal handler: {}", e)))?;

        let output_buffer = self
            .output_buffer
            .take()
            .ok_or_else(|| NdsError::PtyError("No output buffer available".to_string()))?;

        // Support multiple concurrent clients
        let mut active_clients: Vec<UnixStream> = Vec::new();
        let mut buffer = [0u8; 4096];

        // Get session ID from socket path
        let session_id = self
            .socket_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        while running.load(Ordering::SeqCst) {
            // Check for new connections
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_nonblocking(true)?;

                    // Notify existing clients about new connection
                    let notification = format!(
                        "\r\n[Another client connected to this session (total: {})]\r\n",
                        active_clients.len() + 1
                    );
                    for client in &mut active_clients {
                        let _ = client.write_all(notification.as_bytes());
                        let _ = client.flush();
                    }

                    // Send buffered output to new client
                    if !output_buffer.is_empty() {
                        let mut buffered_data = Vec::new();
                        output_buffer.drain_to(&mut buffered_data);

                        // Save cursor position, clear screen, and reset
                        let init_sequence = b"\x1b7\x1b[?47h\x1b[2J\x1b[H"; // Save cursor, alt screen, clear, home
                        let _ = stream.write_all(init_sequence);
                        let _ = stream.flush();

                        // Send buffered data in chunks to avoid overwhelming the client
                        for chunk in buffered_data.chunks(4096) {
                            let _ = stream.write_all(chunk);
                            let _ = stream.flush();
                            std::thread::sleep(std::time::Duration::from_millis(1));
                        }

                        // Exit alt screen and restore cursor
                        let restore_sequence = b"\x1b[?47l\x1b8"; // Exit alt screen, restore cursor
                        let _ = stream.write_all(restore_sequence);
                        let _ = stream.flush();

                        // Small delay for terminal to process
                        std::thread::sleep(std::time::Duration::from_millis(50));

                        // Send a full redraw command to the shell
                        let mut master_file = unsafe { File::from_raw_fd(self.master_fd) };
                        let _ = master_file.write_all(b"\x0c"); // Ctrl+L to refresh
                        std::mem::forget(master_file); // Don't close the fd

                        // Give time for the refresh to complete
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    } else {
                        // No buffer, just request a refresh to sync state
                        let mut master_file = unsafe { File::from_raw_fd(self.master_fd) };
                        let _ = master_file.write_all(b"\x0c"); // Ctrl+L to refresh
                        std::mem::forget(master_file); // Don't close the fd
                    }

                    // Add new client to the list
                    active_clients.push(stream);

                    // Update client count in status file
                    let _ = Session::update_client_count(&session_id, active_clients.len());
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    // No new connections
                }
                Err(_e) => {
                    // Error accepting connection, continue
                }
            }

            // Read from PTY master
            let master_file = unsafe { File::from_raw_fd(self.master_fd) };
            let mut master_file_clone = master_file.try_clone()?;
            std::mem::forget(master_file); // Don't close the fd

            match master_file_clone.read(&mut buffer) {
                Ok(0) => {
                    // Child process exited
                    break;
                }
                Ok(n) => {
                    let data = &buffer[..n];

                    // Broadcast to all connected clients
                    if !active_clients.is_empty() {
                        let mut disconnected_indices = Vec::new();

                        for (i, client) in active_clients.iter_mut().enumerate() {
                            if let Err(e) = client.write_all(data) {
                                if e.kind() == io::ErrorKind::BrokenPipe
                                    || e.kind() == io::ErrorKind::ConnectionAborted
                                {
                                    // Mark client for removal
                                    disconnected_indices.push(i);
                                }
                            } else {
                                let _ = client.flush();
                            }
                        }

                        // Remove disconnected clients and notify others
                        if !disconnected_indices.is_empty() {
                            for i in disconnected_indices.iter().rev() {
                                active_clients.remove(*i);
                            }

                            // Update client count in status file
                            let _ = Session::update_client_count(&session_id, active_clients.len());

                            // Notify remaining clients
                            if !active_clients.is_empty() {
                                let notification = format!(
                                    "\r\n[A client disconnected (remaining: {})]\r\n",
                                    active_clients.len()
                                );
                                for client in &mut active_clients {
                                    let _ = client.write_all(notification.as_bytes());
                                    let _ = client.flush();
                                }
                            }
                        }

                        // If all clients disconnected, start buffering
                        if active_clients.is_empty() {
                            output_buffer.push(data);
                        }
                    } else {
                        // No clients connected, buffer the output
                        output_buffer.push(data);
                    }
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    // No data available
                }
                Err(_) => {
                    // Other error, continue
                }
            }

            // Read from clients and write to PTY
            let mut disconnected_indices = Vec::new();

            for (i, client) in active_clients.iter_mut().enumerate() {
                let mut client_buffer = [0u8; 1024];
                match client.read(&mut client_buffer) {
                    Ok(0) => {
                        // Client disconnected
                        disconnected_indices.push(i);
                    }
                    Ok(n) => {
                        let data = &client_buffer[..n];

                        // Check for special NDS commands
                        // Format: \x1b]nds:resize:<cols>:<rows>\x07
                        if n > 10 && data.starts_with(b"\x1b]nds:") {
                            if let Ok(cmd_str) = std::str::from_utf8(data) {
                                if let Some(end_idx) = cmd_str.find('\x07') {
                                    let cmd = &cmd_str[2..end_idx]; // Skip \x1b]
                                    if cmd.starts_with("nds:resize:") {
                                        // Parse resize command
                                        let parts: Vec<&str> =
                                            cmd["nds:resize:".len()..].split(':').collect();
                                        if parts.len() == 2 {
                                            if let (Ok(cols), Ok(rows)) =
                                                (parts[0].parse::<u16>(), parts[1].parse::<u16>())
                                            {
                                                // Resize the PTY
                                                unsafe {
                                                    let winsize = libc::winsize {
                                                        ws_row: rows,
                                                        ws_col: cols,
                                                        ws_xpixel: 0,
                                                        ws_ypixel: 0,
                                                    };
                                                    libc::ioctl(
                                                        self.master_fd,
                                                        libc::TIOCSWINSZ as u64,
                                                        &winsize,
                                                    );
                                                }

                                                // Send SIGWINCH to the child process to notify of resize
                                                let _ = kill(self.pid, Signal::SIGWINCH);

                                                // Don't forward the resize command to the PTY
                                                // But forward any remaining data after the command
                                                if end_idx + 1 < n {
                                                    let remaining = &data[end_idx + 1..];
                                                    if !remaining.is_empty() {
                                                        let mut master_file = unsafe {
                                                            File::from_raw_fd(self.master_fd)
                                                        };
                                                        let _ = master_file.write_all(remaining);
                                                        std::mem::forget(master_file);
                                                        // Don't close the fd
                                                    }
                                                }
                                                continue; // Skip normal forwarding
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // Normal data - forward to PTY
                        let mut master_file = unsafe { File::from_raw_fd(self.master_fd) };
                        let _ = master_file.write_all(data);
                        std::mem::forget(master_file); // Don't close the fd
                    }
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                        // No data available
                    }
                    Err(_) => {
                        // Client error, mark for removal
                        disconnected_indices.push(i);
                    }
                }
            }

            // Remove disconnected clients and notify others
            if !disconnected_indices.is_empty() {
                for i in disconnected_indices.iter().rev() {
                    active_clients.remove(*i);
                }

                // Update client count in status file
                let _ = Session::update_client_count(&session_id, active_clients.len());

                // Notify remaining clients
                if !active_clients.is_empty() {
                    let notification = format!(
                        "\r\n[A client disconnected (remaining: {})]\r\n",
                        active_clients.len()
                    );
                    for client in &mut active_clients {
                        let _ = client.write_all(notification.as_bytes());
                        let _ = client.flush();
                    }
                }
            }

            // Small sleep to prevent busy loop
            thread::sleep(std::time::Duration::from_millis(10));
        }

        Ok(())
    }

    pub fn kill_session(session_id: &str) -> Result<()> {
        let session = Session::load(session_id)?;

        // Send SIGTERM to the process
        kill(Pid::from_raw(session.pid), Signal::SIGTERM)
            .map_err(|e| NdsError::ProcessError(format!("Failed to kill process: {}", e)))?;

        // Wait a moment for graceful shutdown
        thread::sleep(std::time::Duration::from_millis(500));

        // Force kill if still alive
        if Session::is_process_alive(session.pid) {
            kill(Pid::from_raw(session.pid), Signal::SIGKILL).map_err(|e| {
                NdsError::ProcessError(format!("Failed to force kill process: {}", e))
            })?;
        }

        // Clean up session files
        Session::cleanup(session_id)?;

        Ok(())
    }
}

impl Drop for PtyProcess {
    fn drop(&mut self) {
        let _ = close(self.master_fd);
        if let Some(listener) = self.listener.take() {
            drop(listener);
        }
    }
}
