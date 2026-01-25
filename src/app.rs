// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Iced Application implementation for SootMix.

use crate::audio::{filter_chain, MeterManager, PluginFilterManager, PluginProcessorManager, PwCommand, PwEvent, PwThread};
use crate::config::eq_preset::EqPreset;
use crate::config::{ConfigManager, MixerConfig, SavedChannel};
use crate::message::Message;
use crate::plugins::{PluginFilter, PluginManager, PluginSlotConfig, PluginType};
use crate::state::{db_to_linear, AppState, EditingRule, MixerChannel, SnapshotSlot};
use crate::ui::apps_panel::apps_panel;
use crate::ui::channel_strip::{channel_strip, master_strip};
use crate::ui::routing_rules_panel::routing_rules_panel;
use crate::ui::theme::{self, *};
use iced::widget::{button, column, container, row, scrollable, text, Space};
use iced::{Alignment, Background, Element, Fill, Subscription, Task, Theme};
use std::sync::mpsc;
use std::time::Instant;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Main application state.
pub struct SootMix {
    /// Application state.
    state: AppState,
    /// PipeWire thread handle.
    pw_thread: Option<PwThread>,
    /// Receiver for PipeWire events.
    pw_event_rx: Option<mpsc::Receiver<PwEvent>>,
    /// Configuration manager for persistence.
    config_manager: Option<ConfigManager>,
    /// Startup timestamp for discovery delay.
    startup_time: Instant,
    /// Pending config to restore after discovery.
    pending_config: Option<MixerConfig>,
    /// VU meter manager for audio level display.
    meter_manager: MeterManager,
    /// Last tick time for delta time calculation.
    last_tick: Instant,
    /// Plugin manager for loading and managing audio effect plugins.
    plugin_manager: PluginManager,
    /// Plugin audio processor for routing audio through plugin chains.
    plugin_processor: PluginProcessorManager,
    /// Plugin filter manager for PipeWire audio routing through plugins.
    plugin_filter_manager: PluginFilterManager,
}

impl SootMix {
    /// Create a new application instance.
    pub fn new() -> (Self, Task<Message>) {
        let mut state = AppState::new();

        // Initialize config manager and load saved config
        let config_manager = ConfigManager::new().ok();
        let pending_config = config_manager.as_ref().and_then(|cm| {
            match cm.load_mixer_config() {
                Ok(config) => {
                    info!("Loaded mixer config: {} channels", config.channels.len());
                    Some(config)
                }
                Err(e) => {
                    debug!("No saved config or error loading: {}", e);
                    None
                }
            }
        });

        // Load routing rules
        if let Some(ref cm) = config_manager {
            match cm.load_routing_rules() {
                Ok(rules) => {
                    info!("Loaded {} routing rules", rules.rules.len());
                    state.routing_rules = rules;
                }
                Err(e) => {
                    debug!("No routing rules or error loading: {}", e);
                }
            }
        }

        // Create channel for PipeWire events
        let (event_tx, event_rx) = mpsc::channel();

        // Spawn PipeWire thread
        let pw_thread = match PwThread::spawn(event_tx) {
            Ok(thread) => {
                info!("PipeWire thread started");
                Some(thread)
            }
            Err(e) => {
                error!("Failed to start PipeWire thread: {}", e);
                None
            }
        };

        // Initialize plugin manager and scan for plugins
        let plugin_manager = PluginManager::new();
        let plugin_count = plugin_manager.scan();
        info!("Plugin scan complete: {} plugins found", plugin_count);

        // Initialize plugin filter manager with shared instances
        let mut plugin_filter_manager = PluginFilterManager::new();
        plugin_filter_manager.set_plugin_instances(plugin_manager.shared_instances());

        let now = Instant::now();
        let app = Self {
            state,
            pw_thread,
            pw_event_rx: Some(event_rx),
            config_manager,
            startup_time: now,
            pending_config,
            meter_manager: MeterManager::new(),
            last_tick: now,
            plugin_manager,
            plugin_processor: PluginProcessorManager::new(),
            plugin_filter_manager,
        };

        (app, Task::none())
    }

    /// Application title.
    pub fn title(&self) -> String {
        "SootMix".to_string()
    }

    /// Get plugin chain info for a channel (for UI display).
    /// Returns Vec of (instance_id, plugin_name, bypassed).
    fn get_plugin_chain_info(&self, channel_id: Uuid) -> Vec<(Uuid, String, bool)> {
        let channel = match self.state.channel(channel_id) {
            Some(c) => c,
            None => return Vec::new(),
        };

        channel
            .plugin_instances
            .iter()
            .enumerate()
            .map(|(idx, &instance_id)| {
                // Get plugin name from the manager
                let name = self
                    .plugin_manager
                    .get_info(instance_id)
                    .map(|info| info.name.to_string())
                    .unwrap_or_else(|| {
                        // Fallback to config name
                        channel
                            .plugin_chain
                            .get(idx)
                            .map(|c| c.plugin_id.clone())
                            .unwrap_or_else(|| "Unknown".to_string())
                    });

                // Get bypass state from config
                let bypassed = channel
                    .plugin_chain
                    .get(idx)
                    .map(|c| c.bypassed)
                    .unwrap_or(false);

                (instance_id, name, bypassed)
            })
            .collect()
    }

    /// Get plugin editor info (plugin name and parameters).
    /// Returns (plugin_name, Vec<PluginEditorParam>).
    fn get_plugin_editor_info(&self, instance_id: Uuid) -> Option<(String, Vec<crate::ui::plugin_chain::PluginEditorParam>)> {
        let info = self.plugin_manager.get_info(instance_id)?;
        let plugin_name = info.name.to_string();

        let param_count = self.plugin_manager.get_parameter_count(instance_id)?;
        let params: Vec<crate::ui::plugin_chain::PluginEditorParam> = (0..param_count)
            .filter_map(|idx| {
                let param_info = self.plugin_manager.get_parameter_info(instance_id, idx)?;
                let value = self.plugin_manager.get_parameter(instance_id, idx)?;
                Some(crate::ui::plugin_chain::PluginEditorParam {
                    index: idx,
                    name: param_info.name.to_string(),
                    unit: param_info.unit.to_string(),
                    min: param_info.min,
                    max: param_info.max,
                    value,
                })
            })
            .collect();

        Some((plugin_name, params))
    }

