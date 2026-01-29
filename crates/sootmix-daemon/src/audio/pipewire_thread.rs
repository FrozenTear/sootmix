// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! PipeWire thread management and event handling.

use crate::audio::native_loopback::NativeLoopback;
use crate::audio::types::{AudioChannel, MediaClass, PortDirection, PwLink, PwNode, PwPort};
use pipewire::link::Link;
use pipewire::node::{Node, NodeListener};
use pipewire::properties::properties;
use pipewire::spa::param::ParamType;
use pipewire::spa::pod::Pod;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use thiserror::Error;
use tracing::{debug, error, info, trace, warn};
use uuid::Uuid;

/// Commands sent from the service to the PipeWire thread.
#[derive(Clone)]
pub enum PwCommand {
    CreateVirtualSink {
        channel_id: Uuid,
        name: String,
        /// Target device name for routing (uses system default if None)
        target_device: Option<String>,
    },
    CreateVirtualSource {
        channel_id: Uuid,
        name: String,
        /// Target input device name (uses system default if None)
        target_device: Option<String>,
    },
    DestroyVirtualSink {
        node_id: u32,
    },
    UpdateSinkDescription {
        node_id: u32,
        description: String,
    },
    BindNode {
        node_id: u32,
    },
    UnbindNode {
        node_id: u32,
    },
    CreateLink {
        output_port: u32,
        input_port: u32,
    },
    DestroyLink {
        link_id: u32,
    },
    SetVolume {
        node_id: u32,
        volume: f32,
    },
    SetMute {
        node_id: u32,
        muted: bool,
    },
    SetDefaultSink {
        node_id: u32,
    },
    RouteChannelToDevice {
        loopback_output_node: u32,
        target_device_id: Option<u32>,
        /// Channel ID for native loopback lookup
        channel_id: Option<Uuid>,
    },
    CreateRecordingSource {
        name: String,
    },
    DestroyRecordingSource {
        node_id: u32,
    },
    /// Create a native noise filter for an input channel.
    CreateNativeNoiseFilter {
        channel_id: Uuid,
        name: String,
        target_mic: Option<String>,
        vad_threshold: f32,
    },
    /// Destroy a native noise filter.
    DestroyNativeNoiseFilter {
        channel_id: Uuid,
    },
    /// Link an input channel's capture stream to a target microphone.
    /// This is needed because we use node.autoconnect=false to prevent WirePlumber
    /// from linking the capture stream to ALL available mics.
    LinkInputChannelToMic {
        /// The capture stream node (input side of the loopback)
        capture_node_id: u32,
        /// Target mic node name (description or node.name)
        target_mic_name: String,
    },
    Shutdown,
}

