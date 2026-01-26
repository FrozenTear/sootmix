# SootMix

Audio routing and mixing application for Linux using PipeWire.

## Features

- Per-application volume control
- Audio routing between applications and devices
- System tray integration
- D-Bus API for external control

## Installation

### Quick Install (Debian/Ubuntu/Fedora/Arch)

```bash
curl -sSL https://raw.githubusercontent.com/FrozenTear/sootmix/master/contrib/install.sh | sh
```

### Arch Linux (AUR)

```bash
# Using yay
yay -S sootmix

# Or manually with PKGBUILD
git clone https://github.com/FrozenTear/sootmix.git
cd sootmix
makepkg -si
```

### Manual Build

**Dependencies:**
- Rust toolchain (rustup.rs)
- PipeWire development libraries
- D-Bus development libraries
- clang

**Debian/Ubuntu:**
```bash
sudo apt install build-essential pkg-config libpipewire-0.3-dev libdbus-1-dev clang libclang-dev
```

**Fedora:**
```bash
sudo dnf install rust cargo pkgconf-pkg-config pipewire-devel dbus-devel clang clang-devel
```

**Arch Linux:**
```bash
sudo pacman -S rust pipewire clang
```

**Build and install:**
```bash
git clone https://github.com/FrozenTear/sootmix.git
cd sootmix
make
sudo make install
```

## Usage

Start the daemon:
```bash
systemctl --user enable --now sootmix-daemon.service
```

Launch the UI:
```bash
sootmix
```

Or find "SootMix" in your application menu.

## License

MPL-2.0