    /// Handle messages.
    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            // ==================== Channel Actions ====================
            Message::ChannelVolumeChanged(id, volume) => {
                if let Some(channel) = self.state.channel_mut(id) {
                    channel.volume_db = volume;
                    if let Some(node_id) = channel.pw_sink_id {
                        let linear_vol = channel.volume_linear();
                        debug!(
                            "Volume change: channel={}, node_id={}, db={:.1}, linear={:.3}",
                            channel.name, node_id, volume, linear_vol
                        );
                        self.send_pw_command(PwCommand::SetVolume {
                            node_id,
                            volume: linear_vol,
                        });
                    }
                }
            }
            Message::ChannelVolumeReleased(_id) => {
                // Volume changes don't auto-save to snapshot - user must click the active slot to save
            }
            Message::ChannelMuteToggled(id) => {
                let cmd = if let Some(channel) = self.state.channel_mut(id) {
                    channel.muted = !channel.muted;
                    channel.pw_sink_id.map(|node_id| PwCommand::SetMute {
                        node_id,
                        muted: channel.muted,
                    })
                } else {
                    None
                };
                if let Some(cmd) = cmd {
                    self.send_pw_command(cmd);
                }
            }
            Message::ChannelEqToggled(id) => {
                // Get channel info before mutating
                let channel_info = self.state.channel(id).map(|c| {
                    (c.name.clone(), c.eq_enabled, c.pw_eq_node_id)
                });

                if let Some((name, was_enabled, existing_eq_node)) = channel_info {
                    // Sanitize name for node naming
                    let safe_name: String = name
                        .chars()
                        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                        .collect();

                    // Node names used for routing
                    let loopback_output_name = format!("sootmix.{}.output", safe_name);
                    let eq_sink_name = format!("sootmix.eq.{}", safe_name);
                    let eq_output_name = format!("sootmix.eq.{}.output", safe_name);

                    // Find the master sink for routing
                    let master_sink_name = filter_chain::find_default_sink_name()
                        .unwrap_or_else(|| "alsa_output.pci-0000_00_1f.3.analog-stereo".to_string());

                    if was_enabled {
                        // Disable EQ - unroute and destroy the filter chain
                        if existing_eq_node.is_some() {
                            info!("Disabling EQ for channel '{}'", name);

                            // Remove routing through EQ, connect loopback directly to master
                            filter_chain::unroute_eq(
                                &loopback_output_name,
                                &eq_sink_name,
                                &eq_output_name,
                                &master_sink_name,
                            ).ok();

                            // Destroy the EQ filter
                            filter_chain::destroy_eq_filter(id).ok();
                        }
                        if let Some(channel) = self.state.channel_mut(id) {
                            channel.eq_enabled = false;
                            channel.pw_eq_node_id = None;
                        }
                    } else {
                        // Enable EQ - create the filter chain and route audio through it
                        info!("Enabling EQ for channel '{}'", name);
                        let preset = EqPreset::flat();
                        match filter_chain::create_eq_filter(id, &name, &preset) {
                            Ok((sink_node_id, output_node_id)) => {
                                info!(
                                    "Created EQ filter for '{}': sink={}, output={}",
                                    name, sink_node_id, output_node_id
                                );

                                // Route audio through EQ: loopback -> EQ -> master
                                if let Err(e) = filter_chain::route_through_eq(
                                    &loopback_output_name,
                                    &eq_sink_name,
                                    &eq_output_name,
                                    &master_sink_name,
                                ) {
                                    warn!("Failed to route through EQ: {}", e);
                                }

                                if let Some(channel) = self.state.channel_mut(id) {
                                    channel.eq_enabled = true;
                                    channel.pw_eq_node_id = Some(sink_node_id);
                                }
                            }
                            Err(e) => {
                                error!("Failed to create EQ filter for '{}': {}", name, e);
                            }
                        }
                    }
                }
            }
            Message::StartEditingChannelName(id) => {
                if let Some(channel) = self.state.channel(id) {
                    self.state.editing_channel = Some((id, channel.name.clone()));
                }
            }
            Message::ChannelNameEditChanged(new_value) => {
                if let Some((id, ref mut value)) = self.state.editing_channel {
                    *value = new_value;
                }
            }
            Message::CancelEditingChannelName => {
                self.state.editing_channel = None;
            }
            Message::ChannelRenamed(id, new_name) => {
                let new_name = new_name.trim().to_string();
                if !new_name.is_empty() {
                    // Get current channel info
                    let channel_info = self.state.channel(id).map(|c| {
                        (c.pw_sink_id, c.assigned_apps.clone(), c.name.clone())
                    });

                    if let Some((old_sink_id, assigned_apps, old_name)) = channel_info {
                        // Only do full rename if name actually changed
                        if new_name != old_name {
                            // Collect app node IDs for re-routing later
                            let app_node_ids: Vec<u32> = assigned_apps.iter()
                                .filter_map(|app_id| {
                                    self.state.available_apps.iter()
                                        .find(|a| a.identifier() == *app_id)
                                        .map(|a| a.node_id)
                                })
                                .collect();

                            // Find hardware sink for temporary routing
                            let our_sink_ids: Vec<u32> = self.state.channels.iter()
                                .filter_map(|c| c.pw_sink_id)
                                .collect();
                            let default_sink_id = find_hardware_sink(&self.state.pw_graph, &our_sink_ids);

                            // Temporarily route apps to default sink
                            if let Some(old_sink) = old_sink_id {
                                for &app_node_id in &app_node_ids {
                                    // Connect to default first
                                    if let Some(default_id) = default_sink_id {
                                        let port_pairs = self.state.pw_graph.find_port_pairs(app_node_id, default_id);
                                        for (output_port, input_port) in port_pairs {
                                            self.send_pw_command(PwCommand::CreateLink { output_port, input_port });
                                        }
                                    }
                                    // Then disconnect from old sink
                                    let links_to_destroy: Vec<u32> = self.state.pw_graph.links
                                        .values()
                                        .filter(|l| l.output_node == app_node_id && l.input_node == old_sink)
                                        .map(|l| l.id)
                                        .collect();
                                    for link_id in links_to_destroy {
                                        self.send_pw_command(PwCommand::DestroyLink { link_id });
                                    }
                                }

                                // Destroy old sink
                                self.send_pw_command(PwCommand::DestroyVirtualSink { node_id: old_sink });
                            }

                            // Store apps for re-routing when new sink is created
                            if !app_node_ids.is_empty() {
                                self.state.pending_reroute = Some((id, app_node_ids));
                            }

                            // Create new sink with new name
                            self.send_pw_command(PwCommand::CreateVirtualSink {
                                channel_id: id,
                                name: new_name.clone(),
                            });
                        }

                        // Update UI name
                        if let Some(channel) = self.state.channel_mut(id) {
                            channel.name = new_name;
                        }
                    }
                }
                self.state.editing_channel = None;
            }
            Message::ChannelDeleted(id) => {
                // Get channel info before removing
                let channel_info = self.state.channel(id).map(|c| {
                    (c.pw_sink_id, c.assigned_apps.clone(), c.is_managed, c.pw_eq_node_id)
                });

                if let Some((sink_node_id, assigned_apps, is_managed, eq_node_id)) = channel_info {
                    // Destroy EQ filter if it exists
                    if eq_node_id.is_some() {
                        filter_chain::destroy_eq_filter(id).ok();
                    }
                    // Find hardware output sink (not virtual sinks)
                    let our_sink_ids: Vec<u32> = self.state.channels.iter()
                        .filter_map(|c| c.pw_sink_id)
                        .collect();
                    let default_sink_id = find_hardware_sink(&self.state.pw_graph, &our_sink_ids);

                    // Collect all link destruction commands for after we create new links
                    let mut links_to_destroy: Vec<u32> = Vec::new();

                    for app_id in assigned_apps {
                        // Find app node ID
                        if let Some(app) = self.state.available_apps.iter().find(|a| a.identifier() == app_id) {
                            let app_node_id = app.node_id;

                            // First create links to default sink
                            if let Some(default_id) = default_sink_id {
                                let port_pairs = self.state.pw_graph.find_port_pairs(app_node_id, default_id);
                                for (output_port, input_port) in port_pairs {
                                    self.send_pw_command(PwCommand::CreateLink { output_port, input_port });
                                }
                            }

                            // Collect links to destroy (to our sink)
                            if let Some(sink_id) = sink_node_id {
                                let app_links: Vec<u32> = self.state.pw_graph.links
                                    .values()
                                    .filter(|l| l.output_node == app_node_id && l.input_node == sink_id)
                                    .map(|l| l.id)
                                    .collect();
                                links_to_destroy.extend(app_links);
                            }
                        }
                    }

                    // Wait for new links to be established
                    std::thread::sleep(std::time::Duration::from_millis(100));

                    // Now destroy old links explicitly
                    for link_id in links_to_destroy {
                        self.send_pw_command(PwCommand::DestroyLink { link_id });
                    }

                    // Only destroy the sink if it's managed (SootMix-created)
                    // Adopted sinks are kept alive - we just stop controlling them
                    if is_managed {
                        // Wait a bit more before destroying sink
                        std::thread::sleep(std::time::Duration::from_millis(50));

                        if let Some(node_id) = sink_node_id {
                            self.send_pw_command(PwCommand::DestroyVirtualSink { node_id });
                        }
                    }
                }
                self.state.channels.retain(|c| c.id != id);
                self.save_config();
            }
            Message::NewChannelRequested => {
                let channel_num = self.state.channels.len() + 1;
                let channel = MixerChannel::new(format!("Channel {}", channel_num));
                let id = channel.id;
                let name = channel.name.clone();
                self.state.channels.push(channel);

                // Create virtual sink
                self.send_pw_command(PwCommand::CreateVirtualSink {
                    channel_id: id,
                    name,
                });
                self.save_config();
            }

