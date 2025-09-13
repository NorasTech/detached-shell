# NDS - Noras Detached Shell 🚀

<div align="center">

[![Crates.io](https://img.shields.io/crates/v/detached-shell.svg)](https://crates.io/crates/detached-shell)
[![Rust](https://img.shields.io/badge/rust-%23000000.svg?style=for-the-badge&logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![PRs Welcome](https://img.shields.io/badge/PRs-welcome-brightgreen.svg)](http://makeapullrequest.com)

Simple detachable shell sessions with zero configuration. Not a complex multiplexer like tmux or screen - just clean, persistent sessions you can attach and detach as needed.

[Features](#features) • [Installation](#installation) • [Usage](#usage) • [Documentation](#documentation) • [Contributing](#contributing)

</div>

## ✨ Features

- 🎯 **Simple Session Management**: Create, detach, and reattach shell sessions with ease
- 🪶 **Lightweight**: < 1MB single binary with zero configuration required
- ⚡ **Fast**: Written in Rust for maximum performance
- 🎨 **User-Friendly**: Intuitive commands with partial ID and name matching
- 🖥️ **Interactive Mode**: Sleek TUI session picker with real-time status
- 📝 **Session History**: Track all session events with persistent history
- 🧹 **Auto-Cleanup**: Automatic cleanup of dead sessions
- 🔄 **Session Switching**: Simple attach/detach without complex multiplexing
- 🏷️ **Named Sessions**: Give meaningful names to your sessions
- 🐧 **Cross-Platform**: Works on Linux and macOS

## 🎯 Philosophy

NDS transforms your normal shell into a detachable, persistent session without changing how you work. No panes, no tabs, no complex layouts - just your shell, but better. When you need splits or tabs, your terminal emulator already handles that perfectly. NDS does one thing exceptionally well: making any shell session detachable and persistent.

## 📦 Installation


### Using Cargo

```bash
cargo install detached-shell
```

### From Source

```bash
# Clone the repository
git clone https://github.com/NorasTech/detached-shell.git
cd detached-shell

# Build and install (recommended)
./scripts/install.sh

# Or manually
cargo build --release
sudo cp target/release/nds /usr/local/bin/
```


## 🚀 Quick Start

```bash
# Create a new session
nds new

# List sessions
nds list

# Attach to a session
nds attach abc123

# Detach from current session
# Press Enter, then ~d (like SSH's escape sequences)
```

## 📖 Usage

### Creating Sessions

```bash
# Create and attach to a new session
nds new

# Create a named session
nds new "project-dev"

# Create without attaching
nds new --no-attach
```

### Managing Sessions

```bash
# List all active sessions
nds list
nds ls

# Interactive session picker with TUI
nds interactive  # or just 'nds' for short

# Attach to a session (supports partial ID and name matching)
nds attach abc123
nds attach project-dev  # attach by name
nds a abc  # partial ID works
nds a proj  # partial name works

# Kill sessions (supports ID and name)
nds kill abc123
nds kill project-dev  # kill by name
nds kill abc def ghi  # kill multiple sessions

# Clean up dead sessions
nds clean
```

### Session Information

```bash
# Get detailed info about a session (supports ID and name)
nds info abc123
nds info project-dev  # info by name

# Rename a session (supports ID and name)
nds rename abc123 "new-name"
nds rename project-dev "production"  # rename by current name

# View session history
nds history              # Active sessions only
nds history --all        # Include archived sessions
nds history -s abc123    # History for specific session
```

### Keyboard Shortcuts (Inside Session)

- `Enter, ~d` - Detach from current session (like SSH's `~.` sequence)
- `Ctrl+D` - Detach from current session (when at empty prompt)
- `Enter, ~s` - Switch to another session interactively

## 🏗️ Architecture

NDS uses a simple and robust architecture:

- **PTY Management**: Each session runs in its own pseudo-terminal
- **Unix Sockets**: Communication via Unix domain sockets (0600 permissions)
- **JSON Metadata**: Session info stored in `~/.nds/sessions/`
- **Per-Session History**: History stored in `~/.nds/history/`
- **Zero Dependencies**: Minimal external dependencies for reliability
- **Async I/O Support**: Optional async runtime with Tokio for high concurrency
- **Optimized Buffers**: 16KB buffers for 4x throughput improvement

### Directory Structure

```
~/.nds/
├── sessions/       # Session metadata (JSON)
├── sockets/        # Unix domain sockets (0600 permissions)
└── history/        # Session history
    ├── active/     # Currently running sessions
    └── archived/   # Terminated sessions
```

## 🔐 Security

NDS implements multiple security layers to protect your sessions:

### Session Isolation
- **Unix Socket Permissions**: All sockets created with `0600` (owner read/write only)
- **Session Umask**: Sessions run with `umask 0077` for restrictive file creation
- **Process Isolation**: Each session runs in its own process with separate PTY

### Input Validation
- **Command Whitelisting**: Only safe NDS control commands allowed (`resize`, `detach`, `attach`, etc.)
- **Input Sanitization**: Control characters and potentially harmful inputs are filtered
- **Buffer Limits**: Maximum 8KB command length and 10 arguments to prevent overflow
- **Numeric Bounds**: Terminal dimensions limited to 1-9999 to prevent resource exhaustion

### Important Note
NDS is a terminal multiplexer, not a sandbox. Shell commands within sessions are **not** restricted - you have full access to your shell just as you would in a normal terminal. The security measures protect the NDS control plane and session management, not the shell commands you run inside sessions.

## ⚡ Performance

NDS is optimized for speed and efficiency:

### Buffer Optimization
- **16KB I/O Buffers**: 4x throughput improvement over standard 4KB buffers
- **2MB Scrollback Buffer**: Increased from 1MB for better history retention
- **Benchmarked**: 25+ GB/s throughput in buffer operations

### Async I/O (Optional)
Enable async features for high-concurrency scenarios:

```toml
# Cargo.toml
[dependencies]
detached-shell = { version = "0.1", features = ["async"] }
```

With async enabled:
- Non-blocking socket operations
- Concurrent session management with `Arc<RwLock>`
- Tokio runtime for scalable I/O

## 🔧 Configuration

NDS works out of the box with zero configuration. However, you can customize:

### Environment Variables

```bash
# Change default shell (default: $SHELL or /bin/sh)
export NDS_SHELL=/bin/zsh

# Session identification (automatically set inside sessions)
NDS_SESSION_ID      # Current session ID when attached
NDS_SESSION_NAME    # Current session name (if set)

# Change detach key binding (coming soon)
export NDS_DETACH_KEY="ctrl-a d"
```

## 🤝 Contributing

We love contributions! Please see [CONTRIBUTING.md](CONTRIBUTING.md) for details.

### Development Setup

```bash
# Clone the repo
git clone https://github.com/NorasTech/detached-shell.git
cd detached-shell

# Run tests
cargo test

# Run with debug logs
RUST_LOG=debug cargo run -- list

# Quick rebuild and test
./scripts/dev.sh quick
```

### Running Tests

```bash
# Run all tests (55+ unit and integration tests)
cargo test

# Run with all features including async
cargo test --all-features

# Run specific test categories
cargo test --test security_test     # Security tests
cargo test --test session_lifecycle  # Integration tests

# Run with coverage
cargo tarpaulin --out Html

# Run performance benchmarks
cargo run --release --bin buffer_benchmark
```

## 🚧 Project Status

**Alpha Release** - NDS is in active development. Core functionality is stable, but expect breaking changes.

## 📄 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## 🙏 Acknowledgments

- Inspired by GNU Screen and tmux
- Built with [Rust](https://www.rust-lang.org/) 🦀
- Terminal handling via [libc](https://github.com/rust-lang/libc)

## 🐛 Troubleshooting

### Session not found after reboot
Sessions don't persist across system reboots by design. Use `nds history --all` to see past sessions.

### Permission denied errors
Ensure `~/.nds/` directory has proper permissions:
```bash
chmod 700 ~/.nds
chmod 700 ~/.nds/sockets
```

### Can't detach from session
Make sure you're using the correct key sequence: press Enter first, then `~d` (similar to SSH's escape sequences).

## 📮 Support

- 🐛 [Report bugs](https://github.com/NorasTech/detached-shell/issues)
- 💡 [Request features](https://github.com/NorasTech/detached-shell/issues)
- 💬 [Discussions](https://github.com/NorasTech/detached-shell/discussions)

---

<div align="center">
Made with ❤️ and 🦀 by <a href="https://noras.tech">Noras Technologies</a>
</div>
