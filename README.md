# SootMix

Audio routing and mixing application for Linux using PipeWire.

## Features

- **Channel mixer** with per-application volume control and VU metering
- **Input channels** with mic selection, gain control, and device hot-plug
- **Noise suppression** via built-in RNNoise for input channels
- **Audio routing** between applications, virtual sinks, and hardware devices
- **Plugin system** with LV2 support and a plugin downloader
- **Output device picker** per channel with system default fallback
- **System tray** integration with minimize-to-tray
- **D-Bus API** for external control
- **Auto-reconnect** when hardware devices are added/removed

## Installation

### Quick Install (Debian/Ubuntu/Fedora/Arch)

```bash
# Install from source
curl -sSL https://raw.githubusercontent.com/FrozenTear/sootmix/master/contrib/install.sh | sh

# Install pre-built binary
curl -sSL https://raw.githubusercontent.com/FrozenTear/sootmix/master/contrib/install.sh | sh -s -- --binary
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

### Build from Source

**Dependencies:**
- Rust toolchain (rustup.rs)
- PipeWire development libraries
- PulseAudio client library (for metering)
- D-Bus development libraries
- clang

**Debian/Ubuntu:**
```bash
sudo apt install build-essential pkg-config libpipewire-0.3-dev libpulse-dev libdbus-1-dev clang libclang-dev
```

**Fedora:**
```bash
sudo dnf install rust cargo pkgconf-pkg-config pipewire-devel pulseaudio-libs-devel dbus-devel clang clang-devel
```

**Arch Linux:**
```bash
sudo pacman -S rust pipewire libpulse clang
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
