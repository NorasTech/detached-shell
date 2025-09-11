use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{self, Event, KeyCode, KeyEvent},
    execute,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::io::{self, Write};

use crate::error::Result;

pub struct ScrollbackViewer {
    lines: Vec<String>,
    viewport_start: usize,
    viewport_height: usize,
    total_lines: usize,
}

impl ScrollbackViewer {
    pub fn new(content: &[u8]) -> Self {
        // Convert bytes to lines
        let text = String::from_utf8_lossy(content);
        let lines: Vec<String> = text.lines().map(|s| s.to_string()).collect();
        let total_lines = lines.len();

        // Get terminal height
        let (_, height) = terminal::size().unwrap_or((80, 24));
        let viewport_height = (height - 3) as usize; // Leave room for status bar

        ScrollbackViewer {
            lines,
            viewport_start: 0,
            viewport_height,
            total_lines,
        }
    }

    pub fn run(&mut self) -> Result<()> {
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

    fn event_loop(&mut self) -> Result<()> {
        let mut stdout = io::stdout();

        loop {
            self.draw(&mut stdout)?;

            if let Event::Key(key) = event::read()? {
                match self.handle_key(key) {
                    true => break, // Exit requested
                    false => continue,
                }
            }
        }

        Ok(())
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return true, // Exit

            // Navigation
            KeyCode::Up | KeyCode::Char('k') => {
                if self.viewport_start > 0 {
                    self.viewport_start -= 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.viewport_start + self.viewport_height < self.total_lines {
                    self.viewport_start += 1;
                }
            }
            KeyCode::PageUp | KeyCode::Char('b') => {
                self.viewport_start = self.viewport_start.saturating_sub(self.viewport_height);
            }
            KeyCode::PageDown | KeyCode::Char(' ') | KeyCode::Char('f') => {
                let new_start = self.viewport_start + self.viewport_height;
                if new_start + self.viewport_height <= self.total_lines {
                    self.viewport_start = new_start;
                } else if self.total_lines > self.viewport_height {
                    self.viewport_start = self.total_lines - self.viewport_height;
                }
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.viewport_start = 0;
            }
            KeyCode::End | KeyCode::Char('G') => {
                if self.total_lines > self.viewport_height {
                    self.viewport_start = self.total_lines - self.viewport_height;
                }
            }
            _ => {}
        }
        false
    }

    fn draw(&self, stdout: &mut io::Stdout) -> Result<()> {
        execute!(stdout, Clear(ClearType::All), MoveTo(0, 0))?;

        // Draw content
        let end = (self.viewport_start + self.viewport_height).min(self.total_lines);
        for i in self.viewport_start..end {
            execute!(stdout, Print(&self.lines[i]), Print("\r\n"))?;
        }

        // Fill empty lines if needed
        let displayed = end - self.viewport_start;
        for _ in displayed..self.viewport_height {
            execute!(stdout, Print("~\r\n"))?;
        }

        // Draw status bar
        let (width, _) = terminal::size().unwrap_or((80, 24));
        let position = if self.total_lines == 0 {
            "Empty".to_string()
        } else {
            let percent = if self.total_lines <= self.viewport_height {
                100
            } else {
                ((self.viewport_start + self.viewport_height) * 100 / self.total_lines).min(100)
            };
            format!(
                "Lines {}-{}/{} ({}%)",
                self.viewport_start + 1,
                end,
                self.total_lines,
                percent
            )
        };

        execute!(
            stdout,
            SetForegroundColor(Color::Black),
            crossterm::style::SetBackgroundColor(Color::White),
            Print(format!("{:<width$}", position, width = width as usize)),
            ResetColor,
            Print("\r\n")
        )?;

        // Draw help line
        execute!(
            stdout,
            SetForegroundColor(Color::DarkGrey),
            Print("↑/k:up ↓/j:down PgUp/b:page-up PgDn/f:page-down g:top G:bottom q:quit"),
            ResetColor
        )?;

        stdout.flush()?;
        Ok(())
    }
}