            // ==================== App Drag & Drop ====================
            Message::StartDraggingApp(node_id, app_id) => {
                info!("Started dragging app: {} (node {})", app_id, node_id);
                self.state.dragging_app = Some((node_id, app_id));
            }
            Message::CancelDrag => {
                debug!("Drag cancelled");
                self.state.dragging_app = None;
            }
            Message::DropAppOnChannel(channel_id) => {
                if let Some((app_node_id, app_id)) = self.state.dragging_app.take() {
                    info!("Assigning app {} (node {}) to channel {:?}", app_id, app_node_id, channel_id);

                    // Get the channel's virtual sink node ID
                    let sink_node_id = self.state.channel(channel_id).and_then(|c| c.pw_sink_id);

                    if let Some(sink_node_id) = sink_node_id {
                        // First, disconnect the app from any existing sinks
                        // This ensures audio ONLY goes through our virtual sink
                        let existing_links = self.state.pw_graph.links_from_node(app_node_id);
                        for link in existing_links {
                            // Don't destroy links to our own sinks
                            let is_our_sink = self.state.channels.iter()
                                .any(|c| c.pw_sink_id == Some(link.input_node));
                            if !is_our_sink {
                                info!("Disconnecting app from node {}: destroying link {}", link.input_node, link.id);
                                self.send_pw_command(PwCommand::DestroyLink { link_id: link.id });
                            }
                        }

                        // Show available ports for routing
                        let app_out_ports = self.state.pw_graph.output_ports_for_node(app_node_id);
                        let sink_in_ports = self.state.pw_graph.input_ports_for_node(sink_node_id);
                        info!("App {} has {} output ports: {:?}", app_node_id, app_out_ports.len(),
                            app_out_ports.iter().map(|p| (p.id, &p.name)).collect::<Vec<_>>());
                        info!("Sink {} has {} input ports: {:?}", sink_node_id, sink_in_ports.len(),
                            sink_in_ports.iter().map(|p| (p.id, &p.name)).collect::<Vec<_>>());

                        // Find matching port pairs between app and sink
                        let port_pairs = self.state.pw_graph.find_port_pairs(app_node_id, sink_node_id);

                        if port_pairs.is_empty() {
                            warn!("No matching ports found for app {} -> sink {}", app_node_id, sink_node_id);
                            self.state.last_error = Some("No matching ports found".to_string());
                        } else {
                            // Create links for each port pair
                            for (output_port, input_port) in &port_pairs {
                                info!("Creating link: port {} -> port {}", output_port, input_port);
                                self.send_pw_command(PwCommand::CreateLink {
                                    output_port: *output_port,
                                    input_port: *input_port,
                                });
                            }
                        }

                        // Add app to channel's assigned apps list
                        if let Some(channel) = self.state.channel_mut(channel_id) {
                            if !channel.assigned_apps.contains(&app_id) {
                                channel.assigned_apps.push(app_id);
                            }
                        }
                    } else {
                        warn!("Channel {:?} has no virtual sink yet", channel_id);
                        self.state.last_error = Some("Channel has no sink - try again".to_string());
                        // Put the drag state back so user can try again
                        self.state.dragging_app = Some((app_node_id, app_id));
                    }
                }
            }
            Message::AppAssigned(channel_id, app_id) => {
                if let Some(channel) = self.state.channel_mut(channel_id) {
                    if !channel.assigned_apps.contains(&app_id) {
                        channel.assigned_apps.push(app_id);
                    }
                }
            }
            Message::AppUnassigned(channel_id, app_id) => {
                info!("Unassigning app {} from channel {:?}", app_id, channel_id);

                // Find the app's node ID
                let app_node_id = self.state.available_apps.iter()
                    .find(|a| a.identifier() == app_id)
                    .map(|a| a.node_id);

                // Get the channel's sink node ID
                let sink_node_id = self.state.channel(channel_id).and_then(|c| c.pw_sink_id);

                // Find hardware sink to reconnect to
                let our_sink_ids: Vec<u32> = self.state.channels.iter()
                    .filter_map(|c| c.pw_sink_id)
                    .collect();
                let default_sink_id = find_hardware_sink(&self.state.pw_graph, &our_sink_ids);

                if let (Some(app_node_id), Some(sink_node_id)) = (app_node_id, sink_node_id) {
                    // FIRST: Connect to default sink (before destroying old links)
                    // This ensures there's never a gap where the app has no audio output
                    if let Some(default_id) = default_sink_id {
                        info!("Reconnecting app {} to hardware sink {}", app_node_id, default_id);
                        let port_pairs = self.state.pw_graph.find_port_pairs(app_node_id, default_id);
                        for (output_port, input_port) in port_pairs {
                            self.send_pw_command(PwCommand::CreateLink {
                                output_port,
                                input_port,
                            });
                        }
                    } else {
                        warn!("No hardware sink found to reconnect app");
                    }

                    // THEN: Destroy links from app to our sink
                    let links_to_destroy: Vec<u32> = self.state.pw_graph.links
                        .values()
                        .filter(|l| l.output_node == app_node_id && l.input_node == sink_node_id)
                        .map(|l| l.id)
                        .collect();

                    for link_id in links_to_destroy {
                        info!("Destroying link {} from app to channel sink", link_id);
                        self.send_pw_command(PwCommand::DestroyLink { link_id });
                    }
                }

                // Remove from channel's assigned apps list
                if let Some(channel) = self.state.channel_mut(channel_id) {
                    channel.assigned_apps.retain(|a| a != &app_id);
                }
            }

            // ==================== Master Actions ====================
            Message::MasterVolumeChanged(volume) => {
                self.state.master_volume_db = volume;
                // Apply to selected output device
                if let Some(node_id) = self.get_output_device_node_id() {
                    let linear = db_to_linear(volume);
                    debug!("Master volume: db={:.1}, linear={:.3}, node={}", volume, linear, node_id);
                    self.send_pw_command(PwCommand::SetVolume { node_id, volume: linear });
                } else {
                    debug!("Master volume changed but no output device found. available={}, selected={:?}",
                        self.state.available_outputs.len(), self.state.output_device);
                }
            }
            Message::MasterVolumeReleased => {
                debug!("Master volume released");
                self.save_config();
            }
            Message::MasterMuteToggled => {
                self.state.master_muted = !self.state.master_muted;
                if let Some(node_id) = self.get_output_device_node_id() {
                    self.send_pw_command(PwCommand::SetMute {
                        node_id,
                        muted: self.state.master_muted,
                    });
                }
                self.save_config();
            }
            Message::OutputDeviceChanged(device_name) => {
                info!("Output device changed to: {}", device_name);
                self.state.output_device = Some(device_name.clone());

                // Find the node_id for this device
                if let Some(device) = self.state.available_outputs.iter()
                    .find(|d| d.description == device_name || d.name == device_name)
                {
                    let node_id = device.node_id;

                    // Set as default sink (pw-loopbacks will automatically route here)
                    self.send_pw_command(PwCommand::SetDefaultSink { node_id });

                    // Apply current master volume/mute to new device
                    let linear = db_to_linear(self.state.master_volume_db);
                    self.send_pw_command(PwCommand::SetVolume { node_id, volume: linear });
                    if self.state.master_muted {
                        self.send_pw_command(PwCommand::SetMute { node_id, muted: true });
                    }
                }

                self.save_config();
            }

            // ==================== EQ Panel ====================
            Message::OpenEqPanel(id) => {
                self.state.eq_panel_channel = Some(id);
            }
            Message::CloseEqPanel => {
                self.state.eq_panel_channel = None;
            }

            // ==================== Settings ====================
            Message::OpenSettings => {
                self.state.settings_open = true;
            }
            Message::CloseSettings => {
                self.state.settings_open = false;
            }

            // ==================== Routing Rules ====================
            Message::OpenRoutingRulesPanel => {
                self.state.routing_rules_panel_open = true;
            }
            Message::CloseRoutingRulesPanel => {
                self.state.routing_rules_panel_open = false;
                self.state.editing_rule = None;
            }
            Message::ToggleRoutingRule(id) => {
                self.state.routing_rules.toggle_rule(id);
                self.save_routing_rules();
            }
            Message::DeleteRoutingRule(id) => {
                self.state.routing_rules.remove_rule(id);
                self.save_routing_rules();
            }
            Message::MoveRoutingRuleUp(id) => {
                self.state.routing_rules.move_up(id);
                self.save_routing_rules();
            }
            Message::MoveRoutingRuleDown(id) => {
                self.state.routing_rules.move_down(id);
                self.save_routing_rules();
            }
            Message::StartEditingRule(id) => {
                if let Some(id) = id {
                    if let Some(rule) = self.state.routing_rules.get_rule(id) {
                        self.state.editing_rule = Some(EditingRule::from_rule(rule));
                    }
                } else {
                    self.state.editing_rule = Some(EditingRule::default());
                }
            }
            Message::CancelEditingRule => {
                self.state.editing_rule = None;
            }
            Message::RuleNameChanged(name) => {
                if let Some(ref mut editing) = self.state.editing_rule {
                    editing.name = name;
                }
            }
            Message::RulePatternChanged(pattern) => {
                if let Some(ref mut editing) = self.state.editing_rule {
                    editing.pattern = pattern;
                }
            }
            Message::RuleMatchTypeChanged(match_type) => {
                if let Some(ref mut editing) = self.state.editing_rule {
                    editing.match_type_name = match_type;
                }
            }
            Message::RuleMatchTargetChanged(target) => {
                if let Some(ref mut editing) = self.state.editing_rule {
                    editing.match_target = target;
                }
            }
            Message::RuleTargetChannelChanged(channel) => {
                if let Some(ref mut editing) = self.state.editing_rule {
                    editing.target_channel = channel;
                }
            }
            Message::RulePriorityChanged(priority_str) => {
                if let Some(ref mut editing) = self.state.editing_rule {
                    if let Ok(priority) = priority_str.parse::<u32>() {
                        editing.priority = priority;
                    }
                }
            }
            Message::SaveRoutingRule => {
                if let Some(editing) = self.state.editing_rule.take() {
                    let rule = editing.to_rule();
                    let is_update = editing.id.is_some();

                    if is_update {
                        // Update existing rule
                        if let Some(existing) = self.state.routing_rules.get_rule_mut(rule.id) {
                            *existing = rule;
                        }
                    } else {
                        // Add new rule
                        self.state.routing_rules.add_rule(rule);
                    }
                    self.state.routing_rules.sort_by_priority();
                    self.save_routing_rules();
                }
            }
            Message::CreateQuickRule(app_name, binary, target_channel) => {
                // Create a simple contains rule for the app
                let pattern = binary.as_deref().unwrap_or(&app_name);
                let rule = crate::config::RoutingRule::new(
                    format!("Auto: {}", &app_name),
                    pattern,
                    target_channel,
                );
                self.state.routing_rules.add_rule(rule);
                self.save_routing_rules();
                info!("Created quick routing rule for '{}'", app_name);
            }

