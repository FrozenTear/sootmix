// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! PipeWire thread management and event handling.
//!
//! This module implements native PipeWire control using the pipewire-rs API.
//! All PipeWire operations run on a dedicated thread since PipeWire objects
//! are not Send/Sync.

use crate::audio::control::{build_channel_volumes_pod, build_mute_pod, build_volume_mute_pod};
use crate::audio::types::{AudioChannel, MediaClass, PortDirection, PwLink, PwNode, PwPort};
use pipewire::link::Link;
use pipewire::node::{Node, NodeListener};
use pipewire::properties::properties;
use pipewire::spa::param::ParamType;
use pipewire::spa::pod::Pod;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use thiserror::Error;
use tracing::{debug, error, info, trace, warn};
use uuid::Uuid;

/// Commands sent from the UI thread to the PipeWire thread.
#[derive(Debug, Clone)]
pub enum PwCommand {
    /// Create a virtual sink for a channel.
    CreateVirtualSink { channel_id: Uuid, name: String },
    /// Destroy a virtual sink.
    DestroyVirtualSink { node_id: u32 },
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
    VirtualSinkCreated { channel_id: Uuid, node_id: u32 },
    /// Virtual sink destroyed.
    VirtualSinkDestroyed { node_id: u32 },
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
    /// Event sender for notifying UI.
    event_tx: Rc<mpsc::Sender<PwEvent>>,
    /// Last CLI command time per node (for throttling fallback).
    cli_last_cmd: HashMap<u32, Instant>,
}

impl PwThreadState {
    fn new(event_tx: Rc<mpsc::Sender<PwEvent>>) -> Self {
        Self {
            nodes: HashMap::new(),
            ports: HashMap::new(),
            links: HashMap::new(),
            virtual_sinks: HashMap::new(),
            bound_nodes: HashMap::new(),
            created_links: HashMap::new(),
            event_tx,
            cli_last_cmd: HashMap::new(),
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
            debug!("Creating virtual sink: {} for channel {}", name, channel_id);
            let event_tx = state.borrow().event_tx.clone();
            match crate::audio::virtual_sink::create_virtual_sink(&name, &name) {
                Ok(node_id) => {
                    state.borrow_mut().virtual_sinks.insert(channel_id, node_id);
                    // Note: Node will be auto-bound when it appears in registry listener
                    let _ = event_tx.send(PwEvent::VirtualSinkCreated {
                        channel_id,
                        node_id,
                    });
                }
                Err(e) => {
                    let _ = event_tx.send(PwEvent::Error(format!(
                        "Failed to create virtual sink: {}",
                        e
                    )));
                }
            }
        }

        PwCommand::DestroyVirtualSink { node_id } => {
            debug!("Destroying virtual sink: {}", node_id);
            let event_tx = state.borrow().event_tx.clone();

            // Remove the bound node proxy
            state.borrow_mut().bound_nodes.remove(&node_id);

            if let Err(e) = crate::audio::virtual_sink::destroy_virtual_sink(node_id) {
                warn!("Failed to destroy virtual sink {}: {}", node_id, e);
            }
            state
                .borrow_mut()
                .virtual_sinks
                .retain(|_, &mut id| id != node_id);
            let _ = event_tx.send(PwEvent::VirtualSinkDestroyed { node_id });
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
                    if let Err(e) = crate::audio::routing::create_link(output_port, input_port) {
                        let event_tx = state.borrow().event_tx.clone();
                        let _ = event_tx.send(PwEvent::Error(format!("Failed to create link: {}", e)));
                    }
                    return;
                }
            };

            let in_node = match in_node {
                Some(n) => n,
                None => {
                    warn!("Input port {} not found, using CLI fallback", input_port);
                    if let Err(e) = crate::audio::routing::create_link(output_port, input_port) {
                        let event_tx = state.borrow().event_tx.clone();
                        let _ = event_tx.send(PwEvent::Error(format!("Failed to create link: {}", e)));
                    }
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
                    if let Err(e2) = crate::audio::routing::create_link(output_port, input_port) {
                        let event_tx = state.borrow().event_tx.clone();
                        let _ = event_tx.send(PwEvent::Error(format!("Failed to create link: {}", e2)));
                    }
                }
            }
        }

        PwCommand::DestroyLink { link_id } => {
            debug!("Destroying link: {}", link_id);

            // Try to find and destroy via our created links
            // Note: link_id might be from registry, not our port pair
            // For now, we'll use CLI fallback for link_id-based destruction
            // TODO: Track link_id -> port pair mapping

            if let Err(e) = crate::audio::routing::destroy_link(link_id) {
                let event_tx = state.borrow().event_tx.clone();
                let _ = event_tx.send(PwEvent::Error(format!("Failed to destroy link: {}", e)));
            }
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
                        if let Err(e2) = crate::audio::volume::set_volume(node_id, volume) {
                            error!("CLI volume control also failed: {}", e2);
                        }
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
                        if let Err(e2) = crate::audio::volume::set_mute(node_id, muted) {
                            error!("CLI mute control also failed: {}", e2);
                        }
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
                        if let Err(e2) = crate::audio::volume::set_volume(node_id, volume) {
                            warn!("CLI volume control failed: {}", e2);
                        }
                        if let Err(e3) = crate::audio::volume::set_mute(node_id, muted) {
                            warn!("CLI mute control failed: {}", e3);
                        }
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
            // Use wpctl to set default sink (simpler than native API for metadata)
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
                        let event_tx = state.borrow().event_tx.clone();
                        let _ = event_tx.send(PwEvent::Error(format!(
                            "Failed to set default sink: {}",
                            stderr
                        )));
                    }
                }
                Err(e) => {
                    warn!("Failed to run wpctl: {}", e);
                    let event_tx = state.borrow().event_tx.clone();
                    let _ = event_tx.send(PwEvent::Error(format!(
                        "Failed to set default sink: {}",
                        e
                    )));
                }
            }
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
                    let _ = event_tx_add.send(PwEvent::PortAdded(port));
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
