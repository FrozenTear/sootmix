// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! PipeWire thread management and event handling.
//!
//! This module implements native PipeWire control using the pipewire-rs API.
//! All PipeWire operations run on a dedicated thread since PipeWire objects
//! are not Send/Sync.

use crate::audio::control::{self, spa_const};
use crate::audio::types::{AudioChannel, MediaClass, PortDirection, PwLink, PwNode, PwPort};
use libspa::param::ParamType;
use libspa::pod::Pod;
use pipewire::node::Node;
use pipewire::proxy::{Listener, ProxyT};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
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

/// A bound node with its proxy and listener.
struct BoundNode {
    /// The node proxy for calling set_param, etc.
    proxy: Node,
    /// Listener for param events (must be kept alive).
    _listener: Listener,
    /// Cached node info.
    info: PwNode,
}

/// State tracked within the PipeWire thread.
struct PwThreadState {
    /// Basic node info indexed by node ID.
    nodes: HashMap<u32, PwNode>,
    /// Ports indexed by port ID.
    ports: HashMap<u32, PwPort>,
    /// Links indexed by link ID.
    links: HashMap<u32, PwLink>,
    /// Bound node proxies for control operations.
    bound_nodes: HashMap<u32, BoundNode>,
    /// Map of channel UUID to virtual sink node ID.
    virtual_sinks: HashMap<Uuid, u32>,
    /// Event sender for notifying UI.
    event_tx: Rc<mpsc::Sender<PwEvent>>,
}

impl PwThreadState {
    fn new(event_tx: Rc<mpsc::Sender<PwEvent>>) -> Self {
        Self {
            nodes: HashMap::new(),
            ports: HashMap::new(),
            links: HashMap::new(),
            bound_nodes: HashMap::new(),
            virtual_sinks: HashMap::new(),
            event_tx,
        }
    }

    /// Set volume on a bound node using native API.
    fn set_volume(&self, node_id: u32, volume: f32) -> Result<(), control::ControlError> {
        let bound = self
            .bound_nodes
            .get(&node_id)
            .ok_or(control::ControlError::NodeNotBound(node_id))?;

        let pod_data = control::build_volume_pod(volume)?;

        // Safety: The pod data is valid for the duration of this call
        unsafe {
            let pod = Pod::from_bytes(&pod_data)
                .ok_or_else(|| control::ControlError::SerializationFailed("Invalid pod".into()))?;

            bound.proxy.set_param(ParamType::Props, 0, pod);
        }

        debug!("Set volume on node {} to {:.3}", node_id, volume);
        Ok(())
    }

    /// Set mute state on a bound node using native API.
    fn set_mute(&self, node_id: u32, muted: bool) -> Result<(), control::ControlError> {
        let bound = self
            .bound_nodes
            .get(&node_id)
            .ok_or(control::ControlError::NodeNotBound(node_id))?;

        let pod_data = control::build_mute_pod(muted)?;

        unsafe {
            let pod = Pod::from_bytes(&pod_data)
                .ok_or_else(|| control::ControlError::SerializationFailed("Invalid pod".into()))?;

            bound.proxy.set_param(ParamType::Props, 0, pod);
        }

        debug!("Set mute on node {} to {}", node_id, muted);
        Ok(())
    }

    /// Set volume and mute atomically on a bound node.
    fn set_volume_mute(
        &self,
        node_id: u32,
        volume: f32,
        muted: bool,
    ) -> Result<(), control::ControlError> {
        let bound = self
            .bound_nodes
            .get(&node_id)
            .ok_or(control::ControlError::NodeNotBound(node_id))?;

        let pod_data = control::build_volume_mute_pod(volume, muted)?;

        unsafe {
            let pod = Pod::from_bytes(&pod_data)
                .ok_or_else(|| control::ControlError::SerializationFailed("Invalid pod".into()))?;

            bound.proxy.set_param(ParamType::Props, 0, pod);
        }

        debug!(
            "Set volume+mute on node {}: vol={:.3}, mute={}",
            node_id, volume, muted
        );
        Ok(())
    }

