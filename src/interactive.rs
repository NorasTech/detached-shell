use crate::{NdsError, Result, Session, SessionManager};
use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{self, Event, KeyCode, KeyEvent},
    execute,
    style::{Color, Print, ResetColor, SetForegroundColor, Stylize},
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::io::{self, Write};
use std::time::Duration;

pub struct InteractivePicker {
    sessions: Vec<Session>,
    selected: usize,
}

impl InteractivePicker {
    pub fn new() -> Result<Self> {
        let sessions = SessionManager::list_sessions()?;
        if sessions.is_empty() {
            return Err(NdsError::SessionNotFound("No active sessions".to_string()));
        }

        Ok(Self {
            sessions,
            selected: 0,
        })
    }

    pub fn run(&mut self) -> Result<Option<String>> {
        // Enter alternate screen
        let mut stdout = io::stdout();
        terminal::enable_raw_mode()?;
        execute!(stdout, EnterAlternateScreen, Hide)?;

        let result = self.event_loop();

        // Clean up
        execute!(stdout, LeaveAlternateScreen, Show)?;
        terminal::disable_raw_mode()?;

        result
    }

    fn event_loop(&mut self) -> Result<Option<String>> {
        let mut stdout = io::stdout();

        loop {
            self.draw(&mut stdout)?;

            if event::poll(Duration::from_millis(100))? {
                if let Event::Key(key) = event::read()? {
                    match self.handle_key(key) {
                        Some(session_id) => return Ok(Some(session_id)),
                        None if key.code == KeyCode::Char('q') || key.code == KeyCode::Esc => {
                            return Ok(None);
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> Option<String> {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if self.selected > 0 {
                    self.selected -= 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.selected < self.sessions.len() - 1 {
                    self.selected += 1;
                }
            }
            KeyCode::Enter => {
                return Some(self.sessions[self.selected].id.clone());
            }
            _ => {}
        }
        None
    }

    fn draw(&self, stdout: &mut io::Stdout) -> Result<()> {
        execute!(stdout, Clear(ClearType::All), MoveTo(0, 0))?;

        // Header
        execute!(
            stdout,
            SetForegroundColor(Color::Cyan),
            Print("NDS - Interactive Session Picker\n"),
            ResetColor,
            Print("─────────────────────────────────────────────────\n\n")
        )?;

        // Instructions
        execute!(
            stdout,
            SetForegroundColor(Color::DarkGrey),
            Print("↑/k: up  ↓/j: down  Enter: attach  q/Esc: quit\n\n"),
            ResetColor
        )?;

        // Sessions list
        for (i, session) in self.sessions.iter().enumerate() {
            if i == self.selected {
                execute!(stdout, SetForegroundColor(Color::Green), Print("▶ "))?;
            } else {
                execute!(stdout, Print("  "))?;
            }

            // Session info
            let now = chrono::Utc::now().timestamp();
            let created = session.created_at.timestamp();
            let duration = now - created;
            let uptime = format_duration(duration as u64);

            let line = format!(
                "{:<20} PID: {:<8} Uptime: {:<12} {}",
                session.display_name(),
                session.pid,
                uptime,
                if session.attached {
                    "[ATTACHED]".green().to_string()
                } else {
                    "".to_string()
                }
            );

            if i == self.selected {
                execute!(stdout, Print(line.bold()), ResetColor)?;
            } else {
                execute!(stdout, Print(line))?;
            }

            execute!(stdout, Print("\n"))?;
        }

        // Footer
        execute!(
            stdout,
            Print("\n─────────────────────────────────────────────────\n"),
            SetForegroundColor(Color::DarkGrey),
            Print(format!("{} active session(s)\n", self.sessions.len())),
            ResetColor
        )?;

        stdout.flush()?;
        Ok(())
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
