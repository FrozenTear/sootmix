// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! PipeWire thread management and event handling.
//!
//! This module implements native PipeWire control using the pipewire-rs API.
//! All PipeWire operations run on a dedicated thread since PipeWire objects
//! are not Send/Sync.

use crate::audio::control::{build_channel_volumes_pod, build_mute_pod, build_volume_mute_pod};
use crate::audio::meter_stream::{AtomicMeterLevels, MeterStreamManager};
use crate::audio::plugin_stream::PluginFilterStreams;
use crate::audio::types::{AudioChannel, MediaClass, PortDirection, PwLink, PwNode, PwPort};
use crate::plugins::SharedPluginInstances;
use crate::realtime::{PluginParamUpdate, RingBuffer, RingBufferWriter};
use std::sync::Arc;
use pipewire::link::Link;
use pipewire::node::{Node, NodeListener};
use pipewire::properties::properties;
use pipewire::spa::param::ParamType;
use pipewire::spa::pod::Pod;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::{mpsc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use thiserror::Error;
use tracing::{debug, error, info, trace, warn};
use uuid::Uuid;

/// Commands sent from the UI thread to the PipeWire thread.
#[derive(Clone)]
pub enum PwCommand {
    /// Create a virtual sink for a channel.
    /// Uses channel name for node.name (readable in Helvum), description for display.
    CreateVirtualSink { channel_id: Uuid, name: String },
    /// Create a virtual source for an input (mic) channel.
    CreateVirtualSource { channel_id: Uuid, name: String },
    /// Destroy a virtual sink or source.
    DestroyVirtualSink { node_id: u32 },
    /// Update the display name (node.description) of a virtual sink.
    /// This allows renaming without recreating the sink (no audio interruption).
    UpdateSinkDescription { node_id: u32, description: String },
    /// Bind to an existing node for control (used for adopted sinks).
    BindNode { node_id: u32 },
    /// Unbind from a node (release proxy).
    UnbindNode { node_id: u32 },
    /// Create a link between two ports.
    CreateLink { output_port: u32, input_port: u32 },
    /// Destroy a link.
    DestroyLink { link_id: u32 },
    /// Set volume on a node (linear scale: 0.0-1.5).
    SetVolume { node_id: u32, volume: f32 },
    /// Set mute state on a node.
    SetMute { node_id: u32, muted: bool },
    /// Set both volume and mute atomically.
    SetVolumeMute {
        node_id: u32,
        volume: f32,
        muted: bool,
    },
    /// Set EQ parameters on a filter-chain node.
    SetEqParams {
        node_id: u32,
        band: String,
        freq: f32,
        q: f32,
        gain: f32,
    },
    /// Set the default audio output sink.
    SetDefaultSink { node_id: u32 },
    /// Create a plugin filter stream for a channel.
    ///
    /// This creates a filter node that routes audio through the plugin chain.
    /// The filter captures audio from the channel's virtual sink and outputs
    /// processed audio to the master sink.
    CreatePluginFilter {
        channel_id: Uuid,
        channel_name: String,
        /// Plugin instance IDs in processing order.
        plugin_chain: Vec<Uuid>,
        /// Atomic meter levels for real-time metering.
        meter_levels: Option<std::sync::Arc<crate::audio::meter_stream::AtomicMeterLevels>>,
        /// Optional loopback output node ID for direct routing (bypasses name search).
        loopback_output_node_id: Option<u32>,
    },
    /// Destroy a plugin filter stream.
    DestroyPluginFilter { channel_id: Uuid },
    /// Update the plugin chain for an existing filter.
    UpdatePluginChain {
        channel_id: Uuid,
        plugin_chain: Vec<Uuid>,
    },
    /// Set shared plugin instances for audio processing.
    ///
    /// This should be called once after the PluginManager is initialized.
    /// The SharedPluginInstances will be used by plugin filter streams for
    /// real-time audio processing.
    SetSharedPluginInstances(SharedPluginInstances),
    /// Send a plugin parameter update to the RT thread.
    ///
    /// This is used when plugin parameters are changed from the UI.
    SendPluginParamUpdate {
        channel_id: Uuid,
        instance_id: Uuid,
        param_index: u32,
        value: f32,
    },
    /// Route a channel's loopback output to a specific device.
    ///
    /// This destroys existing links from the loopback output node and creates
    /// new links to the target device. If target_device_id is None, routes to
    /// the default sink.
    RouteChannelToDevice {
        /// The loopback output node ID (Stream/Output/Audio created by pw-loopback).
        loopback_output_node: u32,
        /// Target device node ID, or None to use default sink.
        target_device_id: Option<u32>,
    },
    /// Create a recording source (virtual Audio/Source for capturing SootMix output).
    CreateRecordingSource {
        /// Name for the recording source (e.g., "master").
        name: String,
    },
    /// Destroy a recording source.
    DestroyRecordingSource {
        /// Node ID of the Audio/Source to destroy.
        node_id: u32,
    },
    /// Create a meter capture stream for a channel's virtual sink (output channel).
    ///
    /// This creates a lightweight PipeWire stream that connects to the
    /// virtual sink's monitor ports to capture audio levels in real-time.
    CreateMeterStream {
        channel_id: Uuid,
        channel_name: String,
        /// The virtual sink node ID to meter.
        sink_node_id: u32,
        /// Atomic levels to store peaks (shared with UI).
        meter_levels: Arc<AtomicMeterLevels>,
    },
    /// Create a meter capture stream for a channel's virtual source (input channel).
    ///
    /// This creates a lightweight PipeWire stream that connects to the
    /// virtual source's output ports to capture audio levels in real-time.
    CreateInputMeterStream {
        channel_id: Uuid,
        channel_name: String,
        /// The virtual source node ID to meter.
        source_node_id: u32,
        /// Atomic levels to store peaks (shared with UI).
        meter_levels: Arc<AtomicMeterLevels>,
    },
    /// Destroy a meter capture stream.
    DestroyMeterStream { channel_id: Uuid },
    /// Shutdown the PipeWire thread.
    Shutdown,
}

/// Events sent from the PipeWire thread to the UI.
#[derive(Debug, Clone)]
pub enum PwEvent {
    /// PipeWire connection established.
    Connected,
    /// PipeWire connection lost.
    Disconnected,
    /// Node added to the graph.
    NodeAdded(PwNode),
    /// Node removed from the graph.
    NodeRemoved(u32),
    /// Node properties changed.
    NodeChanged(PwNode),
    /// Port added.
    PortAdded(PwPort),
    /// Port removed.
    PortRemoved(u32),
    /// Link added.
    LinkAdded(PwLink),
    /// Link removed.
    LinkRemoved(u32),
    /// Virtual sink created successfully.
    VirtualSinkCreated { channel_id: Uuid, node_id: u32, loopback_output_node_id: Option<u32> },
    /// Virtual source created successfully (for input/mic channels).
    VirtualSourceCreated { channel_id: Uuid, source_node_id: u32, loopback_capture_node_id: Option<u32> },
    /// Virtual sink destroyed.
    VirtualSinkDestroyed { node_id: u32 },
    /// Recording source created successfully.
    RecordingSourceCreated { name: String, node_id: u32 },
    /// Recording source destroyed.
    RecordingSourceDestroyed { node_id: u32 },
    /// Plugin filter created for a channel.
    PluginFilterCreated {
        channel_id: Uuid,
        sink_node_id: u32,
        output_node_id: u32,
    },
    /// Plugin filter destroyed.
    PluginFilterDestroyed { channel_id: Uuid },
    /// Control parameter changed (volume, mute, etc).
    ParamChanged {
        node_id: u32,
        volume: Option<f32>,
        muted: Option<bool>,
    },
    /// Error occurred.
    Error(String),
}

#[derive(Debug, Error)]
pub enum PwError {
    #[error("PipeWire initialization failed: {0}")]
    InitFailed(String),
    #[error("Failed to connect to PipeWire: {0}")]
    ConnectionFailed(String),
    #[error("PipeWire thread error: {0}")]
    ThreadError(String),
}

/// A bound node proxy with its listener.
struct BoundNode {
    /// The node proxy for controlling the node.
    proxy: Node,
    /// Listener to keep the proxy alive and receive events.
    _listener: NodeListener,
}

/// A created link proxy that we own.
struct CreatedLink {
    /// The link proxy.
    proxy: Link,
}

/// Minimum interval between CLI fallback commands per node.
const CLI_THROTTLE_MS: u64 = 50;

/// Info about a plugin filter for a channel.
struct PluginFilterInfo {
    /// The filter streams.
    streams: PluginFilterStreams,
    /// Ring buffer writer for sending param updates to RT thread.
    param_writer: RingBufferWriter<crate::realtime::PluginParamUpdate>,
}

/// Pending meter stream link creation info.
/// Stored when a meter stream is created, and links are created when ports are discovered.
#[derive(Clone)]
struct PendingMeterLink {
    /// The sink node ID to capture from (for monitor ports).
    sink_node_id: u32,
    /// The sink node name.
    sink_name: String,
    /// The meter stream node ID.
    meter_node_id: u32,
    /// The meter stream node name.
    meter_name: String,
}

/// Default sample rate when PipeWire settings unavailable.
const DEFAULT_SAMPLE_RATE: f32 = 48000.0;
/// Default block size when PipeWire settings unavailable.
const DEFAULT_BLOCK_SIZE: usize = 512;

/// Query PipeWire for the current sample rate and quantum (block size).
///
/// Uses `pw-metadata` to query the default clock settings. Falls back to
/// defaults if the query fails.
fn query_pipewire_audio_settings() -> (f32, usize) {
    use std::process::Command;

    // Query sample rate from pw-metadata
    let sample_rate = Command::new("pw-metadata")
        .args(["-n", "settings", "0", "clock.rate"])
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                // Output format: "Found \"settings\" metadata 0\nvalue:'48000' type:'Spa:Int'"
                stdout
                    .lines()
                    .find(|line| line.contains("value:"))
                    .and_then(|line| {
                        line.split("value:'")
                            .nth(1)
                            .and_then(|s| s.split('\'').next())
                            .and_then(|s| s.parse::<f32>().ok())
                    })
            } else {
                None
            }
        })
        .unwrap_or_else(|| {
            debug!("Could not query PipeWire sample rate, using default: {}", DEFAULT_SAMPLE_RATE);
            DEFAULT_SAMPLE_RATE
        });

    // Query quantum (block size) from pw-metadata
    let block_size = Command::new("pw-metadata")
        .args(["-n", "settings", "0", "clock.quantum"])
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                stdout
                    .lines()
                    .find(|line| line.contains("value:"))
                    .and_then(|line| {
                        line.split("value:'")
                            .nth(1)
                            .and_then(|s| s.split('\'').next())
                            .and_then(|s| s.parse::<usize>().ok())
                    })
            } else {
                None
            }
        })
        .unwrap_or_else(|| {
            debug!("Could not query PipeWire quantum, using default: {}", DEFAULT_BLOCK_SIZE);
            DEFAULT_BLOCK_SIZE
        });

    info!("PipeWire audio settings: sample_rate={}, block_size={}", sample_rate, block_size);
    (sample_rate, block_size)
}