/// Events sent from the PipeWire thread to the service.
#[derive(Debug, Clone)]
pub enum PwEvent {
    Connected,
    Disconnected,
    NodeAdded(PwNode),
    NodeRemoved(u32),
    NodeChanged(PwNode),
    PortAdded(PwPort),
    PortRemoved(u32),
    LinkAdded(PwLink),
    LinkRemoved(u32),
    VirtualSinkCreated {
        channel_id: Uuid,
        node_id: u32,
        loopback_output_node_id: Option<u32>,
    },
    VirtualSourceCreated {
        channel_id: Uuid,
        source_node_id: u32,
        loopback_capture_node_id: Option<u32>,
    },
    VirtualSinkDestroyed {
        node_id: u32,
    },
    RecordingSourceCreated {
        name: String,
        node_id: u32,
    },
    RecordingSourceDestroyed {
        node_id: u32,
    },
    /// Native noise filter was created successfully.
    NativeNoiseFilterCreated {
        channel_id: Uuid,
        source_node_id: u32,
    },
    /// Native noise filter was destroyed.
    NativeNoiseFilterDestroyed {
        channel_id: Uuid,
    },
    /// Native noise filter creation failed.
    NativeNoiseFilterFailed {
        channel_id: Uuid,
        error: String,
    },
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

struct BoundNode {
    proxy: Node,
    _listener: NodeListener,
}

struct CreatedLink {
    #[allow(dead_code)]
    proxy: Link,
}

const CLI_THROTTLE_MS: u64 = 50;

/// Pending CLI command that was throttled
#[derive(Clone)]
enum PendingCliCmd {
    Volume(f32),
    Mute(bool),
}

struct PwThreadState {
    nodes: HashMap<u32, PwNode>,
    ports: HashMap<u32, PwPort>,
    links: HashMap<u32, PwLink>,
    virtual_sinks: HashMap<Uuid, u32>,
    bound_nodes: HashMap<u32, BoundNode>,
    created_links: HashMap<(u32, u32), CreatedLink>,
    event_tx: Rc<mpsc::Sender<PwEvent>>,
    cli_last_cmd: HashMap<u32, Instant>,
    /// Pending CLI commands that were throttled - stores the latest value to apply
    pending_cli_cmds: HashMap<u32, PendingCliCmd>,
    /// Node IDs currently being processed by CLI background threads.
    /// Prevents concurrent CLI operations on the same node.
    cli_in_flight: Arc<Mutex<HashSet<u32>>>,
    /// Native loopback instances (replacing pw-loopback CLI).
    native_loopbacks: HashMap<Uuid, NativeLoopback>,
    /// Pending loopback node discovery: node_name -> (channel_id, is_main_node)
    /// Used to match nodes created by native loopbacks once they appear in registry.
    pending_loopback_nodes: HashMap<String, (Uuid, bool)>,
    /// Discovered node IDs for pending loopbacks: channel_id -> (main_node_id, secondary_node_id)
    /// For sinks: (sink_id, output_id), for sources: (source_id, capture_id)
    discovered_loopback_nodes: HashMap<Uuid, (Option<u32>, Option<u32>)>,
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
            pending_cli_cmds: HashMap::new(),
            cli_in_flight: Arc::new(Mutex::new(HashSet::new())),
            native_loopbacks: HashMap::new(),
            pending_loopback_nodes: HashMap::new(),
            discovered_loopback_nodes: HashMap::new(),
        }
    }

    /// Check if CLI command should run now, or if it should be deferred.
    /// Returns true if the command can run immediately.
    /// If throttled, the pending command is stored and will be retrieved later.
    fn should_run_cli(&mut self, node_id: u32, cmd: PendingCliCmd) -> bool {
        let now = Instant::now();
        let throttle = Duration::from_millis(CLI_THROTTLE_MS);

        if let Some(last) = self.cli_last_cmd.get(&node_id) {
            if now.duration_since(*last) < throttle {
                // Throttled - store the latest pending command
                self.pending_cli_cmds.insert(node_id, cmd);
                return false;
            }
        }
        self.cli_last_cmd.insert(node_id, now);
        // Clear any pending command since we're executing now
        self.pending_cli_cmds.remove(&node_id);
        true
    }

    /// Check if throttle period has passed for any pending commands
    fn get_ready_pending_cmds(&mut self) -> Vec<(u32, PendingCliCmd)> {
        let now = Instant::now();
        let throttle = Duration::from_millis(CLI_THROTTLE_MS);

        let ready: Vec<u32> = self
            .pending_cli_cmds
            .keys()
            .filter(|&node_id| {
                self.cli_last_cmd
                    .get(node_id)
                    .map(|last| now.duration_since(*last) >= throttle)
                    .unwrap_or(true)
            })
            .copied()
            .collect();

        ready
            .iter()
            .filter_map(|&node_id| {
                self.pending_cli_cmds.remove(&node_id).map(|cmd| {
                    self.cli_last_cmd.insert(node_id, now);
                    (node_id, cmd)
                })
            })
            .collect()
    }

    fn get_node_for_port(&self, port_id: u32) -> Option<u32> {
        self.ports.get(&port_id).map(|p| p.node_id)
    }

    fn set_node_volume(&self, node_id: u32, volume: f32) -> Result<(), String> {
        let bound = self
            .bound_nodes
            .get(&node_id)
            .ok_or_else(|| format!("Node {} not bound", node_id))?;

        let pod_data = build_channel_volumes_pod(&[volume, volume]).map_err(|e| e.to_string())?;
        let pod = Pod::from_bytes(&pod_data)
            .ok_or_else(|| "Failed to create Pod from bytes".to_string())?;

        bound.proxy.set_param(ParamType::Props, 0, pod);
        trace!("Native volume set on node {}: {:.3}", node_id, volume);
        Ok(())
    }

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
}

/// Handle to the PipeWire thread.
pub struct PwThread {
    cmd_tx: pipewire::channel::Sender<PwCommand>,
    handle: Option<JoinHandle<()>>,
}