            // ==================== Snapshot A/B Comparison ====================
            Message::CaptureSnapshot(slot) => {
                let snapshot = self.state.capture_snapshot();
                info!(
                    "Captured snapshot {:?}: master_db={:.1}, {} channels",
                    slot, snapshot.master_volume_db, snapshot.channels.len()
                );
                for ch in &snapshot.channels {
                    info!("  Channel {}: volume_db={:.1}, muted={}", ch.id, ch.volume_db, ch.muted);
                }
                match slot {
                    SnapshotSlot::A => self.state.snapshot_a = Some(snapshot),
                    SnapshotSlot::B => self.state.snapshot_b = Some(snapshot),
                }
                self.state.active_snapshot = Some(slot);
            }
            Message::RecallSnapshot(slot) => {
                let snapshot = match slot {
                    SnapshotSlot::A => self.state.snapshot_a.clone(),
                    SnapshotSlot::B => self.state.snapshot_b.clone(),
                };
                if let Some(snapshot) = snapshot {
                    info!(
                        "Recalling snapshot {:?}: master_db={:.1}, {} channels",
                        slot, snapshot.master_volume_db, snapshot.channels.len()
                    );
                    for ch in &snapshot.channels {
                        info!("  Channel {}: volume_db={:.1}", ch.id, ch.volume_db);
                    }

                    let modified = self.state.apply_snapshot(&snapshot);
                    info!("Applied snapshot, modified {} channels", modified.len());

                    // Send PipeWire commands for changed channels
                    for channel_id in modified {
                        if let Some(channel) = self.state.channel(channel_id) {
                            if let Some(node_id) = channel.pw_sink_id {
                                let linear_vol = channel.volume_linear();
                                debug!(
                                    "Setting channel {} volume: db={:.1}, linear={:.3}",
                                    channel.name, channel.volume_db, linear_vol
                                );
                                self.send_pw_command(PwCommand::SetVolume {
                                    node_id,
                                    volume: linear_vol,
                                });
                                self.send_pw_command(PwCommand::SetMute {
                                    node_id,
                                    muted: channel.muted,
                                });
                            }
                        }
                    }

                    // Apply master volume/mute
                    if let Some(node_id) = self.get_output_device_node_id() {
                        let linear = db_to_linear(self.state.master_volume_db);
                        debug!(
                            "Setting master volume: db={:.1}, linear={:.3}",
                            self.state.master_volume_db, linear
                        );
                        self.send_pw_command(PwCommand::SetVolume { node_id, volume: linear });
                        self.send_pw_command(PwCommand::SetMute {
                            node_id,
                            muted: self.state.master_muted,
                        });
                    }

                    self.state.active_snapshot = Some(slot);
                }
            }
            Message::ClearSnapshot(slot) => {
                info!("Clearing snapshot {:?}", slot);
                match slot {
                    SnapshotSlot::A => self.state.snapshot_a = None,
                    SnapshotSlot::B => self.state.snapshot_b = None,
                }
                if self.state.active_snapshot == Some(slot) {
                    self.state.active_snapshot = None;
                }
            }
            Message::SaveChannelToSnapshot(channel_id) => {
                // Save just this channel's current state to the active snapshot
                if let Some(slot) = self.state.active_snapshot {
                    // First, capture the channel data we need
                    let channel_data = self.state.channel(channel_id).map(|channel| {
                        (
                            channel.name.clone(),
                            crate::state::ChannelSnapshot {
                                id: channel.id,
                                volume_db: channel.volume_db,
                                muted: channel.muted,
                                eq_enabled: channel.eq_enabled,
                                eq_preset: channel.eq_preset.clone(),
                            },
                        )
                    });

                    if let Some((channel_name, channel_snapshot)) = channel_data {
                        // Get the snapshot to update
                        let snapshot = match slot {
                            SnapshotSlot::A => &mut self.state.snapshot_a,
                            SnapshotSlot::B => &mut self.state.snapshot_b,
                        };

                        if let Some(ref mut snap) = snapshot {
                            // Find and update the channel in the snapshot, or add it
                            if let Some(existing) = snap.channels.iter_mut().find(|c| c.id == channel_id) {
                                *existing = channel_snapshot;
                                info!("Updated channel {} in snapshot {:?}", channel_name, slot);
                            } else {
                                snap.channels.push(channel_snapshot);
                                info!("Added channel {} to snapshot {:?}", channel_name, slot);
                            }
                        }
                    }
                }
            }