/// State tracked within the PipeWire thread.
struct PwThreadState {
    /// Basic node info indexed by node ID.
    nodes: HashMap<u32, PwNode>,
    /// Ports indexed by port ID.
    ports: HashMap<u32, PwPort>,
    /// Links indexed by link ID.
    links: HashMap<u32, PwLink>,
    /// Map of channel UUID to virtual sink node ID.
    virtual_sinks: HashMap<Uuid, u32>,
    /// Bound node proxies for volume/mute control.
    bound_nodes: HashMap<u32, BoundNode>,
    /// Links we created (port pair -> link proxy).
    created_links: HashMap<(u32, u32), CreatedLink>,
    /// Plugin filter streams by channel ID.
    plugin_filters: HashMap<Uuid, PluginFilterInfo>,
    /// Meter capture streams by channel ID.
    meter_streams: MeterStreamManager,
    /// Pending meter links to create when ports are discovered.
    pending_meter_links: Vec<PendingMeterLink>,
    /// Shared plugin instances for RT audio processing.
    shared_plugin_instances: Option<SharedPluginInstances>,
    /// Event sender for notifying UI.
    event_tx: Rc<mpsc::Sender<PwEvent>>,
    /// Last CLI command time per node (for throttling fallback).
    cli_last_cmd: HashMap<u32, Instant>,
    /// Node IDs currently being processed by CLI background threads.
    /// Prevents concurrent CLI operations on the same node.
    cli_in_flight: Arc<Mutex<HashSet<u32>>>,
    /// PipeWire sample rate (queried on init, defaults to 48000).
    sample_rate: f32,
    /// PipeWire block size (quantum, defaults to 512).
    block_size: usize,
}

impl PwThreadState {
    fn new(event_tx: Rc<mpsc::Sender<PwEvent>>) -> Self {
        // Query PipeWire sample rate from pw-metadata
        let (sample_rate, block_size) = query_pipewire_audio_settings();

        Self {
            nodes: HashMap::new(),
            ports: HashMap::new(),
            links: HashMap::new(),
            virtual_sinks: HashMap::new(),
            bound_nodes: HashMap::new(),
            created_links: HashMap::new(),
            plugin_filters: HashMap::new(),
            meter_streams: MeterStreamManager::new(),
            pending_meter_links: Vec::new(),
            shared_plugin_instances: None,
            event_tx,
            cli_last_cmd: HashMap::new(),
            cli_in_flight: Arc::new(Mutex::new(HashSet::new())),
            sample_rate,
            block_size,
        }
    }

    /// Check if CLI fallback should be throttled for this node.
    /// Returns true if enough time has passed and updates the timestamp.
    fn should_run_cli(&mut self, node_id: u32) -> bool {
        let now = Instant::now();
        let throttle = Duration::from_millis(CLI_THROTTLE_MS);

        if let Some(last) = self.cli_last_cmd.get(&node_id) {
            if now.duration_since(*last) < throttle {
                return false;
            }
        }
        self.cli_last_cmd.insert(node_id, now);
        true
    }

    /// Get node ID for a port.
    fn get_node_for_port(&self, port_id: u32) -> Option<u32> {
        self.ports.get(&port_id).map(|p| p.node_id)
    }

    /// Find a node ID by name.
    fn find_node_by_name(&self, name: &str) -> Option<u32> {
        self.nodes.iter()
            .find(|(_, node)| node.name == name)
            .map(|(&id, _)| id)
    }

