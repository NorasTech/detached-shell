#!/usr/bin/env bash

# NDS Development Helper Script
# Helper commands for development workflow

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

# Project root
PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$PROJECT_ROOT"

# Functions
print_header() {
    echo ""
    echo -e "${CYAN}═══════════════════════════════════════${NC}"
    echo -e "${CYAN}  $1${NC}"
    echo -e "${CYAN}═══════════════════════════════════════${NC}"
    echo ""
}

print_success() {
    echo -e "${GREEN}✓${NC} $1"
}

print_error() {
    echo -e "${RED}✗${NC} $1"
}

print_info() {
    echo -e "${BLUE}→${NC} $1"
}

# Commands
cmd_build() {
    print_header "🔨 Building Debug Version"
    cargo build
    print_success "Debug build completed"
}

cmd_release() {
    print_header "🚀 Building Release Version"
    cargo build --release
    local size=$(ls -lh target/release/nds | awk '{print $5}')
    print_success "Release build completed (Size: $size)"
}

cmd_test() {
    print_header "🧪 Running Tests"
    cargo test
    print_success "Tests completed"
}

cmd_check() {
    print_header "🔍 Running Checks"
    
    print_info "Format check..."
    cargo fmt -- --check || {
        print_error "Format errors found! Run 'cargo fmt'."
        return 1
    }
    
    print_info "Clippy check..."
    cargo clippy -- -W clippy::all || {
        print_error "Clippy warnings found!"
        return 1
    }
    
    print_success "All checks passed"
}

cmd_format() {
    print_header "✨ Formatting Code"
    cargo fmt
    print_success "Code formatted"
}

cmd_clean() {
    print_header "🧹 Cleaning Build Artifacts"
    cargo clean
    rm -rf ~/.nds/sessions/* 2>/dev/null || true
    rm -rf ~/.nds/sockets/* 2>/dev/null || true
    print_success "Cleanup completed"
}

cmd_install() {
    print_header "📦 Installing to System"
    ./scripts/install.sh --skip-tests
}

cmd_watch() {
    print_header "👀 Watching for Changes"
    
    if ! command -v cargo-watch &> /dev/null; then
        print_info "cargo-watch not installed, installing..."
        cargo install cargo-watch
    fi
    
    print_info "Watching for changes... (Ctrl+C to exit)"
    cargo watch -x build -x test
}

cmd_run() {
    print_header "▶️  Running NDS"
    shift # Skip the 'run' command
    cargo run -- "$@"
}

cmd_bench() {
    print_header "📊 Running Benchmarks"
    
    # Simple performance test
    print_info "Session creation test..."
    time for i in {1..10}; do
        ./target/release/nds new --no-attach 2>/dev/null || true
    done
    
    print_info "Session listing test..."
    time ./target/release/nds list > /dev/null
    
    # Clean up
    ./target/release/nds clean > /dev/null 2>&1
    
    print_success "Benchmark completed"
}

cmd_quick() {
    print_header "⚡ Quick Build & Install"
    
    print_info "Building..."
    cargo build --release --quiet
    
    print_info "Installing..."
    ./scripts/install.sh --skip-tests > /dev/null 2>&1
    
    print_success "Quick install completed!"
    echo ""
    nds --version
}

cmd_doc() {
    print_header "📚 Generating Documentation"
    cargo doc --no-deps --open
    print_success "Documentation generated and opened"
}

cmd_size() {
    print_header "📏 Binary Size Analysis"
    
    if [[ ! -f target/release/nds ]]; then
        print_info "Creating release build..."
        cargo build --release --quiet
    fi
    
    echo "Binary Size:"
    ls -lh target/release/nds | awk '{print "  Total: " $5}'
    
    echo ""
    echo "Detailed Analysis:"
    size target/release/nds || true
    
    if command -v cargo-bloat &> /dev/null; then
        echo ""
        echo "Bloat Analysis:"
        cargo bloat --release -n 10
    else
        print_info "For detailed analysis: cargo install cargo-bloat"
    fi
}

# Help message
show_help() {
    cat << EOF
${CYAN}NDS Development Helper${NC}

Usage:
  ./scripts/dev.sh <command> [arguments]

Commands:
  ${GREEN}build${NC}     - Create debug build
  ${GREEN}release${NC}   - Create release build (optimized)
  ${GREEN}test${NC}      - Run tests
  ${GREEN}check${NC}     - Run format and lint checks
  ${GREEN}format${NC}    - Format code
  ${GREEN}clean${NC}     - Clean build artifacts and sessions
  ${GREEN}install${NC}   - Install to system (release build + symlink)
  ${GREEN}watch${NC}     - Watch file changes and auto-rebuild
  ${GREEN}run${NC}       - Run NDS (cargo run wrapper)
  ${GREEN}bench${NC}     - Run simple performance tests
  ${GREEN}quick${NC}     - Quick build & install (skip tests)
  ${GREEN}doc${NC}       - Generate and open documentation
  ${GREEN}size${NC}      - Binary size analysis
  ${GREEN}help${NC}      - Show this help message

Examples:
  ./scripts/dev.sh build           # Debug build
  ./scripts/dev.sh quick           # Quick install
  ./scripts/dev.sh run new         # Create new session
  ./scripts/dev.sh watch           # Auto-rebuild on changes

EOF
}

# Main control
case "${1:-help}" in
    build)   cmd_build ;;
    release) cmd_release ;;
    test)    cmd_test ;;
    check)   cmd_check ;;
    format)  cmd_format ;;
    clean)   cmd_clean ;;
    install) cmd_install ;;
    watch)   cmd_watch ;;
    run)     cmd_run "$@" ;;
    bench)   cmd_bench ;;
    quick)   cmd_quick ;;
    doc)     cmd_doc ;;
    size)    cmd_size ;;
    help|--help|-h) show_help ;;
    *)
        print_error "Unknown command: $1"
        show_help
        exit 1
        ;;
esac