            // ==================== Plugin Chain ====================
            Message::OpenPluginBrowser(channel_id) => {
                info!("Opening plugin browser for channel {}", channel_id);
                self.state.plugin_browser_channel = Some(channel_id);
            }
            Message::ClosePluginBrowser => {
                self.state.plugin_browser_channel = None;
            }
            Message::AddPluginToChannel(channel_id, plugin_id) => {
                info!("Adding plugin '{}' to channel {}", plugin_id, channel_id);

                // Try to load the plugin via PluginManager
                match self.plugin_manager.load(&plugin_id) {
                    Ok(instance_id) => {
                        info!("Loaded plugin instance: {}", instance_id);

                        // Add to channel state
                        if let Some(channel) = self.state.channel_mut(channel_id) {
                            // Add config for persistence
                            let config = PluginSlotConfig::new(
                                plugin_id.clone(),
                                PluginType::Native,
                            );
                            channel.plugin_chain.push(config);

                            // Track the runtime instance ID
                            channel.plugin_instances.push(instance_id);

                            let channel_name = channel.name.clone();
                            let plugin_count = channel.plugin_chain.len();
                            let instances = channel.plugin_instances.clone();

                            info!(
                                "Channel '{}' now has {} plugins",
                                channel_name, plugin_count
                            );

                            // Update plugin processor with new chain
                            if let Err(e) = self.plugin_processor.setup_channel(channel_id, instances.clone()) {
                                warn!("Failed to update plugin processor: {}", e);
                            }

                            // Create or update PipeWire plugin filter
                            if !self.plugin_filter_manager.has_filter(channel_id) {
                                // First plugin - create the filter
                                if let Err(e) = self.plugin_filter_manager.create_filter(
                                    channel_id,
                                    &channel_name,
                                    plugin_count,
                                ) {
                                    warn!("Failed to create plugin filter: {}", e);
                                }

                                // Send command to PipeWire thread
                                if let Some(ref pw) = self.pw_thread {
                                    let _ = pw.send(PwCommand::CreatePluginFilter {
                                        channel_id,
                                        channel_name,
                                        plugin_chain: instances,
                                    });
                                }
                            } else {
                                // Update existing filter's plugin chain
                                if let Some(ref pw) = self.pw_thread {
                                    let _ = pw.send(PwCommand::UpdatePluginChain {
                                        channel_id,
                                        plugin_chain: instances,
                                    });
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error!("Failed to load plugin '{}': {}", plugin_id, e);
                        self.state.last_error = Some(format!("Failed to load plugin: {}", e));
                    }
                }

                // Close browser after adding
                self.state.plugin_browser_channel = None;
            }
            Message::RemovePluginFromChannel(channel_id, instance_id) => {
                info!("Removing plugin {} from channel {}", instance_id, channel_id);

                // Find and remove from channel
                if let Some(channel) = self.state.channel_mut(channel_id) {
                    // Find the index of the instance
                    if let Some(idx) = channel.plugin_instances.iter().position(|&id| id == instance_id) {
                        // Remove from runtime instances
                        channel.plugin_instances.remove(idx);

                        // Remove corresponding config (same index)
                        if idx < channel.plugin_chain.len() {
                            channel.plugin_chain.remove(idx);
                        }

                        // Unload from plugin manager
                        self.plugin_manager.unload(instance_id);

                        let instances = channel.plugin_instances.clone();
                        let plugin_count = channel.plugin_chain.len();

                        info!(
                            "Removed plugin from channel '{}', {} plugins remaining",
                            channel.name, plugin_count
                        );

                        // Update plugin processor with new chain
                        if let Err(e) = self.plugin_processor.setup_channel(channel_id, instances.clone()) {
                            warn!("Failed to update plugin processor: {}", e);
                        }

                        // Update or destroy PipeWire plugin filter
                        if plugin_count == 0 {
                            // No more plugins - destroy the filter
                            self.plugin_filter_manager.destroy_filter(channel_id);
                            if let Some(ref pw) = self.pw_thread {
                                let _ = pw.send(PwCommand::DestroyPluginFilter { channel_id });
                            }
                        } else {
                            // Update the plugin chain
                            if let Some(ref pw) = self.pw_thread {
                                let _ = pw.send(PwCommand::UpdatePluginChain {
                                    channel_id,
                                    plugin_chain: instances,
                                });
                            }
                        }
                    } else {
                        warn!("Plugin instance {} not found in channel", instance_id);
                    }
                }
            }
            Message::MovePluginInChain(channel_id, instance_id, direction) => {
                debug!("Moving plugin {} in channel {} by {}", instance_id, channel_id, direction);

                if let Some(channel) = self.state.channel_mut(channel_id) {
                    // Find the current index of the plugin
                    if let Some(idx) = channel.plugin_instances.iter().position(|&id| id == instance_id) {
                        let new_idx = if direction < 0 {
                            idx.saturating_sub(1)
                        } else {
                            (idx + 1).min(channel.plugin_instances.len().saturating_sub(1))
                        };

                        if new_idx != idx {
                            // Swap in both vectors
                            channel.plugin_instances.swap(idx, new_idx);
                            if idx < channel.plugin_chain.len() && new_idx < channel.plugin_chain.len() {
                                channel.plugin_chain.swap(idx, new_idx);
                            }
                            debug!("Moved plugin from {} to {}", idx, new_idx);
                        }
                    }
                }
            }
            Message::TogglePluginBypass(channel_id, instance_id) => {
                debug!("Toggling bypass for plugin {} in channel {}", instance_id, channel_id);

                if let Some(channel) = self.state.channel_mut(channel_id) {
                    // Find the index of the plugin instance
                    if let Some(idx) = channel.plugin_instances.iter().position(|&id| id == instance_id) {
                        // Toggle bypass in the config
                        if let Some(config) = channel.plugin_chain.get_mut(idx) {
                            config.bypassed = !config.bypassed;
                            info!("Plugin bypass toggled to {}", config.bypassed);
                        }
                    }
                }
            }
            Message::OpenPluginEditor(channel_id, instance_id) => {
                info!("Opening editor for plugin {} in channel {}", instance_id, channel_id);
                self.state.plugin_editor_open = Some((channel_id, instance_id));
            }
            Message::ClosePluginEditor => {
                self.state.plugin_editor_open = None;
            }
            Message::PluginParameterChanged(instance_id, param_idx, value) => {
                debug!("Plugin {} parameter {} changed to {}", instance_id, param_idx, value);

                // Update the plugin instance parameter (direct, for immediate UI feedback)
                self.plugin_manager.set_parameter(instance_id, param_idx, value);

                // Also update the stored config for persistence and send to RT thread
                for channel in &mut self.state.channels {
                    if let Some(idx) = channel.plugin_instances.iter().position(|&id| id == instance_id) {
                        if let Some(config) = channel.plugin_chain.get_mut(idx) {
                            config.parameters.insert(param_idx, value);
                        }

                        // Send parameter update to the RT thread via ring buffer
                        // This allows RT-safe parameter updates during audio processing
                        let channel_id = channel.id;
                        self.plugin_filter_manager.send_param_update(
                            channel_id,
                            instance_id,
                            param_idx,
                            value,
                        );

                        break;
                    }
                }
            }
            Message::PluginChainLoaded(channel_id) => {
                debug!("Plugin chain loaded for channel {}", channel_id);
                // TODO: Handle plugin chain restoration from persistence
            }

            // ==================== PipeWire Events ====================
            Message::PwConnected => {
                info!("Connected to PipeWire");
                self.state.pw_connected = true;
            }
            Message::PwDisconnected => {
                warn!("Disconnected from PipeWire");
                self.state.pw_connected = false;
            }
            Message::PwNodeAdded(node) => {
                debug!("Node added: {} ({})", node.name, node.id);
                self.state.pw_graph.nodes.insert(node.id, node);
                self.state.update_available_apps();
                self.state.update_available_outputs();

                // Check for auto-routing after startup is complete
                if self.state.startup_complete {
                    let to_route = self.check_auto_routing();
                    for (app_node_id, app_id, channel_id) in to_route {
                        self.route_app_to_channel(app_node_id, app_id, channel_id);
                    }
                }
            }
            Message::PwNodeRemoved(id) => {
                debug!("Node removed: {}", id);
                self.state.pw_graph.nodes.remove(&id);
                self.state.update_available_apps();
                self.state.update_available_outputs();
            }
            Message::PwNodeChanged(node) => {
                self.state.pw_graph.nodes.insert(node.id, node);
            }
            Message::PwPortAdded(port) => {
                self.state.pw_graph.ports.insert(port.id, port);
            }
            Message::PwPortRemoved(id) => {
                self.state.pw_graph.ports.remove(&id);
            }
            Message::PwLinkAdded(link) => {
                self.state.pw_graph.links.insert(link.id, link);
            }
            Message::PwLinkRemoved(id) => {
                self.state.pw_graph.links.remove(&id);
            }
            Message::PwVirtualSinkCreated(_channel_id, _node_id) => {
                // Handled in handle_pw_event() for PwEvent::VirtualSinkCreated
            }
            Message::PwError(err) => {
                error!("PipeWire error: {}", err);
                self.state.last_error = Some(err);
            }

            // ==================== Other ====================
            Message::Tick => {
                // Calculate delta time for meter updates
                let now = Instant::now();
                let dt = now.duration_since(self.last_tick).as_secs_f32();
                self.last_tick = now;

                // Update VU meters
                self.meter_manager.update_meters(
                    &mut self.state.channels,
                    &mut self.state.master_meter_display,
                    self.state.master_volume_db,
                    self.state.master_muted,
                    dt,
                );

                // Check for PipeWire events
                self.poll_pw_events();

                // Restore config after PipeWire discovery delay (~200ms)
                if !self.state.startup_complete
                    && self.startup_time.elapsed() > std::time::Duration::from_millis(200)
                {
                    if self.pending_config.is_some() {
                        self.restore_config();
                    } else {
                        // No saved config - just mark startup complete and init snapshot
                        self.state.startup_complete = true;
                        self.initialize_default_snapshot();
                    }
                }

                // Check for auto-routing periodically after startup
                if self.state.startup_complete {
                    let to_route = self.check_auto_routing();
                    for (app_node_id, app_id, channel_id) in to_route {
                        self.route_app_to_channel(app_node_id, app_id, channel_id);
                    }
                }

                // Retry pending re-routing if sink ports are now available
                if let Some((channel_id, ref app_node_ids)) = self.state.pending_reroute.clone() {
                    if let Some(sink_node_id) = self.state.channel(channel_id).and_then(|c| c.pw_sink_id) {
                        // Check if sink has ports now
                        let sink_ports = self.state.pw_graph.input_ports_for_node(sink_node_id);

                        if !sink_ports.is_empty() {
                            // Find hardware sink to disconnect from
                            let our_sink_ids: Vec<u32> = self.state.channels.iter()
                                .filter_map(|c| c.pw_sink_id)
                                .collect();
                            let default_sink_id = find_hardware_sink(&self.state.pw_graph, &our_sink_ids);

                            for &app_node_id in app_node_ids.iter() {
                                // Connect to new sink
                                let port_pairs = self.state.pw_graph.find_port_pairs(app_node_id, sink_node_id);
                                for (output_port, input_port) in port_pairs {
                                    self.send_pw_command(PwCommand::CreateLink { output_port, input_port });
                                }

                                // Disconnect from default sink
                                if let Some(default_id) = default_sink_id {
                                    let links_to_destroy: Vec<u32> = self.state.pw_graph.links
                                        .values()
                                        .filter(|l| l.output_node == app_node_id && l.input_node == default_id)
                                        .map(|l| l.id)
                                        .collect();
                                    for link_id in links_to_destroy {
                                        self.send_pw_command(PwCommand::DestroyLink { link_id });
                                    }
                                }
                            }

                            self.state.pending_reroute = None;
                        }
                    }
                }
            }
            Message::Initialized => {
                info!("Application initialized");
            }

            _ => {
                // Handle remaining message types
                debug!("Unhandled message: {:?}", message);
            }
        }

        Task::none()
    }

    /// Render the application view.
    pub fn view(&self) -> Element<Message> {
        // Header bar
        let header = self.view_header();

        // Channel strips
        let channel_strips = self.view_channel_strips();

        // Apps panel
        let apps = apps_panel(&self.state.available_apps, &self.state.channels, self.state.dragging_app.as_ref());

        // Routing rules panel (shown below apps when open)
        let rules_panel: Element<Message> = if self.state.routing_rules_panel_open {
            let channel_names: Vec<String> = self.state.channels.iter()
                .map(|c| c.name.clone())
                .collect();
            routing_rules_panel(
                &self.state.routing_rules,
                self.state.editing_rule.as_ref(),
                channel_names,
            )
        } else {
            Space::new().height(0).into()
        };

        // Plugin panel (shown when a channel's plugin browser is open)
        let plugin_panel: Element<Message> = if let Some(channel_id) = self.state.plugin_browser_channel {
            // Get channel info
            let channel_name = self.state.channel(channel_id)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| "Unknown".to_string());

            // Get current plugin chain info
            let chain_info = self.get_plugin_chain_info(channel_id);

            // Get available plugins from the PluginManager
            let available_plugins = self.plugin_manager.list_plugins(&PluginFilter::default());

            // Show both chain panel and browser side by side
            row![
                crate::ui::plugin_chain::plugin_chain_panel(
                    channel_id,
                    &channel_name,
                    chain_info,
                ),
                Space::new().width(SPACING),
                crate::ui::plugin_chain::plugin_browser(channel_id, available_plugins),
            ]
            .spacing(SPACING)
            .into()
        } else {
            Space::new().height(0).into()
        };

        // Plugin editor (shown when editing a plugin's parameters)
        let plugin_editor_panel: Element<Message> = if let Some((_channel_id, instance_id)) = self.state.plugin_editor_open {
            if let Some((plugin_name, params)) = self.get_plugin_editor_info(instance_id) {
                crate::ui::plugin_chain::plugin_editor(instance_id, &plugin_name, params)
            } else {
                Space::new().height(0).into()
            }
        } else {
            Space::new().height(0).into()
        };

        // Main content
        let content = column![
            header,
            Space::new().height(SPACING),
            channel_strips,
            Space::new().height(SPACING),
            apps,
            Space::new().height(SPACING),
            rules_panel,
            Space::new().height(SPACING_SMALL),
            plugin_panel,
            Space::new().height(SPACING_SMALL),
            plugin_editor_panel,
            self.view_footer(),
        ]
        .padding(PADDING);

        // Wrap in main container
        container(content)
            .width(Fill)
            .height(Fill)
            .style(|_theme| container::Style {
                background: Some(Background::Color(BACKGROUND)),
                ..container::Style::default()
            })
            .into()
    }

