use std::io::{self, Read, Write};
use std::os::unix::io::RawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use nix::fcntl::{fcntl, FcntlArg, OFlag};
use nix::sys::signal::{kill, Signal};
use nix::sys::termios::Termios;
use nix::unistd::{close, dup2, execvp, fork, setsid, ForkResult, Pid};

use super::client::ClientInfo;
use super::health_monitor::{attempt_recovery, HealthMonitor, RecoveryStrategy};
use super::io_handler::{
    spawn_resize_monitor_thread, spawn_socket_to_stdout_thread, PtyIoHandler, ScrollbackHandler,
    DEFAULT_BUFFER_SIZE,
};
use super::session_switcher::{SessionSwitcher, SwitchResult};
use super::socket::{create_listener, get_command_end, parse_nds_command, send_resize_command};
use super::terminal::{
    capture_terminal_state, get_terminal_size, restore_terminal, save_terminal_state, send_refresh,
    send_terminal_refresh_sequences, set_raw_mode, set_stdin_blocking, set_terminal_size,
};
use crate::error::{NdsError, Result};
use crate::pty_buffer::PtyBuffer;
use crate::scrollback::ScrollbackViewer;
use crate::session::Session;

#[derive(Debug, Clone)]
struct TerminalModeTracker {
    cursor_visible: bool,
    application_cursor_keys: bool,
    alternate_screen: bool,
    bracketed_paste: bool,
    tail: Vec<u8>,
}

impl Default for TerminalModeTracker {
    fn default() -> Self {
        TerminalModeTracker {
            cursor_visible: true,
            application_cursor_keys: false,
            alternate_screen: false,
            bracketed_paste: false,
            tail: Vec::with_capacity(16),
        }
    }
}

impl TerminalModeTracker {
    fn observe(&mut self, chunk: &[u8]) {
        if chunk.is_empty() {
            return;
        }

        let mut changes = Vec::new();
        self.scan(chunk, &mut changes);

        if !self.tail.is_empty() {
            let mut combined = Vec::with_capacity(self.tail.len() + chunk.len());
            combined.extend_from_slice(&self.tail);
            combined.extend_from_slice(chunk);
            self.scan(&combined, &mut changes);
        }

        const MAX_TAIL: usize = 7; // longest tracked sequence length minus one
        self.tail.clear();
        let take = chunk.len().min(MAX_TAIL);
        self.tail.extend_from_slice(&chunk[chunk.len() - take..]);

        if trace_enabled() && !changes.is_empty() {
            trace(|| format!("observed sequences: {}", changes.join(", ")));
        }
    }

    fn scan(&mut self, data: &[u8], changes: &mut Vec<&'static str>) {
        if contains_sequence(data, b"\x1b[?25l") {
            if self.cursor_visible {
                self.cursor_visible = false;
                changes.push("?25l");
            }
        }
        if contains_sequence(data, b"\x1b[?25h") {
            if !self.cursor_visible {
                self.cursor_visible = true;
                changes.push("?25h");
            }
        }
        if contains_sequence(data, b"\x1b[?1h") {
            if !self.application_cursor_keys {
                self.application_cursor_keys = true;
                changes.push("?1h");
            }
        }
        if contains_sequence(data, b"\x1b[?1l") {
            if self.application_cursor_keys {
                self.application_cursor_keys = false;
                changes.push("?1l");
            }
        }
        if contains_sequence(data, b"\x1b[?1049h") || contains_sequence(data, b"\x1b[?47h") {
            if !self.alternate_screen {
                self.alternate_screen = true;
                changes.push("?1049h");
            }
        }
        if contains_sequence(data, b"\x1b[?1049l") || contains_sequence(data, b"\x1b[?47l") {
            if self.alternate_screen {
                self.alternate_screen = false;
                changes.push("?1049l");
            }
        }
        if contains_sequence(data, b"\x1b[?2004h") {
            if !self.bracketed_paste {
                self.bracketed_paste = true;
                changes.push("?2004h");
            }
        }
        if contains_sequence(data, b"\x1b[?2004l") {
            if self.bracketed_paste {
                self.bracketed_paste = false;
                changes.push("?2004l");
            }
        }
    }

    fn apply_to_client(&self, client: &mut ClientInfo) -> io::Result<()> {
        let mut seq = Vec::new();
        let mut applied = Vec::new();

        if self.alternate_screen {
            push_sequence(&mut seq, &mut applied, b"\x1b[?1049h", "?1049h");
        } else {
            push_sequence(&mut seq, &mut applied, b"\x1b[?1049l", "?1049l");
        }

        if self.bracketed_paste {
            push_sequence(&mut seq, &mut applied, b"\x1b[?2004h", "?2004h");
        } else {
            push_sequence(&mut seq, &mut applied, b"\x1b[?2004l", "?2004l");
        }

        if self.application_cursor_keys {
            push_sequence(&mut seq, &mut applied, b"\x1b[?1h", "?1h");
        } else {
            push_sequence(&mut seq, &mut applied, b"\x1b[?1l", "?1l");
        }

        if self.cursor_visible {
            push_sequence(&mut seq, &mut applied, b"\x1b[?25h", "?25h");
        } else {
            push_sequence(&mut seq, &mut applied, b"\x1b[?25l", "?25l");
        }

        if !seq.is_empty() {
            client.send_data(&seq)?;
            client.flush_pending()?;
        }

        if trace_enabled() && !applied.is_empty() {
            trace(|| format!("reapplied to client {}: {}", client.id, applied.join(", ")));
        }

        Ok(())
    }
}