    /// Get the loopback output node ID for a channel's virtual sink.
    ///
    /// Virtual sinks are created with pw-loopback which creates two nodes:
    /// - `sootmix.{name}` - Audio/Sink (apps connect here)
    /// - `output.sootmix.{name}.output` - Stream/Output/Audio (loopback output)
    ///
    /// Note: PipeWire's pw-loopback adds an "output." prefix to the --name value
    /// when creating the output node.
    ///
    /// This returns the output node ID for routing through plugin filters.
    fn get_loopback_output_node(&self, channel_name: &str) -> Option<u32> {
        // PipeWire names loopback outputs: output.sootmix.{name}.output
        let output_name = format!("output.sootmix.{}.output", channel_name);
        self.find_node_by_name(&output_name)
    }

    /// Set volume on a bound node using native API.
    ///
    /// Uses channelVolumes (stereo FL/FR) which is what WirePlumber/wpctl uses.
    fn set_node_volume(&self, node_id: u32, volume: f32) -> Result<(), String> {
        let bound = self
            .bound_nodes
            .get(&node_id)
            .ok_or_else(|| format!("Node {} not bound", node_id))?;

        // Use stereo channel volumes (FL, FR) - this is what wpctl uses
        let pod_data = build_channel_volumes_pod(&[volume, volume]).map_err(|e| e.to_string())?;

        let pod = Pod::from_bytes(&pod_data)
            .ok_or_else(|| "Failed to create Pod from bytes".to_string())?;

        bound.proxy.set_param(ParamType::Props, 0, pod);

        trace!("Native volume set on node {}: {:.3} (stereo)", node_id, volume);
        Ok(())
    }

    /// Set mute on a bound node using native API.
    fn set_node_mute(&self, node_id: u32, muted: bool) -> Result<(), String> {
        let bound = self
            .bound_nodes
            .get(&node_id)
            .ok_or_else(|| format!("Node {} not bound", node_id))?;

        let pod_data = build_mute_pod(muted).map_err(|e| e.to_string())?;
        let pod = Pod::from_bytes(&pod_data)
            .ok_or_else(|| "Failed to create Pod from bytes".to_string())?;

        bound.proxy.set_param(ParamType::Props, 0, pod);

        trace!("Native mute set on node {}: {}", node_id, muted);
        Ok(())
    }

    /// Set both volume and mute atomically on a bound node.
    fn set_node_volume_mute(&self, node_id: u32, volume: f32, muted: bool) -> Result<(), String> {
        let bound = self
            .bound_nodes
            .get(&node_id)
            .ok_or_else(|| format!("Node {} not bound", node_id))?;

        let pod_data = build_volume_mute_pod(volume, muted).map_err(|e| e.to_string())?;
        let pod = Pod::from_bytes(&pod_data)
            .ok_or_else(|| "Failed to create Pod from bytes".to_string())?;

        bound.proxy.set_param(ParamType::Props, 0, pod);

        trace!(
            "Native volume+mute set on node {}: vol={:.3}, mute={}",
            node_id,
            volume,
            muted
        );
        Ok(())
    }
}

/// Handle to the PipeWire thread.
pub struct PwThread {
    /// Channel to send commands to PipeWire thread.
    cmd_tx: pipewire::channel::Sender<PwCommand>,
    /// Handle to the spawned thread.
    handle: Option<JoinHandle<()>>,
}

impl PwThread {
    /// Spawn the PipeWire thread and return a handle.
    pub fn spawn(event_tx: mpsc::Sender<PwEvent>) -> Result<Self, PwError> {
        // Create channel for commands TO the PW thread
        let (cmd_tx, cmd_rx) = pipewire::channel::channel::<PwCommand>();

        let handle = thread::Builder::new()
            .name("pipewire".to_string())
            .spawn(move || {
                if let Err(e) = run_pipewire_loop(cmd_rx, event_tx.clone()) {
                    error!("PipeWire thread error: {}", e);
                    let _ = event_tx.send(PwEvent::Error(e.to_string()));
                }
            })
            .map_err(|e| PwError::ThreadError(e.to_string()))?;

        Ok(Self {
            cmd_tx,
            handle: Some(handle),
        })
    }

    /// Send a command to the PipeWire thread.
    pub fn send(&self, cmd: PwCommand) -> Result<(), PwError> {
        self.cmd_tx
            .send(cmd)
            .map_err(|_| PwError::ThreadError("Channel closed".to_string()))
    }

