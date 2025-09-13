use std::io::{self, Read, Write};
use std::os::unix::io::RawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crossterm::terminal;
use nix::fcntl::{fcntl, FcntlArg, OFlag};
use nix::sys::signal::{kill, Signal};
use nix::sys::termios::Termios;
use nix::unistd::{close, dup2, execvp, fork, setsid, ForkResult, Pid};

use super::client::ClientInfo;
use super::io_handler::{
    send_buffered_output, spawn_resize_monitor_thread, spawn_socket_to_stdout_thread, PtyIoHandler,
    ScrollbackHandler, DEFAULT_BUFFER_SIZE,
};
use super::session_switcher::{SessionSwitcher, SwitchResult};
use super::socket::{create_listener, get_command_end, parse_nds_command, send_resize_command};
use super::terminal::{
    capture_terminal_state, get_terminal_size, restore_terminal, save_terminal_state, send_refresh,
    send_terminal_refresh_sequences, set_raw_mode, set_stdin_nonblocking, set_terminal_size,
};
use crate::error::{NdsError, Result};
use crate::pty_buffer::PtyBuffer;
use crate::scrollback::ScrollbackViewer;
use crate::session::Session;

pub struct PtyProcess {
    pub master_fd: RawFd,
    pub pid: Pid,
    pub socket_path: PathBuf,
    listener: Option<UnixListener>,
    output_buffer: Option<PtyBuffer>,
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
        // Capture terminal size BEFORE detaching
        let (cols, rows) = terminal::size().unwrap_or((80, 24));

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
                    output_buffer: Some(PtyBuffer::new(2 * 1024 * 1024)), // 2MB buffer for better performance
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

        // Send refresh to restore terminal state
        send_refresh(&mut socket)?;
        thread::sleep(Duration::from_millis(50));

        // Set terminal to raw mode
        set_raw_mode(stdin_fd, &original_termios)?;

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
        let socket_to_stdout =
            spawn_socket_to_stdout_thread(socket_clone, r2, scrollback.get_shared_buffer());

        // Set stdin to non-blocking
        set_stdin_nonblocking(stdin_fd)?;

        // Main input loop
        let result = Self::handle_input_loop(
            &mut socket,
            session,
            &original_termios,
            &running,
            &scrollback,
        );

        // Clean up
        running.store(false, Ordering::SeqCst);
        let _ = socket.shutdown(std::net::Shutdown::Both);
        drop(socket);
        thread::sleep(Duration::from_millis(50));
        let _ = socket_to_stdout.join();

