#!/bin/bash
# SootMix installer script
# Usage: curl -sSL https://raw.githubusercontent.com/FrozenTear/sootmix/master/contrib/install.sh | sh
set -e

REPO_URL="https://github.com/FrozenTear/sootmix.git"
INSTALL_DIR="${INSTALL_DIR:-/tmp/sootmix-install}"

echo "==================================="
echo "  SootMix Installer"
echo "==================================="
echo

# Colors (if terminal supports it)
if [ -t 1 ]; then
    RED='\033[0;31m'
    GREEN='\033[0;32m'
    YELLOW='\033[1;33m'
    NC='\033[0m'
else
    RED=''
    GREEN=''
    YELLOW=''
    NC=''
fi

info() { echo -e "${GREEN}[*]${NC} $1"; }
warn() { echo -e "${YELLOW}[!]${NC} $1"; }
error() { echo -e "${RED}[x]${NC} $1"; exit 1; }

# Check for required commands
check_command() {
    if ! command -v "$1" &> /dev/null; then
        return 1
    fi
    return 0
}

# Detect package manager and install dependencies
install_deps() {
    info "Checking dependencies..."

    if check_command pacman; then
        # Arch Linux
        MISSING=""
        check_command cargo || MISSING="$MISSING rust"
        check_command git || MISSING="$MISSING git"
        pkg-config --exists libpipewire-0.3 2>/dev/null || MISSING="$MISSING pipewire"

        if [ -n "$MISSING" ]; then
            info "Installing dependencies:$MISSING"
            sudo pacman -S --needed $MISSING
        fi
    elif check_command apt-get; then
        # Debian/Ubuntu
        MISSING=""
        check_command cargo || MISSING="$MISSING cargo"
        check_command git || MISSING="$MISSING git"
        pkg-config --exists libpipewire-0.3 2>/dev/null || MISSING="$MISSING libpipewire-0.3-dev"
        check_command pkg-config || MISSING="$MISSING pkg-config"
        pkg-config --exists clang 2>/dev/null || MISSING="$MISSING clang"

        if [ -n "$MISSING" ]; then
            info "Installing dependencies:$MISSING"
            sudo apt-get update
            sudo apt-get install -y $MISSING
        fi
    elif check_command dnf; then
        # Fedora
        MISSING=""
        check_command cargo || MISSING="$MISSING rust cargo"
        check_command git || MISSING="$MISSING git"
        pkg-config --exists libpipewire-0.3 2>/dev/null || MISSING="$MISSING pipewire-devel"
        check_command clang || MISSING="$MISSING clang"

        if [ -n "$MISSING" ]; then
            info "Installing dependencies:$MISSING"
            sudo dnf install -y $MISSING
        fi
    else
        warn "Unknown package manager. Please ensure these are installed:"
        echo "  - rust/cargo"
        echo "  - git"
        echo "  - pipewire development libraries"
        echo "  - clang"
        echo
        read -p "Continue anyway? [y/N] " -n 1 -r
        echo
        [[ $REPLY =~ ^[Yy]$ ]] || exit 1
    fi
}

# Verify required tools
verify_deps() {
    info "Verifying dependencies..."
    check_command cargo || error "cargo not found. Please install Rust: https://rustup.rs"
    check_command git || error "git not found. Please install git."
    pkg-config --exists libpipewire-0.3 2>/dev/null || error "PipeWire development libraries not found."
    info "All dependencies satisfied."
}

# Clone and build
build() {
    info "Cloning repository..."
    rm -rf "$INSTALL_DIR"
    git clone --depth 1 "$REPO_URL" "$INSTALL_DIR"

    cd "$INSTALL_DIR"

    info "Building SootMix (this may take a few minutes)..."
    make build
}

# Install
install() {
    cd "$INSTALL_DIR"

    info "Installing SootMix..."
    sudo make install

    info "Updating icon cache..."
    sudo gtk-update-icon-cache -f -t /usr/share/icons/hicolor 2>/dev/null || true

    info "Reloading systemd..."
    systemctl --user daemon-reload
}

# Cleanup
cleanup() {
    info "Cleaning up..."
    rm -rf "$INSTALL_DIR"
}

# Post-install message
post_install() {
    echo
    echo -e "${GREEN}==================================="
    echo "  Installation complete!"
    echo "===================================${NC}"
    echo
    echo "To start the daemon:"
    echo "  systemctl --user enable --now sootmix-daemon.service"
    echo
    echo "To launch the UI:"
    echo "  sootmix"
    echo
    echo "Or find 'SootMix' in your application menu."
    echo
}

# Main
main() {
    install_deps
    verify_deps
    build
    install
    cleanup
    post_install
}

main "$@"
