#!/bin/bash
#
# SootMix Installer
# Audio routing and mixing application for Linux using PipeWire
#
# Usage:
#   curl -sSL https://raw.githubusercontent.com/FrozenTear/sootmix/master/contrib/install.sh | sh
#
# Options (via environment variables):
#   SOOTMIX_PREFIX=/usr/local    Install prefix (default: /usr/local)
#   SOOTMIX_METHOD=binary        Installation method: binary or source (default: binary)
#   SOOTMIX_VERSION=latest       Version to install (default: latest)
#

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
BOLD='\033[1m'
NC='\033[0m' # No Color

# Disable colors if not a terminal
if [[ ! -t 1 ]]; then
    RED=''
    GREEN=''
    YELLOW=''
    BLUE=''
    BOLD=''
    NC=''
fi

# Configuration
REPO_URL="https://github.com/FrozenTear/sootmix"
REPO_NAME="sootmix"
PREFIX="${SOOTMIX_PREFIX:-/usr/local}"
METHOD="${SOOTMIX_METHOD:-binary}"
VERSION="${SOOTMIX_VERSION:-latest}"
TEMP_DIR=""

# Minimum Rust version required
MIN_RUST_VERSION="1.75"

# Cleanup on exit
cleanup() {
    if [[ -n "$TEMP_DIR" && -d "$TEMP_DIR" ]]; then
        rm -rf "$TEMP_DIR"
    fi
}
trap cleanup EXIT

# Logging functions
info() {
    echo -e "${BLUE}::${NC} $1"
}

success() {
    echo -e "${GREEN}::${NC} $1"
}

warn() {
    echo -e "${YELLOW}::${NC} $1"
}

error() {
    echo -e "${RED}error:${NC} $1" >&2
}

die() {
    error "$1"
    exit 1
}

# Print banner
print_banner() {
    echo -e "${BOLD}"
    echo "  ____              _   __  __ _      "
    echo " / ___|  ___   ___ | |_|  \/  (_)_  __"
    echo " \___ \ / _ \ / _ \| __| |\/| | \ \/ /"
    echo "  ___) | (_) | (_) | |_| |  | | |>  < "
    echo " |____/ \___/ \___/ \__|_|  |_|_/_/\_\\"
    echo -e "${NC}"
    echo "Audio routing and mixing for Linux with PipeWire"
    echo ""
}

# Check if running on Linux
check_platform() {
    local os
    os="$(uname -s)"

    if [[ "$os" != "Linux" ]]; then
        die "SootMix is only supported on Linux (detected: $os)"
    fi

    local arch
    arch="$(uname -m)"

    case "$arch" in
        x86_64|aarch64)
            ;;
        *)
            warn "Architecture $arch may not have pre-built binaries."
            warn "Will build from source."
            METHOD="source"
            ;;
    esac

    info "Detected platform: Linux $arch"
}

# Check for required commands
check_command() {
    command -v "$1" &> /dev/null
}

# Compare version strings
version_ge() {
    # Returns 0 if $1 >= $2
    printf '%s\n%s\n' "$2" "$1" | sort -V -C
}

