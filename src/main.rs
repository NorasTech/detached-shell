use clap::{Parser, Subcommand};
use detached_shell::Result;

// Import handler modules
mod handlers;

#[derive(Parser)]
#[command(name = "nds")]
#[command(about = "Noras Detached Shell - A minimalist shell session manager", long_about = None)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a new detached shell session
    New {
        /// Optional session name
        name: Option<String>,
        /// Don't attach to the new session (default is to attach)
        #[arg(long = "no-attach")]
        no_attach: bool,
    },

    /// List all active sessions
    #[command(aliases = &["ls", "l"])]
    List {
        /// Interactive mode - select session to attach
        #[arg(short, long)]
        interactive: bool,
    },

    /// Attach to an existing session
    #[command(aliases = &["a", "at"])]
    Attach {
        /// Session ID or name to attach to (supports partial matching)
        id: String,
    },

    /// Kill one or more sessions
    #[command(aliases = &["k"])]
    Kill {
        /// Session IDs or names to kill (supports partial matching)
        ids: Vec<String>,
    },

    /// Show information about a specific session
    #[command(aliases = &["i"])]
    Info {
        /// Session ID or name to get info about (supports partial matching)
        id: String,
    },

    /// Rename a session
    #[command(aliases = &["rn"])]
    Rename {
        /// Session ID or name to rename (supports partial matching)
        id: String,
        /// New name for the session
        new_name: String,
    },

    /// Clean up dead sessions
    Clean,

    /// Show session history
    #[command(aliases = &["h", "hist"])]
    History {
        /// Show history for a specific session ID or name (supports partial matching)
        #[arg(short, long)]
        session: Option<String>,

        /// Show all history entries (including crashed/killed sessions)
        #[arg(short, long)]
        all: bool,

        /// Limit number of entries to show
        #[arg(short, long, default_value = "50")]
        limit: usize,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::New { name, no_attach }) => {
            handlers::handle_new_session(name, !no_attach)?;
        }
        Some(Commands::List { interactive }) => {
            handlers::handle_list_sessions(interactive)?;
        }
        Some(Commands::Attach { id }) => {
            handlers::handle_attach_session(&id)?;
        }
        Some(Commands::Kill { ids }) => {
            handlers::handle_kill_sessions(&ids)?;
        }
        Some(Commands::Info { id }) => {
            handlers::handle_session_info(&id)?;
        }
        Some(Commands::Rename { id, new_name }) => {
            handlers::handle_rename_session(&id, &new_name)?;
        }
        Some(Commands::Clean) => {
            handlers::handle_clean_sessions()?;
        }
        Some(Commands::History {
            session,
            all,
            limit,
        }) => {
            handlers::handle_session_history(session, all, limit)?;
        }
        None => {
            // Default action: interactive session picker
            handlers::handle_list_sessions(true)?;
        }
    }

    Ok(())
}