fn contains_sequence(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || needle.len() > haystack.len() {
        return false;
    }

    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn send_buffered_output_to_client(
    client: &mut ClientInfo,
    output_buffer: &PtyBuffer,
    io_handler: &PtyIoHandler,
) -> io::Result<()> {
    if !output_buffer.is_empty() {
        let mut buffered_data = Vec::new();
        output_buffer.drain_to(&mut buffered_data);

        if !buffered_data.is_empty() {
            client.send_data(&buffered_data)?;
            client.flush_pending()?;

            if trace_enabled() {
                trace(|| {
                    format!(
                        "replayed {} bytes of scrollback to client {}",
                        buffered_data.len(),
                        client.id
                    )
                });
            }
        }

        // Nudges the PTY to ensure the client sees the latest frame.
        io_handler.send_refresh()?;
    } else {
        io_handler.send_refresh()?;
    }

    Ok(())
}

fn push_sequence(
    seq: &mut Vec<u8>,
    applied: &mut Vec<&'static str>,
    bytes: &[u8],
    label: &'static str,
) {
    seq.extend_from_slice(bytes);
    applied.push(label);
}

fn trace_enabled() -> bool {
    static TRACE: OnceLock<bool> = OnceLock::new();
    *TRACE.get_or_init(|| {
        std::env::var("NDS_TRACE_TERMINAL")
            .map(|v| !v.is_empty() && v != "0")
            .unwrap_or(false)
    })
}

fn trace<F>(msg: F)
where
    F: FnOnce() -> String,
{
    if trace_enabled() {
        eprintln!("[NDS trace] {}", msg());
    }
}

pub struct PtyProcess {
    pub master_fd: RawFd,
    pub pid: Pid,
    pub socket_path: PathBuf,
    listener: Option<UnixListener>,
    output_buffer: Option<PtyBuffer>,
    #[allow(dead_code)]
    shell_pid: Option<Pid>, // Track the actual shell process
    #[allow(dead_code)]
    session_id: String, // Store session ID for restart
}

impl PtyProcess {
    /// Open a new PTY pair (master and slave)
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

    /// Spawn a new detached session
    pub fn spawn_new_detached(session_id: &str) -> Result<Session> {
        Self::spawn_new_detached_with_name(session_id, None)
    }

    /// Spawn a new detached session with a custom name
    pub fn spawn_new_detached_with_name(session_id: &str, name: Option<String>) -> Result<Session> {
        // Capture terminal size BEFORE detaching using proper ioctl
        let (cols, rows) = get_terminal_size().unwrap_or((80, 24));

        // First fork to create intermediate process
        match unsafe { fork() }
            .map_err(|e| NdsError::ForkError(format!("First fork failed: {}", e)))?
        {
            ForkResult::Parent { child: _ } => {
                // Wait for the intermediate process to complete
                thread::sleep(Duration::from_millis(200));

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

                        // Get our own PID (the daemon process that will manage the PTY)
                        let daemon_pid = std::process::id() as i32;

                        // Continue with PTY setup, passing the captured terminal size and daemon PID
                        let (pty_process, _session) = Self::spawn_new_internal_with_size(
                            session_id, name, cols, rows, daemon_pid,
                        )?;

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
        daemon_pid: i32,
    ) -> Result<(Self, Session)> {
        // Open PTY
        let (master_fd, slave_fd) = Self::open_pty()?;

        // Set terminal size on slave
        set_terminal_size(slave_fd, cols, rows)?;

        // Set non-blocking on master
        let flags = fcntl(master_fd, FcntlArg::F_GETFL)
            .map_err(|e| NdsError::PtyError(format!("Failed to get flags: {}", e)))?;
        fcntl(
            master_fd,
            FcntlArg::F_SETFL(OFlag::from_bits_truncate(flags) | OFlag::O_NONBLOCK),
        )
        .map_err(|e| NdsError::PtyError(format!("Failed to set non-blocking: {}", e)))?;

        // Create socket for IPC
        let (listener, socket_path) = create_listener(session_id)?;

        // Fork process
        match unsafe { fork() }.map_err(|e| NdsError::ForkError(e.to_string()))? {
            ForkResult::Parent { child } => {
                // Close slave in parent
                let _ = close(slave_fd);

                // Create session metadata with daemon PID (not child shell PID)
                // This ensures we track the PTY manager process, not the shell
                let session = Session::with_name(
                    session_id.to_string(),
                    name,
                    daemon_pid, // Use daemon PID instead of child PID
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
                    output_buffer: Some(PtyBuffer::new(2 * 1024 * 1024)), // 2MB buffer for better performance
                    shell_pid: Some(child),                               // Initially the shell PID
                    session_id: session_id.to_string(),
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

                // Set environment variables for session tracking and isolation
                std::env::set_var("NDS_SESSION_ID", session_id);
                if let Some(ref session_name) = name {
                    std::env::set_var("NDS_SESSION_NAME", session_name);
                } else {
                    std::env::set_var("NDS_SESSION_NAME", session_id);
                }

                // Set restrictive umask for session isolation
                unsafe {
                    libc::umask(0o077); // Only owner can read/write/execute new files
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

    /// Attach to an existing session
    pub fn attach_to_session(session: &Session) -> Result<Option<String>> {
        // Set environment variables
        std::env::set_var("NDS_SESSION_ID", &session.id);
        std::env::set_var(
            "NDS_SESSION_NAME",
            session.name.as_ref().unwrap_or(&session.id),
        );

        // Save current terminal state
        let stdin_fd = 0;
        let original_termios = save_terminal_state(stdin_fd)?;

        // Capture current terminal state for restoration
        let _terminal_state = capture_terminal_state(stdin_fd)?;

        // Connect to session socket
        let mut socket = session.connect_socket()?;

        // Get current terminal size and send resize command
        let (cols, rows) = get_terminal_size()?;
        send_resize_command(&mut socket, cols, rows)?;
        thread::sleep(Duration::from_millis(50));

        // Don't send refresh - it disrupts running applications like htop
        // send_refresh(&mut socket)?;
        // thread::sleep(Duration::from_millis(50));

        // Create a flag for clean shutdown
        let running = Arc::new(AtomicBool::new(true));
        let r1 = running.clone();
        let r2 = running.clone();

        // Flag to pause stdout output during session picker
        let paused = Arc::new(AtomicBool::new(false));
        let paused_clone = paused.clone();

        // Handle Ctrl+C
        ctrlc::set_handler(move || {
            r1.store(false, Ordering::SeqCst);
        })
        .map_err(|e| NdsError::SignalError(format!("Failed to set signal handler: {}", e)))?;

        // Set terminal to raw mode AFTER setting up signal handler
        set_raw_mode(stdin_fd, &original_termios)?;

        // Don't print messages that can corrupt htop display
        // These messages interfere with full-screen applications
        // println!("\r\n[Attached to session {}]\r", session.id);
        // println!("[Press Enter then ~d to detach, ~s to switch, ~h for history]\r");

        // Create scrollback handler
        let scrollback = ScrollbackHandler::new(10 * 1024 * 1024); // 10MB

        // Spawn resize monitor thread
        let socket_for_resize = socket
            .try_clone()
            .map_err(|e| NdsError::SocketError(format!("Failed to clone socket: {}", e)))?;
        let resize_running = running.clone();
        let _resize_monitor =
            spawn_resize_monitor_thread(socket_for_resize, resize_running, (cols, rows));

        // Spawn socket to stdout thread
        let socket_clone = socket
            .try_clone()
            .map_err(|e| NdsError::SocketError(format!("Failed to clone socket: {}", e)))?;
        let socket_to_stdout = spawn_socket_to_stdout_thread(
            socket_clone,
            r2,
            scrollback.get_shared_buffer(),
            paused_clone,
        );

        // Don't set stdin to non-blocking - keep it blocking
        // We'll handle the non-blocking behavior in the read loop

        // Main input loop
        let result = Self::handle_input_loop(
            &mut socket,
            session,
            &original_termios,
            &running,
            &scrollback,
            &paused,
        );

        // Clean up
        running.store(false, Ordering::SeqCst);
        let _ = socket.shutdown(std::net::Shutdown::Both);
        drop(socket);
        thread::sleep(Duration::from_millis(50));
        let _ = socket_to_stdout.join();

        // Restore terminal
        restore_terminal(stdin_fd, &original_termios)?;

        // IMPORTANT: Reset stdin to blocking mode to fix session switching issues
        set_stdin_blocking(stdin_fd)?;

        // Clear environment variables
        std::env::remove_var("NDS_SESSION_ID");
        std::env::remove_var("NDS_SESSION_NAME");

        println!("\n[Detached from session {}]", session.id);
        let _ = io::stdout().flush();

        result
    }

    fn handle_input_loop(
        socket: &mut UnixStream,
        session: &Session,
        original_termios: &Termios,
        running: &Arc<AtomicBool>,
        scrollback: &ScrollbackHandler,
        paused: &Arc<AtomicBool>,
    ) -> Result<Option<String>> {
        let stdin_fd = 0i32;
        let mut buffer = [0u8; 1024]; // Use smaller buffer for more responsive input

        // SSH-style escape sequence tracking
        let mut at_line_start = true;
        let mut escape_state = 0; // 0=normal, 1=saw tilde at line start
        let mut escape_time = Instant::now();

        // Use poll to check for input availability
        use nix::poll::{poll, PollFd, PollFlags, PollTimeout};

        loop {
            if !running.load(Ordering::SeqCst) {
                break;
            }

            // Poll stdin with a short timeout
            use std::os::unix::io::BorrowedFd;
            let stdin_borrowed = unsafe { BorrowedFd::borrow_raw(stdin_fd) };
            let mut poll_fds = [PollFd::new(stdin_borrowed, PollFlags::POLLIN)];
            let poll_result = poll(&mut poll_fds, PollTimeout::try_from(10).unwrap()); // 10ms timeout

            match poll_result {
                Ok(0) => {
                    // Timeout, no data available
                    continue;
                }
                Ok(_) => {
                    // Data is available, read it
                    let read_result = unsafe {
                        libc::read(
                            stdin_fd,
                            buffer.as_mut_ptr() as *mut libc::c_void,
                            buffer.len(),
                        )
                    };

                    match read_result {
                        0 => {
                            // EOF (Ctrl+D) - treat as detach
                            // Don't print anything that could corrupt the display
                            running.store(false, Ordering::SeqCst);
                            break;
                        }
                        n if n > 0 => {
                            let n = n as usize;
                            let (should_detach, should_switch, should_scroll, data_to_forward) =
                                Self::process_input(
                                    &buffer[..n],
                                    &mut at_line_start,
                                    &mut escape_state,
                                    &mut escape_time,
                                );

                            if should_detach {
                                // Don't print anything that could corrupt the display
                                running.store(false, Ordering::SeqCst);
                                break;
                            }

                            if should_switch {
                                // Pause socket-to-stdout thread to prevent overwriting
                                paused.store(true, Ordering::SeqCst);

                                // Wait a bit for current output to finish
                                thread::sleep(Duration::from_millis(50));

                                // Stop forwarding PTY output to prevent display corruption
                                let switcher =
                                    SessionSwitcher::new(session, stdin_fd, original_termios);

                                // Temporarily restore terminal for switcher UI
                                restore_terminal(stdin_fd, original_termios)?;

                                let switch_result = switcher.show_switcher()?;

                                // Re-enter raw mode after switcher
                                set_raw_mode(stdin_fd, original_termios)?;

                                // Resume socket-to-stdout thread
                                paused.store(false, Ordering::SeqCst);

                                match switch_result {
                                    SwitchResult::SwitchTo(target_id) => {
                                        return Ok(Some(target_id));
                                    }
                                    SwitchResult::Continue => {
                                        escape_state = 0;
                                        at_line_start = true;
                                        // Send refresh to redraw the terminal
                                        send_terminal_refresh_sequences(socket)?;
                                    }
                                }
                            }

                            if should_scroll {
                                Self::show_scrollback_viewer(original_termios, socket, scrollback)?;
                                escape_state = 0;
                                at_line_start = true;
                            }

                            // Forward the processed data
                            if !data_to_forward.is_empty() {
                                if let Err(e) = socket.write_all(&data_to_forward) {
                                    if e.kind() == io::ErrorKind::BrokenPipe {
                                        break;
                                    } else {
                                        eprintln!("\r\nError writing to socket: {}\r", e);
                                        break;
                                    }
                                }
                            }
                        }
                        _ => {
                            // Error reading
                            let err = io::Error::last_os_error();
                            if err.kind() != io::ErrorKind::Interrupted {
                                return Err(NdsError::Io(err));
                            }
                        }
                    }
                }
                Err(e) => {
                    // Poll error
                    eprintln!("Poll error: {:?}", e);
                    return Err(NdsError::Io(io::Error::new(
                        io::ErrorKind::Other,
                        format!("Poll error: {:?}", e),
                    )));
                }
            }
        }

        Ok(None)
    }

    fn process_input(
        buffer: &[u8],
        at_line_start: &mut bool,
        escape_state: &mut u8,
        escape_time: &mut Instant,
    ) -> (bool, bool, bool, Vec<u8>) {
        let mut should_detach = false;
        let mut should_switch = false;
        let mut should_scroll = false;
        let mut data_to_forward = Vec::new();

        // Check for escape timeout (reset after 1 second)
        if *escape_state == 1 && escape_time.elapsed() > Duration::from_secs(1) {
            // Timeout - forward the held tilde and reset
            data_to_forward.push(b'~');
            *escape_state = 0;
        }

        // Process each byte for escape sequence
        for &byte in buffer {
            // Check for Ctrl+D (ASCII 4) - detach this client only
            if byte == 0x04 {
                should_detach = true;
                break;
            }

            match *escape_state {
                0 => {
                    // Normal state
                    if *at_line_start && byte == b'~' {
                        // Start of potential escape sequence
                        *escape_state = 1;
                        *escape_time = Instant::now();
                        // Don't forward the tilde yet
                    } else {
                        // Regular character
                        data_to_forward.push(byte);
                        // Update line start tracking - we're at line start after Enter key
                        *at_line_start = byte == b'\r' || byte == b'\n';
                    }
                }
                1 => {
                    // We saw ~ at the beginning of a line
                    match byte {
                        b'd' => {
                            should_detach = true;
                            break;
                        }
                        b's' => {
                            should_switch = true;
                            break;
                        }
                        b'h' => {
                            should_scroll = true;
                            break;
                        }
                        b'~' => {
                            // ~~ means literal tilde
                            data_to_forward.push(b'~');
                            *escape_state = 0;
                            *at_line_start = false;
                        }
                        _ => {
                            // Not an escape sequence, forward tilde and this char
                            data_to_forward.push(b'~');
                            data_to_forward.push(byte);
                            *escape_state = 0;
                            *at_line_start =
                                byte == b'\r' || byte == b'\n' || byte == 10 || byte == 13;
                        }
                    }
                }
                _ => {
                    *escape_state = 0;
                }
            }
        }

        (should_detach, should_switch, should_scroll, data_to_forward)
    }

    fn show_scrollback_viewer(
        original_termios: &Termios,
        socket: &mut UnixStream,
        scrollback: &ScrollbackHandler,
    ) -> Result<()> {
        use nix::sys::termios::{tcsetattr, SetArg};
        use std::os::unix::io::BorrowedFd;

        println!("\r\n[Opening scrollback viewer...]\r");

        // Get scrollback content
        let content = scrollback.get_buffer();

        // Temporarily restore terminal for viewer
        let stdin_fd = 0;
        let stdin = unsafe { BorrowedFd::borrow_raw(stdin_fd) };

        // Get current raw mode settings
        let raw_termios = nix::sys::termios::tcgetattr(&stdin)?;

        // Restore to original mode for viewer
        tcsetattr(&stdin, SetArg::TCSANOW, original_termios)?;

        // Show scrollback viewer
        let mut viewer = ScrollbackViewer::new(&content);
        let _ = viewer.run(); // Ignore errors, just return to session

        // Re-enter raw mode
        tcsetattr(&stdin, SetArg::TCSANOW, &raw_termios)?;

        // Refresh display
        send_refresh(socket)?;
        println!("\r\n[Returned to session]\r");

        Ok(())
    }

    /// Run the detached PTY handler
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
        let mut active_clients: Vec<ClientInfo> = Vec::new();
        let mut buffer = [0u8; DEFAULT_BUFFER_SIZE]; // Use 16KB buffer
        let mut terminal_modes = TerminalModeTracker::default();

        // Get session ID from socket path
        let session_id = self
            .socket_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        // Create IO handler
        let io_handler = PtyIoHandler::new(self.master_fd);

        // Create health monitor
        let health_monitor = HealthMonitor::new();
        let _monitor_thread = health_monitor.start_monitoring(300); // 5 minutes timeout

        // Track consecutive errors for recovery
        let mut consecutive_pty_errors = 0;
        let max_consecutive_errors = 10;
        let mut last_recovery_attempt = Instant::now();
        let mut last_client_health_check = Instant::now();

        while running.load(Ordering::SeqCst) {
            // Check for new connections (non-critical, ignore errors)
            let _ = self.handle_new_connections(
                &listener,
                &mut active_clients,
                &output_buffer,
                &io_handler,
                &session_id,
                &terminal_modes,
            );

            // Read from PTY master and broadcast
            match self.read_from_pty(&io_handler, &mut buffer) {
                Ok(Some(data)) => {
                    consecutive_pty_errors = 0; // Reset error counter on success
                    health_monitor.update_activity(); // Update health status
                    terminal_modes.observe(&data);
                    let _ = self.broadcast_to_clients(
                        &mut active_clients,
                        &data,
                        &output_buffer,
                        &session_id,
                    );
                }
                Ok(None) => {
                    // No data available, this is normal
                }
                Err(e) => {
                    // Handle PTY errors gracefully
                    consecutive_pty_errors += 1;

                    // Attempt recovery every 5 seconds
                    if last_recovery_attempt.elapsed() > Duration::from_secs(5) {
                        // Try different recovery strategies
                        let _ = attempt_recovery(RecoveryStrategy::RefreshTerminal, self.master_fd);
                        let _ = attempt_recovery(RecoveryStrategy::ResetBuffers, self.master_fd);
                        last_recovery_attempt = Instant::now();
                    }

                    if consecutive_pty_errors >= max_consecutive_errors {
                        // Too many consecutive errors, PTY might be dead
                        eprintln!(
                            "PTY appears to be dead after {} errors: {}",
                            consecutive_pty_errors, e
                        );

                        // Check if session is healthy according to monitor
                        if !health_monitor.is_healthy() {
                            eprintln!("Health monitor confirms session is unhealthy, terminating");
                            return Err(e);
                        }

                        // Give it one more chance if health monitor thinks it's okay
                        consecutive_pty_errors = max_consecutive_errors - 1;
                    }

                    // Try to recover by sleeping a bit longer
                    thread::sleep(Duration::from_millis(100));
                }
            }

            // Read from clients and handle input (non-critical, ignore errors)
            let _ = self.handle_client_input(&mut active_clients, &io_handler, &session_id);

            // Opportunistically flush any queued output so slow terminals catch up
            let _ = self.flush_pending_clients(&mut active_clients, &session_id);

            // Periodic client health check every 10 seconds
            if last_client_health_check.elapsed() > Duration::from_secs(10) {
                self.check_client_health(&mut active_clients, &session_id);
                last_client_health_check = Instant::now();
            }

            // Small sleep to prevent busy loop
            thread::sleep(Duration::from_millis(10));
        }

        // Stop health monitoring
        health_monitor.stop_monitoring();

        Ok(())
    }

    fn handle_new_connections(
        &self,
        listener: &UnixListener,
        active_clients: &mut Vec<ClientInfo>,
        output_buffer: &PtyBuffer,
        io_handler: &PtyIoHandler,
        session_id: &str,
        terminal_modes: &TerminalModeTracker,
    ) -> Result<()> {
        match listener.accept() {
            Ok((stream, _)) => {
                // Switch to non-blocking immediately so we never block the daemon.
                stream.set_nonblocking(true)?;

                let mut client = ClientInfo::new(stream);

                if let Err(e) = terminal_modes.apply_to_client(&mut client) {
                    eprintln!(
                        "Warning: failed to reapply terminal modes for client {}: {}",
                        client.id, e
                    );
                }

                // Don't send notifications - they corrupt the display
                if let Err(e) =
                    send_buffered_output_to_client(&mut client, output_buffer, io_handler)
                {
                    eprintln!(
                        "Warning: failed to send buffered output to new client {}: {}",
                        client.id, e
                    );
                }

                let _ = client.flush_pending();

                active_clients.push(client);

                // Update client count in status file
                let _ = Session::update_client_count(session_id, active_clients.len());
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                // No new connections
            }
            Err(_) => {
                // Error accepting connection, continue
            }
        }
        Ok(())
    }

    fn read_from_pty(
        &self,
        io_handler: &PtyIoHandler,
        buffer: &mut [u8],
    ) -> Result<Option<Vec<u8>>> {
        match io_handler.read_from_pty(buffer) {
            Ok(0) => {
                // Shell exited, but don't kill the daemon!
                // Mark that shell needs restart when client connects
                eprintln!("Shell process exited, session remains alive for restart");
                Ok(None)
            }
            Ok(n) => Ok(Some(buffer[..n].to_vec())),
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(NdsError::Io(e)),
        }
    }

    fn broadcast_to_clients(
        &self,
        active_clients: &mut Vec<ClientInfo>,
        data: &[u8],
        output_buffer: &PtyBuffer,
        session_id: &str,
    ) -> Result<()> {
        if !active_clients.is_empty() {
            let mut disconnected_indices = Vec::new();

            for (i, client) in active_clients.iter_mut().enumerate() {
                let write_result = client.flush_pending().and_then(|_| client.send_data(data));

                if let Err(e) = write_result {
                    match e.kind() {
                        io::ErrorKind::BrokenPipe
                        | io::ErrorKind::ConnectionAborted
                        | io::ErrorKind::ConnectionReset
                        | io::ErrorKind::WriteZero => {
                            disconnected_indices.push(i);
                        }
                        _ => {
                            eprintln!("Warning: failed to write to client {}: {}", client.id, e);
                            disconnected_indices.push(i);
                        }
                    }
                }
            }

            // Remove disconnected clients
            if !disconnected_indices.is_empty() {
                self.handle_client_disconnections(
                    active_clients,
                    disconnected_indices,
                    session_id,
                )?;
            }

            // Buffer if no clients
            if active_clients.is_empty() {
                output_buffer.push(data);
            }
        } else {
            // No clients connected, buffer the output
            output_buffer.push(data);
        }
        Ok(())
    }

    fn handle_client_disconnections(
        &self,
        active_clients: &mut Vec<ClientInfo>,
        disconnected_indices: Vec<usize>,
        session_id: &str,
    ) -> Result<()> {
        for i in disconnected_indices.iter().rev() {
            active_clients.remove(*i);
        }

        // Update client count
        let _ = Session::update_client_count(session_id, active_clients.len());

        // Don't send disconnect notifications - just refresh and resize
        if !active_clients.is_empty() {
            // Send refresh sequence to remaining clients
            for client in active_clients.iter_mut() {
                let _ = send_terminal_refresh_sequences(&mut client.stream);
                let _ = client.stream.flush();
            }

            // Resize to smallest terminal
            self.resize_to_smallest(active_clients)?;
        }
        Ok(())
    }

    fn flush_pending_clients(
        &self,
        active_clients: &mut Vec<ClientInfo>,
        session_id: &str,
    ) -> Result<()> {
        if active_clients.is_empty() {
            return Ok(());
        }

        let mut disconnected_indices = Vec::new();

        for (i, client) in active_clients.iter_mut().enumerate() {
            if let Err(e) = client.flush_pending() {
                match e.kind() {
                    io::ErrorKind::BrokenPipe
                    | io::ErrorKind::ConnectionAborted
                    | io::ErrorKind::ConnectionReset
                    | io::ErrorKind::WriteZero => disconnected_indices.push(i),
                    _ => {
                        eprintln!("Warning: failed to flush client {}: {}", client.id, e);
                        disconnected_indices.push(i);
                    }
                }
            }
        }

        if !disconnected_indices.is_empty() {
            self.handle_client_disconnections(active_clients, disconnected_indices, session_id)?;
        }

        Ok(())
    }

    fn resize_to_smallest(&self, active_clients: &[ClientInfo]) -> Result<()> {
        let mut min_cols = u16::MAX;
        let mut min_rows = u16::MAX;

        for client in active_clients {
            min_cols = min_cols.min(client.cols);
            min_rows = min_rows.min(client.rows);
        }

        if min_cols != u16::MAX && min_rows != u16::MAX {
            set_terminal_size(self.master_fd, min_cols, min_rows)?;
            let _ = kill(self.pid, Signal::SIGWINCH);

            // Send refresh
            let io_handler = PtyIoHandler::new(self.master_fd);
            let _ = io_handler.send_refresh();
        }
        Ok(())
    }

    /// Check if clients are still healthy and remove dead ones
    fn check_client_health(&self, active_clients: &mut Vec<ClientInfo>, session_id: &str) {
        let mut dead_clients = Vec::new();

        for (i, client) in active_clients.iter_mut().enumerate() {
            // Try to send a zero-byte write to check if the socket is still alive
            if client.stream.write(&[]).is_err() {
                dead_clients.push(i);
            }
        }

        if !dead_clients.is_empty() {
            // Remove dead clients in reverse order
            for i in dead_clients.iter().rev() {
                active_clients.remove(*i);
            }

            // Update client count
            let _ = Session::update_client_count(session_id, active_clients.len());

            // Notify remaining clients
            if !active_clients.is_empty() {
                let notification = format!(
                    "\r\n[Cleaned up {} dead connection(s), {} client(s) remaining]\r\n",
                    dead_clients.len(),
                    active_clients.len()
                );
                for client in active_clients.iter_mut() {
                    let _ = client.stream.write_all(notification.as_bytes());
                    let _ = client.stream.flush();
                }
            }
        }
    }

    fn handle_client_input(
        &self,
        active_clients: &mut Vec<ClientInfo>,
        io_handler: &PtyIoHandler,
        session_id: &str,
    ) -> Result<()> {
        let mut disconnected_indices = Vec::new();
        let mut client_buffer = [0u8; DEFAULT_BUFFER_SIZE]; // Use 16KB buffer
        let mut pending_disconnects = Vec::new(); // Track clients to disconnect

        // Store the count before the loop
        let client_count = active_clients.len();

        for (i, client) in active_clients.iter_mut().enumerate() {
            match client.stream.read(&mut client_buffer) {
                Ok(0) => {
                    disconnected_indices.push(i);
                }
                Ok(n) => {
                    let data = &client_buffer[..n];

                    // Check for NDS commands
                    if let Some((cmd, args)) = parse_nds_command(data) {
                        if cmd == "resize" && args.len() == 2 {
                            if let (Ok(cols), Ok(rows)) =
                                (args[0].parse::<u16>(), args[1].parse::<u16>())
                            {
                                client.cols = cols;
                                client.rows = rows;
                                set_terminal_size(self.master_fd, cols, rows)?;
                                let _ = kill(self.pid, Signal::SIGWINCH);

                                // Forward any remaining data after command
                                if let Some(end_idx) = get_command_end(data) {
                                    if end_idx < n {
                                        io_handler.write_to_pty(&data[end_idx..])?;
                                    }
                                }
                                continue;
                            }
                        } else if cmd == "list_clients" {
                            // Handle list clients command
                            // Just send a basic count for now due to borrow checker limitations
                            let response = format!("Connected clients: {}\r\n", client_count);
                            let _ = client.stream.write_all(response.as_bytes());
                            let _ = client.stream.flush();
                            continue; // Don't forward to PTY
                        } else if cmd == "disconnect_client" && !args.is_empty() {
                            // Handle disconnect client command
                            let target_id = args[0].to_string();
                            let current_id = client.id.clone();

                            let response = if current_id == target_id {
                                "Cannot disconnect yourself. Use ~d to detach.\r\n".to_string()
                            } else {
                                // Mark for disconnection after loop completes
                                pending_disconnects.push(target_id.clone());
                                format!("Client {} will be disconnected\r\n", target_id)
                            };

                            let _ = client.stream.write_all(response.as_bytes());
                            let _ = client.stream.flush();
                            continue; // Don't forward to PTY
                        }
                    }

                    // Normal data - forward to PTY
                    // Ignore write errors to prevent session death from transient issues
                    if let Err(e) = io_handler.write_to_pty(data) {
                        eprintln!("Warning: Failed to write to PTY: {}", e);
                        // Don't propagate the error, just log it
                    }
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    // No data available
                }
                Err(_) => {
                    disconnected_indices.push(i);
                }
            }
        }

        // Handle pending disconnects from disconnect_client commands
        for target_id in pending_disconnects {
            if let Some(idx) = active_clients.iter().position(|c| c.id == target_id) {
                // Notify the target client
                let _ = active_clients[idx]
                    .stream
                    .write_all(b"\r\n[You have been disconnected by another client]\r\n");
                let _ = active_clients[idx].stream.flush();
                let _ = active_clients[idx]
                    .stream
                    .shutdown(std::net::Shutdown::Both);
                disconnected_indices.push(idx);
            }
        }

        // Handle disconnections
        if !disconnected_indices.is_empty() {
            self.handle_client_disconnections(active_clients, disconnected_indices, session_id)?;
        }

        Ok(())
    }

    /// Format the client list for display
    #[allow(dead_code)]
    fn format_client_list(&self, clients: &[ClientInfo]) -> String {
        if clients.is_empty() {
            return "No clients connected\r\n".to_string();
        }

        let mut output = format!("Connected clients ({}):\r\n\r\n", clients.len());
        output.push_str("ID       | Size    | Connected    | Duration\r\n");
        output.push_str("---------|---------|--------------|----------\r\n");

        for client in clients {
            output.push_str(&format!(
                "{:<8} | {}x{:<3} | {} | ",
                client.id,
                client.cols,
                client.rows,
                client.connected_at.format("%H:%M:%S")
            ));

            let duration = chrono::Utc::now().signed_duration_since(client.connected_at);
            let hours = duration.num_hours();
            let minutes = (duration.num_minutes() % 60) as u32;
            let seconds = (duration.num_seconds() % 60) as u32;

            if hours > 0 {
                output.push_str(&format!("{}h{}m\r\n", hours, minutes));
            } else if minutes > 0 {
                output.push_str(&format!("{}m{}s\r\n", minutes, seconds));
            } else {
                output.push_str(&format!("{}s\r\n", seconds));
            }
        }

        output
    }

    /// Disconnect a client by ID
    #[allow(dead_code)]
    fn disconnect_client_by_id(
        &self,
        clients: &mut Vec<ClientInfo>,
        target_id: &str,
        requester_index: usize,
    ) -> String {
        // Find the client with the target ID
        if let Some(target_index) = clients.iter().position(|c| c.id == target_id) {
            if target_index == requester_index {
                return "Cannot disconnect yourself. Use ~d to detach.\r\n".to_string();
            }

            // Notify the target client before disconnecting
            let _ = clients[target_index]
                .stream
                .write_all(b"\r\n[You have been disconnected by another client]\r\n");
            let _ = clients[target_index].stream.flush();
            let _ = clients[target_index]
                .stream
                .shutdown(std::net::Shutdown::Both);

            format!("Client {} disconnected successfully\r\n", target_id)
        } else {
            format!("Client {} not found\r\n", target_id)
        }
    }

    /// Kill a session by its ID
    pub fn kill_session(session_id: &str) -> Result<()> {
        let session = Session::load(session_id)?;

        // Send SIGTERM to the process
        kill(Pid::from_raw(session.pid), Signal::SIGTERM)
            .map_err(|e| NdsError::ProcessError(format!("Failed to kill process: {}", e)))?;

        // Wait a moment for graceful shutdown
        thread::sleep(Duration::from_millis(500));

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

/// Static function to format client list without borrowing
#[allow(dead_code)]
fn format_client_list_static(
    client_infos: &[(String, u16, u16, chrono::DateTime<chrono::Utc>)],
) -> String {
    if client_infos.is_empty() {
        return "No clients connected\r\n".to_string();
    }

    let mut output = format!("Connected clients ({}):\r\n\r\n", client_infos.len());
    output.push_str("ID       | Size    | Connected    | Duration\r\n");
    output.push_str("---------|---------|--------------|----------\r\n");

    for (id, cols, rows, connected_at) in client_infos {
        output.push_str(&format!(
            "{:<8} | {}x{:<3} | {} | ",
            id,
            cols,
            rows,
            connected_at.format("%H:%M:%S")
        ));

        let duration = chrono::Utc::now().signed_duration_since(*connected_at);
        let hours = duration.num_hours();
        let minutes = (duration.num_minutes() % 60) as u32;
        let seconds = (duration.num_seconds() % 60) as u32;

        if hours > 0 {
            output.push_str(&format!("{}h{}m\r\n", hours, minutes));
        } else if minutes > 0 {
            output.push_str(&format!("{}m{}s\r\n", minutes, seconds));
        } else {
            output.push_str(&format!("{}s\r\n", seconds));
        }
    }

    output
}

impl Drop for PtyProcess {
    fn drop(&mut self) {
        let _ = close(self.master_fd);
        if let Some(listener) = self.listener.take() {
            drop(listener);
        }
    }
}

// Public convenience functions for backward compatibility
#[allow(dead_code)]
pub fn spawn_new_detached(session_id: &str) -> Result<Session> {
    PtyProcess::spawn_new_detached(session_id)
}

#[allow(dead_code)]
pub fn spawn_new_detached_with_name(session_id: &str, name: Option<String>) -> Result<Session> {
    PtyProcess::spawn_new_detached_with_name(session_id, name)
}

#[allow(dead_code)]
pub fn kill_session(session_id: &str) -> Result<()> {
    PtyProcess::kill_session(session_id)
}