    /// Request shutdown and wait for thread to finish.
    pub fn shutdown(mut self) {
        let _ = self.send(PwCommand::Shutdown);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for PwThread {
    fn drop(&mut self) {
        let _ = self.cmd_tx.send(PwCommand::Shutdown);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Main PipeWire event loop (runs on dedicated thread).
fn run_pipewire_loop(
    cmd_rx: pipewire::channel::Receiver<PwCommand>,
    event_tx: mpsc::Sender<PwEvent>,
) -> Result<(), PwError> {
    // Initialize PipeWire
    pipewire::init();
    info!("PipeWire initialized");

    // Create main loop
    let main_loop =
        pipewire::main_loop::MainLoopRc::new(None).map_err(|e| PwError::InitFailed(e.to_string()))?;

    // Create context and connect
    let context = pipewire::context::ContextRc::new(&main_loop, None)
        .map_err(|e| PwError::InitFailed(e.to_string()))?;

    let core = context
        .connect_rc(None)
        .map_err(|e| PwError::ConnectionFailed(e.to_string()))?;

    let registry = core
        .get_registry_rc()
        .map_err(|e| PwError::ConnectionFailed(e.to_string()))?;

    info!("Connected to PipeWire");
    let _ = event_tx.send(PwEvent::Connected);

    // Thread-local state
    let event_tx = Rc::new(event_tx);
    let state = Rc::new(RefCell::new(PwThreadState::new(event_tx.clone())));

    // Attach command receiver to main loop
    let main_loop_weak = main_loop.downgrade();
    let state_cmd = state.clone();
    let core_cmd = core.clone();
    let registry_cmd = registry.clone();
    let _cmd_receiver = cmd_rx.attach(main_loop.loop_(), move |cmd| {
        handle_command(cmd, &state_cmd, &main_loop_weak, &core_cmd, &registry_cmd);
    });

    // Set up registry listener for discovering nodes, ports, links
    let _registry_listener = setup_registry_listener(&registry, state.clone(), event_tx.clone());

    // Run the main loop (blocks until quit)
    main_loop.run();

    info!("PipeWire thread shutting down");
    let _ = event_tx.send(PwEvent::Disconnected);

    Ok(())
}

/// Spawn a background thread for blocking CLI operations that would stall the PW main loop.
/// Clones the event sender so the background thread can report results/errors.
fn spawn_cli_work<F>(event_tx: &Rc<mpsc::Sender<PwEvent>>, work: F)
where
    F: FnOnce(mpsc::Sender<PwEvent>) + Send + 'static,
{
    let tx = mpsc::Sender::clone(event_tx);
    thread::spawn(move || work(tx));
}

/// Spawn a CLI background thread for a specific node, with in-flight tracking.
/// If the node is already being processed by another CLI thread, the work is skipped.
fn spawn_cli_work_for_node<F>(
    event_tx: &Rc<mpsc::Sender<PwEvent>>,
    in_flight: &Arc<Mutex<HashSet<u32>>>,
    node_id: u32,
    work: F,
) where
    F: FnOnce(mpsc::Sender<PwEvent>) + Send + 'static,
{
    {
        let mut set = in_flight.lock().unwrap_or_else(|e| e.into_inner());
        if set.contains(&node_id) {
            trace!("Node {} already has CLI operation in-flight, skipping", node_id);
            return;
        }
        set.insert(node_id);
    }

    let tx = mpsc::Sender::clone(event_tx);
    let in_flight_clone = Arc::clone(in_flight);
    thread::spawn(move || {
        work(tx);
        let mut set = in_flight_clone.lock().unwrap_or_else(|e| e.into_inner());
        set.remove(&node_id);
    });
}

/// Handle a command from the UI thread.
fn handle_command(
    cmd: PwCommand,
    state: &Rc<RefCell<PwThreadState>>,
    main_loop_weak: &pipewire::main_loop::MainLoopWeak,
    core: &pipewire::core::CoreRc,
    _registry: &pipewire::registry::RegistryRc,
) {
    match cmd {
        PwCommand::Shutdown => {
            debug!("Received shutdown command");
            if let Some(main_loop) = main_loop_weak.upgrade() {
                main_loop.quit();
            }
        }

        PwCommand::CreateVirtualSink { channel_id, name } => {
            debug!("Creating virtual sink: '{}' for channel {}", name, channel_id);
            // Virtual sink creation spawns pw-loopback and polls for the node,
            // which blocks for 200-300ms. Run on a background thread.
            spawn_cli_work(&state.borrow().event_tx, move |event_tx| {
                match crate::audio::virtual_sink::create_virtual_sink_full(&name, &name) {
                    Ok(result) => {
                        let _ = event_tx.send(PwEvent::VirtualSinkCreated {
                            channel_id,
                            node_id: result.sink_node_id,
                            loopback_output_node_id: result.loopback_output_node_id,
                        });
                    }
                    Err(e) => {
                        let _ = event_tx.send(PwEvent::Error(format!(
                            "Failed to create virtual sink: {}",
                            e
                        )));
                    }
                }
            });
        }

        PwCommand::CreateVirtualSource { channel_id, name } => {
            debug!("Creating virtual source: '{}' for channel {}", name, channel_id);
            spawn_cli_work(&state.borrow().event_tx, move |event_tx| {
                match crate::audio::virtual_sink::create_virtual_source(&name, &name) {
                    Ok(result) => {
                        let _ = event_tx.send(PwEvent::VirtualSourceCreated {
                            channel_id,
                            source_node_id: result.source_node_id,
                            loopback_capture_node_id: result.loopback_capture_node_id,
                        });
                    }
                    Err(e) => {
                        let _ = event_tx.send(PwEvent::Error(format!(
                            "Failed to create virtual source: {}",
                            e
                        )));
                    }
                }
            });
        }

        PwCommand::UpdateSinkDescription { node_id, description } => {
            debug!("Updating sink {} description to '{}'", node_id, description);
            spawn_cli_work(&state.borrow().event_tx, move |_event_tx| {
                if let Err(e) = crate::audio::virtual_sink::update_node_description(node_id, &description) {
                    warn!("Failed to update sink description: {}", e);
                }
            });
        }

        PwCommand::DestroyVirtualSink { node_id } => {
            debug!("Destroying virtual sink: {}", node_id);
            state.borrow_mut().bound_nodes.remove(&node_id);
            state
                .borrow_mut()
                .virtual_sinks
                .retain(|_, &mut id| id != node_id);
            spawn_cli_work(&state.borrow().event_tx, move |event_tx| {
                if let Err(e) = crate::audio::virtual_sink::destroy_virtual_sink(node_id) {
                    warn!("Failed to destroy virtual sink {}: {}", node_id, e);
                }
                let _ = event_tx.send(PwEvent::VirtualSinkDestroyed { node_id });
            });
        }

        PwCommand::CreateLink {
            output_port,
            input_port,
        } => {
            info!("PW cmd: CreateLink output_port={} -> input_port={}", output_port, input_port);

            // Get node IDs for the ports
            let (out_node, in_node) = {
                let s = state.borrow();
                (s.get_node_for_port(output_port), s.get_node_for_port(input_port))
            };

            let out_node = match out_node {
                Some(n) => n,
                None => {
                    warn!("Output port {} not found, using CLI fallback", output_port);
                    spawn_cli_work(&state.borrow().event_tx, move |event_tx| {
                        if let Err(e) = crate::audio::routing::create_link(output_port, input_port) {
                            let _ = event_tx.send(PwEvent::Error(format!("Failed to create link: {}", e)));
                        }
                    });
                    return;
                }
            };

            let in_node = match in_node {
                Some(n) => n,
                None => {
                    warn!("Input port {} not found, using CLI fallback", input_port);
                    spawn_cli_work(&state.borrow().event_tx, move |event_tx| {
                        if let Err(e) = crate::audio::routing::create_link(output_port, input_port) {
                            let _ = event_tx.send(PwEvent::Error(format!("Failed to create link: {}", e)));
                        }
                    });
                    return;
                }
            };

            // Create link using native API
            let link_result = core.create_object::<Link>(
                "link-factory",
                &properties! {
                    "link.output.port" => output_port.to_string(),
                    "link.input.port" => input_port.to_string(),
                    "link.output.node" => out_node.to_string(),
                    "link.input.node" => in_node.to_string(),
                    "object.linger" => "true"
                },
            );

            match link_result {
                Ok(link) => {
                    info!("Native link created: {} -> {}", output_port, input_port);
                    state.borrow_mut().created_links.insert(
                        (output_port, input_port),
                        CreatedLink { proxy: link },
                    );
                }
                Err(e) => {
                    warn!("Native link creation failed: {:?}, using CLI fallback", e);
                    spawn_cli_work(&state.borrow().event_tx, move |event_tx| {
                        if let Err(e2) = crate::audio::routing::create_link(output_port, input_port) {
                            let _ = event_tx.send(PwEvent::Error(format!("Failed to create link: {}", e2)));
                        }
                    });
                }
            }
        }

        PwCommand::DestroyLink { link_id } => {
            debug!("Destroying link: {}", link_id);
            spawn_cli_work(&state.borrow().event_tx, move |event_tx| {
                if let Err(e) = crate::audio::routing::destroy_link(link_id) {
                    let _ = event_tx.send(PwEvent::Error(format!("Failed to destroy link: {}", e)));
                }
            });
        }

        PwCommand::BindNode { node_id } => {
            info!("Request to bind to node {} for control", node_id);

            // Check if already bound
            if state.borrow().bound_nodes.contains_key(&node_id) {
                debug!("Node {} already bound", node_id);
                return;
            }

            // Note: We can't bind to a node by ID alone - we need the GlobalObject
            // which comes from the registry listener. For adopted sinks, we'll
            // rely on the CLI fallback (wpctl) for volume/mute control.
            //
            // The node will be auto-bound if it appears in a future registry event.
            // For now, log this and the CLI fallback will handle control.
            debug!(
                "Node {} not in registry cache, will use CLI fallback for control",
                node_id
            );
        }

        PwCommand::UnbindNode { node_id } => {
            debug!("Unbinding from node {}", node_id);
            state.borrow_mut().bound_nodes.remove(&node_id);
        }

        PwCommand::SetVolume { node_id, volume } => {
            trace!("PW cmd: SetVolume node={} volume={:.3}", node_id, volume);

            // Try native API first if node is bound
            let result = state.borrow().set_node_volume(node_id, volume);
            match result {
                Ok(()) => {
                    trace!("Native volume control succeeded for node {}", node_id);
                }
                Err(e) => {
                    // Node not bound or native failed, use throttled CLI fallback
                    if state.borrow_mut().should_run_cli(node_id) {
                        debug!("Native volume failed ({}), using CLI fallback", e);
                        let in_flight = Arc::clone(&state.borrow().cli_in_flight);
                        spawn_cli_work_for_node(&state.borrow().event_tx, &in_flight, node_id, move |_event_tx| {
                            if let Err(e2) = crate::audio::volume::set_volume(node_id, volume) {
                                error!("CLI volume control also failed: {}", e2);
                            }
                        });
                    }
                }
            }
        }

        PwCommand::SetMute { node_id, muted } => {
            trace!("PW cmd: SetMute node={} muted={}", node_id, muted);

            // Try native API first if node is bound
            let result = state.borrow().set_node_mute(node_id, muted);
            match result {
                Ok(()) => {
                    trace!("Native mute control succeeded for node {}", node_id);
                }
                Err(e) => {
                    // Node not bound or native failed, use throttled CLI fallback
                    if state.borrow_mut().should_run_cli(node_id) {
                        debug!("Native mute failed ({}), using CLI fallback", e);
                        let in_flight = Arc::clone(&state.borrow().cli_in_flight);
                        spawn_cli_work_for_node(&state.borrow().event_tx, &in_flight, node_id, move |_event_tx| {
                            if let Err(e2) = crate::audio::volume::set_mute(node_id, muted) {
                                error!("CLI mute control also failed: {}", e2);
                            }
                        });
                    }
                }
            }
        }

        PwCommand::SetVolumeMute {
            node_id,
            volume,
            muted,
        } => {
            trace!(
                "Setting volume+mute on node {}: vol={:.3}, mute={}",
                node_id, volume, muted
            );

            // Try native API first if node is bound
            let result = state.borrow().set_node_volume_mute(node_id, volume, muted);
            match result {
                Ok(()) => {
                    trace!("Native volume+mute control succeeded for node {}", node_id);
                }
                Err(e) => {
                    // Node not bound or native failed, use throttled CLI fallback
                    if state.borrow_mut().should_run_cli(node_id) {
                        debug!("Native volume+mute failed ({}), using CLI fallback", e);
                        let in_flight = Arc::clone(&state.borrow().cli_in_flight);
                        spawn_cli_work_for_node(&state.borrow().event_tx, &in_flight, node_id, move |_event_tx| {
                            if let Err(e2) = crate::audio::volume::set_volume(node_id, volume) {
                                warn!("CLI volume control failed: {}", e2);
                            }
                            if let Err(e3) = crate::audio::volume::set_mute(node_id, muted) {
                                warn!("CLI mute control failed: {}", e3);
                            }
                        });
                    }
                }
            }
        }

        PwCommand::SetEqParams {
            node_id,
            band,
            freq,
            q,
            gain,
        } => {
            debug!(
                "Setting EQ on node {} band {}: freq={:.1}, Q={:.2}, gain={:.1}",
                node_id, band, freq, q, gain
            );
            // TODO: Implement EQ control
            warn!("EQ control not yet implemented");
        }

        PwCommand::SetDefaultSink { node_id } => {
            info!("Setting default sink to node {}", node_id);
            spawn_cli_work(&state.borrow().event_tx, move |event_tx| {
                let output = std::process::Command::new("wpctl")
                    .args(["set-default", &node_id.to_string()])
                    .output();

                match output {
                    Ok(result) => {
                        if result.status.success() {
                            info!("Successfully set default sink to node {}", node_id);
                        } else {
                            let stderr = String::from_utf8_lossy(&result.stderr);
                            warn!("wpctl set-default failed: {}", stderr);
                            let _ = event_tx.send(PwEvent::Error(format!(
                                "Failed to set default sink: {}",
                                stderr
                            )));
                        }
                    }
                    Err(e) => {
                        warn!("Failed to run wpctl: {}", e);
                        let _ = event_tx.send(PwEvent::Error(format!(
                            "Failed to set default sink: {}",
                            e
                        )));
                    }
                }
            });
        }

        PwCommand::CreatePluginFilter {
            channel_id,
            channel_name,
            plugin_chain,
            meter_levels,
            loopback_output_node_id,
        } => {
            info!(
                "Creating plugin filter for channel '{}' with {} plugins",
                channel_name,
                plugin_chain.len()
            );

            let event_tx = state.borrow().event_tx.clone();
            let sample_rate = state.borrow().sample_rate;
            let block_size = state.borrow().block_size;

            // Check if we have shared plugin instances
            let shared_instances = match &state.borrow().shared_plugin_instances {
                Some(instances) => instances.clone(),
                None => {
                    warn!("SharedPluginInstances not set, cannot create plugin filter");
                    let _ = event_tx.send(PwEvent::Error(
                        "Plugin filter creation failed: SharedPluginInstances not initialized".to_string()
                    ));
                    return;
                }
            };

            // Create ring buffer for parameter updates
            let (param_writer, param_reader) = RingBuffer::<PluginParamUpdate>::new(256).split();

            // Create the plugin filter streams with meter levels
            match PluginFilterStreams::new(
                core,
                channel_id,
                &channel_name,
                shared_instances,
                plugin_chain,
                param_reader,
                sample_rate,
                block_size,
                meter_levels,
            ) {
                Ok(streams) => {
                    let capture_node_id = streams.capture_node_id();
                    let playback_node_id = streams.playback_node_id();

                    // Use provided loopback output node ID if available, otherwise search by name
                    let loopback_output_id = loopback_output_node_id.or_else(|| {
                        // Sanitize channel name to match virtual sink naming
                        let safe_name: String = channel_name
                            .chars()
                            .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                            .collect();

                        // Find the loopback output node to capture from
                        // The virtual sink creates: sootmix.{name} (sink) + sootmix.{name}.output (output)
                        let output_id = state.borrow().get_loopback_output_node(&safe_name);
                        if output_id.is_none() {
                            debug!(
                                "Loopback output node 'output.sootmix.{}.output' not found yet, will auto-connect",
                                safe_name
                            );
                        }
                        output_id
                    });

                    // Connect the streams
                    // - Capture from loopback output (or auto-connect if not found)
                    // - Playback to default sink (None = auto-connect to default)
                    if let Err(e) = streams.connect(loopback_output_id, None) {
                        warn!("Failed to connect plugin filter streams: {:?}", e);
                        let _ = event_tx.send(PwEvent::Error(
                            format!("Failed to connect plugin filter: {:?}", e)
                        ));
                        return;
                    }

                    // Store the filter info
                    let filter_info = PluginFilterInfo {
                        streams,
                        param_writer,
                    };
                    state.borrow_mut().plugin_filters.insert(channel_id, filter_info);

                    info!(
                        "Plugin filter created for channel '{}': capture={}, playback={}, loopback_output={:?}",
                        channel_name, capture_node_id, playback_node_id, loopback_output_id
                    );

                    let _ = event_tx.send(PwEvent::PluginFilterCreated {
                        channel_id,
                        sink_node_id: capture_node_id,
                        output_node_id: playback_node_id,
                    });
                }
                Err(e) => {
                    warn!("Failed to create plugin filter streams: {:?}", e);
                    let _ = event_tx.send(PwEvent::Error(
                        format!("Failed to create plugin filter: {:?}", e)
                    ));
                }
            }
        }

        PwCommand::DestroyPluginFilter { channel_id } => {
            info!("Destroying plugin filter for channel {}", channel_id);

            let event_tx = state.borrow().event_tx.clone();

            // Remove and destroy the filter
            if let Some(mut filter_info) = state.borrow_mut().plugin_filters.remove(&channel_id) {
                // Disconnect the streams
                if let Err(e) = filter_info.streams.disconnect() {
                    warn!("Error disconnecting plugin filter streams: {:?}", e);
                }
                // The streams will be dropped here, cleaning up PipeWire resources
                info!("Plugin filter destroyed for channel {}", channel_id);
            } else {
                debug!("No plugin filter found for channel {} to destroy", channel_id);
            }

            let _ = event_tx.send(PwEvent::PluginFilterDestroyed { channel_id });
        }

        PwCommand::UpdatePluginChain {
            channel_id,
            plugin_chain,
        } => {
            debug!(
                "Updating plugin chain for channel {}: {} plugins",
                channel_id,
                plugin_chain.len()
            );

            // Update the plugin chain in the existing filter
            if let Some(filter_info) = state.borrow().plugin_filters.get(&channel_id) {
                filter_info.streams.update_plugin_chain(plugin_chain);
            } else {
                warn!("No plugin filter found for channel {} to update", channel_id);
            }
        }

        PwCommand::SetSharedPluginInstances(instances) => {
            info!("Setting shared plugin instances for RT audio processing");
            state.borrow_mut().shared_plugin_instances = Some(instances);
        }

        PwCommand::SendPluginParamUpdate {
            channel_id,
            instance_id,
            param_index,
            value,
        } => {
            trace!(
                "Sending param update: channel={}, plugin={}, param={}, value={}",
                channel_id, instance_id, param_index, value
            );

            // Send the parameter update to the filter's ring buffer
            if let Some(filter_info) = state.borrow_mut().plugin_filters.get_mut(&channel_id) {
                let update = crate::realtime::PluginParamUpdate::new(instance_id, param_index, value);
                filter_info.param_writer.push(update);
            }
        }

        PwCommand::RouteChannelToDevice {
            loopback_output_node,
            target_device_id,
        } => {
            info!(
                "Routing loopback output {} to device {:?}",
                loopback_output_node, target_device_id
            );

            // Collect all state data on the PW thread (non-blocking reads)
            let links_to_destroy: Vec<u32> = {
                let s = state.borrow();
                s.links
                    .values()
                    .filter(|l| l.output_node == loopback_output_node)
                    .map(|l| l.id)
                    .collect()
            };

            let target_node_id = target_device_id.or_else(|| {
                let s = state.borrow();
                s.nodes
                    .values()
                    .find(|n| n.media_class == MediaClass::AudioSink && !n.name.starts_with("sootmix."))
                    .map(|n| n.id)
            });

            let (port_pairs, out_port_count, in_port_count): (Vec<(u32, u32)>, usize, usize) =
                if let Some(target_id) = target_node_id {
                    let s = state.borrow();
                    let out_ports: Vec<_> = s.ports
                        .values()
                        .filter(|p| p.node_id == loopback_output_node && p.direction == PortDirection::Output)
                        .collect();
                    let in_ports: Vec<_> = s.ports
                        .values()
                        .filter(|p| p.node_id == target_id && p.direction == PortDirection::Input)
                        .collect();

                    let out_count = out_ports.len();
                    let in_count = in_ports.len();

                    let mut pairs = Vec::new();
                    for out_port in &out_ports {
                        for in_port in &in_ports {
                            let out_name = out_port.name.to_lowercase();
                            let in_name = in_port.name.to_lowercase();
                            let is_match =
                                (out_name.contains("fl") && in_name.contains("fl"))
                                || (out_name.contains("fr") && in_name.contains("fr"))
                                || (out_name.contains("_0") && in_name.contains("_0"))
                                || (out_name.contains("_1") && in_name.contains("_1"));
                            if is_match {
                                pairs.push((out_port.id, in_port.id));
                            }
                        }
                    }

                    if pairs.is_empty() && !out_ports.is_empty() && !in_ports.is_empty() {
                        debug!("No channel name matches, pairing ports in order");
                        for (out_port, in_port) in out_ports.iter().zip(in_ports.iter()) {
                            pairs.push((out_port.id, in_port.id));
                        }
                    }

                    (pairs, out_count, in_count)
                } else {
                    (Vec::new(), 0, 0)
                };

            if port_pairs.is_empty() && target_node_id.is_some() {
                let target_id = target_node_id.unwrap();
                warn!(
                    "No port pairs found for routing: loopback node {} has {} output ports, target {} has {} input ports",
                    loopback_output_node, out_port_count, target_id, in_port_count
                );
                if out_port_count == 0 {
                    warn!("Loopback output ports not yet discovered - this is a timing issue");
                }
            }

            // Dispatch all CLI link operations to a background thread
            spawn_cli_work(&state.borrow().event_tx, move |event_tx| {
                for link_id in links_to_destroy {
                    debug!("Destroying existing link {} from loopback output", link_id);
                    if let Err(e) = crate::audio::routing::destroy_link(link_id) {
                        warn!("Failed to destroy link {}: {}", link_id, e);
                    }
                }

                if target_node_id.is_none() {
                    warn!("No target device found for routing");
                    let _ = event_tx.send(PwEvent::Error("No target device found".to_string()));
                    return;
                }

                for (output_port, input_port) in port_pairs {
                    info!("Creating link from loopback: {} -> {}", output_port, input_port);
                    if let Err(e) = crate::audio::routing::create_link(output_port, input_port) {
                        warn!("Failed to create link {} -> {}: {}", output_port, input_port, e);
                        let _ = event_tx.send(PwEvent::Error(format!(
                            "Failed to route to device: {}", e
                        )));
                    }
                }
            });
        }

        PwCommand::CreateRecordingSource { name } => {
            info!("Creating recording source: {}", name);
            spawn_cli_work(&state.borrow().event_tx, move |event_tx| {
                match crate::audio::virtual_sink::create_virtual_source(&name, &format!("SootMix Recording: {}", name)) {
                    Ok(result) => {
                        let node_id = result.source_node_id;
                        info!("Created recording source '{}' with node_id={}", name, node_id);
                        let _ = event_tx.send(PwEvent::RecordingSourceCreated {
                            name,
                            node_id,
                        });
                    }
                    Err(e) => {
                        let _ = event_tx.send(PwEvent::Error(format!(
                            "Failed to create recording source: {}", e
                        )));
                    }
                }
            });
        }

        PwCommand::DestroyRecordingSource { node_id } => {
            info!("Destroying recording source: {}", node_id);
            spawn_cli_work(&state.borrow().event_tx, move |event_tx| {
                if let Err(e) = crate::audio::virtual_sink::destroy_virtual_source(node_id) {
                    warn!("Failed to destroy recording source {}: {}", node_id, e);
                }
                let _ = event_tx.send(PwEvent::RecordingSourceDestroyed { node_id });
            });
        }

        PwCommand::CreateMeterStream {
            channel_id,
            channel_name,
            sink_node_id,
            meter_levels,
        } => {
            // Skip if meter stream already exists for this channel
            if state.borrow().meter_streams.has_stream(channel_id) {
                debug!("Meter stream already exists for channel '{}', skipping", channel_name);
                return;
            }

            // Get the target node name from state
            let target_node_name = state.borrow().nodes.get(&sink_node_id)
                .map(|n| n.name.clone())
                .unwrap_or_else(|| format!("node_{}", sink_node_id));

            info!(
                "Creating meter stream for channel '{}' (target node {} = '{}')",
                channel_name, sink_node_id, target_node_name
            );

            // Create the meter stream with target.object property
            let result = state.borrow_mut().meter_streams.create_stream(
                core,
                channel_id,
                &channel_name,
                &target_node_name,
                meter_levels,
            );

            match result {
                Ok(()) => {
                    // Connect the meter stream
                    let connect_result = state.borrow().meter_streams.connect_stream(channel_id, sink_node_id);
                    if let Err(e) = connect_result {
                        warn!("Failed to connect meter stream for channel '{}': {:?}", channel_name, e);
                    } else {
                        info!("Meter stream created and connected for channel '{}'", channel_name);

                        // WirePlumber doesn't auto-link meter streams to monitor ports.
                        // We need to create the links manually when ports are discovered.
                        // The meter stream node ID isn't available yet (it's assigned after connecting),
                        // so we store the meter name pattern and look it up when the node is discovered.
                        let meter_name = format!("sootmix.meter.{}", channel_name);
                        state.borrow_mut().pending_meter_links.push(PendingMeterLink {
                            sink_node_id,
                            sink_name: target_node_name.clone(),
                            meter_node_id: 0, // Will be filled when node is discovered
                            meter_name,
                        });
                        debug!("Queued pending meter link for discovery: sink {} -> meter 'sootmix.meter.{}'", sink_node_id, channel_name);
                    }
                }
                Err(e) => {
                    warn!("Failed to create meter stream for channel '{}': {:?}", channel_name, e);
                }
            }
        }

        PwCommand::CreateInputMeterStream {
            channel_id,
            channel_name,
            source_node_id,
            meter_levels,
        } => {
            // Skip if meter stream already exists for this channel
            if state.borrow().meter_streams.has_stream(channel_id) {
                debug!("Meter stream already exists for input channel '{}', skipping", channel_name);
                return;
            }

            // Get the target node name from state
            let target_node_name = state.borrow().nodes.get(&source_node_id)
                .map(|n| n.name.clone())
                .unwrap_or_else(|| format!("node_{}", source_node_id));

            info!(
                "Creating input meter stream for channel '{}' (target source {} = '{}')",
                channel_name, source_node_id, target_node_name
            );

            // Create the meter stream for source (without stream.monitor)
            let result = state.borrow_mut().meter_streams.create_source_stream(
                core,
                channel_id,
                &channel_name,
                &target_node_name,
                meter_levels,
            );

            match result {
                Ok(()) => {
                    // Connect the meter stream to the source
                    let connect_result = state.borrow().meter_streams.connect_stream(channel_id, source_node_id);
                    if let Err(e) = connect_result {
                        warn!("Failed to connect input meter stream for channel '{}': {:?}", channel_name, e);
                    } else {
                        info!("Input meter stream created and connected for channel '{}'", channel_name);
                    }
                }
                Err(e) => {
                    warn!("Failed to create input meter stream for channel '{}': {:?}", channel_name, e);
                }
            }
        }

        PwCommand::DestroyMeterStream { channel_id } => {
            info!("Destroying meter stream for channel {}", channel_id);
            state.borrow_mut().meter_streams.destroy_stream(channel_id);
        }
    }
}

/// Bind a node proxy for native control from a GlobalObject.
/// This is called from the registry listener when we detect nodes we want to control.
fn bind_node_from_global(
    global: &pipewire::registry::GlobalObject<&pipewire::spa::utils::dict::DictRef>,
    state: &Rc<RefCell<PwThreadState>>,
    registry: &pipewire::registry::RegistryRc,
) -> Result<(), String> {
    let node_id = global.id;

    // Check if already bound
    if state.borrow().bound_nodes.contains_key(&node_id) {
        return Ok(());
    }

    debug!("Binding node proxy for node {}", node_id);

    // Bind the node
    let node: Node = registry
        .bind(global)
        .map_err(|e| format!("Failed to bind node {}: {:?}", node_id, e))?;

    // Set up listener for param changes
    let listener = node
        .add_listener_local()
        .param(move |_seq, _id, _index, _next, _pod| {
            // Could parse the pod here to get volume/mute feedback
            trace!("Received param update for bound node");
        })
        .register();

    // Store the bound node
    state.borrow_mut().bound_nodes.insert(
        node_id,
        BoundNode {
            proxy: node,
            _listener: listener,
        },
    );

    info!("Successfully bound node {} for native control", node_id);
    Ok(())
}

/// Set up the registry listener to watch for nodes, ports, and links.
/// Also binds audio sink nodes (sootmix virtual sinks and hardware outputs) for native control.
fn setup_registry_listener(
    registry: &pipewire::registry::RegistryRc,
    state: Rc<RefCell<PwThreadState>>,
    event_tx: Rc<mpsc::Sender<PwEvent>>,
) -> pipewire::registry::Listener {
    let state_add = state.clone();
    let state_remove = state;
    let event_tx_add = event_tx.clone();
    let event_tx_remove = event_tx;

    // Clone registry for use in the closure
    let registry_clone = registry.clone();

    registry
        .add_listener_local()
        .global(move |global| {
            use pipewire::types::ObjectType;

            let props = match global.props {
                Some(p) => p,
                None => return,
            };

            match global.type_ {
                ObjectType::Node => {
                    let mut node = PwNode::new(global.id);

                    // Extract properties
                    if let Some(name) = props.get("node.name") {
                        node.name = name.to_string();
                    }
                    if let Some(desc) = props.get("node.description") {
                        node.description = desc.to_string();
                    }
                    if let Some(class) = props.get("media.class") {
                        node.media_class = MediaClass::from_str(class);
                    }
                    if let Some(app) = props.get("application.name") {
                        node.app_name = Some(app.to_string());
                    }
                    if let Some(binary) = props.get("application.process.binary") {
                        node.binary_name = Some(binary.to_string());
                    }

                    // Store all properties
                    for (k, v) in props.iter() {
                        node.properties.insert(k.to_string(), v.to_string());
                    }

                    debug!(
                        "Node added: id={}, name={}, class={:?}",
                        node.id, node.name, node.media_class
                    );

                    // Extra debug for potential input devices
                    if node.name.contains("alsa_input") || node.name.contains("input") {
                        debug!("  Potential input device detected - props available:");
                        debug!("    media.class from props: {:?}", props.get("media.class"));
                        debug!("    node.description: {:?}", props.get("node.description"));
                        debug!("    factory.name: {:?}", props.get("factory.name"));
                    }

                    // Auto-bind nodes for native volume/mute control:
                    // - sootmix virtual sinks (our channel sinks)
                    // - hardware audio sinks (for master volume control)
                    let should_bind = node.name.starts_with("sootmix.")
                        || node.media_class == MediaClass::AudioSink;

                    if should_bind {
                        debug!("Binding node for native control: {} ({:?})", node.name, node.media_class);
                        if let Err(e) = bind_node_from_global(global, &state_add, &registry_clone) {
                            warn!("Failed to bind node {}: {}", node.id, e);
                        }
                    }

                    // Check if this is a meter stream node and update pending meter links
                    if node.name.starts_with("sootmix.meter.") {
                        let meter_name = node.name.clone();
                        let meter_node_id = node.id;
                        let mut state_mut = state_add.borrow_mut();
                        for pending in state_mut.pending_meter_links.iter_mut() {
                            if pending.meter_name == meter_name && pending.meter_node_id == 0 {
                                pending.meter_node_id = meter_node_id;
                                debug!("Updated pending meter link with node ID: sink {} -> meter {} (node {})",
                                    pending.sink_node_id, pending.meter_name, meter_node_id);
                                break;
                            }
                        }
                    }

                    state_add.borrow_mut().nodes.insert(global.id, node.clone());
                    let _ = event_tx_add.send(PwEvent::NodeAdded(node));
                }
                ObjectType::Port => {
                    let mut port = PwPort::new(global.id, 0);

                    if let Some(name) = props.get("port.name") {
                        port.name = name.to_string();
                        port.channel = AudioChannel::from_str(name);
                    }
                    if let Some(node_id) = props.get("node.id") {
                        port.node_id = node_id.parse().unwrap_or(0);
                    }
                    if let Some(dir) = props.get("port.direction") {
                        port.direction = PortDirection::from_str(dir);
                    }

                    debug!(
                        "Port added: id={}, node={}, name={}, dir={:?}",
                        port.id, port.node_id, port.name, port.direction
                    );

                    state_add.borrow_mut().ports.insert(global.id, port.clone());
                    let _ = event_tx_add.send(PwEvent::PortAdded(port.clone()));

                    // Check if this port belongs to a meter stream that needs links
                    process_pending_meter_links(&state_add, &port);
                }
                ObjectType::Link => {
                    let mut link = PwLink::new(global.id);

                    if let Some(out_node) = props.get("link.output.node") {
                        link.output_node = out_node.parse().unwrap_or(0);
                    }
                    if let Some(out_port) = props.get("link.output.port") {
                        link.output_port = out_port.parse().unwrap_or(0);
                    }
                    if let Some(in_node) = props.get("link.input.node") {
                        link.input_node = in_node.parse().unwrap_or(0);
                    }
                    if let Some(in_port) = props.get("link.input.port") {
                        link.input_port = in_port.parse().unwrap_or(0);
                    }

                    debug!(
                        "Link added: id={}, {}:{} -> {}:{}",
                        link.id,
                        link.output_node,
                        link.output_port,
                        link.input_node,
                        link.input_port
                    );

                    state_add.borrow_mut().links.insert(global.id, link.clone());
                    let _ = event_tx_add.send(PwEvent::LinkAdded(link));
                }
                _ => {}
            }
        })
        .global_remove(move |id| {
            let mut state = state_remove.borrow_mut();

            // Remove bound node proxy if it exists
            state.bound_nodes.remove(&id);

            if state.nodes.remove(&id).is_some() {
                debug!("Node removed: {}", id);
                let _ = event_tx_remove.send(PwEvent::NodeRemoved(id));
            } else if state.ports.remove(&id).is_some() {
                debug!("Port removed: {}", id);
                let _ = event_tx_remove.send(PwEvent::PortRemoved(id));
            } else if state.links.remove(&id).is_some() {
                debug!("Link removed: {}", id);
                let _ = event_tx_remove.send(PwEvent::LinkRemoved(id));
            }
        })
        .register()
}

/// Process pending meter links when a new port is discovered.
///
/// This function checks if we now have all the ports needed to link a meter stream
/// to a sink's monitor ports, and creates the links if so.
fn process_pending_meter_links(state: &Rc<RefCell<PwThreadState>>, added_port: &PwPort) {
    // Quick check: is this port from a meter stream or a sink we care about?
    let pending_links: Vec<PendingMeterLink> = {
        let state_ref = state.borrow();
        state_ref.pending_meter_links.clone()
    };

    if pending_links.is_empty() {
        return;
    }

    // Check if this port belongs to any of our pending meter links
    let port_node_id = added_port.node_id;
    let relevant = pending_links.iter().any(|pl| {
        pl.meter_node_id == port_node_id || pl.sink_node_id == port_node_id
    });

    if !relevant {
        return;
    }

    debug!("Checking pending meter links after port {} added to node {}", added_port.id, port_node_id);

    // Collect port info for all pending links
    let mut completed_indices = Vec::new();

    for (index, pending) in pending_links.iter().enumerate() {
        // Skip entries where meter node hasn't been discovered yet
        if pending.meter_node_id == 0 {
            continue;
        }

        // Find sink monitor ports (FL and FR) for stereo metering.
        // PipeWire allows multiple output ports linked to one input port — it
        // sums them, but with resample.peaks=true we get per-update peak values.
        // Linking both channels lets each peak through independently when the
        // stream negotiates stereo (2-channel interleaved) format.
        let sink_monitor_ports = {
            let state_ref = state.borrow();
            let mut ports: Vec<(u32, String)> = state_ref.ports.values()
                .filter(|p| {
                    p.node_id == pending.sink_node_id
                        && p.direction == PortDirection::Output
                        && p.name.starts_with("monitor_")
                })
                .map(|p| (p.id, p.name.clone()))
                .collect();
            // Sort so FL/0 comes before FR/1
            ports.sort_by(|a, b| a.1.cmp(&b.1));
            ports
        };

        // Find all meter stream input ports.
        // pw_stream may create one port (mono/interleaved) or two (stereo negotiated).
        let meter_input_ports = {
            let state_ref = state.borrow();
            let mut ports: Vec<(u32, String)> = state_ref.ports.values()
                .filter(|p| p.node_id == pending.meter_node_id && p.direction == PortDirection::Input)
                .map(|p| (p.id, p.name.clone()))
                .collect();
            ports.sort_by(|a, b| a.1.cmp(&b.1));
            ports
        };

        if sink_monitor_ports.is_empty() || meter_input_ports.is_empty() {
            continue;
        }

        // Link strategy:
        // - If meter has 2+ input ports: link FL->input0, FR->input1 (true stereo)
        // - If meter has 1 input port: link both FL and FR to it (PipeWire mixes,
        //   giving us the combined peak — both meters show the same value)
        let mut linked = false;
        if meter_input_ports.len() >= 2 && sink_monitor_ports.len() >= 2 {
            // True stereo: pair monitor ports with meter input ports
            for (monitor, meter) in sink_monitor_ports.iter().zip(meter_input_ports.iter()) {
                info!(
                    "Creating stereo meter link: sink {} ({} port {}) -> meter {} ({} port {})",
                    pending.sink_name, monitor.1, monitor.0,
                    pending.meter_name, meter.1, meter.0,
                );
                if let Err(e) = crate::audio::routing::create_link(monitor.0, meter.0) {
                    warn!("Failed to create meter link: {:?}", e);
                } else {
                    linked = true;
                }
            }
        } else {
            // Single input port: link all monitor ports to it
            let input_port = meter_input_ports[0].0;
            for (monitor_id, monitor_name) in &sink_monitor_ports {
                info!(
                    "Creating meter link: sink {} ({} port {}) -> meter {} (input port {})",
                    pending.sink_name, monitor_name, monitor_id,
                    pending.meter_name, input_port,
                );
                if let Err(e) = crate::audio::routing::create_link(*monitor_id, input_port) {
                    warn!("Failed to create meter link: {:?}", e);
                } else {
                    linked = true;
                }
            }
        }

        if linked {
            completed_indices.push(index);
        }
    }

    // Remove completed pending links
    if !completed_indices.is_empty() {
        let mut state_mut = state.borrow_mut();
        // Remove in reverse order to preserve indices
        for index in completed_indices.into_iter().rev() {
            state_mut.pending_meter_links.remove(index);
        }
    }
}