# Check dependencies for binary installation
check_binary_deps() {
    local missing=()

    if ! check_command curl && ! check_command wget; then
        missing+=("curl or wget")
    fi

    if ! check_command tar; then
        missing+=("tar")
    fi

    if [[ ${#missing[@]} -gt 0 ]]; then
        die "Missing required tools: ${missing[*]}"
    fi

    # Runtime dependencies
    if ! pkg-config --exists libpipewire-0.3 2>/dev/null; then
        warn "PipeWire libraries not found. SootMix requires PipeWire at runtime."
    fi
}

# Check dependencies for source installation
check_source_deps() {
    local missing=()

    if ! check_command cargo; then
        missing+=("cargo (Rust toolchain)")
    fi

    if ! check_command git; then
        missing+=("git")
    fi

    if ! check_command pkg-config; then
        missing+=("pkg-config")
    fi

    if ! check_command clang; then
        missing+=("clang")
    fi

    if [[ ${#missing[@]} -gt 0 ]]; then
        echo ""
        error "Missing build dependencies: ${missing[*]}"
        echo ""
        echo "To install Rust toolchain:"
        echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
        echo ""
        echo "To install dependencies on Debian/Ubuntu:"
        echo "  sudo apt install git build-essential pkg-config libpipewire-0.3-dev libdbus-1-dev clang libclang-dev"
        echo ""
        echo "To install dependencies on Fedora:"
        echo "  sudo dnf install git rust cargo pkgconf-pkg-config pipewire-devel dbus-devel clang clang-devel"
        echo ""
        echo "To install dependencies on Arch Linux:"
        echo "  sudo pacman -S git rust pipewire clang"
        echo ""
        die "Please install missing dependencies and try again."
    fi

    # Check for PipeWire development libraries
    if ! pkg-config --exists libpipewire-0.3 2>/dev/null; then
        die "PipeWire development libraries not found. Install libpipewire-0.3-dev (Debian/Ubuntu) or pipewire-devel (Fedora) or pipewire (Arch)."
    fi

    # Check for D-Bus development libraries
    if ! pkg-config --exists dbus-1 2>/dev/null; then
        die "D-Bus development libraries not found. Install libdbus-1-dev (Debian/Ubuntu) or dbus-devel (Fedora)."
    fi

    # Check Rust version
    local rust_version
    rust_version=$(rustc --version | grep -oE '[0-9]+\.[0-9]+' | head -1)

    if ! version_ge "$rust_version" "$MIN_RUST_VERSION"; then
        die "Rust $MIN_RUST_VERSION or later is required (found: $rust_version). Run 'rustup update' to upgrade."
    fi

    info "Rust version: $rust_version"
}

# Attempt to install dependencies via package manager
try_install_deps() {
    info "Checking for missing dependencies..."

    if check_command pacman; then
        # Arch Linux
        local missing=""
        check_command cargo || missing="$missing rust"
        check_command git || missing="$missing git"
        check_command clang || missing="$missing clang"
        pkg-config --exists libpipewire-0.3 2>/dev/null || missing="$missing pipewire"

        if [[ -n "$missing" ]]; then
            info "Installing dependencies:$missing"
            sudo pacman -S --needed $missing
        fi

    elif check_command apt-get; then
        # Debian/Ubuntu
        local missing=""
        check_command cargo || missing="$missing cargo"
        check_command git || missing="$missing git"
        check_command cc || missing="$missing build-essential"
        check_command pkg-config || missing="$missing pkg-config"
        check_command clang || missing="$missing clang libclang-dev"
        pkg-config --exists libpipewire-0.3 2>/dev/null || missing="$missing libpipewire-0.3-dev"
        pkg-config --exists dbus-1 2>/dev/null || missing="$missing libdbus-1-dev"

        if [[ -n "$missing" ]]; then
            info "Installing dependencies:$missing"
            sudo apt-get update
            sudo apt-get install -y $missing
        fi

    elif check_command dnf; then
        # Fedora
        local missing=""
        check_command cargo || missing="$missing rust cargo"
        check_command git || missing="$missing git"
        check_command pkg-config || missing="$missing pkgconf-pkg-config"
        check_command clang || missing="$missing clang clang-devel"
        pkg-config --exists libpipewire-0.3 2>/dev/null || missing="$missing pipewire-devel"
        pkg-config --exists dbus-1 2>/dev/null || missing="$missing dbus-devel"

        if [[ -n "$missing" ]]; then
            info "Installing dependencies:$missing"
            sudo dnf install -y $missing
        fi

    elif check_command zypper; then
        # openSUSE
        local missing=""
        check_command cargo || missing="$missing rust cargo"
        check_command git || missing="$missing git"
        check_command pkg-config || missing="$missing pkg-config"
        check_command clang || missing="$missing clang llvm-devel"
        pkg-config --exists libpipewire-0.3 2>/dev/null || missing="$missing pipewire-devel"
        pkg-config --exists dbus-1 2>/dev/null || missing="$missing dbus-1-devel"

        if [[ -n "$missing" ]]; then
            info "Installing dependencies:$missing"
            sudo zypper install -y $missing
        fi

    else
        warn "Unknown package manager. Skipping automatic dependency installation."
        warn "Please ensure required dependencies are installed."
    fi
}

# Get latest version from GitHub
get_latest_version() {
    local version=""

    if check_command curl; then
        version=$(curl -fsSL "https://api.github.com/repos/FrozenTear/sootmix/releases/latest" 2>/dev/null | grep -oE '"tag_name":\s*"[^"]+"' | grep -oE 'v[0-9.]+' || echo "")
    elif check_command wget; then
        version=$(wget -qO- "https://api.github.com/repos/FrozenTear/sootmix/releases/latest" 2>/dev/null | grep -oE '"tag_name":\s*"[^"]+"' | grep -oE 'v[0-9.]+' || echo "")
    fi

    if [[ -z "$version" ]]; then
        warn "Could not fetch latest version from GitHub. Building from main branch."
        version="latest"
    fi

    echo "$version"
}

# Download file
download() {
    local url="$1"
    local dest="$2"

    if check_command curl; then
        curl -fsSL "$url" -o "$dest"
    elif check_command wget; then
        wget -q "$url" -O "$dest"
    else
        die "No download tool available (curl or wget)"
    fi
}

# Reload systemd and restart daemon if it was running
restart_daemon_if_running() {
    if ! check_command systemctl; then
        return
    fi

    systemctl --user daemon-reload 2>/dev/null || true

    if systemctl --user is-active sootmix-daemon.service >/dev/null 2>&1; then
        info "Restarting sootmix-daemon..."
        systemctl --user restart sootmix-daemon.service
        success "sootmix-daemon restarted with new version."
    fi
}

# Install from pre-built binary
install_binary() {
    info "Installing from pre-built binary..."

    check_binary_deps

    # Get version
    local version="$VERSION"
    if [[ "$version" == "latest" ]]; then
        info "Fetching latest version..."
        version=$(get_latest_version)
    fi

    if [[ "$version" == "latest" ]]; then
        warn "No releases found. Falling back to source installation..."
        METHOD="source"
        install_source
        return
    fi

    # Ensure version starts with 'v'
    if [[ "$version" != v* ]]; then
        version="v$version"
    fi

    info "Version: $version"

    # Create temp directory
    TEMP_DIR=$(mktemp -d)
    cd "$TEMP_DIR"

    # Download archive
    local arch
    arch="$(uname -m)"
    local archive_name="sootmix-${version}-${arch}-unknown-linux-gnu.tar.gz"
    local download_url="${REPO_URL}/releases/download/${version}/${archive_name}"

    info "Downloading $archive_name..."
    if ! download "$download_url" "$archive_name" 2>/dev/null; then
        warn "Pre-built binary not available for this version/architecture."
        warn "Falling back to source installation..."
        METHOD="source"
        install_source
        return
    fi

    # Extract
    info "Extracting..."
    tar -xzf "$archive_name"

    # Enter extracted directory
    local extract_dir="sootmix-${version}-${arch}-unknown-linux-gnu"
    cd "$extract_dir"

    # Install binaries
    info "Installing to $PREFIX..."

    local use_sudo=""
    if [[ ! -w "$PREFIX/bin" ]] && [[ -d "$PREFIX/bin" || ! -w "$(dirname "$PREFIX")" ]]; then
        info "Requesting sudo for installation..."
        use_sudo="sudo"
    fi

    $use_sudo install -Dm755 sootmix "$PREFIX/bin/sootmix"
    $use_sudo install -Dm755 sootmix-daemon "$PREFIX/bin/sootmix-daemon"

    # Install desktop file and icons if present
    if [[ -f "sootmix.desktop" ]]; then
        $use_sudo install -Dm644 sootmix.desktop "$PREFIX/share/applications/sootmix.desktop"
    fi

    if [[ -f "sootmix.svg" ]]; then
        $use_sudo install -Dm644 sootmix.svg "$PREFIX/share/icons/hicolor/scalable/apps/sootmix.svg"
    fi

    # Install systemd service if present
    if [[ -f "sootmix-daemon.service" ]]; then
        local systemd_user_dir="$HOME/.config/systemd/user"
        mkdir -p "$systemd_user_dir"
        install -Dm644 sootmix-daemon.service "$systemd_user_dir/sootmix-daemon.service"
    fi

    # Reload and restart daemon if running
    restart_daemon_if_running

    success "Binary installation complete!"
}

# Install from source
install_source() {
    info "Installing from source..."

    try_install_deps
    check_source_deps

    # Create temp directory
    if [[ -z "$TEMP_DIR" ]]; then
        TEMP_DIR=$(mktemp -d)
    fi
    cd "$TEMP_DIR"

    # Clone repository
    info "Cloning repository..."

    local clone_args=("--depth=1")
    if [[ "$VERSION" != "latest" ]]; then
        local version="$VERSION"
        if [[ "$version" != v* ]]; then
            version="v$version"
        fi
        clone_args+=("--branch" "$version")
    fi

    git clone "${clone_args[@]}" "${REPO_URL}.git" "$REPO_NAME"
    cd "$REPO_NAME"

    # Build
    info "Building SootMix (this may take a few minutes)..."
    make build

    # Install
    info "Installing to $PREFIX..."

    local use_sudo=""
    if [[ ! -w "$PREFIX/bin" ]] && [[ -d "$PREFIX/bin" || ! -w "$(dirname "$PREFIX")" ]]; then
        info "Requesting sudo for installation..."
        use_sudo="sudo"
    fi

    $use_sudo make install PREFIX="$PREFIX"

    # Update icon cache
    if check_command gtk-update-icon-cache; then
        $use_sudo gtk-update-icon-cache -f -t "$PREFIX/share/icons/hicolor" 2>/dev/null || true
    fi

    # Reload and restart daemon if running
    restart_daemon_if_running

    success "Source installation complete!"
}

# Print post-install instructions
print_post_install() {
    echo ""
    success "SootMix has been installed successfully!"
    echo ""
    echo "To get started:"
    echo "  1. Enable the daemon:  systemctl --user enable --now sootmix-daemon.service"
    echo "  2. Launch the UI:      sootmix"
    echo ""
    echo "Or find 'SootMix' in your application menu."
    echo ""
    echo "To view daemon logs:"
    echo "  journalctl --user -u sootmix-daemon.service -f"
    echo ""
    echo "To uninstall:"
    echo "  sudo rm -f $PREFIX/bin/sootmix $PREFIX/bin/sootmix-daemon"
    echo "  sudo rm -f $PREFIX/share/applications/sootmix.desktop"
    echo "  sudo rm -f $PREFIX/share/icons/hicolor/scalable/apps/sootmix.svg"
    echo "  rm -f ~/.config/systemd/user/sootmix-daemon.service"
    echo ""

    # Check if binaries are in PATH
    if ! check_command sootmix; then
        warn "$PREFIX/bin is not in your PATH"
        echo "Add it with:"
        echo "  export PATH=\"$PREFIX/bin:\$PATH\""
        echo ""
    fi
}

# Parse arguments
parse_args() {
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --prefix=*)
                PREFIX="${1#*=}"
                shift
                ;;
            --prefix)
                PREFIX="$2"
                shift 2
                ;;
            --method=*)
                METHOD="${1#*=}"
                shift
                ;;
            --method)
                METHOD="$2"
                shift 2
                ;;
            --version=*)
                VERSION="${1#*=}"
                shift
                ;;
            --version)
                VERSION="$2"
                shift 2
                ;;
            --binary)
                METHOD="binary"
                shift
                ;;
            --source)
                METHOD="source"
                shift
                ;;
            --help|-h)
                echo "SootMix Installer"
                echo ""
                echo "Usage: $0 [OPTIONS]"
                echo ""
                echo "Options:"
                echo "  --prefix=PATH    Installation prefix (default: /usr/local)"
                echo "  --method=METHOD  Installation method: binary or source (default: binary)"
                echo "  --version=VER    Version to install (default: latest)"
                echo "  --binary         Shorthand for --method=binary"
                echo "  --source         Shorthand for --method=source"
                echo "  -h, --help       Show this help message"
                echo ""
                echo "Environment variables:"
                echo "  SOOTMIX_PREFIX    Same as --prefix"
                echo "  SOOTMIX_METHOD    Same as --method"
                echo "  SOOTMIX_VERSION   Same as --version"
                echo ""
                echo "Examples:"
                echo "  # Install pre-built binary (default)"
                echo "  curl -sSL https://raw.githubusercontent.com/FrozenTear/sootmix/master/contrib/install.sh | sh"
                echo ""
                echo "  # Install from source"
                echo "  curl -sSL https://raw.githubusercontent.com/FrozenTear/sootmix/master/contrib/install.sh | sh -s -- --source"
                echo ""
                echo "  # Install specific version to custom prefix"
                echo "  curl -sSL https://raw.githubusercontent.com/FrozenTear/sootmix/master/contrib/install.sh | sh -s -- --prefix=/usr --version=0.1.0"
                echo ""
                exit 0
                ;;
            *)
                warn "Unknown option: $1"
                shift
                ;;
        esac
    done
}

# Main
main() {
    parse_args "$@"

    print_banner
    check_platform

    echo ""
    info "Installation prefix: $PREFIX"
    info "Installation method: $METHOD"
    info "Version: $VERSION"
    echo ""

    case "$METHOD" in
        binary)
            install_binary
            ;;
        source)
            install_source
            ;;
        *)
            die "Unknown installation method: $METHOD (use 'binary' or 'source')"
            ;;
    esac

    print_post_install
}

main "$@"
