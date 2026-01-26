#!/bin/bash
# Install SootMix daemon as a systemd user service
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SERVICE_FILE="$SCRIPT_DIR/sootmix-daemon.service"

# Check if running as root
if [ "$EUID" -eq 0 ]; then
    echo "Error: Do not run this script as root. It installs a user service."
    exit 1
fi

# Create systemd user directory
SYSTEMD_USER_DIR="$HOME/.config/systemd/user"
mkdir -p "$SYSTEMD_USER_DIR"

# Install service file
echo "Installing systemd service..."
cp "$SERVICE_FILE" "$SYSTEMD_USER_DIR/sootmix-daemon.service"

# Check if daemon binary exists
if command -v sootmix-daemon &> /dev/null; then
    DAEMON_PATH="$(which sootmix-daemon)"
    echo "Found daemon at: $DAEMON_PATH"
elif [ -f "$HOME/.cargo/bin/sootmix-daemon" ]; then
    DAEMON_PATH="$HOME/.cargo/bin/sootmix-daemon"
    echo "Found daemon at: $DAEMON_PATH"
else
    echo "Warning: sootmix-daemon not found in PATH or ~/.cargo/bin"
    echo "Please install it with: cargo install --path crates/sootmix-daemon"
fi

# Reload systemd
echo "Reloading systemd user daemon..."
systemctl --user daemon-reload

echo ""
echo "Installation complete!"
echo ""
echo "To enable the daemon to start automatically:"
echo "  systemctl --user enable sootmix-daemon.service"
echo ""
echo "To start the daemon now:"
echo "  systemctl --user start sootmix-daemon.service"
echo ""
echo "To check status:"
echo "  systemctl --user status sootmix-daemon.service"
echo ""
echo "To view logs:"
echo "  journalctl --user -u sootmix-daemon.service -f"
