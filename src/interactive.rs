use crate::{NdsError, Result, Session, SessionManager};
use chrono::Timelike;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{self, disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame, Terminal,
};
use std::{
    io,
    time::{Duration, Instant},
};

pub struct InteractivePicker {
    sessions: Vec<Session>,
    state: ListState,
    current_session_id: Option<String>,
}

impl InteractivePicker {
    pub fn new() -> Result<Self> {
        let sessions = SessionManager::list_sessions()?;
        if sessions.is_empty() {
            return Err(NdsError::SessionNotFound("No active sessions".to_string()));
        }

        let mut state = ListState::default();
        state.select(Some(0));
        
        // Check if we're currently attached to a session
        let mut current_session_id = std::env::var("NDS_SESSION_ID").ok();
        
        // Fallback: If no environment variable, try to detect from parent processes
        if current_session_id.is_none() {
            current_session_id = Self::detect_current_session(&sessions);
        }

        Ok(Self { sessions, state, current_session_id })
    }

    fn detect_current_session(sessions: &[Session]) -> Option<String> {
        // Try to detect current session by checking parent processes
        let mut ppid = std::process::id();
        
        // Walk up the process tree (max 10 levels to avoid infinite loops)
        for _ in 0..10 {
            // Get parent process ID
            let ppid_result = Self::get_parent_pid(ppid as i32);
            if let Some(parent_pid) = ppid_result {
                // Check if this PID matches any session
                for session in sessions {
                    if session.pid == parent_pid {
                        return Some(session.id.clone());
                    }
                }
                ppid = parent_pid as u32;
            } else {
                break;
            }
        }
        
        None
    }
    