    /// View the header bar.
    fn view_header(&self) -> Element<Message> {
        let title = text("SootMix")
            .size(20)
            .color(TEXT);

        let status = if self.state.pw_connected {
            text("Connected").size(12).color(SUCCESS)
        } else {
            text("Disconnected").size(12).color(MUTED_COLOR)
        };

        let preset_text = text(format!("Preset: {}", self.state.current_preset))
            .size(12)
            .color(TEXT_DIM);

        // A/B Snapshot buttons and Save button
        let snapshot_a_button = self.snapshot_button(SnapshotSlot::A);
        let snapshot_b_button = self.snapshot_button(SnapshotSlot::B);
        let snapshot_save_button = self.snapshot_save_button();

        let rules_count = self.state.routing_rules.rules.len();
        let rules_button = button(
            text(format!("Rules ({})", rules_count)).size(12)
        )
            .padding([6, 12])
            .style(|_theme: &Theme, _status| button::Style {
                background: Some(Background::Color(SURFACE_LIGHT)),
                text_color: TEXT,
                border: standard_border(),
                ..button::Style::default()
            })
            .on_press(Message::OpenRoutingRulesPanel);

        let settings_button = button(text("Settings").size(12))
            .padding([6, 12])
            .style(|_theme: &Theme, _status| button::Style {
                background: Some(Background::Color(SURFACE_LIGHT)),
                text_color: TEXT,
                border: standard_border(),
                ..button::Style::default()
            })
            .on_press(Message::OpenSettings);

        row![
            title,
            Space::new().width(SPACING),
            status,
            Space::new().width(Fill),
            preset_text,
            Space::new().width(SPACING),
            snapshot_a_button,
            Space::new().width(SPACING_SMALL),
            snapshot_b_button,
            Space::new().width(SPACING_SMALL),
            snapshot_save_button,
            Space::new().width(SPACING),
            rules_button,
            Space::new().width(SPACING_SMALL),
            settings_button,
        ]
        .align_y(Alignment::Center)
        .into()
    }

    /// Create a snapshot button (A or B) with appropriate styling based on state.
    /// Behavior:
    /// - Empty slot: click to capture current state
    /// - Filled slot (not active): click to recall
    /// - Filled slot (active): no action (use Save button to update)
    fn snapshot_button(&self, slot: SnapshotSlot) -> Element<Message> {
        let label = match slot {
            SnapshotSlot::A => "A",
            SnapshotSlot::B => "B",
        };

        let has_snapshot = match slot {
            SnapshotSlot::A => self.state.snapshot_a.is_some(),
            SnapshotSlot::B => self.state.snapshot_b.is_some(),
        };

        let is_active = self.state.active_snapshot == Some(slot);

        // Style based on state
        let (bg_color, text_color, border_color) = if is_active {
            // Active snapshot: highlighted
            (PRIMARY, TEXT, PRIMARY)
        } else if has_snapshot {
            // Filled but not active: normal
            (SURFACE_LIGHT, TEXT, SURFACE_LIGHT)
        } else {
            // Empty slot: dim
            (SURFACE, TEXT_DIM, SURFACE)
        };

        let btn = button(text(label).size(12))
            .padding([6, 10])
            .style(move |_theme: &Theme, _status| button::Style {
                background: Some(Background::Color(bg_color)),
                text_color,
                border: iced::Border {
                    color: border_color,
                    width: if is_active { 2.0 } else { 1.0 },
                    radius: 4.0.into(),
                },
                ..button::Style::default()
            });

        // Determine action based on state:
        // - Empty slot: capture
        // - Filled and not active: recall
        // - Filled and active: no action
        if is_active {
            btn.into() // No on_press - button is disabled when active
        } else if has_snapshot {
            btn.on_press(Message::RecallSnapshot(slot)).into()
        } else {
            btn.on_press(Message::CaptureSnapshot(slot)).into()
        }
    }

    /// Create the "Save All" button for saving entire mixer state to active snapshot.
    fn snapshot_save_button(&self) -> Element<Message> {
        let has_active = self.state.active_snapshot.is_some();

        let btn = button(text("Save").size(11))
            .padding([5, 8])
            .style(move |_theme: &Theme, status| {
                let is_hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
                let bg = if !has_active {
                    SURFACE
                } else if is_hovered {
                    SUCCESS
                } else {
                    SURFACE_LIGHT
                };
                let txt = if !has_active {
                    TEXT_DIM
                } else if is_hovered {
                    TEXT
                } else {
                    SUCCESS
                };
                button::Style {
                    background: Some(Background::Color(bg)),
                    text_color: txt,
                    border: iced::Border {
                        color: if has_active { SUCCESS } else { SURFACE },
                        width: 1.0,
                        radius: 4.0.into(),
                    },
                    ..button::Style::default()
                }
            });

        if let Some(slot) = self.state.active_snapshot {
            btn.on_press(Message::CaptureSnapshot(slot)).into()
        } else {
            btn.into() // Disabled when no active snapshot
        }
    }

    /// View the channel strips area.
    fn view_channel_strips(&self) -> Element<Message> {
        let dragging = self.state.dragging_app.as_ref();
        let editing = self.state.editing_channel.as_ref();
        let has_active_snapshot = self.state.active_snapshot.is_some();

        // Build channel strip widgets
        let mut strips: Vec<Element<Message>> = self
            .state
            .channels
            .iter()
            .map(|c| channel_strip(c, dragging, editing, has_active_snapshot))
            .collect();

        // Add separator before master
        strips.push(
            container(Space::new().width(2))
                .height(CHANNEL_STRIP_HEIGHT)
                .style(|_| container::Style {
                    background: Some(Background::Color(SURFACE_LIGHT)),
                    ..container::Style::default()
                })
                .into(),
        );

        // Add master strip
        strips.push(master_strip(
            self.state.master_volume_db,
            self.state.master_muted,
            &self.state.available_outputs,
            self.state.output_device.as_deref(),
            &self.state.master_meter_display,
        ));

        let strips_row = row(strips)
            .spacing(SPACING)
            .align_y(Alignment::Start);

        scrollable(strips_row)
            .direction(scrollable::Direction::Horizontal(
                scrollable::Scrollbar::default(),
            ))
            .into()
    }

    /// View the footer with add channel buttons.
    fn view_footer(&self) -> Element<Message> {
        let add_button = button(text("+ New Channel").size(14))
            .padding([10, 20])
            .style(|_theme: &Theme, _status| button::Style {
                background: Some(Background::Color(PRIMARY)),
                text_color: TEXT,
                border: standard_border(),
                ..button::Style::default()
            })
            .on_press(Message::NewChannelRequested);

        // Error display
        let error_text: Element<Message> = if let Some(ref err) = self.state.last_error {
            text(format!("Error: {}", err))
                .size(11)
                .color(MUTED_COLOR)
                .into()
        } else {
            Space::new().width(0).into()
        };

        row![
            add_button,
            Space::new().width(Fill),
            error_text,
        ]
        .align_y(Alignment::Center)
        .into()
    }

    /// Get the application theme.
    pub fn theme(&self) -> Theme {
        theme::sootmix_theme()
    }

    /// Subscription for external events.
    pub fn subscription(&self) -> Subscription<Message> {
        // Tick every 50ms to poll PipeWire events
        iced::time::every(std::time::Duration::from_millis(50)).map(|_| Message::Tick)
    }

    /// Send a command to the PipeWire thread.
    fn send_pw_command(&self, cmd: PwCommand) {
        if let Some(ref thread) = self.pw_thread {
            if let Err(e) = thread.send(cmd) {
                error!("Failed to send command to PipeWire thread: {}", e);
            }
        }
    }

    /// Get the node ID of the selected output device (or find default hardware sink).
    fn get_output_device_node_id(&self) -> Option<u32> {
        // If user selected a device, find its node_id
        if let Some(ref device_name) = self.state.output_device {
            self.state.available_outputs.iter()
                .find(|d| d.description == *device_name || d.name == *device_name)
                .map(|d| d.node_id)
        } else {
            // Use hardware sink finder as fallback
            let our_sink_ids: Vec<u32> = self.state.channels.iter()
                .filter_map(|c| c.pw_sink_id)
                .collect();
            find_hardware_sink(&self.state.pw_graph, &our_sink_ids)
        }
    }

