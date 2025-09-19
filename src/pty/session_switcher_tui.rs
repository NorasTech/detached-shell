use std::io::{self, BufRead, Write};
use std::os::unix::io::{BorrowedFd, RawFd};
use std::time::{Duration, Instant};

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use nix::sys::termios::{tcflush, tcgetattr, tcsetattr, SetArg, Termios};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame, Terminal,
};

use super::terminal::{restore_terminal, set_raw_mode};
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

/// TUI-based session picker
struct TuiSessionPicker {
    sessions: Vec<Session>,
    current_session: Session,
    state: ListState,
    show_new_session_input: bool,
    new_session_name: String,
}

impl TuiSessionPicker {
    fn run<B: Backend>(&mut self, terminal: &mut Terminal<B>) -> Result<SwitchResult> {
        let tick_rate = Duration::from_millis(100);
        let mut last_tick = Instant::now();

        loop {
            terminal.draw(|f| self.ui(f))?;

            let timeout = tick_rate
                .checked_sub(last_tick.elapsed())
                .unwrap_or_else(|| Duration::from_secs(0));

            if crossterm::event::poll(timeout)? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        if self.show_new_session_input {
                            // Handle input for new session name
                            match key.code {
                                KeyCode::Esc => {
                                    self.show_new_session_input = false;
                                    self.new_session_name.clear();
                                }
                                KeyCode::Enter => {
                                    let name = if self.new_session_name.is_empty() {
                                        None
                                    } else {
                                        Some(self.new_session_name.clone())
                                    };
                                    return self.create_new_session(name);
                                }
                                KeyCode::Backspace => {
                                    self.new_session_name.pop();
                                }
                                KeyCode::Char(c) => {
                                    self.new_session_name.push(c);
                                }
                                _ => {}
                            }
                        } else {
                            // Normal navigation
                            match key.code {
                                KeyCode::Char('q') | KeyCode::Esc => {
                                    return Ok(SwitchResult::Continue);
                                }
                                KeyCode::Down | KeyCode::Char('j') => self.next(),
                                KeyCode::Up | KeyCode::Char('k') => self.previous(),
                                KeyCode::Enter => {
                                    if let Some(selected) = self.state.selected() {
                                        if selected < self.sessions.len() {
                                            // Selected an existing session
                                            let session = &self.sessions[selected];
                                            if session.id != self.current_session.id {
                                                return Ok(SwitchResult::SwitchTo(
                                                    session.id.clone(),
                                                ));
                                            }
                                        } else {
                                            // Selected "New Session"
                                            self.show_new_session_input = true;
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }

            if last_tick.elapsed() >= tick_rate {
                last_tick = Instant::now();
            }
        }
    }

    fn next(&mut self) {
        let total_items = self.sessions.len() + 1; // +1 for "New Session"
        let i = match self.state.selected() {
            Some(i) => {
                if i >= total_items - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }

    fn previous(&mut self) {
        let total_items = self.sessions.len() + 1; // +1 for "New Session"
        let i = match self.state.selected() {
            Some(i) => {
                if i == 0 {
                    total_items - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }

    fn ui(&mut self, f: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(3),
            ])
            .split(f.area());

        // Header
        let header_text = if self.show_new_session_input {
            "NEW SESSION"
        } else {
            "SESSION SWITCHER"
        };
        let header = Paragraph::new(header_text)
            .style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::BOTTOM)
                    .border_style(Style::default().fg(Color::DarkGray)),
            );
        f.render_widget(header, chunks[0]);

        if self.show_new_session_input {
            // Show input for new session name
            let input_block = Block::default()
                .title("Enter session name (or press Enter for no name)")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow));

            let input = Paragraph::new(self.new_session_name.as_str())
                .style(Style::default().fg(Color::White))
                .block(input_block);

            f.render_widget(input, chunks[1]);
        } else {
            // Show session list
            let items: Vec<ListItem> = self
                .sessions
                .iter()
                .map(|session| {
                    let is_current = session.id == self.current_session.id;
                    let client_count = session.get_client_count();

                    // Status indicator
                    let (status_icon, status_color) = if is_current {
                        ("★", Color::Cyan)
                    } else if client_count > 0 {
                        ("●", Color::Green)
                    } else {
                        ("○", Color::Gray)
                    };

                    let content = vec![Line::from(vec![
                        Span::styled(status_icon, Style::default().fg(status_color)),
                        Span::raw(" "),
                        Span::styled(
                            session.display_name(),
                            if is_current {
                                Style::default()
                                    .fg(Color::Cyan)
                                    .add_modifier(Modifier::ITALIC)
                            } else {
                                Style::default().fg(Color::White)
                            },
                        ),
                        Span::raw(" "),
                        Span::styled(
                            format!("[{}]", &session.id[..8]),
                            Style::default().fg(Color::DarkGray),
                        ),
                        if is_current {
                            Span::styled(" (current)", Style::default().fg(Color::DarkGray))
                        } else {
                            Span::raw("")
                        },
                    ])];

                    ListItem::new(content)
                })
                .chain(std::iter::once(ListItem::new(vec![Line::from(vec![
                    Span::styled("➕", Style::default().fg(Color::Yellow)),
                    Span::raw(" "),
                    Span::styled("Create New Session", Style::default().fg(Color::Yellow)),
                ])])))
                .collect();

            let sessions = List::new(items)
                .block(Block::default().borders(Borders::NONE))
                .highlight_style(
                    Style::default()
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol("> ");

            f.render_stateful_widget(sessions, chunks[1], &mut self.state);
        }

        // Footer
        let footer_text = if self.show_new_session_input {
            "[Enter] Create  [Esc] Cancel"
        } else {
            "[↑/↓/j/k] Navigate  [Enter] Select  [q/Esc] Cancel"
        };
        let footer = Paragraph::new(footer_text)
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(Style::default().fg(Color::DarkGray)),
            );
        f.render_widget(footer, chunks[2]);
    }

    fn create_new_session(&self, name: Option<String>) -> Result<SwitchResult> {
        match SessionManager::create_session_with_name(name.clone()) {
            Ok(new_session) => Ok(SwitchResult::SwitchTo(new_session.id)),
            Err(e) => {
                eprintln!("Error creating session: {}", e);
                Ok(SwitchResult::Continue)
            }
        }
    }
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
        // Enter alternate screen and setup terminal
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;

        // Create terminal backend
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;
        terminal.clear()?;

        // Get all sessions
        let sessions = SessionManager::list_sessions()?;

        // Create picker state
        let mut picker = TuiSessionPicker {
            sessions: sessions.clone(),
            current_session: self.current_session.clone(),
            state: ListState::default(),
            show_new_session_input: false,
            new_session_name: String::new(),
        };

        // Set initial selection to first non-current session or "New Session"
        let initial_selection = if sessions.len() > 1 {
            0
        } else {
            sessions.len() // Points to "New Session" option
        };
        picker.state.select(Some(initial_selection));

        // Run the TUI event loop
        let result = picker.run(&mut terminal)?;

        // Restore terminal
        // Restore terminal completely
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        terminal.show_cursor()?;
        crossterm::terminal::disable_raw_mode()?;

        // Set back to raw mode for session
        set_raw_mode(self.stdin_fd, self.original_termios)?;

        Ok(result)
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
        tcsetattr(&stdin_borrowed, SetArg::TCSAFLUSH, self.original_termios)?;

        // Ensure stdin is in blocking mode for reading
        unsafe {
            let flags = libc::fcntl(self.stdin_fd, libc::F_GETFL);
            if flags >= 0 {
                let _ = libc::fcntl(self.stdin_fd, libc::F_SETFL, flags & !libc::O_NONBLOCK);
            }
        }

        // Flush any pending input
        tcflush(&stdin_borrowed, nix::sys::termios::FlushArg::TCIFLUSH)?;

        // Read user input
        let stdin = io::stdin();
        let mut buffer = String::new();
        let mut stdin_lock = stdin.lock();
        let read_result = stdin_lock.read_line(&mut buffer);

        // Restore non-blocking mode if it was set
        unsafe {
            let flags = libc::fcntl(self.stdin_fd, libc::F_GETFL);
            if flags >= 0 {
                let _ = libc::fcntl(self.stdin_fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
            }
        }

        // Restore raw mode
        tcsetattr(&stdin_borrowed, SetArg::TCSANOW, &current_termios)?;

        read_result.map_err(|e| NdsError::Io(e))?;
        Ok(buffer)
    }
}

/// Show the session help message
#[allow(dead_code)]
pub fn show_session_help() {
    println!("\r\n[Session Commands]\r");
    println!("\r  ~d - Detach from current session\r");
    println!("\r  ~s - Switch sessions\r");
    println!("\r  ~h - Show scrollback history\r");
    println!("\r  ~~ - Send literal tilde\r");
    println!("\r\n[Press any key to continue]\r");
}