        // Restore terminal
        restore_terminal(stdin_fd, &original_termios)?;

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
    ) -> Result<Option<String>> {
        let stdin_fd = 0i32;
        let mut stdin = io::stdin();
        let mut buffer = [0u8; DEFAULT_BUFFER_SIZE]; // Use 16KB buffer

        // SSH-style escape sequence tracking
        let mut at_line_start = true;
        let mut escape_state = 0; // 0=normal, 1=saw tilde at line start
        let mut escape_time = Instant::now();

        loop {
            if !running.load(Ordering::SeqCst) {
                break;
            }

            match stdin.read(&mut buffer) {
                Ok(0) => {
                    // EOF (Ctrl+D) - treat as detach
                    println!("\r\n[Detaching from session {}]\r", session.id);
                    running.store(false, Ordering::SeqCst);
                    break;
                }
                Ok(n) => {
                    let (should_detach, should_switch, should_scroll, data_to_forward) =
                        Self::process_input(
                            &buffer[..n],
                            &mut at_line_start,
                            &mut escape_state,
                            &mut escape_time,
                        );

                    if should_detach {
                        println!("\r\n[Detaching from session {}]\r", session.id);
                        running.store(false, Ordering::SeqCst);
                        break;
                    }

                    if should_switch {
                        let switcher = SessionSwitcher::new(session, stdin_fd, original_termios);
                        match switcher.show_switcher()? {
                            SwitchResult::SwitchTo(target_id) => {
                                return Ok(Some(target_id));
                            }
                            SwitchResult::Continue => {
                                escape_state = 0;
                                at_line_start = true;
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
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    if !running.load(Ordering::SeqCst) {
                        break;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(e) => {
                    eprintln!("\r\nError reading stdin: {}\r", e);
                    break;
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
                        // Update line start tracking
                        *at_line_start = byte == b'\r' || byte == b'\n' || byte == 10 || byte == 13;
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

        // Get session ID from socket path
        let session_id = self
            .socket_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        // Create IO handler
        let io_handler = PtyIoHandler::new(self.master_fd);

        while running.load(Ordering::SeqCst) {
            // Check for new connections
            self.handle_new_connections(
                &listener,
                &mut active_clients,
                &output_buffer,
                &io_handler,
                &session_id,
            )?;

            // Read from PTY master and broadcast
            if let Some(data) = self.read_from_pty(&io_handler, &mut buffer)? {
                self.broadcast_to_clients(&mut active_clients, &data, &output_buffer, &session_id)?;
            }

            // Read from clients and handle input
            self.handle_client_input(&mut active_clients, &io_handler, &session_id)?;

            // Small sleep to prevent busy loop
            thread::sleep(Duration::from_millis(10));
        }

        Ok(())
    }

    fn handle_new_connections(
        &self,
        listener: &UnixListener,
        active_clients: &mut Vec<ClientInfo>,
        output_buffer: &PtyBuffer,
        io_handler: &PtyIoHandler,
        session_id: &str,
    ) -> Result<()> {
        match listener.accept() {
            Ok((mut stream, _)) => {
                stream.set_nonblocking(true)?;

                // Notify existing clients
                if !active_clients.is_empty() {
                    let notification = format!(
                        "\r\n[Another client connected to this session (total: {})]\r\n",
                        active_clients.len() + 1
                    );
                    for client in active_clients.iter_mut() {
                        let _ = client.stream.write_all(notification.as_bytes());
                        let _ = client.stream.flush();
                    }
                }

                // Send buffered output to new client
                send_buffered_output(&mut stream, output_buffer, io_handler)?;

                // Add new client
                active_clients.push(ClientInfo {
                    stream,
                    cols: 80,
                    rows: 24,
                });

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
            Ok(0) => Err(NdsError::PtyError("Child process exited".to_string())),
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
                if let Err(e) = client.stream.write_all(data) {
                    if e.kind() == io::ErrorKind::BrokenPipe
                        || e.kind() == io::ErrorKind::ConnectionAborted
                    {
                        disconnected_indices.push(i);
                    }
                } else {
                    let _ = client.stream.flush();
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

        // Notify remaining clients and resize
        if !active_clients.is_empty() {
            let notification = format!(
                "\r\n[A client disconnected (remaining: {})]\r\n",
                active_clients.len()
            );

            for client in active_clients.iter_mut() {
                let _ = client.stream.write_all(notification.as_bytes());
                let _ = send_terminal_refresh_sequences(&mut client.stream);
                let _ = client.stream.flush();
            }

            // Resize to smallest terminal
            self.resize_to_smallest(active_clients)?;
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

    fn handle_client_input(
        &self,
        active_clients: &mut Vec<ClientInfo>,
        io_handler: &PtyIoHandler,
        session_id: &str,
    ) -> Result<()> {
        let mut disconnected_indices = Vec::new();
        let mut client_buffer = [0u8; DEFAULT_BUFFER_SIZE]; // Use 16KB buffer

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
                        }
                    }

                    // Normal data - forward to PTY
                    io_handler.write_to_pty(data)?;
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    // No data available
                }
                Err(_) => {
                    disconnected_indices.push(i);
                }
            }
        }

        // Handle disconnections
        if !disconnected_indices.is_empty() {
            self.handle_client_disconnections(active_clients, disconnected_indices, session_id)?;
        }

        Ok(())
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