    /// Initialize snapshot A with current state if not already set.
    fn initialize_default_snapshot(&mut self) {
        if self.state.snapshot_a.is_none() && self.state.active_snapshot.is_none() {
            let snapshot = self.state.capture_snapshot();
            info!(
                "Initializing snapshot A: master_db={:.1}, {} channels",
                snapshot.master_volume_db, snapshot.channels.len()
            );
            self.state.snapshot_a = Some(snapshot);
            self.state.active_snapshot = Some(SnapshotSlot::A);
        }
    }

    /// Poll for PipeWire events.
    fn poll_pw_events(&mut self) {
        // Collect events first to avoid borrow conflict
        let events: Vec<PwEvent> = if let Some(ref rx) = self.pw_event_rx {
            let mut events = Vec::new();
            while let Ok(event) = rx.try_recv() {
                events.push(event);
            }
            events
        } else {
            Vec::new()
        };

        // Now handle them with mutable self
        for event in events {
            self.handle_pw_event(event);
        }
    }

    /// Handle a PipeWire event.
    fn handle_pw_event(&mut self, event: PwEvent) {
        match event {
            PwEvent::Connected => {
                self.state.pw_connected = true;
            }
            PwEvent::Disconnected => {
                self.state.pw_connected = false;
            }
            PwEvent::NodeAdded(node) => {
                self.state.pw_graph.nodes.insert(node.id, node);
                self.state.update_available_apps();
                self.state.update_available_outputs();
            }
            PwEvent::NodeRemoved(id) => {
                self.state.pw_graph.nodes.remove(&id);
                self.state.update_available_apps();
                self.state.update_available_outputs();
            }
            PwEvent::NodeChanged(node) => {
                self.state.pw_graph.nodes.insert(node.id, node);
            }
            PwEvent::PortAdded(port) => {
                self.state.pw_graph.ports.insert(port.id, port);
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
            PwEvent::VirtualSinkCreated { channel_id, node_id } => {
                info!("PwEvent: VirtualSinkCreated channel={} node={}", channel_id, node_id);

                // Get assigned apps before mutating the channel
                let assigned_apps = self.state.channel(channel_id)
                    .map(|c| c.assigned_apps.clone())
                    .unwrap_or_default();

                if let Some(channel) = self.state.channel_mut(channel_id) {
                    channel.pw_sink_id = Some(node_id);
                    info!("Updated channel '{}' pw_sink_id to {}", channel.name, node_id);
                }

                // Route assigned apps from restored config to this sink
                if !assigned_apps.is_empty() {
                    info!("Routing {} assigned apps to sink {}", assigned_apps.len(), node_id);

                    // Find hardware sink to disconnect apps from
                    let our_sink_ids: Vec<u32> = self.state.channels.iter()
                        .filter_map(|c| c.pw_sink_id)
                        .collect();
                    let default_sink_id = find_hardware_sink(&self.state.pw_graph, &our_sink_ids);

                    for app_id in &assigned_apps {
                        // Find the app's current node ID
                        if let Some(app) = self.state.available_apps.iter().find(|a| a.identifier() == *app_id) {
                            let app_node_id = app.node_id;

                            // Connect app to our virtual sink
                            let port_pairs = self.state.pw_graph.find_port_pairs(app_node_id, node_id);
                            if !port_pairs.is_empty() {
                                for (output_port, input_port) in &port_pairs {
                                    self.send_pw_command(PwCommand::CreateLink {
                                        output_port: *output_port,
                                        input_port: *input_port,
                                    });
                                }
                                info!("Routed app '{}' to sink {}", app_id, node_id);

                                // Disconnect app from default sink
                                if let Some(default_id) = default_sink_id {
                                    let links_to_destroy: Vec<u32> = self.state.pw_graph.links
                                        .values()
                                        .filter(|l| l.output_node == app_node_id && l.input_node == default_id)
                                        .map(|l| l.id)
                                        .collect();
                                    for link_id in links_to_destroy {
                                        self.send_pw_command(PwCommand::DestroyLink { link_id });
                                    }
                                }
                            } else {
                                debug!("Ports not ready for app '{}', will retry later", app_id);
                            }
                        } else {
                            debug!("App '{}' not currently running, skipping", app_id);
                        }
                    }
                }

                // Check if there are apps waiting to be re-routed to this channel (from rename)
                if let Some((pending_channel_id, ref app_node_ids)) = self.state.pending_reroute.clone() {
                    if pending_channel_id == channel_id {
                        info!("Re-routing {} apps to renamed sink {}", app_node_ids.len(), node_id);

                        let our_sink_ids: Vec<u32> = self.state.channels.iter()
                            .filter_map(|c| c.pw_sink_id)
                            .collect();
                        let default_sink_id = find_hardware_sink(&self.state.pw_graph, &our_sink_ids);

                        for &app_node_id in app_node_ids.iter() {
                            let port_pairs = self.state.pw_graph.find_port_pairs(app_node_id, node_id);
                            for (output_port, input_port) in &port_pairs {
                                self.send_pw_command(PwCommand::CreateLink {
                                    output_port: *output_port,
                                    input_port: *input_port,
                                });
                            }

                            if let Some(default_id) = default_sink_id {
                                let links_to_destroy: Vec<u32> = self.state.pw_graph.links
                                    .values()
                                    .filter(|l| l.output_node == app_node_id && l.input_node == default_id)
                                    .map(|l| l.id)
                                    .collect();
                                for link_id in links_to_destroy {
                                    self.send_pw_command(PwCommand::DestroyLink { link_id });
                                }
                            }
                        }

                        self.state.pending_reroute = None;
                    }
                }
            }
            PwEvent::VirtualSinkDestroyed { node_id } => {
                for channel in &mut self.state.channels {
                    if channel.pw_sink_id == Some(node_id) {
                        channel.pw_sink_id = None;
                    }
                }
            }
            PwEvent::PluginFilterCreated {
                channel_id,
                sink_node_id,
                output_node_id,
            } => {
                info!(
                    "Plugin filter created for channel {}: sink={}, output={}",
                    channel_id, sink_node_id, output_node_id
                );
                // TODO: Store node IDs and set up routing
                // The filter is now ready to process audio
            }
            PwEvent::PluginFilterDestroyed { channel_id } => {
                info!("Plugin filter destroyed for channel {}", channel_id);
                // TODO: Clean up any routing state
            }
            PwEvent::Error(err) => {
                self.state.last_error = Some(err);
            }
            PwEvent::ParamChanged { node_id, volume, muted } => {
                // Handle control parameter feedback from PipeWire
                debug!("Param changed on node {}: vol={:?}, mute={:?}", node_id, volume, muted);
            }
        }
    }

    /// Save routing rules to disk.
    fn save_routing_rules(&self) {
        if let Some(ref cm) = self.config_manager {
            if let Err(e) = cm.save_routing_rules(&self.state.routing_rules) {
                error!("Failed to save routing rules: {}", e);
            } else {
                debug!("Saved {} routing rules", self.state.routing_rules.rules.len());
            }
        }
    }

    /// Check for apps that should be auto-routed based on rules.
    /// Returns a list of (app_node_id, app_identifier, channel_id) tuples to route.
    fn check_auto_routing(&mut self) -> Vec<(u32, String, Uuid)> {
        let mut to_route = Vec::new();

        for app in &self.state.available_apps {
            let app_id = app.identifier().to_string();

            // Skip if already routed in this session
            if self.state.auto_routed_apps.contains(&app_id) {
                continue;
            }

            // Check if already assigned to a channel (from saved config)
            let assigned_channel = self.state.channels.iter()
                .find(|c| c.assigned_apps.contains(&app_id))
                .map(|c| (c.id, c.pw_sink_id));

            if let Some((channel_id, Some(sink_id))) = assigned_channel {
                // App is in assigned_apps but wasn't routed yet (e.g., app started after sink was created)
                // Check if actually connected by looking for existing links
                let is_connected = self.state.pw_graph.links.values()
                    .any(|l| l.output_node == app.node_id && l.input_node == sink_id);

                if !is_connected {
                    info!("Reconnecting saved app '{}' to its assigned channel", app_id);
                    to_route.push((app.node_id, app_id.clone(), channel_id));
                }
                self.state.auto_routed_apps.insert(app_id);
                continue;
            } else if assigned_channel.is_some() {
                // Assigned but sink not ready yet
                continue;
            }

            // Check if any rule matches
            if let Some(rule) = self.state.routing_rules.find_match(&app.name, app.binary.as_deref()) {
                // Find the target channel
                if let Some(channel) = self.state.channel_by_name(&rule.target_channel) {
                    if channel.pw_sink_id.is_some() {
                        info!("Auto-routing '{}' to channel '{}' (rule: {})",
                            app.name, rule.target_channel, rule.name);
                        to_route.push((app.node_id, app_id.clone(), channel.id));
                        self.state.auto_routed_apps.insert(app_id);
                    }
                }
            }
        }

        to_route
    }

    /// Route an app to a channel (extracted from DropAppOnChannel for reuse).
    fn route_app_to_channel(&mut self, app_node_id: u32, app_id: String, channel_id: Uuid) {
        // Get the channel's virtual sink node ID
        let sink_node_id = self.state.channel(channel_id).and_then(|c| c.pw_sink_id);

        if let Some(sink_node_id) = sink_node_id {
            // First, disconnect the app from any existing sinks
            let existing_links = self.state.pw_graph.links_from_node(app_node_id);
            for link in existing_links {
                // Don't destroy links to our own sinks
                let is_our_sink = self.state.channels.iter()
                    .any(|c| c.pw_sink_id == Some(link.input_node));
                if !is_our_sink {
                    debug!("Auto-routing: disconnecting app from node {}", link.input_node);
                    self.send_pw_command(PwCommand::DestroyLink { link_id: link.id });
                }
            }

            // Find matching port pairs between app and sink
            let port_pairs = self.state.pw_graph.find_port_pairs(app_node_id, sink_node_id);

            if !port_pairs.is_empty() {
                // Create links for each port pair
                for (output_port, input_port) in &port_pairs {
                    debug!("Auto-routing: creating link port {} -> port {}", output_port, input_port);
                    self.send_pw_command(PwCommand::CreateLink {
                        output_port: *output_port,
                        input_port: *input_port,
                    });
                }

                // Add app to channel's assigned apps list
                if let Some(channel) = self.state.channel_mut(channel_id) {
                    if !channel.assigned_apps.contains(&app_id) {
                        channel.assigned_apps.push(app_id);
                    }
                }
            }
        }
    }

    /// Save current mixer configuration to disk.
    fn save_config(&self) {
        if let Some(ref cm) = self.config_manager {
            let config = MixerConfig {
                master: crate::config::MasterConfig {
                    volume_db: self.state.master_volume_db,
                    muted: self.state.master_muted,
                    output_device: self.state.output_device.clone(),
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
                        plugin_chain: c.plugin_chain.clone(),
                    })
                    .collect(),
            };

            if let Err(e) = cm.save_mixer_config(&config) {
                error!("Failed to save mixer config: {}", e);
            } else {
                debug!("Saved mixer config: {} channels", config.channels.len());
            }
        }
    }