    /// Set EQ parameters on a filter-chain node.
    fn set_eq_params(
        &self,
        node_id: u32,
        band: &str,
        freq: f32,
        q: f32,
        gain: f32,
    ) -> Result<(), control::ControlError> {
        let bound = self
            .bound_nodes
            .get(&node_id)
            .ok_or(control::ControlError::NodeNotBound(node_id))?;

        let pod_data = control::build_eq_band_pod(band, freq, q, gain)?;

        unsafe {
            let pod = Pod::from_bytes(&pod_data)
                .ok_or_else(|| control::ControlError::SerializationFailed("Invalid pod".into()))?;

            bound.proxy.set_param(ParamType::Props, 0, pod);
        }

        debug!(
            "Set EQ on node {} band {}: freq={:.1}, Q={:.2}, gain={:.1}",
            node_id, band, freq, q, gain
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
        pipewire::main_loop::MainLoop::new(None).map_err(|e| PwError::InitFailed(e.to_string()))?;

    // Create context and connect
    let context = pipewire::context::Context::new(&main_loop)
        .map_err(|e| PwError::InitFailed(e.to_string()))?;

    let core = context
        .connect(None)
        .map_err(|e| PwError::ConnectionFailed(e.to_string()))?;

    let registry = core
        .get_registry()
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
        handle_command(cmd, &state_cmd, &core_cmd, &registry_cmd, &main_loop_weak);
    });

