#!/usr/bin/env bash

# NDS (Noras Detached Shell) Auto-Build & Install Script
# This script automatically builds and links to system after each development session

set -e  # Exit on error

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
BINARY_NAME="nds"
INSTALL_DIR="$HOME/.local/bin"
PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Icons
SUCCESS="✅"
ERROR="❌"
INFO="ℹ️"
BUILD="🔨"
LINK="🔗"
ROCKET="🚀"

# Functions
log_info() {
    echo -e "${BLUE}${INFO}${NC} $1"
}

log_success() {
    echo -e "${GREEN}${SUCCESS}${NC} $1"
}

log_error() {
    echo -e "${RED}${ERROR}${NC} $1"
}

log_warning() {
    echo -e "${YELLOW}⚠️${NC} $1"
}

# Check if Rust is installed
check_rust() {
    if ! command -v cargo &> /dev/null; then
        log_error "Rust is not installed! Please install Rust first: https://rustup.rs"
        exit 1
    fi
    log_success "Rust found: $(rustc --version)"
}

# Navigate to project directory
cd_to_project() {
    cd "$PROJECT_ROOT"
    log_info "Project directory: $PROJECT_ROOT"
}

# Run tests (optional)
run_tests() {
    if [[ "$1" == "--skip-tests" ]]; then
        log_warning "Skipping tests..."
        return 0
    fi
    
    log_info "Running tests..."
    if cargo test --quiet 2>/dev/null; then
        log_success "All tests passed!"
    else
        log_warning "Some tests failed, continuing..."
    fi
}

# Build
build_release() {
    log_info "${BUILD} Starting release build..."
    
    if cargo build --release --quiet; then
        log_success "Build successful!"
        
        # Show binary size
        local size=$(ls -lh target/release/$BINARY_NAME | awk '{print $5}')
        log_info "Binary size: $size"
    else
        log_error "Build failed!"
        exit 1
    fi
}

# Link to system
install_binary() {
    # Create .local/bin directory if it doesn't exist
    if [[ ! -d "$INSTALL_DIR" ]]; then
        log_info "Creating $INSTALL_DIR directory..."
        mkdir -p "$INSTALL_DIR"
    fi
    
    local source_binary="$PROJECT_ROOT/target/release/$BINARY_NAME"
    local target_link="$INSTALL_DIR/$BINARY_NAME"
    
    # Remove old link if exists
    if [[ -L "$target_link" ]]; then
        log_info "Removing old symlink..."
        rm "$target_link"
    elif [[ -f "$target_link" ]]; then
        log_warning "Existing binary found, backing up..."
        mv "$target_link" "${target_link}.backup.$(date +%Y%m%d_%H%M%S)"
    fi
    
    # Create new symlink
    log_info "${LINK} Creating symlink..."
    ln -s "$source_binary" "$target_link"
    
    log_success "Binary linked to system: $target_link"
}

# PATH check
check_path() {
    if [[ ":$PATH:" != *":$INSTALL_DIR:"* ]]; then
        log_warning "$INSTALL_DIR is not in PATH!"
        echo ""
        echo "To add to PATH, add this to your shell config file (~/.bashrc, ~/.zshrc, etc.):"
        echo ""
        echo "  export PATH=\"\$HOME/.local/bin:\$PATH\""
        echo ""
    else
        log_success "$INSTALL_DIR is in PATH"
    fi
}

# Verify installation
verify_installation() {
    if command -v $BINARY_NAME &> /dev/null; then
        log_success "${ROCKET} NDS successfully installed!"
        echo ""
        echo "Version: $($BINARY_NAME --version)"
        echo ""
        echo "Usage:"
        echo "  nds new          # Create new session"
        echo "  nds list         # List sessions"
        echo "  nds attach <id>  # Attach to session"
        echo "  nds kill <id>    # Kill session"
        echo ""
    else
        log_warning "Binary installed but not found in PATH"
        log_info "Restart your shell or run 'source ~/.bashrc'"
    fi
}

# Main flow
main() {
    echo ""
    echo "╔════════════════════════════════════════╗"
    echo "║   NDS Auto-Build & Install Script      ║"
    echo "╚════════════════════════════════════════╝"
    echo ""
    
    check_rust
    cd_to_project
    run_tests "$1"
    build_release
    install_binary
    check_path
    verify_installation
    
    echo ""
    log_success "Process completed! 🎉"
    echo ""
}

# Run the script
main "$@"