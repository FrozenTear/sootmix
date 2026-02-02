# Claude Code Guidelines for SootMix

This document contains project-specific instructions for Claude Code when working on the SootMix codebase.

## PipeWire API: Hybrid Approach (Native + CLI)

SootMix uses a **hybrid approach** for PipeWire integration: native API where possible, CLI tools where necessary.

### Why Hybrid? (pw_stream Limitation)

**Critical technical limitation discovered**: The native `pw_stream` API creates a single interleaved audio port regardless of channel count settings. Hardware audio devices have separate FL/FR ports. WirePlumber cannot link these due to format mismatch.

- `pw_stream`: Always creates one port (e.g., `output_1`) even with `audio.channels=2` and `audio.position=FL,FR`
- `pw_filter`: Can create per-channel ports but has **no Rust bindings** in pipewire-rs
- **Adapter module** (`libspa-audioconvert`): Creates proper FL/FR DSP ports, but only loaded for:
  - PulseAudio compatibility layer
  - `pw-loopback` CLI tool
  - NOT for raw pw_stream connections

**Result**: Virtual sinks/sources MUST use `pw-loopback` CLI for proper stereo port creation.

### What to Use Native API For

Use `pipewire-rs` native API for:
```rust
// Monitoring the graph (registry listener)
registry.add_listener_local()
    .global(|global| { /* handle nodes, ports, links */ })
    .register();

// Creating links between existing nodes
let link = core.create_object::<Link>("link-factory", &properties! {
    "link.output.port" => output_port,
    "link.input.port" => input_port,
});

// WirePlumber metadata for routing preferences
// Volume control via Props params (future work)
```

### What Requires CLI Tools

**pw-loopback** - Required for virtual sink/source creation:
```rust
// Creates Audio/Sink with proper FL/FR ports
Command::new("pw-loopback")
    .arg("--capture-props")
    .arg("media.class=Audio/Sink node.name=sootmix.Channel ...")
    .arg("--playback-props")
    .arg("media.class=Stream/Output/Audio ...")
```

**wpctl** - Volume control (until native Props implementation):
```rust
Command::new("wpctl").args(["set-volume", &node_id.to_string(), volume_str])
```

### Patterns to Avoid

- Don't attempt native `pw_stream` for loopback creation - it won't link to hardware
- Don't add `audio.position`, `factory.mode`, or `audio.adapt.follower` properties to streams hoping for adapter loading - they don't work
- Don't use `pw-dump` for continuous monitoring - use registry listener instead

### Reference Implementations

- `crates/sootmix-daemon/src/audio/virtual_sink.rs` - CLI-based virtual sink/source (correct approach)
- `crates/sootmix-daemon/src/audio/pipewire_thread.rs` - Registry listener and native link creation
- `crates/sootmix-daemon/src/audio/native_loopback.rs` - Native stream code (kept for reference, not used for loopback)

## Code Style

### Avoid Over-Engineering
- Keep solutions simple and focused
- Don't add features beyond what was asked
- Don't add comments to code you didn't change
- Prefer editing existing files over creating new ones

### Error Handling
- Use `Result` types properly
- Don't silently ignore errors
- Log warnings/errors with `tracing`

### Thread Safety
- All PipeWire operations MUST happen on the PipeWire main loop thread
- Use `Rc<RefCell<>>` for PW thread-local state
- Use `Arc<Mutex<>>` for cross-thread communication
- Audio processing callbacks must be RT-safe (no allocations, no locks that can block)

## Architecture Notes

### GUI vs Daemon
- `src/` - Standalone GUI app (native PW API for metering/plugins)
- `crates/sootmix-daemon/` - Background daemon (hybrid: CLI for sinks, native for graph monitoring)

The daemon receives commands via D-Bus and controls PipeWire. The GUI can run standalone or connect to the daemon.

### Audio Flow
```
Apps → Virtual Sinks (Audio/Sink) → Plugin Processing → Master Output
Mics → Virtual Sources (Audio/Source) → Apps
```

## Testing

Before considering work complete:
1. `cargo check` passes
2. `cargo build` succeeds
3. Manual testing in Helvum shows correct node topology
4. Audio actually flows through the system
