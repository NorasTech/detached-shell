// PTY process management module
mod client;
mod socket;
mod terminal;
mod io_handler;
mod session_switcher;
mod spawn;

// Re-export main types for backward compatibility
pub use spawn::PtyProcess;

// Note: ClientInfo is now internal to the module
// If it needs to be public, uncomment the line below:
// pub use client::ClientInfo;