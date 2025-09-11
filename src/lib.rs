pub mod error;
pub mod history;
pub mod history_v2;
pub mod interactive;
pub mod manager;
pub mod pty;
pub mod pty_buffer;
pub mod pty_handler;
pub mod scrollback;
pub mod session;
pub mod terminal_state;

#[cfg(test)]
mod tests;

pub use error::{NdsError, Result};
// Use v2 history as the main history module
pub use history_v2::{HistoryEntry, SessionEvent, SessionHistory};
pub use interactive::InteractivePicker;
pub use manager::{SessionManager, SessionTable};
pub use pty::PtyProcess;
pub use session::Session;