    fn get_parent_pid(pid: i32) -> Option<i32> {
        // Read /proc/[pid]/stat on Linux or use ps on macOS
        #[cfg(target_os = "macos")]
        {
            use std::process::Command;
            let output = Command::new("ps")
                .args(&["-p", &pid.to_string(), "-o", "ppid="])
                .output()
                .ok()?;
            
            if output.status.success() {
                let ppid_str = String::from_utf8_lossy(&output.stdout);
                ppid_str.trim().parse::<i32>().ok()
            } else {
                None
            }
        }
        
        #[cfg(target_os = "linux")]
        {
            use std::fs;
            let stat_path = format!("/proc/{}/stat", pid);
            let stat_content = fs::read_to_string(stat_path).ok()?;
            let parts: Vec<&str> = stat_content.split_whitespace().collect();
            // Parent PID is the 4th field in /proc/[pid]/stat
            if parts.len() > 3 {
                parts[3].parse::<i32>().ok()
            } else {
                None
            }
        }
        
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            None
        }
    }
    
    pub fn run(&mut self) -> Result<Option<String>> {
        // Setup terminal
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let result = self.run_app(&mut terminal);

        // Restore terminal
        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        terminal.show_cursor()?;

        result
    }

    fn run_app<B: Backend>(&mut self, terminal: &mut Terminal<B>) -> Result<Option<String>> {
        let mut last_tick = Instant::now();
        let tick_rate = Duration::from_millis(250);

        loop {
            terminal.draw(|f| self.ui(f))?;

            let timeout = tick_rate
                .checked_sub(last_tick.elapsed())
                .unwrap_or_else(|| Duration::from_secs(0));

            if crossterm::event::poll(timeout)? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        match key.code {
                            KeyCode::Char('q') | KeyCode::Esc => return Ok(None),
                            KeyCode::Down | KeyCode::Char('j') => self.next(),
                            KeyCode::Up | KeyCode::Char('k') => self.previous(),
                            KeyCode::Enter => {
                                if let Some(selected) = self.state.selected() {
                                    return Ok(Some(self.sessions[selected].id.clone()));
                                }
                            }
                            _ => {}
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
        let i = match self.state.selected() {
            Some(i) => {
                if i >= self.sessions.len() - 1 {
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
        let i = match self.state.selected() {
            Some(i) => {
                if i == 0 {
                    self.sessions.len() - 1
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

        // Header - more minimal
        let header = Paragraph::new("SESSIONS")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Left)
            .block(
                Block::default()
                    .borders(Borders::BOTTOM)
                    .border_style(Style::default().fg(Color::DarkGray)),
            );
        f.render_widget(header, chunks[0]);

        // Sessions list
        let items: Vec<ListItem> = self
            .sessions
            .iter()
            .map(|session| {
                let client_count = session.get_client_count();

                let now = chrono::Utc::now().timestamp();
                let created = session.created_at.timestamp();
                let duration = now - created;
                let uptime = format_duration(duration as u64);

                // Check if this is the current attached session
                let is_current = self.current_session_id.as_ref() == Some(&session.id);
                
                // Status indicator - simplified
                let (status_icon, status_color) = if is_current {
                    ("★", Color::Cyan)
                } else if client_count > 0 {
                    ("●", Color::Green) 
                } else {
                    ("○", Color::Gray)
                };
                
                // Session name styling
                let name_style = if is_current {
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };
                
                // Build the status text that appears on the right
                let status_text = if is_current {
                    if client_count > 0 {
                        format!("CURRENT SESSION · {} CLIENT{}", client_count, if client_count == 1 { "" } else { "S" })
                    } else {
                        "CURRENT SESSION".to_string()
                    }
                } else if client_count > 0 {
                    format!("{} CLIENT{}", client_count, if client_count == 1 { "" } else { "S" })
                } else {
                    "DETACHED".to_string()
                };
                
                // Format created time
                let now = chrono::Local::now();
                let local_time: chrono::DateTime<chrono::Local> = session.created_at.into();
                let duration = now.signed_duration_since(local_time);
                
                let created_time = if duration.num_days() > 0 {
                    format!("{}d, {:02}:{:02}", 
                        duration.num_days(), 
                        local_time.hour(), 
                        local_time.minute())
                } else {
                    local_time.format("%H:%M:%S").to_string()
                };
                
                // Truncate working dir if too long
                let mut working_dir = session.working_dir.clone();
                if working_dir.len() > 30 {
                    working_dir = format!("...{}", &session.working_dir[session.working_dir.len() - 27..]);
                }
                
                // Build left side with fixed widths
                let left_side = format!(
                    " {} {:<25} │ PID {:<6} │ {:<8} │ {:<8} │ {:<30}",
                    status_icon,
                    session.display_name(),
                    session.pid,
                    uptime,
                    created_time,
                    working_dir
                );
                
                // Calculate padding for right alignment
                let terminal_width = terminal::size().unwrap_or((80, 24)).0 as usize;
                let left_len = left_side.chars().count();
                let status_len = status_text.chars().count();
                let padding = terminal_width.saturating_sub(left_len + status_len + 2);

                let content = vec![
                    Line::from(vec![
                        Span::styled(
                            format!(" {} ", status_icon),
                            Style::default().fg(status_color).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!("{:<25}", session.display_name()),
                            name_style,
                        ),
                        Span::styled(
                            " │ ",
                            Style::default().fg(Color::DarkGray),
                        ),
                        Span::styled(
                            format!("PID {:<6}", session.pid),
                            Style::default().fg(Color::DarkGray),
                        ),
                        Span::styled(
                            " │ ",
                            Style::default().fg(Color::DarkGray),
                        ),
                        Span::styled(
                            format!("{:<8}", uptime),
                            Style::default().fg(Color::DarkGray),
                        ),
                        Span::styled(
                            " │ ",
                            Style::default().fg(Color::DarkGray),
                        ),
                        Span::styled(
                            format!("{:<8}", created_time),
                            Style::default().fg(Color::DarkGray),
                        ),
                        Span::styled(
                            " │ ",
                            Style::default().fg(Color::DarkGray),
                        ),
                        Span::styled(
                            format!("{:<30}", working_dir),
                            Style::default().fg(Color::DarkGray),
                        ),
                        Span::styled(
                            " ".repeat(padding),
                            Style::default(),
                        ),
                        Span::styled(
                            status_text.clone(),
                            if is_current {
                                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
                            } else if client_count > 0 {
                                Style::default().fg(Color::Green)
                            } else {
                                Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM)
                            },
                        ),
                    ]),
                ];
                ListItem::new(content)
            })
            .collect();

        let sessions_list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::NONE),
            )
            .highlight_style(
                Style::default()
                    .bg(Color::Rgb(40, 40, 40))
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("");

        f.render_stateful_widget(sessions_list, chunks[1], &mut self.state);

        // Footer - cleaner design
        let help_text = vec![
            Span::styled("↑↓/jk ", Style::default().fg(Color::DarkGray)),
            Span::styled("navigate", Style::default().fg(Color::Gray)),
            Span::styled("  ", Style::default()),
            Span::styled("⏎ ", Style::default().fg(Color::DarkGray)),
            Span::styled("attach", Style::default().fg(Color::Gray)),
            Span::styled("  ", Style::default()),
            Span::styled("q ", Style::default().fg(Color::DarkGray)),
            Span::styled("quit", Style::default().fg(Color::Gray)),
        ];
        
        let session_info = format!("{} sessions", self.sessions.len());
        
        let footer = Paragraph::new(Line::from(help_text))
            .style(Style::default())
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(Style::default().fg(Color::DarkGray)),
            );
        f.render_widget(footer, chunks[2]);
        
        // Session count on the right
        let count_widget = Paragraph::new(session_info)
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Right);
        let count_area = Rect {
            x: chunks[2].x + 2,
            y: chunks[2].y + 1,
            width: chunks[2].width - 4,
            height: 1,
        };
        f.render_widget(count_widget, count_area);
    }
}

fn format_duration(seconds: u64) -> String {
    if seconds < 60 {
        format!("{}s", seconds)
    } else if seconds < 3600 {
        format!("{}m", seconds / 60)
    } else if seconds < 86400 {
        format!("{}h {}m", seconds / 3600, (seconds % 3600) / 60)
    } else {
        format!("{}d {}h", seconds / 86400, (seconds % 86400) / 3600)
    }
}