    /// Restore channels from saved configuration.
    fn restore_config(&mut self) {
        if let Some(config) = self.pending_config.take() {
            info!("Restoring {} channels from config", config.channels.len());

            // Restore master settings
            self.state.master_volume_db = config.master.volume_db;
            self.state.master_muted = config.master.muted;
            self.state.output_device = config.master.output_device.clone();

            // Apply master volume/mute/device to output
            if let Some(ref device_name) = config.master.output_device {
                // Find the device and set it as default with proper volume/mute
                if let Some(device) = self.state.available_outputs.iter()
                    .find(|d| d.description == *device_name || d.name == *device_name)
                {
                    let node_id = device.node_id;
                    info!("Restoring output device: {} (node {})", device_name, node_id);

                    // Set as default sink
                    self.send_pw_command(PwCommand::SetDefaultSink { node_id });

                    // Apply volume and mute
                    let linear = db_to_linear(config.master.volume_db);
                    self.send_pw_command(PwCommand::SetVolume { node_id, volume: linear });
                    if config.master.muted {
                        self.send_pw_command(PwCommand::SetMute { node_id, muted: true });
                    }
                }
            } else if let Some(node_id) = self.get_output_device_node_id() {
                // No saved device, but apply volume/mute to default
                let linear = db_to_linear(config.master.volume_db);
                self.send_pw_command(PwCommand::SetVolume { node_id, volume: linear });
                if config.master.muted {
                    self.send_pw_command(PwCommand::SetMute { node_id, muted: true });
                }
            }

            for saved in config.channels {
                if saved.is_managed {
                    // Managed channel - create new pw-loopback sink
                    debug!("Restoring channel '{}' (id={}) with {} assigned apps: {:?}",
                        saved.name, saved.id, saved.assigned_apps.len(), saved.assigned_apps);

                    let mut channel = MixerChannel::new(&saved.name);
                    channel.id = saved.id;
                    channel.volume_db = saved.volume_db;
                    channel.muted = saved.muted;
                    channel.eq_enabled = saved.eq_enabled;
                    channel.eq_preset = saved.eq_preset;
                    channel.assigned_apps = saved.assigned_apps;
                    channel.plugin_chain = saved.plugin_chain.clone();

                    let id = channel.id;
                    let name = channel.name.clone();

                    // Reload plugins from saved config
                    let mut loaded_instances = Vec::new();
                    for slot_config in &saved.plugin_chain {
                        match self.plugin_manager.load(&slot_config.plugin_id) {
                            Ok(instance_id) => {
                                info!(
                                    "Restored plugin '{}' for channel '{}' (instance {})",
                                    slot_config.plugin_id, name, instance_id
                                );
                                // Restore parameter values
                                for (&param_idx, &value) in &slot_config.parameters {
                                    self.plugin_manager.set_parameter(instance_id, param_idx, value);
                                }
                                loaded_instances.push(instance_id);
                            }
                            Err(e) => {
                                warn!(
                                    "Failed to restore plugin '{}' for channel '{}': {}",
                                    slot_config.plugin_id, name, e
                                );
                            }
                        }
                    }
                    channel.plugin_instances = loaded_instances;

                    self.state.channels.push(channel);

                    info!("Created channel in state: id={}, name='{}', assigned_apps count={}, plugins={}",
                        id, name,
                        self.state.channel(id).map(|c| c.assigned_apps.len()).unwrap_or(0),
                        self.state.channel(id).map(|c| c.plugin_instances.len()).unwrap_or(0)
                    );

                    // Create the virtual sink
                    self.send_pw_command(PwCommand::CreateVirtualSink {
                        channel_id: id,
                        name,
                    });
                }
            }

            self.state.startup_complete = true;

            // Initialize snapshot A with the restored state
            self.initialize_default_snapshot();
        }
    }
}

/// Find a hardware output sink (not virtual sinks).
/// Prefers actual hardware devices over pw-loopback virtual sinks.
fn find_hardware_sink(graph: &crate::state::PwGraphState, exclude_ids: &[u32]) -> Option<u32> {
    use crate::audio::types::MediaClass;

    // First, try to find actual hardware sinks (ALSA, Bluetooth, etc.)
    let hardware_sink = graph.nodes.values()
        .find(|n| {
            n.media_class == MediaClass::AudioSink
                && !exclude_ids.contains(&n.id)
                && !n.name.starts_with("sootmix.")
                && (n.name.starts_with("alsa_output")
                    || n.name.starts_with("bluez_output")
                    || n.name.contains("pci-")
                    || n.name.contains("usb-"))
        })
        .map(|n| n.id);

    if hardware_sink.is_some() {
        return hardware_sink;
    }

    // Fallback: find any sink that looks like a real device (has "ALSA" or device-like description)
    graph.nodes.values()
        .find(|n| {
            n.media_class == MediaClass::AudioSink
                && !exclude_ids.contains(&n.id)
                && !n.name.starts_with("sootmix.")
                && !n.name.contains("Virtual Sink")
                && !n.name.contains("virtual")
                && !n.name.starts_with("LB-")
        })
        .map(|n| n.id)
}

impl Drop for SootMix {
    fn drop(&mut self) {
        info!("SootMix shutting down...");

        // Save configuration before cleanup
        self.save_config();
        info!("Configuration saved");

        // Find hardware output sink (not virtual sinks)
        info!("Reconnecting apps to default sink...");
        let our_sink_ids: Vec<u32> = self.state.channels.iter()
            .filter_map(|c| c.pw_sink_id)
            .collect();
        let default_sink_id = find_hardware_sink(&self.state.pw_graph, &our_sink_ids);

        // Reconnect apps from all channels to default sink before destroying
        for channel in &self.state.channels {
            if let Some(sink_node_id) = channel.pw_sink_id {
                for app_id in &channel.assigned_apps {
                    if let Some(app) = self.state.available_apps.iter().find(|a| a.identifier() == *app_id) {
                        let app_node_id = app.node_id;

                        // FIRST: Reconnect to default sink using CLI (before destroying old links)
                        if let Some(default_id) = default_sink_id {
                            let port_pairs = self.state.pw_graph.find_port_pairs(app_node_id, default_id);
                            for (output_port, input_port) in port_pairs {
                                let _ = crate::audio::routing::create_link(output_port, input_port);
                            }
                        }

                        // THEN: Destroy links from app to our sink
                        let links_to_destroy: Vec<u32> = self.state.pw_graph.links
                            .values()
                            .filter(|l| l.output_node == app_node_id && l.input_node == sink_node_id)
                            .map(|l| l.id)
                            .collect();
                        for link_id in links_to_destroy {
                            let _ = crate::audio::routing::destroy_link(link_id);
                        }
                    }
                }
            }
        }

        // Clean up managed virtual sinks (destroy_all_virtual_sinks only destroys sootmix.* sinks)
        crate::audio::virtual_sink::destroy_all_virtual_sinks();

        // Clean up EQ filter chains
        filter_chain::destroy_all_eq_filters();

        // Shutdown PipeWire thread
        if let Some(thread) = self.pw_thread.take() {
            thread.shutdown();
        }
    }
}
