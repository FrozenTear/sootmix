// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Core daemon service logic and state management.

use crate::audio::pipewire_thread::{PwCommand, PwEvent, PwThread};
use crate::audio::types::{MediaClass, PortDirection, PwLink, PwNode, PwPort};
use crate::config::{ConfigManager, MixerConfig, RoutingRulesConfig, SavedChannel};
use sootmix_ipc::{AppInfo, ChannelInfo, OutputInfo, RoutingRuleInfo};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::sync::mpsc as tokio_mpsc;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// D-Bus signal events that need to be emitted.
#[derive(Debug, Clone)]
pub enum SignalEvent {
    AppDiscovered(AppInfo),
    AppRemoved(String),
    OutputsChanged,
}

#[derive(Debug, Error)]
pub enum ServiceError {
    #[error("PipeWire error: {0}")]
    PipeWire(String),
    #[error("Channel not found: {0}")]
    ChannelNotFound(String),
    #[error("App not found: {0}")]
    AppNotFound(String),
    #[error("Config error: {0}")]
    Config(#[from] crate::config::ConfigError),
}

/// Internal channel state.
#[derive(Debug, Clone)]
pub struct ChannelState {
    pub id: Uuid,
    pub name: String,
    pub volume_db: f32,
    pub muted: bool,
    pub eq_enabled: bool,
    pub eq_preset: String,
    pub assigned_apps: Vec<String>,
    pub is_managed: bool,
    pub sink_name: Option<String>,
    pub output_device_name: Option<String>,
    pub pw_sink_id: Option<u32>,
    pub pw_loopback_output_id: Option<u32>,
    pub meter_levels: (f32, f32),
}

impl ChannelState {
    pub fn new(name: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
            volume_db: 0.0,
            muted: false,
            eq_enabled: false,
            eq_preset: "Flat".to_string(),
            assigned_apps: Vec::new(),
            is_managed: true,
            sink_name: None,
            output_device_name: None,
            pw_sink_id: None,
            pw_loopback_output_id: None,
            meter_levels: (0.0, 0.0),
        }
    }

    pub fn from_saved(saved: &SavedChannel) -> Self {
        Self {
            id: saved.id,
            name: saved.name.clone(),
            volume_db: saved.volume_db,
            muted: saved.muted,
            eq_enabled: saved.eq_enabled,
            eq_preset: saved.eq_preset.clone(),
            assigned_apps: saved.assigned_apps.clone(),
            is_managed: saved.is_managed,
            sink_name: saved.sink_name.clone(),
            output_device_name: saved.output_device_name.clone(),
            pw_sink_id: None,
            pw_loopback_output_id: None,
            meter_levels: (0.0, 0.0),
        }
    }

    pub fn to_channel_info(&self) -> ChannelInfo {
        let (left_db, right_db) = self.meter_levels_db();
        ChannelInfo {
            id: self.id.to_string(),
            name: self.name.clone(),
            volume_db: self.volume_db as f64,
            muted: self.muted,
            eq_enabled: self.eq_enabled,
            eq_preset: self.eq_preset.clone(),
            assigned_apps: self.assigned_apps.clone(),
            output_device: self.output_device_name.clone().unwrap_or_default(),
            meter_levels: (left_db as f64, right_db as f64),
        }
    }

    fn meter_levels_db(&self) -> (f32, f32) {
        // NOTE: Keep in sync with canonical implementation in src/audio/control.rs
        fn linear_to_db(linear: f32) -> f32 {
            if linear <= 0.0 {
                -96.0
            } else {
                20.0 * linear.log10()
            }
        }
        (
            linear_to_db(self.meter_levels.0),
            linear_to_db(self.meter_levels.1),
        )
    }

    pub fn volume_linear(&self) -> f32 {
        if self.muted {
            0.0
        } else {
            db_to_linear(self.volume_db)
        }
    }
}

// NOTE: Keep in sync with canonical implementation in src/audio/control.rs
fn db_to_linear(db: f32) -> f32 {
    if db <= -96.0 {
        0.0
    } else {
        10.0_f32.powf(db / 20.0)
    }
}

/// Internal app info.
#[derive(Debug, Clone)]
pub struct AppState {
    pub node_id: u32,
    pub name: String,
    pub binary: Option<String>,
}

impl AppState {
    pub fn identifier(&self) -> &str {
        self.binary.as_deref().unwrap_or(&self.name)
    }

    pub fn to_app_info(&self) -> AppInfo {
        AppInfo {
            id: self.node_id.to_string(),
            name: self.name.clone(),
            binary: self.binary.clone().unwrap_or_default(),
            icon: String::new(),
            node_id: self.node_id,
        }
    }
}

/// PipeWire graph state.
#[derive(Debug, Default, Clone)]
pub struct PwGraphState {
    pub nodes: HashMap<u32, PwNode>,
    pub ports: HashMap<u32, PwPort>,
    pub links: HashMap<u32, PwLink>,
}

impl PwGraphState {
    pub fn playback_streams(&self) -> Vec<&PwNode> {
        self.nodes
            .values()
            .filter(|n| n.is_playback_stream())
            .collect()
    }

    pub fn output_devices(&self, exclude_names: &[&str]) -> Vec<OutputInfo> {
        self.nodes
            .values()
            .filter(|n| {
                n.media_class == MediaClass::AudioSink
                    && !exclude_names.iter().any(|ex| n.name.contains(ex))
            })
            .map(|n| OutputInfo {
                node_id: n.id,
                name: n.name.clone(),
                description: n.description.clone(),
            })
            .collect()
    }