impl PwThread {
    /// Spawn the PipeWire thread.
    pub fn spawn(event_tx: mpsc::Sender<PwEvent>) -> Result<Self, PwError> {
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

    /// Shutdown the PipeWire thread.
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

fn run_pipewire_loop(
    cmd_rx: pipewire::channel::Receiver<PwCommand>,
    event_tx: mpsc::Sender<PwEvent>,
) -> Result<(), PwError> {
    pipewire::init();
    info!("PipeWire initialized");

    let main_loop = pipewire::main_loop::MainLoopRc::new(None)
        .map_err(|e| PwError::InitFailed(e.to_string()))?;

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

    let event_tx = Rc::new(event_tx);
    let state = Rc::new(RefCell::new(PwThreadState::new(event_tx.clone())));

    let main_loop_weak = main_loop.downgrade();
    let state_cmd = state.clone();
    let core_cmd = core.clone();
    let registry_cmd = registry.clone();
    let _cmd_receiver = cmd_rx.attach(main_loop.loop_(), move |cmd| {
        handle_command(cmd, &state_cmd, &main_loop_weak, &core_cmd, &registry_cmd);
    });

    let _registry_listener = setup_registry_listener(&registry, state.clone(), event_tx.clone());

    // Set up a timer to process pending CLI commands that were throttled
    let state_timer = state.clone();
    let timer = main_loop.loop_().add_timer(move |_| {
        process_pending_cli_commands(&state_timer);
    });
    // Fire every 60ms (slightly longer than CLI_THROTTLE_MS) to process any pending commands
    if timer
        .update_timer(
            Some(Duration::from_millis(CLI_THROTTLE_MS + 10)),
            Some(Duration::from_millis(CLI_THROTTLE_MS + 10)),
        )
        .into_result()
        .is_err()
    {
        warn!("Failed to set CLI throttle timer interval");
    }

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
/// The pending CLI timer will retry later.
fn spawn_cli_work_for_node<F>(
    event_tx: &Rc<mpsc::Sender<PwEvent>>,
    in_flight: &Arc<Mutex<HashSet<u32>>>,
    node_id: u32,
    work: F,
) where
    F: FnOnce(mpsc::Sender<PwEvent>) + Send + 'static,
{
    // Check if node is already in-flight
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
        // Remove node from in-flight set when done
        let mut set = in_flight_clone.lock().unwrap_or_else(|e| e.into_inner());
        set.remove(&node_id);
    });
}

/// Process any pending CLI commands that were throttled but are now ready to execute.
/// CLI commands are dispatched to background threads to avoid blocking the PW loop.
/// Uses in-flight tracking to prevent concurrent CLI operations on the same node.
fn process_pending_cli_commands(state: &Rc<RefCell<PwThreadState>>) {
    let ready_cmds = state.borrow_mut().get_ready_pending_cmds();

    if ready_cmds.is_empty() {
        return;
    }

    let in_flight = Arc::clone(&state.borrow().cli_in_flight);
    let _tx = mpsc::Sender::clone(&state.borrow().event_tx);

    // Filter out commands for nodes already in-flight
    let filtered_cmds: Vec<(u32, PendingCliCmd)> = {
        let set = in_flight.lock().unwrap_or_else(|e| e.into_inner());
        ready_cmds
            .into_iter()
            .filter(|(node_id, _)| !set.contains(node_id))
            .collect()
    };

    if filtered_cmds.is_empty() {
        return;
    }

    // Mark all as in-flight
    {
        let mut set = in_flight.lock().unwrap_or_else(|e| e.into_inner());
        for (node_id, _) in &filtered_cmds {
            set.insert(*node_id);
        }
    }

    let in_flight_clone = Arc::clone(&in_flight);
    thread::spawn(move || {
        for (node_id, cmd) in &filtered_cmds {
            match cmd {
                PendingCliCmd::Volume(volume) => {
                    trace!(
                        "Processing pending volume command: node={} volume={:.3}",
                        node_id,
                        volume
                    );
                    if let Err(e) = crate::audio::volume::set_volume(*node_id, *volume) {
                        error!(
                            "Pending CLI volume control failed for node {}: {}",
                            node_id, e
                        );
                    }
                }
                PendingCliCmd::Mute(muted) => {
                    trace!(
                        "Processing pending mute command: node={} muted={}",
                        node_id,
                        muted
                    );
                    if let Err(e) = crate::audio::volume::set_mute(*node_id, *muted) {
                        error!(
                            "Pending CLI mute control failed for node {}: {}",
                            node_id, e
                        );
                    }
                }
            }
        }
        // Remove all from in-flight set
        let mut set = in_flight_clone.lock().unwrap_or_else(|e| e.into_inner());
        for (node_id, _) in &filtered_cmds {
            set.remove(node_id);
        }
    });
}

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

