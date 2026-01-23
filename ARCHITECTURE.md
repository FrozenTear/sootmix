# SootMix Architecture

This document describes the architecture of SootMix, a modular audio routing and mixing application for Linux built on PipeWire.

## Table of Contents

1. [Overview](#overview)
2. [System Architecture](#system-architecture)
3. [Module Structure](#module-structure)
4. [Threading Model](#threading-model)
5. [Plugin System](#plugin-system)
6. [State Management](#state-management)
7. [Audio Pipeline](#audio-pipeline)
8. [Configuration](#configuration)
9. [Future Extensibility](#future-extensibility)

---

## Overview

SootMix is designed with these core principles:

- **Modularity**: Clean separation between audio backend, UI, plugins, and configuration
- **Real-time Safety**: Lock-free communication on audio paths
- **Extensibility**: Plugin architecture supporting both native and sandboxed plugins
- **Reliability**: Graceful degradation when PipeWire is unavailable

### Tech Stack

| Layer | Technology | Purpose |
|-------|------------|---------|
| GUI | Iced 0.14 | Cross-platform Elm-architecture UI |
| Audio | PipeWire 0.9 | Linux audio routing and mixing |
| Async | Tokio | Background tasks, file I/O |
| Plugins | abi_stable + WASM | Native performance + sandboxed third-party |
| Config | TOML + Serde | Human-readable configuration |

---

## System Architecture

```
┌─────────────────────────────────────────────────────────────────────────┐
│                              SootMix                                     │
├─────────────────────────────────────────────────────────────────────────┤
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐    │
│  │     UI      │  │   State     │  │   Plugins   │  │   Config    │    │
│  │   (Iced)    │  │  Manager    │  │   Manager   │  │   Manager   │    │
│  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘    │
│         │                │                │                │            │
│         └────────────────┼────────────────┼────────────────┘            │
│                          │                │                             │
│                    ┌─────▼─────┐    ┌─────▼─────┐                       │
│                    │  Message  │    │  Plugin   │                       │
│                    │   Bus     │    │   Host    │                       │
│                    └─────┬─────┘    └─────┬─────┘                       │
│                          │                │                             │
├──────────────────────────┼────────────────┼─────────────────────────────┤
│                          │                │                             │
│  ┌───────────────────────▼────────────────▼───────────────────────┐    │
│  │                    Audio Subsystem                              │    │
│  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐             │    │
│  │  │  PW Thread  │  │   Routing   │  │   DSP       │             │    │
│  │  │  (MainLoop) │  │   Engine    │  │   Chain     │             │    │
│  │  └──────┬──────┘  └─────────────┘  └─────────────┘             │    │
│  └─────────┼──────────────────────────────────────────────────────┘    │
│            │                                                            │
├────────────┼────────────────────────────────────────────────────────────┤
│            ▼                                                            │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │                        PipeWire Server                           │   │
│  │  [Apps] ──► [Virtual Sinks] ──► [Plugins] ──► [Output Device]   │   │
│  └─────────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────────┘
```

---

## Module Structure

```
sootmix/
├── Cargo.toml                    # Workspace root (future)
│
├── src/
│   ├── main.rs                   # Entry point, logging init
│   ├── app.rs                    # Iced application, update/view
│   ├── state.rs                  # Application state, MixerChannel
│   ├── message.rs                # Message enum for all events
│   │
│   ├── audio/                    # Audio subsystem
│   │   ├── mod.rs
│   │   ├── pipewire_thread.rs    # PW MainLoop, command handling
│   │   ├── types.rs              # PwNode, PwPort, PwLink
│   │   ├── routing.rs            # Link management, app routing
│   │   ├── virtual_sink.rs       # Virtual sink creation/destruction
│   │   └── volume.rs             # Volume control via Props
│   │
│   ├── ui/                       # User interface components
│   │   ├── mod.rs
│   │   ├── theme.rs              # Colors, styling constants
│   │   └── channel_strip.rs      # Mixer channel widget
│   │
│   ├── config/                   # Configuration management
│   │   ├── mod.rs
│   │   ├── app_config.rs         # Global application settings
│   │   ├── preset.rs             # Channel/routing presets
│   │   ├── eq_preset.rs          # EQ curve presets
│   │   └── persistence.rs        # Load/save, migrations
│   │
│   ├── plugins/                  # Plugin system (NEW)
│   │   ├── mod.rs
│   │   ├── manager.rs            # Discovery, lifecycle
│   │   ├── host.rs               # Host functions for plugins
│   │   ├── native.rs             # abi_stable native loader
│   │   └── wasm.rs               # WASM sandbox loader
│   │
│   ├── dsp/                      # Built-in DSP (NEW)
│   │   ├── mod.rs
│   │   ├── eq.rs                 # Parametric EQ
│   │   └── dynamics.rs           # Compressor/limiter
│   │
│   └── realtime/                 # Real-time utilities (NEW)
│       ├── mod.rs
│       ├── ringbuf.rs            # Lock-free ring buffer
│       └── atomic_params.rs      # Atomic parameter updates
│
└── crates/                       # Workspace crates (future)
    └── sootmix-plugin-api/       # Shared plugin interface
        ├── Cargo.toml
        └── src/
            └── lib.rs            # AudioEffect trait, FFI types
```

---

## Threading Model

SootMix uses a multi-threaded architecture with careful separation of concerns:

### Thread Responsibilities

| Thread | Responsibility | Blocking Allowed |
|--------|---------------|------------------|
| **Main (UI)** | Iced event loop, rendering | Yes (UI only) |
| **PipeWire** | PW MainLoop, graph events | No (real-time) |
| **Tokio Pool** | File I/O, config saves | Yes |

### Communication Channels

```
┌──────────────┐                    ┌──────────────┐
│   UI Thread  │                    │  PW Thread   │
│    (Iced)    │                    │  (MainLoop)  │
└──────┬───────┘                    └──────┬───────┘
       │                                   │
       │  PwCommand (SetVolume, etc.)      │
       ├──────────────────────────────────►│
       │  (pipewire::channel::Sender)      │
       │                                   │
       │  PwEvent (NodeAdded, etc.)        │
       │◄──────────────────────────────────┤
       │  (std::sync::mpsc::Receiver)      │
       │                                   │
```

### Real-Time Safety Rules

1. **Never allocate** on audio/PW thread
2. **Never lock** mutexes that UI thread holds
3. **Use atomic operations** for parameters (volume, mute)
4. **Use lock-free queues** for events (rtrb, crossbeam)

### Channel Types by Use Case

| Use Case | Channel Type | Crate |
|----------|-------------|-------|
| UI → PipeWire commands | `pipewire::channel` | pipewire |
| PipeWire → UI events | `mpsc::channel` | std |
| Audio param updates | `AtomicF32` | std::sync::atomic |
| Large state swaps | `Arc<T>` + atomic swap | std |
| Meter data (VU, peak) | SPSC ring buffer | rtrb |

---

## Plugin System

### Overview

SootMix supports two types of plugins for different use cases:

| Type | Technology | Use Case | Safety |
|------|------------|----------|--------|
| **Native** | abi_stable + libloading | Trusted, performance-critical | Full system access |
| **WASM** | wasmtime | Third-party, user-installed | Sandboxed, capability-based |

### Plugin API (sootmix-plugin-api)

```rust
/// Core trait all audio effect plugins must implement.
#[repr(C)]
#[sabi(StableAbi)]
pub trait AudioEffect: Send + Sync {
    /// Plugin metadata.
    fn info(&self) -> PluginInfo;

    /// Called when plugin is loaded. Initialize state here.
    fn activate(&mut self, sample_rate: f32, max_block_size: usize);

    /// Called when plugin is unloaded. Clean up here.
    fn deactivate(&mut self);

    /// Process audio. MUST be real-time safe.
    fn process(&mut self, inputs: &[&[f32]], outputs: &mut [&mut [f32]]);

    /// Get parameter count.
    fn parameter_count(&self) -> usize;

    /// Get parameter info by index.
    fn parameter_info(&self, index: usize) -> Option<ParameterInfo>;

    /// Get current parameter value.
    fn get_parameter(&self, index: usize) -> f32;

    /// Set parameter value. Must be thread-safe.
    fn set_parameter(&mut self, index: usize, value: f32);

    /// Serialize plugin state for preset saving.
    fn save_state(&self) -> Vec<u8>;

    /// Restore plugin state from preset.
    fn load_state(&mut self, data: &[u8]) -> Result<(), PluginError>;
}
```

### Plugin Discovery

```
~/.local/share/sootmix/plugins/
├── native/                       # Native .so plugins
│   ├── sootmix-eq.so
│   └── sootmix-compressor.so
└── wasm/                         # WASM plugins
    ├── community-reverb.wasm
    └── user-filter.wasm
```

### Plugin Lifecycle

```
1. Discovery     ─► Scan plugin directories
2. Validation    ─► Check ABI version, capabilities
3. Loading       ─► dlopen (native) or instantiate (WASM)
4. Activation    ─► Call activate() with audio params
5. Processing    ─► Call process() on audio thread
6. Deactivation  ─► Call deactivate() before unload
7. Unloading     ─► dlclose or drop WASM instance
```

### WASM Capabilities (Sandboxing)

WASM plugins run with restricted capabilities:

```rust
pub struct WasmCapabilities {
    /// Can read files (for loading samples/IRs)
    pub file_read: Option<Vec<PathBuf>>,
    /// Can write files (for caching)
    pub file_write: Option<Vec<PathBuf>>,
    /// Network access (for online presets)
    pub network: bool,
    /// Max memory usage
    pub max_memory_mb: usize,
    /// Max CPU time per process call
    pub max_cpu_us: u64,
}
```

---

## State Management

### State Hierarchy

```
AppState (src/state.rs)
├── channels: Vec<MixerChannel>     # User-created mixer channels
│   ├── id: Uuid
│   ├── name: String
│   ├── volume_db: f32
│   ├── muted: bool
│   ├── eq_enabled: bool
│   ├── assigned_apps: Vec<String>
│   ├── pw_sink_id: Option<u32>     # Runtime only
│   └── plugins: Vec<PluginInstance># Active plugin chain
│
├── master_volume_db: f32
├── master_muted: bool
├── output_device: Option<String>
├── current_preset: String
│
├── pw_graph: PwGraphState          # Live PipeWire state
│   ├── nodes: HashMap<u32, PwNode>
│   ├── ports: HashMap<u32, PwPort>
│   └── links: HashMap<u32, PwLink>
│
└── ui_state: UiState               # Transient UI state
    ├── eq_panel_channel: Option<Uuid>
    ├── settings_open: bool
    └── dragging_app: Option<AppDrag>
```

### Message Flow (Elm Architecture)

```
User Action
    │
    ▼
┌─────────────────┐
│  UI Event       │  (click, drag, slider)
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  Message        │  ChannelVolumeChanged(id, 0.75)
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  update()       │  Modify state, spawn tasks
└────────┬────────┘
         │
         ├──────────────────────────────────┐
         │                                  │
         ▼                                  ▼
┌─────────────────┐              ┌─────────────────┐
│  view()         │              │  Side Effects   │
│  (re-render)    │              │  (PW commands)  │
└─────────────────┘              └─────────────────┘
```

### Undo/Redo (Future)

```rust
pub struct UndoManager {
    undo_stack: Vec<Command>,
    redo_stack: Vec<Command>,
    max_history: usize,
}

pub enum Command {
    SetChannelVolume { id: Uuid, old: f32, new: f32 },
    SetChannelMute { id: Uuid, old: bool, new: bool },
    MoveChannel { id: Uuid, old_index: usize, new_index: usize },
    // ... coalesced for continuous changes (slider drag)
}
```

---

## Audio Pipeline

### Signal Flow

```
┌─────────────────────────────────────────────────────────────────────┐
│                        Per-Channel Pipeline                         │
│                                                                     │
│  ┌─────────┐   ┌─────────┐   ┌─────────┐   ┌─────────┐            │
│  │  Input  │──►│  Gain   │──►│   EQ    │──►│ Plugins │──► To Mix  │
│  │  (Sink) │   │ (Volume)│   │         │   │  Chain  │            │
│  └─────────┘   └─────────┘   └─────────┘   └─────────┘            │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────────┐
│                         Master Pipeline                             │
│                                                                     │
│  ┌─────────┐   ┌─────────┐   ┌─────────┐   ┌─────────┐            │
│  │   Mix   │──►│ Master  │──►│ Limiter │──►│ Output  │            │
│  │ (Sum)   │   │  Gain   │   │ (Brick) │   │ Device  │            │
│  └─────────┘   └─────────┘   └─────────┘   └─────────┘            │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

### PipeWire Graph Structure

```
                    SootMix Virtual Sinks
                    ─────────────────────
                              │
Firefox ────────────┐         │
                    │         ▼
Spotify ────────────┼───► [Game Sink] ──► [EQ Node] ──┐
                    │                                  │
Discord ────────────┼───► [Voice Sink] ───────────────┼──► [Master Mix] ──► Speakers
                    │                                  │
Game ───────────────┼───► [Music Sink] ──► [Comp] ────┘
                    │
System Sounds ──────┘
```

---

## Configuration

### Directory Structure

```
~/.config/sootmix/
├── config.toml              # Global app settings
├── presets/
│   ├── default.toml         # Default channel layout
│   ├── gaming.toml          # Gaming preset
│   └── streaming.toml       # OBS/streaming preset
└── eq/
    ├── flat.toml            # Flat EQ
    ├── bass-boost.toml      # Bass boost curve
    └── custom-1.toml        # User-created

~/.local/share/sootmix/
├── plugins/                 # Plugin storage
│   ├── native/
│   └── wasm/
└── cache/                   # Plugin metadata cache
```

### Config File Format (config.toml)

```toml
[general]
start_minimized = false
minimize_to_tray = true
auto_connect_apps = true

[audio]
sample_rate = 48000
buffer_size = 512
default_volume_db = 0.0

[ui]
theme = "dark"
show_vu_meters = true
channel_width = 120

[keybinds]
mute_all = "Ctrl+M"
next_preset = "Ctrl+Right"
prev_preset = "Ctrl+Left"
```

### Preset File Format (presets/default.toml)

```toml
version = 1
name = "Default"
description = "Standard mixer layout"

[[channels]]
name = "Game"
volume_db = 0.0
muted = false
eq_enabled = false
eq_preset = "Flat"
app_patterns = ["steam_app_*", "*.exe"]

[[channels]]
name = "Voice"
volume_db = -3.0
muted = false
eq_enabled = true
eq_preset = "voice-clarity"
app_patterns = ["discord", "zoom", "teams"]

[master]
volume_db = 0.0
muted = false
output_device = "alsa_output.pci-0000_00_1f.3.analog-stereo"
```

---

## Future Extensibility

### Phase 2: Enhanced Features

- [ ] VU meters with peak hold
- [ ] Drag-and-drop app assignment
- [ ] Per-channel plugin chains
- [ ] Snapshot recall (A/B comparison)
- [ ] MIDI controller mapping

### Phase 3: Advanced Features

- [ ] Multi-device routing (different outputs per channel)
- [ ] Sidechain compression
- [ ] Recording/loopback
- [ ] Remote control API (WebSocket)
- [ ] Lua scripting for automation

### Plugin Ecosystem Goals

1. **Core Plugins** (native, bundled)
   - Parametric EQ
   - Compressor/Limiter
   - Noise Gate

2. **Community Plugins** (WASM, downloadable)
   - Reverb/Delay effects
   - Vocal processors
   - Creative effects

3. **Third-party Bridges** (future)
   - LV2 plugin host
   - VST3 bridge (wine-based)

---

## Development Guidelines

### Adding a New Feature

1. Define messages in `src/message.rs`
2. Update state in `src/state.rs`
3. Handle messages in `src/app.rs` update()
4. Add UI in `src/ui/` modules
5. Add tests for state transitions

### Adding a New Plugin Type

1. Define interface in `sootmix-plugin-api`
2. Implement loader in `src/plugins/`
3. Register with PluginManager
4. Add UI for plugin parameters

### Real-Time Code Checklist

- [ ] No heap allocations (Vec::push, String, Box::new)
- [ ] No mutex locks (use atomics or channels)
- [ ] No file I/O
- [ ] No unbounded loops
- [ ] No panics (use checked arithmetic)

---

## References

- [PipeWire Documentation](https://docs.pipewire.org/)
- [pipewire-rs Crate](https://crates.io/crates/pipewire)
- [Iced GUI Framework](https://iced.rs/)
- [abi_stable Crate](https://crates.io/crates/abi_stable)
- [Real-time Audio Programming 101](https://www.rossbencina.com/code/real-time-audio-programming-101-time-waits-for-nothing)