    pub fn find_port_pairs(&self, output_node: u32, input_node: u32) -> Vec<(u32, u32)> {
        use crate::audio::types::AudioChannel;

        // Collect and sort ports by channel for consistent ordering
        let mut output_ports: Vec<_> = self
            .ports
            .values()
            .filter(|p| p.node_id == output_node && p.direction == PortDirection::Output)
            .collect();
        let mut input_ports: Vec<_> = self
            .ports
            .values()
            .filter(|p| p.node_id == input_node && p.direction == PortDirection::Input)
            .collect();

        // Sort by channel first, then by port ID for stability
        output_ports.sort_by(|a, b| (&a.channel, a.id).cmp(&(&b.channel, b.id)));
        input_ports.sort_by(|a, b| (&a.channel, a.id).cmp(&(&b.channel, b.id)));

        let mut pairs = Vec::new();
        let mut used_inputs: std::collections::HashSet<u32> = std::collections::HashSet::new();

        // First pass: match by compatible channels
        for out_port in &output_ports {
            for in_port in &input_ports {
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

        // Second pass: if no channel matches found, try name-based matching
        if pairs.is_empty() {
            for out_port in &output_ports {
                for in_port in &input_ports {
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

        // Fallback: pair by sorted position (deterministic order)
        if pairs.is_empty() && !output_ports.is_empty() && !input_ports.is_empty() {
            for (out_port, in_port) in output_ports.iter().zip(input_ports.iter()) {
                pairs.push((out_port.id, in_port.id));
            }
        }

        pairs
    }

    pub fn links_from_node(&self, node_id: u32) -> Vec<&PwLink> {
        self.links
            .values()
            .filter(|l| l.output_node == node_id)
            .collect()
    }
}

/// Daemon state that is Send+Sync.
#[derive(Debug, Clone, Default)]
pub struct DaemonState {
    pub channels: Vec<ChannelState>,
    pub master_volume_db: f32,
    pub master_muted: bool,
    pub master_output: Option<String>,
    pub apps: Vec<AppState>,
    pub pw_graph: PwGraphState,
    pub pw_connected: bool,
    pub master_recording_enabled: bool,
    pub master_recording_source_id: Option<u32>,
    pub routing_rules: RoutingRulesConfig,
    pub auto_routed_apps: HashSet<u32>,
    /// Channels waiting for their sink ports to be ready for auto-routing
    pub pending_auto_route_channels: HashSet<Uuid>,
    /// Counter for periodic app refresh
    pub refresh_counter: u32,
    /// Time of last PipeWire reconnection attempt (for backoff)
    pub last_reconnect_attempt: Option<Instant>,
    /// Number of consecutive reconnection failures (for exponential backoff)
    pub reconnect_failures: u32,
}

impl DaemonState {
    pub fn new(mixer_config: MixerConfig, routing_rules: RoutingRulesConfig) -> Self {
        let channels: Vec<ChannelState> = mixer_config
            .channels
            .iter()
            .map(ChannelState::from_saved)
            .collect();

        Self {
            channels,
            master_volume_db: mixer_config.master.volume_db,
            master_muted: mixer_config.master.muted,
            master_output: mixer_config.master.output_device,
            apps: Vec::new(),
            pw_graph: PwGraphState::default(),
            pw_connected: false,
            master_recording_enabled: false,
            master_recording_source_id: None,
            routing_rules,
            auto_routed_apps: HashSet::new(),
            pending_auto_route_channels: HashSet::new(),
            refresh_counter: 0,
            last_reconnect_attempt: None,
            reconnect_failures: 0,
        }
    }

    pub fn get_channels(&self) -> Vec<ChannelInfo> {
        self.channels.iter().map(|c| c.to_channel_info()).collect()
    }

    pub fn get_apps(&mut self) -> Vec<AppInfo> {
        // Refresh app list before returning to ensure we have the latest
        self.update_available_apps();
        self.apps.iter().map(|a| a.to_app_info()).collect()
    }

    pub fn get_outputs(&self) -> Vec<OutputInfo> {
        let exclude: Vec<&str> = self
            .channels
            .iter()
            .filter_map(|c| c.pw_sink_id.map(|_| c.name.as_str()))
            .collect();
        let mut outputs = vec![OutputInfo {
            node_id: 0,
            name: "system-default".to_string(),
            description: "System Default".to_string(),
        }];
        outputs.extend(self.pw_graph.output_devices(&exclude));
        outputs
    }

    pub fn update_available_apps(&mut self) {
        self.apps = self
            .pw_graph
            .playback_streams()
            .iter()
            .filter(|node| {
                let name = &node.name;
                !name.contains("sootmix.")
                    && !name.starts_with("LB-")
                    && !name.contains("loopback")
                    && !name.starts_with("filter-chain")
            })
            .map(|node| AppState {
                node_id: node.id,
                name: node
                    .app_name
                    .clone()
                    .or_else(|| {
                        if !node.description.is_empty() {
                            Some(node.description.clone())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_else(|| node.name.clone()),
                binary: node.binary_name.clone(),
            })
            .collect();
    }

    pub fn get_routing_rules(&self) -> Vec<RoutingRuleInfo> {
        self.routing_rules
            .rules
            .iter()
            .map(|r| RoutingRuleInfo {
                id: r.id.to_string(),
                name: r.name.clone(),
                enabled: r.enabled,
                match_target: match r.match_target {
                    crate::config::MatchTarget::Name => "name".to_string(),
                    crate::config::MatchTarget::Binary => "binary".to_string(),
                    crate::config::MatchTarget::Either => "either".to_string(),
                },
                match_type: r.match_type.type_name().to_string(),
                pattern: r.match_type.pattern().to_string(),
                target_channel: r.target_channel.clone(),
                priority: r.priority,
            })
            .collect()
    }
}

/// The main daemon service.
pub struct DaemonService {
    pub state: DaemonState,
    pw_thread: Option<PwThread>,
    pw_event_rx: Option<mpsc::Receiver<PwEvent>>,
    config_manager: ConfigManager,
    /// Sender for D-Bus signal events.
    signal_tx: Option<tokio_mpsc::UnboundedSender<SignalEvent>>,
}


impl DaemonService {
    pub fn new(
        mixer_config: MixerConfig,
        routing_rules: RoutingRulesConfig,
        config_manager: ConfigManager,
    ) -> Self {
        Self {
            state: DaemonState::new(mixer_config, routing_rules),
            pw_thread: None,
            pw_event_rx: None,
            config_manager,
            signal_tx: None,
        }
    }

    /// Set the signal sender for D-Bus signal events.
    pub fn set_signal_sender(&mut self, tx: tokio_mpsc::UnboundedSender<SignalEvent>) {
        self.signal_tx = Some(tx);
    }

    /// Send a signal event to be emitted via D-Bus.
    fn emit_signal(&self, event: SignalEvent) {
        if let Some(ref tx) = self.signal_tx {
            if let Err(e) = tx.send(event) {
                warn!("Failed to send signal event: {}", e);
            }
        }
    }

    /// Update available apps and emit D-Bus signals for changes.
    fn update_apps_and_emit_signals(&mut self) {
        // Capture old app node IDs
        let old_app_ids: HashSet<u32> = self.state.apps.iter().map(|a| a.node_id).collect();

        // Update the app list
        self.state.update_available_apps();

        // Find new and removed apps
        let new_app_ids: HashSet<u32> = self.state.apps.iter().map(|a| a.node_id).collect();

        // Emit signals for newly discovered apps
        for app in &self.state.apps {
            if !old_app_ids.contains(&app.node_id) {
                debug!("Emitting AppDiscovered signal for: {} (node {})", app.name, app.node_id);
                self.emit_signal(SignalEvent::AppDiscovered(app.to_app_info()));
            }
        }

        // Emit signals for removed apps
        for old_id in &old_app_ids {
            if !new_app_ids.contains(old_id) {
                debug!("Emitting AppRemoved signal for node {}", old_id);
                self.emit_signal(SignalEvent::AppRemoved(old_id.to_string()));
            }
        }
    }

    /// Start the PipeWire thread.
    pub fn start_pipewire(&mut self) -> Result<(), ServiceError> {
        let (event_tx, event_rx) = mpsc::channel();

        let pw_thread =
            PwThread::spawn(event_tx).map_err(|e| ServiceError::PipeWire(e.to_string()))?;

        self.pw_thread = Some(pw_thread);
        self.pw_event_rx = Some(event_rx);

        info!("PipeWire thread started");
        Ok(())
    }

    /// Wait for initial PipeWire discovery to complete.
    pub fn wait_for_discovery(&mut self) {
        let max_wait = Duration::from_millis(1500);
        let min_wait = Duration::from_millis(300);
        let poll_interval = Duration::from_millis(50);
        let start = std::time::Instant::now();

        let mut last_node_count = 0;
        let mut stable_iterations = 0;
        const STABILITY_THRESHOLD: u32 = 4; // 200ms of no new nodes

        while start.elapsed() < max_wait {
            self.process_pw_events();

            let current_count = self.state.pw_graph.nodes.len();
            if current_count == last_node_count {
                stable_iterations += 1;
            } else {
                stable_iterations = 0;
                last_node_count = current_count;
            }

            // Exit early if we have nodes and they've been stable for a bit
            // but ensure we wait at least min_wait
            if start.elapsed() >= min_wait
                && current_count > 0
                && stable_iterations >= STABILITY_THRESHOLD
            {
                break;
            }

            std::thread::sleep(poll_interval);
        }

        // Final refresh of app list
        self.state.update_available_apps();

        info!(
            "PipeWire discovery complete: {} nodes, {} ports, {} apps (waited {:?})",
            self.state.pw_graph.nodes.len(),
            self.state.pw_graph.ports.len(),
            self.state.apps.len(),
            start.elapsed()
        );
    }

    /// Restore channels from config.
    pub fn restore_channels(&mut self) -> Result<(), ServiceError> {
        let channels_to_create: Vec<(Uuid, String)> = self
            .state
            .channels
            .iter()
            .filter(|c| c.is_managed && c.pw_sink_id.is_none())
            .map(|c| (c.id, c.name.clone()))
            .collect();

        for (id, name) in channels_to_create {
            info!("Restoring channel: {} ({})", name, id);
            self.send_pw_command(PwCommand::CreateVirtualSink {
                channel_id: id,
                name,
            });
        }

        std::thread::sleep(Duration::from_millis(300));
        self.process_pw_events();

        Ok(())
    }

    /// Process pending PipeWire events.
    pub fn process_pw_events(&mut self) {
        let events: Vec<PwEvent> = if let Some(ref rx) = self.pw_event_rx {
            rx.try_iter().collect()
        } else {
            // No event receiver -- check if we need to reconnect
            self.attempt_reconnect_if_needed();
            return;
        };

        for event in events {
            self.handle_pw_event(event);
        }

        // If PW disconnected during event processing, the receiver was dropped
        if !self.state.pw_connected && self.pw_thread.is_none() {
            self.attempt_reconnect_if_needed();
        }

        // Periodic app refresh - every ~2 seconds (20 iterations at 100ms each)
        // This catches apps that were added with incomplete properties
        self.state.refresh_counter += 1;
        if self.state.refresh_counter >= 20 {
            self.state.refresh_counter = 0;
            self.update_apps_and_emit_signals();
        }
    }

    /// Attempt PipeWire reconnection with exponential backoff.
    fn attempt_reconnect_if_needed(&mut self) {
        // Exponential backoff: 2s, 4s, 8s, 16s, capped at 30s
        let backoff = Duration::from_secs(
            (2u64 << self.state.reconnect_failures.min(4)).min(30),
        );

        let should_attempt = match self.state.last_reconnect_attempt {
            None => true,
            Some(last) => last.elapsed() >= backoff,
        };

        if !should_attempt {
            return;
        }

        self.state.last_reconnect_attempt = Some(Instant::now());
        info!(
            "Attempting PipeWire reconnection (attempt #{})...",
            self.state.reconnect_failures + 1
        );

        match self.start_pipewire() {
            Ok(()) => {
                // Wait for PipeWire to discover the graph
                self.wait_for_discovery();

                // Restore channels
                if let Err(e) = self.restore_channels() {
                    warn!("Failed to restore channels after reconnection: {}", e);
                }

                self.state.reconnect_failures = 0;
                self.state.last_reconnect_attempt = None;
                info!("PipeWire reconnected successfully");
            }
            Err(e) => {
                self.state.reconnect_failures += 1;
                warn!(
                    "PipeWire reconnection failed: {} (next attempt in {:?})",
                    e,
                    Duration::from_secs(
                        (2u64 << self.state.reconnect_failures.min(4)).min(30)
                    )
                );
            }
        }
    }

    fn handle_pw_event(&mut self, event: PwEvent) {
        match event {
            PwEvent::Connected => {
                self.state.pw_connected = true;
                info!("PipeWire connected");
            }
            PwEvent::Disconnected => {
                self.state.pw_connected = false;
                warn!("PipeWire disconnected, will attempt reconnection");
                // Drop the old PW thread so we can create a new one
                self.pw_thread = None;
                self.pw_event_rx = None;
                // Clear stale PW state
                self.state.pw_graph = PwGraphState::default();
                for channel in &mut self.state.channels {
                    channel.pw_sink_id = None;
                    channel.pw_loopback_output_id = None;
                }
                self.state.auto_routed_apps.clear();
                self.state.pending_auto_route_channels.clear();
                self.state.master_recording_source_id = None;
            }
            PwEvent::NodeAdded(node) => {
                let node_id = node.id;
                let node_name = node.name.clone();
                let node_class = node.media_class.clone();
                let is_hw_sink = node_class == MediaClass::AudioSink
                    && !node.name.starts_with("sootmix.");
                let node_desc = node.description.clone();

                if is_hw_sink {
                    info!(
                        "Hardware sink appeared: id={}, name='{}', desc='{}', class={:?}",
                        node_id, node_name, node_desc, node_class
                    );
                }

                self.state.pw_graph.nodes.insert(node.id, node);
                self.update_apps_and_emit_signals();

                // Auto-route apps that match saved assignments
                self.try_auto_route_app(node_id, &node_name);

                // When a hardware sink (re)appears (e.g. Bluetooth reconnect),
                // re-route any channels whose output device matches this sink.
                if is_hw_sink {
                    self.try_reroute_channels_to_device(node_id, &node_name, &node_desc);
                }
            }
            PwEvent::NodeRemoved(id) => {
                let was_hw_sink = self.state.pw_graph.nodes.get(&id).map_or(false, |removed| {
                    let is_hw = removed.media_class == MediaClass::AudioSink
                        && !removed.name.starts_with("sootmix.");
                    if is_hw {
                        info!(
                            "Hardware sink removed: id={}, name='{}', desc='{}'",
                            id, removed.name, removed.description
                        );
                    }
                    is_hw
                });

                self.state.pw_graph.nodes.remove(&id);
                self.update_apps_and_emit_signals();

                // Check if this was a channel's sink or loopback output and clear stale IDs
                let mut channels_to_recreate: Vec<(Uuid, String)> = Vec::new();
                for channel in &mut self.state.channels {
                    if channel.pw_sink_id == Some(id) {
                        warn!(
                            "Channel '{}' sink node {} was removed externally",
                            channel.name, id
                        );
                        channel.pw_sink_id = None;
                        channel.pw_loopback_output_id = None;
                        if channel.is_managed {
                            channels_to_recreate.push((channel.id, channel.name.clone()));
                        }
                    } else if channel.pw_loopback_output_id == Some(id) {
                        warn!(
                            "Channel '{}' loopback output node {} was removed externally",
                            channel.name, id
                        );
                        channel.pw_loopback_output_id = None;
                    }
                }

                // Auto-recreate managed virtual sinks that were killed externally
                for (channel_id, name) in channels_to_recreate {
                    info!("Recreating virtual sink for channel '{}' after external removal", name);
                    self.send_pw_command(PwCommand::CreateVirtualSink {
                        channel_id,
                        name,
                    });
                }

                // When a hardware sink disappears, re-route orphaned channels
                // to the fallback device (configured master or system default)
                if was_hw_sink {
                    self.try_fallback_orphaned_channels();
                }
            }
            PwEvent::NodeChanged(node) => {
                self.state.pw_graph.nodes.insert(node.id, node);
                self.update_apps_and_emit_signals();
            }
            PwEvent::PortAdded(port) => {
                let port_node_id = port.node_id;
                self.state.pw_graph.ports.insert(port.id, port);

                // Port arrival is a good signal that a node is fully initialized.
                // Refresh app list to catch nodes that were added with incomplete properties.
                let old_app_count = self.state.apps.len();
                self.update_apps_and_emit_signals();
                if self.state.apps.len() > old_app_count {
                    debug!(
                        "Found {} new app(s) after port added for node {}",
                        self.state.apps.len() - old_app_count,
                        port_node_id
                    );
                    // Try auto-routing newly discovered apps
                    // Collect app info first to avoid borrow conflict with try_auto_route_app
                    let apps_to_route: Vec<_> = self
                        .state
                        .apps
                        .iter()
                        .filter(|app| app.node_id == port_node_id)
                        .map(|app| (app.node_id, app.name.clone()))
                        .collect();
                    for (node_id, name) in apps_to_route {
                        self.try_auto_route_app(node_id, &name);
                    }
                }

                // Check if this port belongs to a sink that's pending auto-routing
                // Always check - don't rely on pending_auto_route_channels since ports arrive one at a time
                let channel_to_route = self
                    .state
                    .channels
                    .iter()
                    .find(|c| c.pw_sink_id == Some(port_node_id) && !c.assigned_apps.is_empty())
                    .map(|c| c.id);

                if let Some(channel_id) = channel_to_route {
                    debug!(
                        "Port added for channel sink {}, trying auto-route",
                        port_node_id
                    );
                    self.try_auto_route_pending_apps(channel_id);
                }

                // Check if this port belongs to a loopback output that needs routing
                // to the hardware sink. This handles the timing issue where
                // RouteChannelToDevice fires before the loopback output ports exist.
                let loopback_route = self
                    .state
                    .channels
                    .iter()
                    .find(|c| c.pw_loopback_output_id == Some(port_node_id))
                    .map(|c| (c.id, c.pw_loopback_output_id));

                if let Some((_channel_id, Some(loopback_id))) = loopback_route {
                    // Check if this loopback output already has links to a hardware sink
                    let has_links = self
                        .state
                        .pw_graph
                        .links
                        .values()
                        .any(|l| l.output_node == loopback_id);

                    if !has_links {
                        debug!(
                            "Loopback output node {} has new ports but no links, retrying route to hardware",
                            loopback_id
                        );
                        let target_device_id = self.get_master_output_device_id();
                        self.send_pw_command(PwCommand::RouteChannelToDevice {
                            loopback_output_node: loopback_id,
                            target_device_id,
                        });
                    }
                }

                // Check if this port belongs to a hardware sink that channels are
                // routed to. This handles the timing issue where a Bluetooth device
                // reconnects and try_reroute_channels_to_device fires before the
                // sink's input ports are registered, causing empty port pairs.
                let is_hw_sink = self
                    .state
                    .pw_graph
                    .nodes
                    .get(&port_node_id)
                    .map_or(false, |n| {
                        n.media_class == MediaClass::AudioSink
                            && !n.name.starts_with("sootmix.")
                    });

                if is_hw_sink {
                    // Find channels whose loopback output should be linked to this
                    // sink but currently has no links (routing raced ahead of ports).
                    let orphaned_loopbacks: Vec<u32> = self
                        .state
                        .channels
                        .iter()
                        .filter_map(|c| {
                            let loopback_id = c.pw_loopback_output_id?;
                            let has_links = self
                                .state
                                .pw_graph
                                .links
                                .values()
                                .any(|l| l.output_node == loopback_id);
                            if !has_links {
                                Some(loopback_id)
                            } else {
                                None
                            }
                        })
                        .collect();

                    for loopback_id in orphaned_loopbacks {
                        debug!(
                            "Hardware sink {} got new port, retrying route for orphaned loopback {}",
                            port_node_id, loopback_id
                        );
                        self.send_pw_command(PwCommand::RouteChannelToDevice {
                            loopback_output_node: loopback_id,
                            target_device_id: Some(port_node_id),
                        });
                    }
                }
            }
            PwEvent::PortRemoved(id) => {
                self.state.pw_graph.ports.remove(&id);
            }
            PwEvent::LinkAdded(link) => {
                self.state.pw_graph.links.insert(link.id, link);
            }
            PwEvent::LinkRemoved(id) => {
                self.state.pw_graph.links.remove(&id);
            }
            PwEvent::VirtualSinkCreated {
                channel_id,
                node_id,
                loopback_output_node_id,
            } => {
                let channel_update = self
                    .state
                    .channels
                    .iter_mut()
                    .find(|c| c.id == channel_id)
                    .map(|channel| {
                        channel.pw_sink_id = Some(node_id);
                        channel.pw_loopback_output_id = loopback_output_node_id;
                        info!(
                            "Virtual sink created for channel '{}': sink={}, loopback={:?}",
                            channel.name, node_id, loopback_output_node_id
                        );
                        (
                            channel.volume_linear(),
                            channel.muted,
                            loopback_output_node_id,
                        )
                    });

                if let Some((volume, muted, Some(loopback_id))) = channel_update {
                    self.send_pw_command(PwCommand::SetVolume {
                        node_id: loopback_id,
                        volume,
                    });
                    if muted {
                        self.send_pw_command(PwCommand::SetMute {
                            node_id: loopback_id,
                            muted: true,
                        });
                    }
                    // Route loopback output to master output device
                    // Look up the master output device ID by name
                    let target_device_id = self.get_master_output_device_id();
                    self.send_pw_command(PwCommand::RouteChannelToDevice {
                        loopback_output_node: loopback_id,
                        target_device_id,
                    });
                }

                // Mark channel for pending auto-routing (ports may not be ready yet)
                self.state.pending_auto_route_channels.insert(channel_id);

                // Try immediately in case ports are already available
                self.try_auto_route_pending_apps(channel_id);
            }
            PwEvent::VirtualSinkDestroyed { node_id } => {
                for channel in &mut self.state.channels {
                    if channel.pw_sink_id == Some(node_id) {
                        channel.pw_sink_id = None;
                        channel.pw_loopback_output_id = None;
                    }
                }
            }
            PwEvent::RecordingSourceCreated { name, node_id } => {
                info!("Recording source created: {} (node {})", name, node_id);
                self.state.master_recording_source_id = Some(node_id);
            }
            PwEvent::RecordingSourceDestroyed { node_id } => {
                if self.state.master_recording_source_id == Some(node_id) {
                    self.state.master_recording_source_id = None;
                    self.state.master_recording_enabled = false;
                }
            }
            PwEvent::Error(msg) => {
                error!("PipeWire error: {}", msg);
            }
        }
    }

    pub fn send_pw_command(&self, cmd: PwCommand) {
        if let Some(ref pw) = self.pw_thread {
            if let Err(e) = pw.send(cmd) {
                error!("Failed to send PW command: {}", e);
            }
        }
    }

    pub fn shutdown(&mut self) {
        info!("Shutting down daemon service");
        self.save_config();
        crate::audio::virtual_sink::destroy_all_virtual_sinks();
        if let Some(pw) = self.pw_thread.take() {
            pw.shutdown();
        }
    }

    pub fn save_config(&self) {
        let config = MixerConfig {
            master: crate::config::MasterConfig {
                volume_db: self.state.master_volume_db,
                muted: self.state.master_muted,
                output_device: self.state.master_output.clone(),
            },
            channels: self
                .state
                .channels
                .iter()
                .map(|c| SavedChannel {
                    id: c.id,
                    name: c.name.clone(),
                    is_managed: c.is_managed,
                    sink_name: c.sink_name.clone(),
                    volume_db: c.volume_db,
                    muted: c.muted,
                    eq_enabled: c.eq_enabled,
                    eq_preset: c.eq_preset.clone(),
                    assigned_apps: c.assigned_apps.clone(),
                    plugin_chain: Vec::new(),
                    output_device_name: c.output_device_name.clone(),
                })
                .collect(),
        };

        if let Err(e) = self.config_manager.save_mixer_config(&config) {
            error!("Failed to save config: {}", e);
        }
    }

    // ==================== Public API methods ====================

    pub fn create_channel(&mut self, name: &str) -> Result<String, ServiceError> {
        let mut channel = ChannelState::new(name.to_string());
        channel.sink_name = Some(format!("sootmix.{}", name));
        let id = channel.id;

        self.state.channels.push(channel);

        self.send_pw_command(PwCommand::CreateVirtualSink {
            channel_id: id,
            name: name.to_string(),
        });

        self.save_config();
        Ok(id.to_string())
    }

    pub fn delete_channel(&mut self, channel_id: &str) -> Result<(), ServiceError> {
        let id = Uuid::parse_str(channel_id)
            .map_err(|_| ServiceError::ChannelNotFound(channel_id.to_string()))?;

        let (sink_id, is_managed) = self
            .state
            .channels
            .iter()
            .find(|c| c.id == id)
            .map(|c| (c.pw_sink_id, c.is_managed))
            .ok_or_else(|| ServiceError::ChannelNotFound(channel_id.to_string()))?;

        if let Some(sink_id) = sink_id {
            if is_managed {
                self.send_pw_command(PwCommand::DestroyVirtualSink { node_id: sink_id });
            }
        }

        self.state.channels.retain(|c| c.id != id);
        self.save_config();
        Ok(())
    }

    pub fn rename_channel(&mut self, channel_id: &str, name: &str) -> Result<(), ServiceError> {
        let id = Uuid::parse_str(channel_id)
            .map_err(|_| ServiceError::ChannelNotFound(channel_id.to_string()))?;

        let sink_id = {
            let channel = self
                .state
                .channels
                .iter_mut()
                .find(|c| c.id == id)
                .ok_or_else(|| ServiceError::ChannelNotFound(channel_id.to_string()))?;

            let old_name = channel.name.clone();
            channel.name = name.to_string();
            info!("Renamed channel '{}' to '{}'", old_name, name);
            channel.pw_sink_id
        };

        if let Some(sink_id) = sink_id {
            self.send_pw_command(PwCommand::UpdateSinkDescription {
                node_id: sink_id,
                description: name.to_string(),
            });
        }

        self.save_config();
        Ok(())
    }

    pub fn set_channel_volume(
        &mut self,
        channel_id: &str,
        volume_db: f64,
    ) -> Result<(), ServiceError> {
        let id = Uuid::parse_str(channel_id)
            .map_err(|_| ServiceError::ChannelNotFound(channel_id.to_string()))?;

        let (volume, loopback_id) = {
            let channel = self
                .state
                .channels
                .iter_mut()
                .find(|c| c.id == id)
                .ok_or_else(|| ServiceError::ChannelNotFound(channel_id.to_string()))?;

            channel.volume_db = volume_db as f32;
            (channel.volume_linear(), channel.pw_loopback_output_id)
        };

        if let Some(loopback_id) = loopback_id {
            self.send_pw_command(PwCommand::SetVolume {
                node_id: loopback_id,
                volume,
            });
        }

        Ok(())
    }

    pub fn set_channel_mute(&mut self, channel_id: &str, muted: bool) -> Result<(), ServiceError> {
        let id = Uuid::parse_str(channel_id)
            .map_err(|_| ServiceError::ChannelNotFound(channel_id.to_string()))?;

        let loopback_id = {
            let channel = self
                .state
                .channels
                .iter_mut()
                .find(|c| c.id == id)
                .ok_or_else(|| ServiceError::ChannelNotFound(channel_id.to_string()))?;

            channel.muted = muted;
            channel.pw_loopback_output_id
        };

        if let Some(loopback_id) = loopback_id {
            self.send_pw_command(PwCommand::SetMute {
                node_id: loopback_id,
                muted,
            });
        }

        self.save_config();
        Ok(())
    }

    pub fn set_master_volume(&mut self, volume_db: f64) -> Result<(), ServiceError> {
        self.state.master_volume_db = volume_db as f32;

        if let Some(node_id) = self.get_master_output_device_id() {
            self.send_pw_command(PwCommand::SetVolume {
                node_id,
                volume: db_to_linear(volume_db as f32),
            });
        }

        self.save_config();
        Ok(())
    }

    pub fn set_master_mute(&mut self, muted: bool) -> Result<(), ServiceError> {
        self.state.master_muted = muted;

        if let Some(node_id) = self.get_master_output_device_id() {
            self.send_pw_command(PwCommand::SetMute {
                node_id,
                muted,
            });
        }

        self.save_config();
        Ok(())
    }

    pub fn assign_app(&mut self, app_id: &str, channel_id: &str) -> Result<(), ServiceError> {
        let channel_uuid = Uuid::parse_str(channel_id)
            .map_err(|_| ServiceError::ChannelNotFound(channel_id.to_string()))?;

        let app_node_id: u32 = app_id
            .parse()
            .map_err(|_| ServiceError::AppNotFound(app_id.to_string()))?;

        let app_identifier = self
            .state
            .apps
            .iter()
            .find(|a| a.node_id == app_node_id)
            .ok_or_else(|| ServiceError::AppNotFound(app_id.to_string()))?
            .identifier()
            .to_string();

        let sink_node_id = self
            .state
            .channels
            .iter()
            .find(|c| c.id == channel_uuid)
            .ok_or_else(|| ServiceError::ChannelNotFound(channel_id.to_string()))?
            .pw_sink_id
            .ok_or_else(|| ServiceError::ChannelNotFound("No sink for channel".to_string()))?;

        // Set the stream's target to our sink - this tells WirePlumber to stop
        // auto-managing this stream and prevents it from recreating links to default sink
        if let Err(e) = crate::audio::routing::set_stream_target(app_node_id, sink_node_id) {
            warn!("Failed to set stream target: {}", e);
        }

        let our_sinks: Vec<u32> = self
            .state
            .channels
            .iter()
            .filter_map(|c| c.pw_sink_id)
            .collect();

        // Destroy links to non-sootmix sinks FIRST
        let links_to_destroy: Vec<u32> = self
            .state
            .pw_graph
            .links_from_node(app_node_id)
            .iter()
            .filter(|link| !our_sinks.contains(&link.input_node))
            .map(|l| l.id)
            .collect();

        for link_id in links_to_destroy {
            self.send_pw_command(PwCommand::DestroyLink { link_id });
        }

        // Then create links to our sink
        let port_pairs = self
            .state
            .pw_graph
            .find_port_pairs(app_node_id, sink_node_id);
        for (output_port, input_port) in port_pairs {
            self.send_pw_command(PwCommand::CreateLink {
                output_port,
                input_port,
            });
        }

        // Add to assigned apps list
        if let Some(channel) = self
            .state
            .channels
            .iter_mut()
            .find(|c| c.id == channel_uuid)
        {
            if !channel.assigned_apps.contains(&app_identifier) {
                channel.assigned_apps.push(app_identifier);
            }
        }

        self.save_config();
        Ok(())
    }

    pub fn unassign_app(&mut self, app_id: &str, channel_id: &str) -> Result<(), ServiceError> {
        let channel_uuid = Uuid::parse_str(channel_id)
            .map_err(|_| ServiceError::ChannelNotFound(channel_id.to_string()))?;

        let app_node_id: u32 = app_id
            .parse()
            .map_err(|_| ServiceError::AppNotFound(app_id.to_string()))?;

        let app_identifier = self
            .state
            .apps
            .iter()
            .find(|a| a.node_id == app_node_id)
            .ok_or_else(|| ServiceError::AppNotFound(app_id.to_string()))?
            .identifier()
            .to_string();

        let sink_node_id = self
            .state
            .channels
            .iter()
            .find(|c| c.id == channel_uuid)
            .map(|c| c.pw_sink_id)
            .ok_or_else(|| ServiceError::ChannelNotFound(channel_id.to_string()))?;

        // Clear the stream's target so WirePlumber can manage it again
        if let Err(e) = crate::audio::routing::clear_stream_target(app_node_id) {
            warn!("Failed to clear stream target: {}", e);
        }

        // Find hardware sink to reconnect to
        let outputs = self.state.get_outputs();
        if let Some(default) = outputs.first() {
            let port_pairs = self
                .state
                .pw_graph
                .find_port_pairs(app_node_id, default.node_id);
            for (output_port, input_port) in port_pairs {
                self.send_pw_command(PwCommand::CreateLink {
                    output_port,
                    input_port,
                });
            }
        }

        // Destroy links to our sink
        if let Some(sink_id) = sink_node_id {
            let links_to_destroy: Vec<u32> = self
                .state
                .pw_graph
                .links
                .values()
                .filter(|l| l.output_node == app_node_id && l.input_node == sink_id)
                .map(|l| l.id)
                .collect();

            for link_id in links_to_destroy {
                self.send_pw_command(PwCommand::DestroyLink { link_id });
            }
        }

        // Remove from assigned apps list
        if let Some(channel) = self
            .state
            .channels
            .iter_mut()
            .find(|c| c.id == channel_uuid)
        {
            channel.assigned_apps.retain(|a| a != &app_identifier);
        }

        self.save_config();
        Ok(())
    }

    pub fn set_channel_output(
        &mut self,
        channel_id: &str,
        device_name: &str,
    ) -> Result<(), ServiceError> {
        let channel_uuid = Uuid::parse_str(channel_id)
            .map_err(|_| ServiceError::ChannelNotFound(channel_id.to_string()))?;

        let device_name_opt = if device_name.is_empty() {
            None
        } else {
            Some(device_name.to_string())
        };

        let outputs = self.state.get_outputs();
        let target_device_id = device_name_opt.as_ref().and_then(|name| {
            outputs
                .iter()
                .find(|d| d.description == *name || d.name == *name)
                .map(|d| d.node_id)
        });

        let loopback_id = {
            let channel = self
                .state
                .channels
                .iter_mut()
                .find(|c| c.id == channel_uuid)
                .ok_or_else(|| ServiceError::ChannelNotFound(channel_id.to_string()))?;

            channel.output_device_name = device_name_opt;
            channel.pw_loopback_output_id
        };

        if let Some(loopback_id) = loopback_id {
            self.send_pw_command(PwCommand::RouteChannelToDevice {
                loopback_output_node: loopback_id,
                target_device_id,
            });
        }

        self.save_config();
        Ok(())
    }

    pub fn set_master_output(&mut self, device_name: &str) -> Result<(), ServiceError> {
        self.state.master_output = if device_name.is_empty() {
            None
        } else {
            Some(device_name.to_string())
        };

        let is_system_default = device_name == Self::SYSTEM_DEFAULT_SENTINEL;
        let target_device_id = self.get_master_output_device_id();

        if !is_system_default {
            if let Some(node_id) = target_device_id {
                // Set as system default only when selecting a specific device
                self.send_pw_command(PwCommand::SetDefaultSink { node_id });
            }
        }

        // Re-route all existing channels to the new master output
        let loopback_ids: Vec<u32> = self
            .state
            .channels
            .iter()
            .filter(|c| c.output_device_name.is_none())
            .filter_map(|c| c.pw_loopback_output_id)
            .collect();

        for loopback_id in loopback_ids {
            self.send_pw_command(PwCommand::RouteChannelToDevice {
                loopback_output_node: loopback_id,
                target_device_id,
            });
        }

        self.save_config();
        Ok(())
    }

    /// Sentinel value for "follow the system default sink".
    const SYSTEM_DEFAULT_SENTINEL: &'static str = "system-default";

    /// Get the node ID of the master output device.
    ///
    /// Checks the configured master output name first, then falls back to the
    /// WirePlumber system default sink. This ensures channels always route to
    /// a valid hardware device even if the configured device isn't available.
    fn get_master_output_device_id(&self) -> Option<u32> {
        // Try the configured master output first
        if let Some(name) = self.state.master_output.as_ref() {
            // "system-default" means always follow WirePlumber's default
            if name == Self::SYSTEM_DEFAULT_SENTINEL {
                return crate::audio::routing::get_default_sink_id();
            }

            let outputs = self.state.get_outputs();
            if let Some(output) = outputs
                .iter()
                .find(|o| o.description == *name || o.name == *name)
            {
                return Some(output.node_id);
            }
            debug!(
                "Configured master output '{}' not found, falling back to system default",
                name
            );
        }

        // Fall back to WirePlumber default sink
        crate::audio::routing::get_default_sink_id()
    }

    pub fn set_master_recording(&mut self, enabled: bool) -> Result<(), ServiceError> {
        if enabled {
            if !self.state.master_recording_enabled {
                self.send_pw_command(PwCommand::CreateRecordingSource {
                    name: "master".to_string(),
                });
                self.state.master_recording_enabled = true;
            }
        } else if let Some(node_id) = self.state.master_recording_source_id {
            self.send_pw_command(PwCommand::DestroyRecordingSource { node_id });
            self.state.master_recording_enabled = false;
            self.state.master_recording_source_id = None;
        }
        Ok(())
    }

    /// Try to auto-route an app to a channel if it matches a saved assignment.
    /// Called when a new node appears in PipeWire.
    fn try_auto_route_app(&mut self, node_id: u32, _node_name: &str) {
        // Find the app by node_id
        let (app_identifier, app_name) = match self.state.apps.iter().find(|a| a.node_id == node_id)
        {
            Some(app) => (app.identifier().to_string(), app.name.clone()),
            None => {
                debug!("try_auto_route_app: node {} not in apps list", node_id);
                return;
            }
        };

        // Find a channel that has this app in its assigned_apps
        // Check both identifier (binary name) and display name for compatibility
        let channel_match = self
            .state
            .channels
            .iter()
            .find(|c| {
                c.assigned_apps.contains(&app_identifier)
                    || c.assigned_apps.iter().any(|a| a == &app_name)
            })
            .map(|c| (c.id, c.pw_sink_id, c.name.clone()));

        let (channel_id, sink_id, channel_name) = match channel_match {
            Some((id, Some(sink_id), name)) => (id, sink_id, name),
            Some((_id, None, name)) => {
                debug!(
                    "try_auto_route_app: app '{}' matches channel '{}' but sink not ready yet",
                    app_identifier, name
                );
                return;
            }
            _ => {
                debug!(
                    "try_auto_route_app: app '{}' has no matching channel assignment",
                    app_identifier
                );
                return;
            }
        };
        let _ = channel_name; // suppress warning

        info!(
            "Auto-routing app '{}' (node {}) to channel {}",
            app_identifier, node_id, channel_id
        );

        // Set the stream's target to our sink - this tells WirePlumber to stop
        // auto-managing this stream and prevents it from recreating links to default sink
        if let Err(e) = crate::audio::routing::set_stream_target(node_id, sink_id) {
            warn!("Failed to set stream target for auto-route: {}", e);
        }

        let our_sinks: Vec<u32> = self
            .state
            .channels
            .iter()
            .filter_map(|c| c.pw_sink_id)
            .collect();

        // Destroy links to non-sootmix sinks FIRST
        let links_to_destroy: Vec<u32> = self
            .state
            .pw_graph
            .links_from_node(node_id)
            .iter()
            .filter(|link| !our_sinks.contains(&link.input_node))
            .map(|l| l.id)
            .collect();

        for link_id in links_to_destroy {
            self.send_pw_command(PwCommand::DestroyLink { link_id });
        }

        // Then create links to our sink
        let port_pairs = self.state.pw_graph.find_port_pairs(node_id, sink_id);
        for (output_port, input_port) in port_pairs {
            self.send_pw_command(PwCommand::CreateLink {
                output_port,
                input_port,
            });
        }
    }

    /// Try to auto-route all apps that match a channel's assigned_apps.
    /// Called when a virtual sink is created (sink is now ready to receive apps).
    fn try_auto_route_pending_apps(&mut self, channel_id: Uuid) {
        // Get channel info
        let (assigned_apps, sink_id) = match self
            .state
            .channels
            .iter()
            .find(|c| c.id == channel_id)
            .map(|c| (c.assigned_apps.clone(), c.pw_sink_id))
        {
            Some((apps, Some(sink_id))) => (apps, sink_id),
            _ => return,
        };

        if assigned_apps.is_empty() {
            return;
        }

        // Find apps that match the assigned list
        // Check both identifier (binary name) and display name for compatibility
        let apps_to_route: Vec<(u32, String)> = self
            .state
            .apps
            .iter()
            .filter(|app| {
                let id = app.identifier().to_string();
                let name = &app.name;
                assigned_apps.contains(&id) || assigned_apps.iter().any(|a| a == name)
            })
            .map(|app| (app.node_id, app.name.clone()))
            .collect();

        // Collect our sink IDs for filtering
        let our_sinks: Vec<u32> = self
            .state
            .channels
            .iter()
            .filter_map(|c| c.pw_sink_id)
            .collect();

        for (app_node_id, app_identifier) in apps_to_route {
            let port_pairs = self.state.pw_graph.find_port_pairs(app_node_id, sink_id);

            if port_pairs.is_empty() {
                debug!(
                    "No port pairs found for {}→{}, will retry later",
                    app_node_id, sink_id
                );
                continue;
            }

            info!("Auto-routing app '{}' to channel", app_identifier);

            // Set the stream's target to our sink - prevents WirePlumber from recreating links
            if let Err(e) = crate::audio::routing::set_stream_target(app_node_id, sink_id) {
                warn!("Failed to set stream target: {}", e);
            }

            // Destroy links to non-sootmix sinks FIRST
            let links_to_destroy: Vec<u32> = self
                .state
                .pw_graph
                .links_from_node(app_node_id)
                .iter()
                .filter(|link| !our_sinks.contains(&link.input_node))
                .map(|l| l.id)
                .collect();

            for link_id in links_to_destroy {
                self.send_pw_command(PwCommand::DestroyLink { link_id });
            }

            // Then create links to our sink
            for (output_port, input_port) in port_pairs {
                self.send_pw_command(PwCommand::CreateLink {
                    output_port,
                    input_port,
                });
            }
        }

        // Remove from pending set (we've tried, links are created as ports become available)
        self.state.pending_auto_route_channels.remove(&channel_id);
    }

    /// Re-route orphaned channel loopback outputs to the fallback device.
    ///
    /// Called when a hardware sink is removed (BT disconnect, USB unplug).
    /// Finds channels whose loopback output has no remaining links and
    /// routes them to the configured master output or system default.
    fn try_fallback_orphaned_channels(&mut self) {
        let target_device_id = self.get_master_output_device_id();

        let orphaned_loopbacks: Vec<(u32, String)> = self
            .state
            .channels
            .iter()
            .filter_map(|c| {
                let loopback_id = c.pw_loopback_output_id?;
                let has_links = self
                    .state
                    .pw_graph
                    .links
                    .values()
                    .any(|l| l.output_node == loopback_id);
                if !has_links {
                    Some((loopback_id, c.name.clone()))
                } else {
                    None
                }
            })
            .collect();

        if orphaned_loopbacks.is_empty() {
            return;
        }

        for (loopback_id, channel_name) in orphaned_loopbacks {
            info!(
                "Fallback: re-routing orphaned channel '{}' (loopback {}) to device {:?}",
                channel_name, loopback_id, target_device_id
            );
            self.send_pw_command(PwCommand::RouteChannelToDevice {
                loopback_output_node: loopback_id,
                target_device_id,
            });
        }
    }

    /// Re-route channel loopback outputs when a hardware sink (re)appears.
    ///
    /// Handles Bluetooth reconnects, USB audio plugged in, etc.
    ///
    /// Strategy:
    /// 1. If a channel has a per-channel output matching this device, re-route it
    /// 2. If the master output matches this device, re-route channels using master
    /// 3. For any channel whose loopback output has no links (orphaned), re-route
    ///    to the best available target (configured master or system default)
    fn try_reroute_channels_to_device(
        &mut self,
        device_node_id: u32,
        device_name: &str,
        device_desc: &str,
    ) {
        let matches_device = |configured_name: &str| -> bool {
            configured_name == device_name || configured_name == device_desc
        };

        // Check if this device matches the master output.
        // "system-default" matches ANY hardware sink (it dynamically follows the default).
        let master_is_system_default = self
            .state
            .master_output
            .as_ref()
            .map(|name| name == Self::SYSTEM_DEFAULT_SENTINEL)
            .unwrap_or(false);
        let master_matches = self
            .state
            .master_output
            .as_ref()
            .map(|name| name == Self::SYSTEM_DEFAULT_SENTINEL || matches_device(name))
            .unwrap_or(false);

        // Collect loopback IDs that need re-routing
        info!(
            "try_reroute_channels_to_device: device_name='{}', device_desc='{}', master_output={:?}, master_matches={}",
            device_name, device_desc, self.state.master_output, master_matches
        );

        let mut loopbacks_to_route: Vec<(u32, String, Option<u32>)> = Vec::new();

        for c in &self.state.channels {
            let loopback_id = match c.pw_loopback_output_id {
                Some(id) => id,
                None => {
                    debug!(
                        "Channel '{}': no loopback output ID, skipping",
                        c.name
                    );
                    continue;
                }
            };

            // Per-channel output takes priority
            if let Some(ref dev_name) = c.output_device_name {
                if matches_device(dev_name) {
                    loopbacks_to_route.push((
                        loopback_id,
                        c.name.clone(),
                        Some(device_node_id),
                    ));
                    continue;
                }
            }

            // Master output matches this device (or master is "system-default")
            if master_matches && c.output_device_name.is_none() {
                // When master is "system-default", route to whatever the
                // current system default is, not necessarily this new device
                let target = if master_is_system_default {
                    None // resolved later via get_master_output_device_id()
                } else {
                    Some(device_node_id)
                };
                loopbacks_to_route.push((loopback_id, c.name.clone(), target));
                continue;
            }

            // Check if this channel's loopback output has no links (orphaned).
            // This catches the case where no master output is configured, or
            // the device was disconnected and links were destroyed.
            if c.output_device_name.is_none() {
                let has_links = self
                    .state
                    .pw_graph
                    .links
                    .values()
                    .any(|l| l.output_node == loopback_id);

                debug!(
                    "Channel '{}': loopback_id={}, has_links={}, output_device=None",
                    c.name, loopback_id, has_links
                );

                if !has_links {
                    loopbacks_to_route.push((loopback_id, c.name.clone(), None));
                }
            }
        }

        if loopbacks_to_route.is_empty() {
            info!("try_reroute_channels_to_device: no channels need re-routing");
        }

        for (loopback_id, channel_name, target) in loopbacks_to_route {
            let target_id = target.or_else(|| self.get_master_output_device_id());
            info!(
                "Re-routing channel '{}' loopback output to device node {:?} (trigger: '{}' node {})",
                channel_name, target_id, device_desc, device_node_id
            );
            self.send_pw_command(PwCommand::RouteChannelToDevice {
                loopback_output_node: loopback_id,
                target_device_id: target_id,
            });
        }
    }
}