        PwCommand::CreateVirtualSink { channel_id, name, target_device: _ } => {
            debug!(
                "Creating virtual sink via pw-loopback: '{}' for channel {}",
                name, channel_id
            );
            // Use pw-loopback CLI which properly handles adapter for FL/FR ports
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

        PwCommand::CreateVirtualSource { channel_id, name, target_device } => {
            debug!(
                "Creating virtual source via pw-loopback: '{}' for channel {} (target: {:?})",
                name, channel_id, target_device
            );
            // Use pw-loopback CLI which properly handles adapter for FL/FR ports
            spawn_cli_work(&state.borrow().event_tx, move |event_tx| {
                match crate::audio::virtual_sink::create_virtual_source(&name, target_device.as_deref()) {
                    Ok(result) => {
                        let _ = event_tx.send(PwEvent::VirtualSourceCreated {
                            channel_id,
                            source_node_id: result.source_node_id,
                            loopback_capture_node_id: result.capture_stream_node_id,
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

        PwCommand::UpdateSinkDescription {
            node_id,
            description,
        } => {
            debug!("Updating sink {} description to '{}'", node_id, description);
            spawn_cli_work(&state.borrow().event_tx, move |_event_tx| {
                if let Err(e) =
                    crate::audio::virtual_sink::update_node_description(node_id, &description)
                {
                    warn!("Failed to update sink description: {}", e);
                }
            });
        }

        PwCommand::DestroyVirtualSink { node_id } => {
            debug!("Destroying virtual sink/source with node_id: {}", node_id);
            state.borrow_mut().bound_nodes.remove(&node_id);
            state
                .borrow_mut()
                .virtual_sinks
                .retain(|_, &mut id| id != node_id);

            // Find and remove native loopback by node ID
            let channel_to_remove: Option<Uuid> = {
                let state_ref = state.borrow();
                state_ref
                    .native_loopbacks
                    .iter()
                    .find(|(_, loopback)| loopback.main_node_id() == node_id)
                    .map(|(id, _)| *id)
            };

            if let Some(channel_id) = channel_to_remove {
                if let Some(loopback) = state.borrow_mut().native_loopbacks.remove(&channel_id) {
                    if let Err(e) = loopback.disconnect() {
                        warn!("Failed to disconnect loopback: {}", e);
                    }
                    // Loopback is dropped here, cleaning up streams
                    info!("Destroyed native loopback for channel {}", channel_id);
                }
            } else {
                // Fallback to CLI destroy for legacy loopbacks
                warn!("No native loopback found for node {}, trying CLI destroy", node_id);
                spawn_cli_work(&state.borrow().event_tx, move |_event_tx| {
                    if let Err(e) = crate::audio::virtual_sink::destroy_virtual_sink(node_id) {
                        warn!("Failed to destroy virtual sink {}: {}", node_id, e);
                    }
                });
            }

            let _ = state.borrow().event_tx.send(PwEvent::VirtualSinkDestroyed { node_id });
        }

        PwCommand::CreateLink {
            output_port,
            input_port,
        } => {
            info!(
                "PW cmd: CreateLink output_port={} -> input_port={}",
                output_port, input_port
            );

            let (out_node, in_node) = {
                let s = state.borrow();
                (
                    s.get_node_for_port(output_port),
                    s.get_node_for_port(input_port),
                )
            };

            let out_node = match out_node {
                Some(n) => n,
                None => {
                    warn!("Output port {} not found, using CLI fallback", output_port);
                    spawn_cli_work(&state.borrow().event_tx, move |event_tx| {
                        if let Err(e) = crate::audio::routing::create_link(output_port, input_port) {
                            let _ =
                                event_tx.send(PwEvent::Error(format!("Failed to create link: {}", e)));
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
                            let _ =
                                event_tx.send(PwEvent::Error(format!("Failed to create link: {}", e)));
                        }
                    });
                    return;
                }
            };

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
                    state
                        .borrow_mut()
                        .created_links
                        .insert((output_port, input_port), CreatedLink { proxy: link });
                }
                Err(e) => {
                    warn!("Native link creation failed: {:?}, using CLI fallback", e);
                    spawn_cli_work(&state.borrow().event_tx, move |event_tx| {
                        if let Err(e2) = crate::audio::routing::create_link(output_port, input_port) {
                            let _ =
                                event_tx.send(PwEvent::Error(format!("Failed to create link: {}", e2)));
                        }
                    });
                }
            }
        }

        PwCommand::DestroyLink { link_id } => {
            debug!("Destroying link: {}", link_id);
            // Destroy links SYNCHRONOUSLY to avoid race conditions with WirePlumber.
            // If we destroy async and create sync, WirePlumber can re-route the stream
            // to a different sink during the window between destroy and create.
            if let Err(e) = crate::audio::routing::destroy_link(link_id) {
                let _ = state.borrow().event_tx.send(PwEvent::Error(format!("Failed to destroy link: {}", e)));
            }
        }

        PwCommand::BindNode { node_id } => {
            debug!("Request to bind to node {} for control", node_id);
            if state.borrow().bound_nodes.contains_key(&node_id) {
                debug!("Node {} already bound", node_id);
            }
        }

        PwCommand::UnbindNode { node_id } => {
            debug!("Unbinding from node {}", node_id);
            state.borrow_mut().bound_nodes.remove(&node_id);
        }

        PwCommand::SetVolume { node_id, volume } => {
            trace!("PW cmd: SetVolume node={} volume={:.3}", node_id, volume);
            let result = state.borrow().set_node_volume(node_id, volume);
            if let Err(e) = result {
                if state
                    .borrow_mut()
                    .should_run_cli(node_id, PendingCliCmd::Volume(volume))
                {
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

        PwCommand::SetMute { node_id, muted } => {
            trace!("PW cmd: SetMute node={} muted={}", node_id, muted);
            let result = state.borrow().set_node_mute(node_id, muted);
            if let Err(e) = result {
                if state
                    .borrow_mut()
                    .should_run_cli(node_id, PendingCliCmd::Mute(muted))
                {
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

        PwCommand::SetDefaultSink { node_id } => {
            info!("Setting default sink to node {}", node_id);
            spawn_cli_work(&state.borrow().event_tx, move |_event_tx| {
                let output = std::process::Command::new("wpctl")
                    .args(["set-default", &node_id.to_string()])
                    .output();

                if let Ok(result) = output {
                    if !result.status.success() {
                        let stderr = String::from_utf8_lossy(&result.stderr);
                        warn!("wpctl set-default failed: {}", stderr);
                    }
                }
            });
        }

        PwCommand::RouteChannelToDevice {
            loopback_output_node,
            target_device_id,
            channel_id,
        } => {
            info!(
                "Routing loopback output {} to device {:?} (channel={:?})",
                loopback_output_node, target_device_id, channel_id
            );

            // Resolve the target device ID
            let target_node_id = target_device_id.or_else(|| {
                // Use the WirePlumber default sink as fallback
                crate::audio::routing::get_default_sink_id().and_then(|id| {
                    let s = state.borrow();
                    if s.nodes.get(&id).map_or(false, |n| {
                        n.media_class == MediaClass::AudioSink
                            && !n.name.starts_with("sootmix.")
                    }) {
                        Some(id)
                    } else {
                        None
                    }
                })
            });

            // Check if this is a native loopback - if so, use native re-routing
            let is_native = channel_id
                .map(|id| state.borrow().native_loopbacks.contains_key(&id))
                .unwrap_or(false);

            if is_native {
                // Native loopback: use WirePlumber metadata for re-routing
                // This lets WirePlumber handle format conversion properly
                if let Some(ch_id) = channel_id {
                    let reroute_result = {
                        let st = state.borrow();
                        if let Some(loopback) = st.native_loopbacks.get(&ch_id) {
                            Some(loopback.reroute_to_device(target_node_id))
                        } else {
                            None
                        }
                    };

                    match reroute_result {
                        Some(Ok(())) => {
                            info!("Native loopback {} re-routed to {:?}", ch_id, target_node_id);
                        }
                        Some(Err(e)) => {
                            // Don't try CLI fallback - pw-link would fail due to format mismatch
                            warn!("Native re-route failed: {}", e);
                        }
                        None => {
                            warn!("Native loopback not found for channel {}", ch_id);
                        }
                    }
                }
                // Native routing via metadata doesn't require manual link creation
                // WirePlumber will handle it
                return;
            }

            // Legacy CLI-based routing for non-native loopbacks
            let links_to_destroy: Vec<u32> = {
                let s = state.borrow();
                s.links
                    .values()
                    .filter(|l| l.output_node == loopback_output_node)
                    .map(|l| l.id)
                    .collect()
            };

            let port_pairs: Vec<(u32, u32)> = if let Some(target_id) = target_node_id {
                let s = state.borrow();
                let mut out_ports: Vec<_> = s
                    .ports
                    .values()
                    .filter(|p| {
                        p.node_id == loopback_output_node
                            && p.direction == PortDirection::Output
                    })
                    .collect();
                let mut in_ports: Vec<_> = s
                    .ports
                    .values()
                    .filter(|p| p.node_id == target_id && p.direction == PortDirection::Input)
                    .collect();

                out_ports.sort_by(|a, b| (&a.channel, a.id).cmp(&(&b.channel, b.id)));
                in_ports.sort_by(|a, b| (&a.channel, a.id).cmp(&(&b.channel, b.id)));

                let mut pairs = Vec::new();
                let mut used_inputs: std::collections::HashSet<u32> =
                    std::collections::HashSet::new();

                for out_port in &out_ports {
                    for in_port in &in_ports {
                        if used_inputs.contains(&in_port.id) {
                            continue;
                        }
                        if out_port.channel.is_compatible(&in_port.channel) {
                            pairs.push((out_port.id, in_port.id));
                            used_inputs.insert(in_port.id);
                            break;
                        }
                    }
                }

                if pairs.is_empty() {
                    for out_port in &out_ports {
                        for in_port in &in_ports {
                            if used_inputs.contains(&in_port.id) {
                                continue;
                            }
                            let out_name = out_port.name.to_lowercase();
                            let in_name = in_port.name.to_lowercase();
                            let is_match = (out_name.contains("_0") && in_name.contains("_0"))
                                || (out_name.contains("_1") && in_name.contains("_1"));
                            if is_match {
                                pairs.push((out_port.id, in_port.id));
                                used_inputs.insert(in_port.id);
                                break;
                            }
                        }
                    }
                }

                if pairs.is_empty() && !out_ports.is_empty() && !in_ports.is_empty() {
                    for (out_port, in_port) in out_ports.iter().zip(in_ports.iter()) {
                        pairs.push((out_port.id, in_port.id));
                    }
                }

                pairs
            } else {
                Vec::new()
            };

            // Dispatch CLI link operations to a background thread
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
                    info!(
                        "Creating link from loopback: {} -> {}",
                        output_port, input_port
                    );
                    if let Err(e) = crate::audio::routing::create_link(output_port, input_port) {
                        warn!(
                            "Failed to create link {} -> {}: {}",
                            output_port, input_port, e
                        );
                    }
                }
            });
        }

        PwCommand::CreateRecordingSource { name } => {
            info!("Creating recording source: {}", name);
            spawn_cli_work(&state.borrow().event_tx, move |event_tx| {
                match crate::audio::virtual_sink::create_virtual_source(&name, None) {
                    Ok(result) => {
                        let _ = event_tx.send(PwEvent::RecordingSourceCreated {
                            name,
                            node_id: result.source_node_id,
                        });
                    }
                    Err(e) => {
                        let _ = event_tx.send(PwEvent::Error(format!(
                            "Failed to create recording source: {}",
                            e
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

        PwCommand::CreateNativeNoiseFilter { channel_id, name, target_mic, vad_threshold } => {
            info!(
                "Creating noise filter (CLI) for channel {} (name={}, target={:?}, vad={}%)",
                channel_id, name, target_mic, vad_threshold
            );

            // Use CLI-based filter-chain with RNNoise LADSPA plugin
            spawn_cli_work(&state.borrow().event_tx, move |event_tx| {
                match crate::audio::noise_filter::create_noise_filter(
                    channel_id,
                    &name,
                    target_mic.as_deref(),
                    false, // mono
                    vad_threshold,
                ) {
                    Ok(source_node_id) => {
                        info!(
                            "Noise filter created for {}: source_node={}",
                            channel_id, source_node_id
                        );
                        let _ = event_tx.send(PwEvent::NativeNoiseFilterCreated {
                            channel_id,
                            source_node_id,
                        });
                    }
                    Err(e) => {
                        error!("Failed to create noise filter: {}", e);
                        let _ = event_tx.send(PwEvent::NativeNoiseFilterFailed {
                            channel_id,
                            error: e.to_string(),
                        });
                    }
                }
            });
        }

        PwCommand::DestroyNativeNoiseFilter { channel_id } => {
            info!("Destroying noise filter (CLI) for channel {}", channel_id);
            // Use CLI-based destroy - kills the pipewire filter-chain process
            spawn_cli_work(&state.borrow().event_tx, move |event_tx| {
                if let Err(e) = crate::audio::noise_filter::destroy_noise_filter(channel_id) {
                    warn!("Error destroying noise filter: {}", e);
                }
                let _ = event_tx.send(PwEvent::NativeNoiseFilterDestroyed { channel_id });
            });
        }

        PwCommand::LinkInputChannelToMic { capture_node_id, target_mic_name } => {
            info!(
                "Linking capture stream {} to mic '{}'",
                capture_node_id, target_mic_name
            );

            // Find the mic node by name/description
            let mic_node_id = {
                let st = state.borrow();
                st.nodes.values().find(|n| {
                    n.media_class == MediaClass::AudioSource
                        && (n.name == target_mic_name || n.description == target_mic_name)
                }).map(|n| n.id)
            };

            if let Some(mic_id) = mic_node_id {
                // Get ports for both nodes
                let (mic_output_ports, capture_input_ports) = {
                    let st = state.borrow();
                    let mic_ports: Vec<_> = st.ports.values()
                        .filter(|p| p.node_id == mic_id && p.direction == PortDirection::Output)
                        .cloned()
                        .collect();
                    let capture_ports: Vec<_> = st.ports.values()
                        .filter(|p| p.node_id == capture_node_id && p.direction == PortDirection::Input)
                        .cloned()
                        .collect();
                    (mic_ports, capture_ports)
                };

                // Create links between matching channels (FL->FL, FR->FR, or MONO->FL/FR)
                for mic_port in &mic_output_ports {
                    for capture_port in &capture_input_ports {
                        // Match by channel name suffix (FL, FR, MONO)
                        let mic_ch = mic_port.name.rsplit('_').next().unwrap_or("");
                        let cap_ch = capture_port.name.rsplit('_').next().unwrap_or("");

                        // Link if channels match, or if mic is mono (link to both)
                        if mic_ch == cap_ch || mic_ch == "MONO" {
                            debug!(
                                "Creating link: mic port {} ({}) -> capture port {} ({})",
                                mic_port.id, mic_port.name, capture_port.id, capture_port.name
                            );
                            if let Err(e) = crate::audio::routing::create_link(mic_port.id, capture_port.id) {
                                warn!("Failed to link mic to capture stream: {}", e);
                            }
                        }
                    }
                }
            } else {
                warn!("Could not find mic '{}' to link to capture stream", target_mic_name);
            }
        }
    }
}

fn bind_node_from_global(
    global: &pipewire::registry::GlobalObject<&pipewire::spa::utils::dict::DictRef>,
    state: &Rc<RefCell<PwThreadState>>,
    registry: &pipewire::registry::RegistryRc,
) -> Result<(), String> {
    let node_id = global.id;

    if state.borrow().bound_nodes.contains_key(&node_id) {
        return Ok(());
    }

    debug!("Binding node proxy for node {}", node_id);

    let node: Node = registry
        .bind(global)
        .map_err(|e| format!("Failed to bind node {}: {:?}", node_id, e))?;

    let listener = node
        .add_listener_local()
        .param(move |_seq, _id, _index, _next, _pod| {
            trace!("Received param update for bound node");
        })
        .register();

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

fn setup_registry_listener(
    registry: &pipewire::registry::RegistryRc,
    state: Rc<RefCell<PwThreadState>>,
    event_tx: Rc<mpsc::Sender<PwEvent>>,
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

                    for (k, v) in props.iter() {
                        node.properties.insert(k.to_string(), v.to_string());
                    }

                    debug!(
                        "Node added: id={}, name={}, class={:?}",
                        node.id, node.name, node.media_class
                    );

                    let should_bind = node.name.starts_with("sootmix.")
                        || node.media_class == MediaClass::AudioSink;

                    if should_bind {
                        if let Err(e) = bind_node_from_global(global, &state_add, &registry_clone) {
                            warn!("Failed to bind node {}: {}", node.id, e);
                        }
                    }

                    state_add.borrow_mut().nodes.insert(global.id, node.clone());

                    // Check if this node matches a pending native loopback
                    let pending_match = state_add.borrow().pending_loopback_nodes.get(&node.name).cloned();
                    if let Some((channel_id, is_main)) = pending_match {
                        debug!(
                            "Matched pending loopback node: {} -> channel {} (is_main={})",
                            node.name, channel_id, is_main
                        );

                        // Record the discovered node ID
                        {
                            let mut st = state_add.borrow_mut();
                            let entry = st.discovered_loopback_nodes.entry(channel_id).or_insert((None, None));
                            if is_main {
                                entry.0 = Some(node.id);
                            } else {
                                entry.1 = Some(node.id);
                            }
                            st.pending_loopback_nodes.remove(&node.name);
                        }

                        // Check if we have both nodes for this channel
                        let discovered = state_add.borrow().discovered_loopback_nodes.get(&channel_id).cloned();
                        if let Some((Some(main_id), Some(secondary_id))) = discovered {
                            // Determine if this is a sink or source by checking if we have the loopback
                            let is_source = state_add.borrow().native_loopbacks
                                .get(&channel_id)
                                .map(|lb| lb.is_source)
                                .unwrap_or(false);

                            if is_source {
                                info!(
                                    "Native virtual source ready: channel={}, source={}, capture={}",
                                    channel_id, main_id, secondary_id
                                );
                                let _ = event_tx_add.send(PwEvent::VirtualSourceCreated {
                                    channel_id,
                                    source_node_id: main_id,
                                    loopback_capture_node_id: Some(secondary_id),
                                });
                            } else {
                                info!(
                                    "Native virtual sink ready: channel={}, sink={}, output={}",
                                    channel_id, main_id, secondary_id
                                );
                                let _ = event_tx_add.send(PwEvent::VirtualSinkCreated {
                                    channel_id,
                                    node_id: main_id,
                                    loopback_output_node_id: Some(secondary_id),
                                });
                            }

                            // Clean up
                            state_add.borrow_mut().discovered_loopback_nodes.remove(&channel_id);
                        }
                    }

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

                    // Clean up created_links entry now that the link exists in PipeWire registry
                    // This prevents unbounded growth of the created_links map
                    state_add
                        .borrow_mut()
                        .created_links
                        .remove(&(link.output_port, link.input_port));

                    state_add.borrow_mut().links.insert(global.id, link.clone());
                    let _ = event_tx_add.send(PwEvent::LinkAdded(link));
                }
                _ => {}
            }
        })
        .global_remove(move |id| {
            let mut state = state_remove.borrow_mut();
            state.bound_nodes.remove(&id);

            if state.nodes.remove(&id).is_some() {
                debug!("Node removed: {}", id);
                // Clean up virtual_sinks map if this was a virtual sink node
                state.virtual_sinks.retain(|_, &mut sink_id| sink_id != id);
                // Clean up any pending CLI commands for this node
                state.pending_cli_cmds.remove(&id);
                state.cli_last_cmd.remove(&id);
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

// Helper functions for building POD data

fn build_channel_volumes_pod(volumes: &[f32]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    use libspa::pod::serialize::PodSerializer;
    use libspa::pod::Value;
    use std::io::Cursor;

    let props = Value::Object(libspa::pod::Object {
        type_: libspa::sys::SPA_TYPE_OBJECT_Props,
        id: libspa::sys::SPA_PARAM_Props,
        properties: vec![libspa::pod::Property {
            key: libspa::sys::SPA_PROP_channelVolumes,
            flags: libspa::pod::PropertyFlags::empty(),
            value: Value::ValueArray(libspa::pod::ValueArray::Float(
                volumes.iter().copied().collect(),
            )),
        }],
    });

    let mut buffer = Vec::new();
    let cursor = Cursor::new(&mut buffer);
    PodSerializer::serialize(cursor, &props)?;

    Ok(buffer)
}

fn build_mute_pod(muted: bool) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    use libspa::pod::serialize::PodSerializer;
    use libspa::pod::Value;
    use std::io::Cursor;

    let props = Value::Object(libspa::pod::Object {
        type_: libspa::sys::SPA_TYPE_OBJECT_Props,
        id: libspa::sys::SPA_PARAM_Props,
        properties: vec![libspa::pod::Property {
            key: libspa::sys::SPA_PROP_mute,
            flags: libspa::pod::PropertyFlags::empty(),
            value: Value::Bool(muted),
        }],
    });

    let mut buffer = Vec::new();
    let cursor = Cursor::new(&mut buffer);
    PodSerializer::serialize(cursor, &props)?;

    Ok(buffer)
}