    // Set up registry listener for discovering nodes, ports, links
    let _registry_listener =
        setup_registry_listener(&registry, state.clone(), event_tx.clone(), &core);

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
    core: &pipewire::core::Core,
    registry: &pipewire::registry::Registry,
    main_loop_weak: &pipewire::main_loop::WeakMainLoop,
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
            // For now, still use the config-based approach
            // TODO: Implement native module loading
            let event_tx = state.borrow().event_tx.clone();
            match crate::audio::virtual_sink::create_virtual_sink(&name, &name) {
                Ok(node_id) => {
                    state.borrow_mut().virtual_sinks.insert(channel_id, node_id);
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
            debug!("Creating link: {} -> {}", output_port, input_port);
            create_link_native(core, output_port, input_port, state);
        }

        PwCommand::DestroyLink { link_id } => {
            debug!("Destroying link: {}", link_id);
            destroy_link_native(core, link_id, state);
        }

        PwCommand::SetVolume { node_id, volume } => {
            trace!("Setting volume on node {}: {:.3}", node_id, volume);
            let state_ref = state.borrow();

            // Try native first, fall back to ensuring node is bound
            if let Err(e) = state_ref.set_volume(node_id, volume) {
                warn!("Failed to set volume on node {}: {}", node_id, e);
                // Try to bind the node if not already bound
                drop(state_ref);
                if !state.borrow().bound_nodes.contains_key(&node_id) {
                    bind_node(registry, node_id, state.clone());
                    // Retry
                    if let Err(e) = state.borrow().set_volume(node_id, volume) {
                        let event_tx = state.borrow().event_tx.clone();
                        let _ =
                            event_tx.send(PwEvent::Error(format!("Failed to set volume: {}", e)));
                    }
                }
            }
        }

        PwCommand::SetMute { node_id, muted } => {
            trace!("Setting mute on node {}: {}", node_id, muted);
            let state_ref = state.borrow();

            if let Err(e) = state_ref.set_mute(node_id, muted) {
                warn!("Failed to set mute on node {}: {}", node_id, e);
                drop(state_ref);
                if !state.borrow().bound_nodes.contains_key(&node_id) {
                    bind_node(registry, node_id, state.clone());
                    if let Err(e) = state.borrow().set_mute(node_id, muted) {
                        let event_tx = state.borrow().event_tx.clone();
                        let _ = event_tx.send(PwEvent::Error(format!("Failed to set mute: {}", e)));
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
                node_id,
                volume,
                muted
            );
            let state_ref = state.borrow();

            if let Err(e) = state_ref.set_volume_mute(node_id, volume, muted) {
                warn!("Failed to set volume+mute on node {}: {}", node_id, e);
                drop(state_ref);
                if !state.borrow().bound_nodes.contains_key(&node_id) {
                    bind_node(registry, node_id, state.clone());
                    if let Err(e) = state.borrow().set_volume_mute(node_id, volume, muted) {
                        let event_tx = state.borrow().event_tx.clone();
                        let _ = event_tx
                            .send(PwEvent::Error(format!("Failed to set volume+mute: {}", e)));
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
            let state_ref = state.borrow();

            if let Err(e) = state_ref.set_eq_params(node_id, &band, freq, q, gain) {
                warn!("Failed to set EQ on node {}: {}", node_id, e);
                drop(state_ref);
                if !state.borrow().bound_nodes.contains_key(&node_id) {
                    bind_node(registry, node_id, state.clone());
                    if let Err(e) = state.borrow().set_eq_params(node_id, &band, freq, q, gain) {
                        let event_tx = state.borrow().event_tx.clone();
                        let _ = event_tx.send(PwEvent::Error(format!("Failed to set EQ: {}", e)));
                    }
                }
            }
        }
    }
}

/// Create a link between ports using native API.
fn create_link_native(
    core: &pipewire::core::Core,
    output_port: u32,
    input_port: u32,
    state: &Rc<RefCell<PwThreadState>>,
) {
    let props = pipewire::properties::properties! {
        "link.output.port" => output_port.to_string(),
        "link.input.port" => input_port.to_string(),
        "object.linger" => "true",
    };

    match core.create_object::<pipewire::link::Link>("link-factory", &props) {
        Ok(_link) => {
            debug!("Created link: {} -> {}", output_port, input_port);
        }
        Err(e) => {
            let event_tx = state.borrow().event_tx.clone();
            let _ = event_tx.send(PwEvent::Error(format!("Failed to create link: {}", e)));
        }
    }
}

/// Destroy a link using native API.
fn destroy_link_native(
    core: &pipewire::core::Core,
    link_id: u32,
    state: &Rc<RefCell<PwThreadState>>,
) {
    // To destroy a link, we need to call destroy() on the proxy
    // Since we don't store link proxies, we use pw-cli for now
    // TODO: Store link proxies and destroy natively
    if let Err(e) = crate::audio::routing::destroy_link(link_id) {
        let event_tx = state.borrow().event_tx.clone();
        let _ = event_tx.send(PwEvent::Error(format!("Failed to destroy link: {}", e)));
    }
}

/// Bind to a node to get a proxy for control operations.
fn bind_node(
    registry: &pipewire::registry::Registry,
    node_id: u32,
    state: Rc<RefCell<PwThreadState>>,
) {
    debug!("Binding to node {}", node_id);

    // Get cached node info if available
    let node_info = state
        .borrow()
        .nodes
        .get(&node_id)
        .cloned()
        .unwrap_or_else(|| PwNode::new(node_id));

    // Bind to the node
    let proxy: Node = match registry.bind(node_id, pipewire::types::ObjectType::Node, 0) {
        Ok(p) => p,
        Err(e) => {
            warn!("Failed to bind to node {}: {}", node_id, e);
            return;
        }
    };

    // Subscribe to param changes
    proxy.subscribe_params(&[ParamType::Props.as_raw()]);

    // Set up listener for param events
    let state_listener = state.clone();
    let listener = proxy
        .add_listener_local()
        .param(move |_seq, param_type, _index, _next, param| {
            if param_type == Some(ParamType::Props) {
                if let Some(pod) = param {
                    parse_props_param(node_id, pod, &state_listener);
                }
            }
        })
        .register();

    // Store the bound node
    state.borrow_mut().bound_nodes.insert(
        node_id,
        BoundNode {
            proxy,
            _listener: listener,
            info: node_info,
        },
    );

    debug!("Successfully bound to node {}", node_id);
}

/// Parse a Props param pod to extract volume and mute values.
fn parse_props_param(node_id: u32, pod: &Pod, state: &Rc<RefCell<PwThreadState>>) {
    // TODO: Implement proper pod parsing to extract volume/mute values
    // For now, we just log that we received a param update
    trace!("Received Props param for node {}", node_id);

    // The pod contains an Object with properties for volume, mute, etc.
    // Proper parsing would iterate through the properties and extract values.
    // This is complex due to the SPA pod format.
}

/// Set up the registry listener to watch for nodes, ports, and links.
fn setup_registry_listener(
    registry: &pipewire::registry::Registry,
    state: Rc<RefCell<PwThreadState>>,
    event_tx: Rc<mpsc::Sender<PwEvent>>,
    core: &pipewire::core::Core,
) -> pipewire::registry::Listener {
    let state_add = state.clone();
    let state_remove = state;
    let event_tx_add = event_tx.clone();
    let event_tx_remove = event_tx;
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

                    // Auto-bind to sinks and streams for volume control
                    let should_bind = matches!(
                        node.media_class,
                        MediaClass::AudioSink | MediaClass::StreamOutputAudio
                    ) || node.name.starts_with("sootmix.");

                    state_add.borrow_mut().nodes.insert(global.id, node.clone());
                    let _ = event_tx_add.send(PwEvent::NodeAdded(node));

                    // Bind to controllable nodes
                    if should_bind {
                        bind_node(&registry_clone, global.id, state_add.clone());
                    }
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

            // Remove bound node proxy if exists
            if state.bound_nodes.remove(&id).is_some() {
                debug!("Unbound node: {}", id);
            }

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
