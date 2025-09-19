// Module declarations
pub mod clients;
pub mod info;
pub mod session;

#[cfg(test)]
mod test;

// Re-export commonly used items for convenience
pub use session::{
    handle_attach_session, handle_clean_sessions, handle_kill_sessions, handle_new_session,
    handle_rename_session,
};

pub use clients::{handle_disconnect_client, handle_list_clients};
pub use info::{handle_list_sessions, handle_session_history, handle_session_info};
