// PTY process management module
mod client;
mod health_monitor;
mod io_handler;
mod session_switcher;
mod socket;
mod spawn;
mod terminal;

// Async versions for tokio runtime
#[cfg(feature = "async")]
mod io_handler_async;
#[cfg(feature = "async")]
#[cfg(feature = "async")]
pub mod socket_async;

#[cfg(test)]
mod tests;

// Re-export main types for backward compatibility
pub use spawn::PtyProcess;

// Note: ClientInfo is now internal to the module
// If it needs to be public, uncomment the line below:
// pub use client::ClientInfo;
