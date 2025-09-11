# NDS - Noras Detached Shell 🚀

<div align="center">

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
- 🎨 **User-Friendly**: Intuitive commands with partial ID matching
- 🖥️ **Interactive Mode**: Visual session picker with arrow key navigation
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

# Interactive session picker
nds  # or nds list -i

# Attach to a session (supports partial ID matching)
nds attach abc123
nds a abc  # partial ID works

# Kill sessions
nds kill abc123
nds kill abc def ghi  # kill multiple sessions

# Clean up dead sessions
nds clean
```

### Session Information

```bash
# Get detailed info about a session
nds info abc123

# Rename a session
nds rename abc123 "new-name"

# View session history
nds history              # Active sessions only
nds history --all        # Include archived sessions
nds history -s abc123    # History for specific session
```

### Keyboard Shortcuts (Inside Session)

- `Enter, ~d` - Detach from current session (like SSH's `~.` sequence)

## 🏗️ Architecture

NDS uses a simple and robust architecture:

- **PTY Management**: Each session runs in its own pseudo-terminal
- **Unix Sockets**: Communication via Unix domain sockets
- **JSON Metadata**: Session info stored in `~/.nds/sessions/`
- **Per-Session History**: History stored in `~/.nds/history/`
- **Zero Dependencies**: Minimal external dependencies for reliability

### Directory Structure

```
~/.nds/
├── sessions/       # Session metadata (JSON)
├── sockets/        # Unix domain sockets
└── history/        # Session history
    ├── active/     # Currently running sessions
    └── archived/   # Terminated sessions
```

## 🔧 Configuration

NDS works out of the box with zero configuration. However, you can customize:

### Environment Variables

```bash
# Change default shell (default: $SHELL or /bin/sh)
export NDS_SHELL=/bin/zsh

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
# Run all tests
make test

# Run with coverage
cargo tarpaulin --out Html

# Run benchmarks
cargo bench
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
