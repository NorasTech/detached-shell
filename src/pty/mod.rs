// PTY process management module
mod client;
mod io_handler;
mod session_switcher;
mod socket;
mod spawn;
mod terminal;

#[cfg(test)]
mod tests;

// Re-export main types for backward compatibility
pub use spawn::PtyProcess;

// Note: ClientInfo is now internal to the module
// If it needs to be public, uncomment the line below:
// pub use client::ClientInfo;
