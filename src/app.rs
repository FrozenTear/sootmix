// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Iced Application implementation for SootMix.

use crate::audio::{filter_chain, MeterManager, PluginFilterManager, PluginProcessorManager, PwCommand, PwEvent, PwThread};
use crate::config::eq_preset::EqPreset;
use crate::config::{ConfigManager, MixerConfig, SavedChannel};
use crate::daemon_client::{self, DaemonEvent};
use crate::message::Message;
use crate::plugins::{PluginFilter, PluginManager, PluginSlotConfig, PluginType};
use crate::state::{db_to_linear, AppState, EditingRule, MixerChannel, SnapshotSlot};
use crate::tray::{TrayHandle, TrayMessage};
use crate::ui::apps_panel::apps_panel;
use crate::ui::channel_strip::{channel_strip, master_strip};
use crate::ui::routing_rules_panel::routing_rules_panel;
use crate::ui::theme::{self, *};
use iced::widget::{button, column, container, row, scrollable, text, Space};
use iced::{Alignment, Background, Border, Color, Element, Fill, Length, Subscription, Task, Theme};
use std::sync::mpsc;
use std::time::Instant;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Main application state.
pub struct SootMix {
    /// Application state.
    state: AppState,
    /// PipeWire thread handle (only used in standalone mode).
    pw_thread: Option<PwThread>,
    /// Receiver for PipeWire events (only used in standalone mode).
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
    /// System tray handle for background operation.
    tray_handle: Option<TrayHandle>,
    /// Receiver for tray messages.
    tray_rx: Option<mpsc::Receiver<TrayMessage>>,
    /// Current main window ID (None when window is closed/hidden).
    main_window_id: Option<iced::window::Id>,
    /// Whether we're connected to the daemon (vs running standalone).
    daemon_connected: bool,
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

        // Don't spawn PipeWire thread yet - wait to see if daemon is available.
        // If daemon connects, we'll use daemon mode.
        // If daemon doesn't connect, we'll spawn local PW thread on first tick.
        info!("Waiting for daemon connection...");

        // Initialize plugin manager and scan for plugins
        let mut plugin_manager = PluginManager::new();
        let plugin_count = plugin_manager.scan();
        info!("Plugin scan complete: {} plugins found", plugin_count);

        // Initialize plugin filter manager with shared instances
        let mut plugin_filter_manager = PluginFilterManager::new();
        plugin_filter_manager.set_plugin_instances(plugin_manager.shared_instances());

        // Start system tray
        let (tray_rx, tray_handle) = match crate::tray::start_tray() {
            Some((rx, handle)) => (Some(rx), Some(handle)),
            None => {
                warn!("System tray not available - close will exit the app");
                (None, None)
            }
        };

        let now = Instant::now();

        // Open the initial window (daemon mode doesn't open one by default)
        let (window_id, open_window) = iced::window::open(iced::window::Settings {
            size: iced::Size::new(900.0, 600.0),
            platform_specific: iced::window::settings::PlatformSpecific {
                application_id: "sootmix".to_string(),
                ..Default::default()
            },
            ..Default::default()
        });

        let app = Self {
            state,
            pw_thread: None,
            pw_event_rx: None,
            config_manager,
            startup_time: now,
            pending_config,
            meter_manager: MeterManager::new(),
            last_tick: now,
            plugin_manager,
            plugin_processor: PluginProcessorManager::new(),
            plugin_filter_manager,
            tray_handle,
            tray_rx,
            main_window_id: Some(window_id),
            daemon_connected: false,
        };

        (app, open_window.discard())
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
                self.cmd_set_channel_volume(id, volume);
            }
            Message::ChannelVolumeReleased(_id) => {
                // Volume changes don't auto-save to snapshot - user must click the active slot to save
            }
            Message::ChannelMuteToggled(id) => {
                let new_muted = self.state.channel(id).map(|c| !c.muted).unwrap_or(false);
                self.cmd_set_channel_mute(id, new_muted);
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
                    // Note: pw-loopback adds "output." prefix to the --name value
                    let loopback_output_name = format!("output.sootmix.{}.output", safe_name);
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
                            if let Err(e) = filter_chain::unroute_eq(
                                &loopback_output_name,
                                &eq_sink_name,
                                &eq_output_name,
                                &master_sink_name,
                            ) {
                                warn!("Failed to unroute EQ for channel '{}': {}", name, e);
                            }

                            // Destroy the EQ filter
                            if let Err(e) = filter_chain::destroy_eq_filter(id) {
                                warn!("Failed to destroy EQ filter for channel '{}': {}", name, e);
                            }
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
                    self.cmd_rename_channel(id, &new_name);
                    self.save_config();
                }
                self.state.editing_channel = None;
            }
            Message::ChannelDeleted(id) => {
                self.cmd_delete_channel(id);
                self.save_config();
            }
            Message::NewChannelRequested => {
                let channel_num = self.state.channels.len() + 1;
                let name = format!("Channel {}", channel_num);
                self.cmd_create_channel(&name);
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

                    if self.daemon_connected {
                        // Daemon mode: send command to daemon, it has the sink IDs
                        if !self.cmd_assign_app(app_node_id, channel_id) {
                            self.state.last_error = Some("Failed to assign app".to_string());
                        }
                    } else {
                        // Standalone mode: check if channel has a sink before attempting assignment
                        let has_sink = self.state.channel(channel_id).and_then(|c| c.pw_sink_id).is_some();

                        if has_sink {
                            if !self.cmd_assign_app(app_node_id, channel_id) {
                                self.state.last_error = Some("No matching ports found".to_string());
                            }
                        } else {
                            warn!("Channel {:?} has no virtual sink yet", channel_id);
                            self.state.last_error = Some("Channel has no sink - try again".to_string());
                            // Put the drag state back so user can try again
                            self.state.dragging_app = Some((app_node_id, app_id));
                        }
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
                if let Some(app_node_id) = self.state.available_apps.iter()
                    .find(|a| a.identifier() == app_id)
                    .map(|a| a.node_id)
                {
                    self.cmd_unassign_app(app_node_id, channel_id);
                } else {
                    // App not found (might have been removed), just update local state
                    if let Some(channel) = self.state.channel_mut(channel_id) {
                        channel.assigned_apps.retain(|a| a != &app_id);
                    }
                }
            }
            Message::ChannelOutputDeviceChanged(channel_id, device_name) => {
                self.cmd_set_channel_output(channel_id, device_name.as_deref());
                self.save_config();
            }

            // ==================== Master Actions ====================
            Message::MasterVolumeChanged(volume) => {
                self.cmd_set_master_volume(volume);
            }
            Message::MasterVolumeReleased => {
                debug!("Master volume released");
                self.save_config();
            }
            Message::MasterMuteToggled => {
                let new_muted = !self.state.master_muted;
                self.cmd_set_master_mute(new_muted);
                self.save_config();
            }
            Message::OutputDeviceChanged(device_name) => {
                info!("Output device changed to: {}", device_name);
                self.cmd_set_master_output(Some(&device_name));
                self.save_config();
            }
            Message::ToggleMasterRecording => {
                let new_enabled = !self.state.master_recording_enabled;
                self.cmd_set_master_recording(new_enabled);
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

            // ==================== Layout & Selection ====================
            Message::SelectChannel(channel_id) => {
                self.state.selected_channel = channel_id;
                // Close plugin browser when selecting a different channel
                if channel_id != self.state.plugin_browser_channel {
                    self.state.plugin_browser_channel = None;
                }
            }
            Message::ToggleLeftSidebar => {
                self.state.left_sidebar_collapsed = !self.state.left_sidebar_collapsed;
            }
            Message::ToggleBottomPanel => {
                self.state.bottom_panel_expanded = !self.state.bottom_panel_expanded;
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

                    // Apply channel volume/mute changes using cmd methods
                    for channel_id in modified {
                        if let Some(channel) = self.state.channel(channel_id) {
                            debug!(
                                "Setting channel {} volume: db={:.1}",
                                channel.name, channel.volume_db
                            );
                            let volume = channel.volume_db;
                            let muted = channel.muted;
                            // Use cmd methods - they handle daemon vs standalone
                            self.cmd_set_channel_volume(channel_id, volume);
                            self.cmd_set_channel_mute(channel_id, muted);
                        }
                    }

                    // Apply master volume/mute
                    debug!(
                        "Setting master volume: db={:.1}",
                        self.state.master_volume_db
                    );
                    self.cmd_set_master_volume(self.state.master_volume_db);
                    self.cmd_set_master_mute(self.state.master_muted);

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
                                    let meter_levels = self.state.channel(channel_id)
                                        .and_then(|c| c.meter_levels.clone());
                                    let _ = pw.send(PwCommand::CreatePluginFilter {
                                        channel_id,
                                        channel_name,
                                        plugin_chain: instances,
                                        meter_levels,
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

                        // Send parameter update to the RT thread via PipeWire thread
                        // This is the primary path for RT-safe parameter updates
                        let channel_id = channel.id;
                        if let Some(ref pw) = self.pw_thread {
                            let _ = pw.send(PwCommand::SendPluginParamUpdate {
                                channel_id,
                                instance_id,
                                param_index: param_idx,
                                value,
                            });
                        }

                        // Also send through PluginFilterManager (legacy path)
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
            Message::PluginSidechainSourceChanged(channel_id, slot_index, source_channel_id) => {
                info!(
                    "Plugin sidechain source changed: channel={} slot={} source={:?}",
                    channel_id, slot_index, source_channel_id
                );
                if let Some(channel) = self.state.channel_mut(channel_id) {
                    if let Some(slot_config) = channel.plugin_chain.get_mut(slot_index) {
                        slot_config.sidechain_source = source_channel_id;
                    }
                }
                self.save_config();
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

                // Pump sidechain levels to plugin parameters
                self.pump_sidechain_levels();

                // Check for PipeWire events
                self.poll_pw_events();

                // Poll tray messages
                if let Some(tray_msgs) = self.poll_tray_messages() {
                    return tray_msgs;
                }

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

            // ==================== Window & Tray ====================
            Message::WindowCloseRequested(window_id) => {
                // Hide window to tray instead of quitting
                // The daemon keeps running so audio continues
                if self.tray_handle.is_some() {
                    info!("Window close requested - minimizing to tray (daemon keeps running)");
                    // Minimize instead of close to keep subscriptions running
                    // iced daemon mode stops processing when all windows are closed
                    return iced::window::minimize(window_id, true);
                } else {
                    // No tray available - actually quit
                    info!("Window close requested - no tray, exiting");
                    self.cleanup();
                    return iced::exit();
                }
            }

            Message::TrayShowWindow => {
                // Restore minimized window and focus it
                info!("Tray: Restoring window");
                return iced::window::oldest().and_then(|id| {
                    Task::batch([
                        iced::window::minimize(id, false),
                        iced::window::gain_focus(id),
                    ])
                });
            }

            Message::TrayToggleMuteAll => {
                info!("Tray: Toggle mute all");
                let new_mute_state = !self.state.master_muted;
                self.cmd_set_master_mute(new_mute_state);

                // Update tray icon state
                if let Some(ref handle) = self.tray_handle {
                    handle.set_muted(new_mute_state);
                }
            }

            Message::TrayQuit => {
                info!("Tray: Quit requested");
                self.cleanup();
                return iced::exit();
            }

            // ==================== Daemon Events ====================
            Message::Daemon(event) => {
                self.handle_daemon_event(event);
            }

            _ => {
                // Handle remaining message types
                debug!("Unhandled message: {:?}", message);
            }
        }

        Task::none()
    }

    /// Render the application view.
    ///
    /// Bottom panel layout (Ableton/FL Studio style):
    /// - Header at top
    /// - Channel strips (full width, horizontally scrollable)
    /// - Apps panel (compact, below strips)
    /// - Collapsible bottom panel for selected channel detail
    pub fn view(&self, _window: iced::window::Id) -> Element<Message> {
        // Header bar
        let header = self.view_header();

        // Channel strips (full width)
        let channel_strips = self.view_channel_strips();

        // Apps panel (compact horizontal)
        let apps = apps_panel(
            &self.state.available_apps,
            &self.state.channels,
            self.state.dragging_app.as_ref(),
        );

        // Routing rules panel (shown inline when open)
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

        // Plugin browser/editor (shown when open)
        let plugin_panel: Element<Message> = if let Some(channel_id) = self.state.plugin_browser_channel {
            let channel_name = self.state.channel(channel_id)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| "Unknown".to_string());
            let chain_info = self.get_plugin_chain_info(channel_id);
            let available_plugins = self.plugin_manager.list_plugins(&PluginFilter::default());

            row![
                crate::ui::plugin_chain::plugin_chain_panel(channel_id, &channel_name, chain_info),
                Space::new().width(SPACING),
                crate::ui::plugin_chain::plugin_browser(channel_id, available_plugins),
            ]
            .spacing(SPACING)
            .into()
        } else if let Some((_channel_id, instance_id)) = self.state.plugin_editor_open {
            if let Some((plugin_name, params)) = self.get_plugin_editor_info(instance_id) {
                crate::ui::plugin_chain::plugin_editor(instance_id, &plugin_name, params)
            } else {
                Space::new().height(0).into()
            }
        } else {
            Space::new().height(0).into()
        };

        // Bottom detail panel
        let bottom_panel = self.view_bottom_panel();

        // Footer
        let footer = self.view_footer();

        // Main layout
        let content = column![
            header,
            Space::new().height(SPACING),
            container(channel_strips).height(Fill),
            Space::new().height(SPACING_SM),
            apps,
            rules_panel,
            plugin_panel,
            Space::new().height(SPACING_SM),
            bottom_panel,
            footer,
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

    /// View the bottom detail panel (Ableton-style).
    fn view_bottom_panel(&self) -> Element<Message> {
        if self.state.bottom_panel_expanded {
            // Expanded state: show drag handle + content
            let drag_handle = container(
                container(Space::new().width(60).height(4))
                    .style(|_| container::Style {
                        background: Some(Background::Color(SOOTMIX_DARK.border_emphasis)),
                        border: Border::default().rounded(2.0),
                        ..container::Style::default()
                    }),
            )
            .width(Fill)
            .height(8)
            .center_x(Fill)
            .center_y(8)
            .style(|_| container::Style {
                background: Some(Background::Color(SOOTMIX_DARK.border_subtle)),
                ..container::Style::default()
            });

            let panel_content: Element<Message> = if let Some(channel_id) = self.state.selected_channel {
                if let Some(channel) = self.state.channel(channel_id) {
                    Self::view_bottom_panel_content(channel)
                } else {
                    self.view_bottom_panel_empty()
                }
            } else {
                self.view_bottom_panel_empty()
            };

            column![
                drag_handle,
                container(panel_content)
                    .width(Fill)
                    .height(Length::Fixed(self.state.bottom_panel_height))
                    .style(|_| container::Style {
                        background: Some(Background::Color(SURFACE)),
                        border: Border::default()
                            .color(SOOTMIX_DARK.border_subtle)
                            .width(1.0),
                        ..container::Style::default()
                    }),
            ]
            .into()
        } else {
            // Collapsed state: just show expand bar
            button(
                container(text("▲ Show Detail").size(TEXT_CAPTION).color(TEXT_DIM))
                    .center_x(Fill)
                    .padding([SPACING_XS, 0.0]),
            )
            .width(Fill)
            .padding(0)
            .style(|_: &Theme, status| {
                let is_hovered = matches!(status, button::Status::Hovered);
                button::Style {
                    background: Some(Background::Color(if is_hovered {
                        SURFACE_LIGHT
                    } else {
                        SURFACE
                    })),
                    text_color: TEXT_DIM,
                    border: Border::default()
                        .color(SOOTMIX_DARK.border_subtle)
                        .width(1.0),
                    ..button::Style::default()
                }
            })
            .on_press(Message::ToggleBottomPanel)
            .into()
        }
    }

    /// Content for the bottom panel when a channel is selected.
    fn view_bottom_panel_content(channel: &MixerChannel) -> Element<'_, Message> {
        let id = channel.id;
        let channel_name = channel.name.clone();

        // Header row
        let title = text(channel_name).size(TEXT_HEADING).color(TEXT);
        let close_btn = button(text("▼ Hide").size(TEXT_CAPTION).color(TEXT_DIM))
            .padding([SPACING_XS, SPACING_SM])
            .style(|_: &Theme, status| {
                let is_hovered = matches!(status, button::Status::Hovered);
                button::Style {
                    background: Some(Background::Color(if is_hovered {
                        SURFACE_LIGHT
                    } else {
                        Color::TRANSPARENT
                    })),
                    text_color: TEXT_DIM,
                    border: Border::default().rounded(RADIUS_SM),
                    ..button::Style::default()
                }
            })
            .on_press(Message::ToggleBottomPanel);

        let header_row = row![title, Space::new().width(Fill), close_btn,]
            .align_y(Alignment::Center);

        // Content sections (horizontal layout)
        let eq_section = Self::view_bottom_eq_section(channel);
        let plugins_section = Self::view_bottom_plugins_section(channel);
        let routing_section = Self::view_bottom_routing_section(channel);
        let apps_section = Self::view_bottom_apps_section(channel);

        // Mute button
        let muted = channel.muted;
        let mute_btn = button(
            text(if muted { "UNMUTE" } else { "MUTE" })
                .size(TEXT_SMALL)
                .color(if muted { SOOTMIX_DARK.semantic_error } else { TEXT }),
        )
        .padding([SPACING_SM, SPACING])
        .style(move |_: &Theme, status| {
            let is_hovered = matches!(status, button::Status::Hovered);
            button::Style {
                background: Some(Background::Color(if muted {
                    if is_hovered { SOOTMIX_DARK.semantic_error } else { SOOTMIX_DARK.semantic_error.scale_alpha(0.3) }
                } else if is_hovered { SURFACE_LIGHT } else { SURFACE })),
                border: Border::default()
                    .rounded(RADIUS)
                    .color(if muted { SOOTMIX_DARK.semantic_error } else { SOOTMIX_DARK.border_subtle })
                    .width(1.0),
                ..button::Style::default()
            }
        })
        .on_press(Message::ChannelMuteToggled(id));

        let content_row = row![
            eq_section,
            Space::new().width(SPACING),
            plugins_section,
            Space::new().width(SPACING),
            routing_section,
            Space::new().width(SPACING),
            apps_section,
            Space::new().width(Fill),
            mute_btn,
        ]
        .align_y(Alignment::Start);

        let scrollable_content = scrollable(content_row)
            .direction(scrollable::Direction::Horizontal(
                scrollable::Scrollbar::default().width(4).scroller_width(4),
            ));

        column![header_row, Space::new().height(SPACING), scrollable_content,]
            .padding(SPACING)
            .into()
    }

    /// EQ section for bottom panel.
    fn view_bottom_eq_section(channel: &MixerChannel) -> Element<'_, Message> {
        let id = channel.id;
        let eq_enabled = channel.eq_enabled;
        container(
            column![
                text("EQ").size(TEXT_SMALL).color(TEXT_DIM),
                Space::new().height(SPACING_XS),
                container(Space::new().width(120).height(50))
                    .style(|_| container::Style {
                        background: Some(Background::Color(BACKGROUND)),
                        border: Border::default().rounded(RADIUS_SM).color(SOOTMIX_DARK.border_subtle).width(1.0),
                        ..container::Style::default()
                    }),
                Space::new().height(SPACING_XS),
                button(
                    text(if eq_enabled { "ON" } else { "OFF" })
                        .size(TEXT_CAPTION)
                        .color(if eq_enabled { TEXT } else { TEXT_DIM }),
                )
                .padding([SPACING_XS, SPACING_SM])
                .style(move |_: &Theme, _| button::Style {
                    background: Some(Background::Color(if eq_enabled {
                        SOOTMIX_DARK.semantic_success.scale_alpha(0.3)
                    } else { SURFACE })),
                    border: Border::default().rounded(RADIUS_SM),
                    ..button::Style::default()
                })
                .on_press(Message::ChannelEqToggled(id)),
            ]
            .align_x(Alignment::Center),
        )
        .padding(SPACING)
        .style(|_| container::Style {
            background: Some(Background::Color(BACKGROUND)),
            border: Border::default().rounded(RADIUS).color(SOOTMIX_DARK.border_subtle).width(1.0),
            ..container::Style::default()
        })
        .into()
    }

    /// Plugins section for bottom panel.
    fn view_bottom_plugins_section(channel: &MixerChannel) -> Element<'_, Message> {
        let id = channel.id;
        let plugin_count = channel.plugin_chain.len();
        container(
            column![
                text("Plugins").size(TEXT_SMALL).color(TEXT_DIM),
                Space::new().height(SPACING_XS),
                text(format!("{} loaded", plugin_count)).size(TEXT_BODY).color(TEXT),
                Space::new().height(SPACING_XS),
                button(text("Open Browser").size(TEXT_CAPTION).color(PRIMARY))
                    .padding([SPACING_XS, SPACING_SM])
                    .style(|_: &Theme, status| {
                        let is_hovered = matches!(status, button::Status::Hovered);
                        button::Style {
                            background: Some(Background::Color(if is_hovered { PRIMARY.scale_alpha(0.2) } else { Color::TRANSPARENT })),
                            text_color: PRIMARY,
                            border: Border::default().rounded(RADIUS_SM).color(PRIMARY).width(1.0),
                            ..button::Style::default()
                        }
                    })
                    .on_press(Message::OpenPluginBrowser(id)),
            ]
            .align_x(Alignment::Center),
        )
        .padding(SPACING)
        .width(Length::Fixed(140.0))
        .style(|_| container::Style {
            background: Some(Background::Color(BACKGROUND)),
            border: Border::default().rounded(RADIUS).color(SOOTMIX_DARK.border_subtle).width(1.0),
            ..container::Style::default()
        })
        .into()
    }

    /// Routing section for bottom panel.
    fn view_bottom_routing_section(channel: &MixerChannel) -> Element<'_, Message> {
        let output_name = channel.output_device_name.clone().unwrap_or_else(|| "Default".to_string());
        let volume_db = channel.volume_db;
        container(
            column![
                text("Output").size(TEXT_SMALL).color(TEXT_DIM),
                Space::new().height(SPACING_XS),
                text(output_name).size(TEXT_BODY).color(TEXT),
                Space::new().height(SPACING_XS),
                text(format!("{:+.1} dB", volume_db))
                    .size(TEXT_BODY)
                    .color(SOOTMIX_DARK.accent_warm),
            ]
            .align_x(Alignment::Center),
        )
        .padding(SPACING)
        .width(Length::Fixed(120.0))
        .style(|_| container::Style {
            background: Some(Background::Color(BACKGROUND)),
            border: Border::default().rounded(RADIUS).color(SOOTMIX_DARK.border_subtle).width(1.0),
            ..container::Style::default()
        })
        .into()
    }

    /// Apps section for bottom panel.
    fn view_bottom_apps_section(channel: &MixerChannel) -> Element<'_, Message> {
        let apps_count = channel.assigned_apps.len();
        container(
            column![
                text("Sources").size(TEXT_SMALL).color(TEXT_DIM),
                Space::new().height(SPACING_XS),
                text(format!("{} app{}", apps_count, if apps_count == 1 { "" } else { "s" }))
                    .size(TEXT_BODY)
                    .color(TEXT),
            ]
            .align_x(Alignment::Center),
        )
        .padding(SPACING)
        .width(Length::Fixed(100.0))
        .style(|_| container::Style {
            background: Some(Background::Color(BACKGROUND)),
            border: Border::default().rounded(RADIUS).color(SOOTMIX_DARK.border_subtle).width(1.0),
            ..container::Style::default()
        })
        .into()
    }

    /// Empty state for bottom panel.
    fn view_bottom_panel_empty(&self) -> Element<Message> {
        container(
            text("Select a channel to view details").size(TEXT_BODY).color(TEXT_DIM),
        )
        .width(Fill)
        .height(Fill)
        .center_x(Fill)
        .center_y(Fill)
        .into()
    }

    /// View the left sidebar (apps panel + routing rules).
    fn view_left_sidebar(&self) -> Element<Message> {
        use crate::ui::focus_panel::FOCUS_PANEL_WIDTH;

        // Sidebar width (same as focus panel for symmetry)
        let sidebar_width = if self.state.left_sidebar_collapsed {
            48.0 // Collapsed: just show toggle button
        } else {
            FOCUS_PANEL_WIDTH
        };

        // Toggle button
        let toggle_icon = if self.state.left_sidebar_collapsed { ">" } else { "<" };
        let toggle_btn = button(text(toggle_icon).size(TEXT_BODY).color(TEXT_DIM))
            .padding([SPACING_SM, SPACING])
            .style(|_theme: &Theme, status| {
                let is_hovered = matches!(status, button::Status::Hovered);
                button::Style {
                    background: Some(Background::Color(if is_hovered {
                        SURFACE_LIGHT
                    } else {
                        SURFACE
                    })),
                    text_color: TEXT_DIM,
                    border: Border::default().rounded(RADIUS_SM),
                    ..button::Style::default()
                }
            })
            .on_press(Message::ToggleLeftSidebar);

        if self.state.left_sidebar_collapsed {
            // Collapsed state: just show toggle button
            container(
                column![toggle_btn,]
                    .align_x(Alignment::Center)
                    .padding(SPACING_SM),
            )
            .width(Length::Fixed(sidebar_width))
            .height(Fill)
            .style(|_| container::Style {
                background: Some(Background::Color(SURFACE)),
                border: Border::default()
                    .rounded(RADIUS)
                    .color(SOOTMIX_DARK.border_subtle)
                    .width(1.0),
                ..container::Style::default()
            })
            .into()
        } else {
            // Expanded state: apps panel + routing rules
            let apps = apps_panel(
                &self.state.available_apps,
                &self.state.channels,
                self.state.dragging_app.as_ref(),
            );

            // Routing rules panel (inline in sidebar when open)
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

            let sidebar_content = column![
                row![
                    text("Apps & Routing").size(TEXT_SMALL).color(TEXT_DIM),
                    Space::new().width(Fill),
                    toggle_btn,
                ]
                .align_y(Alignment::Center),
                Space::new().height(SPACING_SM),
                apps,
                Space::new().height(SPACING),
                rules_panel,
            ]
            .padding(SPACING);

            let scrollable_sidebar = scrollable(sidebar_content)
                .direction(scrollable::Direction::Vertical(
                    scrollable::Scrollbar::default().width(4).scroller_width(4),
                ));

            container(scrollable_sidebar)
                .width(Length::Fixed(sidebar_width))
                .height(Fill)
                .style(|_| container::Style {
                    background: Some(Background::Color(SURFACE)),
                    border: Border::default()
                        .rounded(RADIUS)
                        .color(SOOTMIX_DARK.border_subtle)
                        .width(1.0),
                    ..container::Style::default()
                })
                .into()
        }
    }

    /// View the center panel (channel strips + footer).
    fn view_center_panel(&self) -> Element<Message> {
        let channel_strips = self.view_channel_strips();
        let footer = self.view_footer();

        column![
            channel_strips,
            Space::new().height(SPACING),
            footer,
        ]
        .width(Fill)
        .height(Fill)
        .into()
    }

    /// View the right panel (focus panel, plugin browser, or plugin editor).
    fn view_right_panel(&self) -> Element<Message> {
        use crate::ui::focus_panel::{focus_panel, focus_panel_empty, FocusPluginInfo};

        // Priority: Plugin editor > Plugin browser > Focus panel

        // Plugin editor takes precedence
        if let Some((_channel_id, instance_id)) = self.state.plugin_editor_open {
            if let Some((plugin_name, params)) = self.get_plugin_editor_info(instance_id) {
                return crate::ui::plugin_chain::plugin_editor(instance_id, &plugin_name, params);
            }
        }

        // Plugin browser is next
        if let Some(channel_id) = self.state.plugin_browser_channel {
            let channel_name = self.state.channel(channel_id)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| "Unknown".to_string());

            let chain_info = self.get_plugin_chain_info(channel_id);
            let available_plugins = self.plugin_manager.list_plugins(&PluginFilter::default());

            return column![
                crate::ui::plugin_chain::plugin_chain_panel(
                    channel_id,
                    &channel_name,
                    chain_info,
                ),
                Space::new().height(SPACING),
                crate::ui::plugin_chain::plugin_browser(channel_id, available_plugins),
            ]
            .spacing(SPACING)
            .into();
        }

        // Focus panel for selected channel
        if let Some(channel_id) = self.state.selected_channel {
            if let Some(channel) = self.state.channel(channel_id) {
                // Build plugin info for the focus panel
                let plugin_chain: Vec<FocusPluginInfo> = self.get_plugin_chain_info(channel_id)
                    .into_iter()
                    .map(|(instance_id, name, bypassed)| FocusPluginInfo {
                        instance_id,
                        name,
                        bypassed,
                    })
                    .collect();

                return focus_panel(channel, &self.state.available_outputs, plugin_chain);
            }
        }

        // No selection: show empty state
        focus_panel_empty()
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
        let selected_channel = self.state.selected_channel;

        // Build channel strip widgets
        let available_outputs = &self.state.available_outputs;
        let mut strips: Vec<Element<Message>> = self
            .state
            .channels
            .iter()
            .map(|c| {
                let is_selected = selected_channel == Some(c.id);
                channel_strip(c, dragging, editing, has_active_snapshot, available_outputs, is_selected)
            })
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
            self.state.master_recording_enabled,
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
        Subscription::batch([
            // Tick every 50ms to poll PipeWire events and tray messages
            iced::time::every(std::time::Duration::from_millis(50)).map(|_| Message::Tick),
            // Listen for window close requests
            iced::window::close_requests().map(Message::WindowCloseRequested),
            // Listen for daemon events via D-Bus
            daemon_client::daemon_subscription().map(Message::Daemon),
        ])
    }

    /// Send a command to the PipeWire thread (standalone mode only).
    fn send_pw_command(&self, cmd: PwCommand) {
        if let Some(ref thread) = self.pw_thread {
            if let Err(e) = thread.send(cmd) {
                error!("Failed to send command to PipeWire thread: {}", e);
            }
        }
    }

    // ==================== High-Level Audio Commands ====================
    // These methods work with both daemon mode and standalone mode.

    /// Create a new mixer channel.
    fn cmd_create_channel(&mut self, name: &str) -> Option<Uuid> {
        if self.daemon_connected {
            // In daemon mode, send command via D-Bus
            if let Err(e) = daemon_client::send_daemon_command(
                daemon_client::DaemonCommand::CreateChannel(name.to_string())
            ) {
                error!("Failed to send create channel command to daemon: {}", e);
            }
            // Channel will be added via ChannelAdded signal
            None
        } else {
            // In standalone mode, create channel locally
            let channel = MixerChannel::new(name);
            let id = channel.id;
            self.state.channels.push(channel);
            self.send_pw_command(PwCommand::CreateVirtualSink {
                channel_id: id,
                name: name.to_string(),
            });
            Some(id)
        }
    }

    /// Delete a mixer channel.
    fn cmd_delete_channel(&mut self, channel_id: Uuid) {
        if self.daemon_connected {
            if let Err(e) = daemon_client::send_daemon_command(
                daemon_client::DaemonCommand::DeleteChannel(channel_id.to_string())
            ) {
                error!("Failed to send delete channel command to daemon: {}", e);
            }
        } else {
            // Get channel info before removing
            let channel_info = self.state.channel(channel_id).map(|c| {
                (c.pw_sink_id, c.assigned_apps.clone(), c.is_managed, c.pw_eq_node_id)
            });

            if let Some((sink_node_id, assigned_apps, is_managed, eq_node_id)) = channel_info {
                // Destroy EQ filter if it exists
                if eq_node_id.is_some() {
                    if let Err(e) = filter_chain::destroy_eq_filter(channel_id) {
                        warn!("Failed to destroy EQ filter during channel deletion: {}", e);
                    }
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
            self.state.channels.retain(|c| c.id != channel_id);
        }
    }

    /// Rename a mixer channel.
    fn cmd_rename_channel(&mut self, channel_id: Uuid, new_name: &str) {
        if self.daemon_connected {
            if let Err(e) = daemon_client::send_daemon_command(
                daemon_client::DaemonCommand::RenameChannel {
                    id: channel_id.to_string(),
                    name: new_name.to_string(),
                }
            ) {
                error!("Failed to send rename channel command to daemon: {}", e);
            }
            // Update local state immediately for responsive UI
            if let Some(channel) = self.state.channel_mut(channel_id) {
                channel.name = new_name.to_string();
            }
        } else {
            // Get current channel info
            let channel_info = self.state.channel(channel_id).map(|c| {
                (c.pw_sink_id, c.name.clone(), c.assigned_apps.clone())
            });

            if let Some((sink_node_id, old_name, assigned_apps)) = channel_info {
                // Only update if name actually changed
                if new_name != old_name {
                    // Check if any apps are currently routed to this channel
                    let has_routed_apps = !assigned_apps.is_empty() &&
                        assigned_apps.iter().any(|app_id| {
                            self.state.available_apps.iter().any(|a| a.identifier() == *app_id)
                        });

                    if has_routed_apps {
                        // Apps are routed - only update description (seamless, no audio glitch)
                        info!("Renaming channel to '{}' (description only, apps routed)", new_name);
                        if let Some(node_id) = sink_node_id {
                            self.send_pw_command(PwCommand::UpdateSinkDescription {
                                node_id,
                                description: new_name.to_string(),
                            });
                        }
                    } else {
                        // No apps routed - full rename (recreate sink with new node.name)
                        info!("Renaming channel to '{}' (full rename, no apps routed)", new_name);
                        if let Some(node_id) = sink_node_id {
                            self.send_pw_command(PwCommand::DestroyVirtualSink { node_id });
                        }
                        self.send_pw_command(PwCommand::CreateVirtualSink {
                            channel_id,
                            name: new_name.to_string(),
                        });
                    }

                    // Update UI name
                    if let Some(channel) = self.state.channel_mut(channel_id) {
                        channel.name = new_name.to_string();
                    }
                }
            }
        }
    }

    /// Set channel volume in dB.
    fn cmd_set_channel_volume(&mut self, channel_id: Uuid, volume_db: f32) {
        if self.daemon_connected {
            if let Err(e) = daemon_client::send_daemon_command(
                daemon_client::DaemonCommand::SetChannelVolume {
                    id: channel_id.to_string(),
                    volume_db: volume_db as f64,
                }
            ) {
                error!("Failed to send set volume command to daemon: {}", e);
            }
            // Update local state immediately for responsive UI
            if let Some(channel) = self.state.channel_mut(channel_id) {
                channel.volume_db = volume_db;
            }
        } else {
            if let Some(channel) = self.state.channel_mut(channel_id) {
                channel.volume_db = volume_db;
                let linear = db_to_linear(if channel.muted { -60.0 } else { volume_db });
                if let Some(node_id) = channel.pw_loopback_output_id {
                    self.send_pw_command(PwCommand::SetVolume { node_id, volume: linear });
                }
            }
        }
    }

    /// Set channel mute state.
    fn cmd_set_channel_mute(&mut self, channel_id: Uuid, muted: bool) {
        if self.daemon_connected {
            if let Err(e) = daemon_client::send_daemon_command(
                daemon_client::DaemonCommand::SetChannelMute {
                    id: channel_id.to_string(),
                    muted,
                }
            ) {
                error!("Failed to send set mute command to daemon: {}", e);
            }
            // Update local state immediately for responsive UI
            if let Some(channel) = self.state.channel_mut(channel_id) {
                channel.muted = muted;
            }
        } else {
            if let Some(channel) = self.state.channel_mut(channel_id) {
                channel.muted = muted;
                if let Some(node_id) = channel.pw_loopback_output_id {
                    self.send_pw_command(PwCommand::SetMute { node_id, muted });
                }
            }
        }
    }

    /// Set master volume in dB.
    fn cmd_set_master_volume(&mut self, volume_db: f32) {
        self.state.master_volume_db = volume_db;
        if self.daemon_connected {
            if let Err(e) = daemon_client::send_daemon_command(
                daemon_client::DaemonCommand::SetMasterVolume(volume_db as f64)
            ) {
                error!("Failed to send set master volume command to daemon: {}", e);
            }
        } else {
            if let Some(node_id) = self.get_output_device_node_id() {
                let linear = db_to_linear(if self.state.master_muted { -60.0 } else { volume_db });
                self.send_pw_command(PwCommand::SetVolume { node_id, volume: linear });
            }
        }
    }

    /// Set master mute state.
    fn cmd_set_master_mute(&mut self, muted: bool) {
        self.state.master_muted = muted;
        if self.daemon_connected {
            if let Err(e) = daemon_client::send_daemon_command(
                daemon_client::DaemonCommand::SetMasterMute(muted)
            ) {
                error!("Failed to send set master mute command to daemon: {}", e);
            }
        } else {
            if let Some(node_id) = self.get_output_device_node_id() {
                self.send_pw_command(PwCommand::SetMute { node_id, muted });
            }
        }
    }

    /// Assign an app to a channel.
    fn cmd_assign_app(&mut self, app_node_id: u32, channel_id: Uuid) -> bool {
        if self.daemon_connected {
            if let Err(e) = daemon_client::send_daemon_command(
                daemon_client::DaemonCommand::AssignApp {
                    app_id: app_node_id.to_string(),
                    channel_id: channel_id.to_string(),
                }
            ) {
                error!("Failed to send assign app command to daemon: {}", e);
                return false;
            }
            true
        } else {
            // Standalone mode: create links directly
            self.route_app_to_channel_standalone(app_node_id, channel_id)
        }
    }

    /// Unassign an app from a channel.
    fn cmd_unassign_app(&mut self, app_node_id: u32, channel_id: Uuid) {
        if self.daemon_connected {
            if let Err(e) = daemon_client::send_daemon_command(
                daemon_client::DaemonCommand::UnassignApp {
                    app_id: app_node_id.to_string(),
                    channel_id: channel_id.to_string(),
                }
            ) {
                error!("Failed to send unassign app command to daemon: {}", e);
            }
        } else {
            // Standalone mode: destroy links and reconnect to default
            self.unroute_app_from_channel_standalone(app_node_id, channel_id);
        }
    }

    /// Set channel output device.
    fn cmd_set_channel_output(&mut self, channel_id: Uuid, device_name: Option<&str>) {
        info!("Channel {:?} output device changed to {:?}", channel_id, device_name);

        if self.daemon_connected {
            if let Err(e) = daemon_client::send_daemon_command(
                daemon_client::DaemonCommand::SetChannelOutput {
                    channel_id: channel_id.to_string(),
                    device_name: device_name.unwrap_or("").to_string(),
                }
            ) {
                error!("Failed to send set channel output command to daemon: {}", e);
            }
        } else {
            // Look up target device ID first to avoid borrow conflicts
            let target_device_id = device_name.and_then(|name| {
                self.state.available_outputs.iter()
                    .find(|d| d.description == name || d.name == name)
                    .map(|d| d.node_id)
            });

            // Get channel info for loopback lookup
            let channel_info = self.state.channel(channel_id).map(|c| {
                (c.pw_loopback_output_id, c.name.clone())
            });

            // Update channel state
            if let Some(channel) = self.state.channel_mut(channel_id) {
                channel.output_device_name = device_name.map(String::from);
                channel.output_device_id = target_device_id;
            }

            // Route to device
            if let Some((loopback_output_id, channel_name)) = channel_info {
                if let Some(loopback_id) = loopback_output_id {
                    self.send_pw_command(PwCommand::RouteChannelToDevice {
                        loopback_output_node: loopback_id,
                        target_device_id,
                    });
                } else {
                    // Try to find the loopback output node by name
                    let safe_name: String = channel_name
                        .chars()
                        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                        .collect();
                    let loopback_output_name = format!("output.sootmix.{}.output", safe_name);

                    if let Some(loopback_node) = self.state.pw_graph.nodes.values()
                        .find(|n| n.name == loopback_output_name)
                    {
                        let loopback_id = loopback_node.id;
                        // Update the channel with the found ID
                        if let Some(ch) = self.state.channel_mut(channel_id) {
                            ch.pw_loopback_output_id = Some(loopback_id);
                        }
                        self.send_pw_command(PwCommand::RouteChannelToDevice {
                            loopback_output_node: loopback_id,
                            target_device_id,
                        });
                    } else {
                        warn!("Loopback output node '{}' not found for routing", loopback_output_name);
                    }
                }
            }
        }
    }

    /// Set master output device.
    fn cmd_set_master_output(&mut self, device_name: Option<&str>) {
        self.state.output_device = device_name.map(String::from);
        if self.daemon_connected {
            if let Err(e) = daemon_client::send_daemon_command(
                daemon_client::DaemonCommand::SetMasterOutput(device_name.unwrap_or("").to_string())
            ) {
                error!("Failed to send set master output command to daemon: {}", e);
            }
        } else {
            if let Some(node_id) = self.get_output_device_node_id() {
                self.send_pw_command(PwCommand::SetDefaultSink { node_id });
            }
        }
    }

    /// Enable/disable master recording.
    fn cmd_set_master_recording(&mut self, enabled: bool) {
        if self.daemon_connected {
            if let Err(e) = daemon_client::send_daemon_command(
                daemon_client::DaemonCommand::SetMasterRecording(enabled)
            ) {
                error!("Failed to send set recording command to daemon: {}", e);
            }
        } else {
            if enabled {
                if !self.state.master_recording_enabled {
                    self.send_pw_command(PwCommand::CreateRecordingSource {
                        name: "master".to_string(),
                    });
                    self.state.master_recording_enabled = true;
                }
            } else {
                if let Some(node_id) = self.state.master_recording_source_id {
                    self.send_pw_command(PwCommand::DestroyRecordingSource { node_id });
                }
                self.state.master_recording_enabled = false;
                self.state.master_recording_source_id = None;
            }
        }
    }

    /// Route an app to a channel (standalone mode helper).
    fn route_app_to_channel_standalone(&mut self, app_node_id: u32, channel_id: Uuid) -> bool {
        let channel = match self.state.channel(channel_id) {
            Some(c) => c,
            None => return false,
        };
        let sink_id = match channel.pw_sink_id {
            Some(id) => id,
            None => return false,
        };

        // Get app identifier for tracking
        let app_identifier = self.state.available_apps.iter()
            .find(|a| a.node_id == app_node_id)
            .map(|a| a.identifier().to_string());

        // First, disconnect the app from any existing sinks (except our own)
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

        // Find port pairs and create links
        let port_pairs = self.state.pw_graph.find_port_pairs(app_node_id, sink_id);
        if port_pairs.is_empty() {
            warn!("No matching ports found for app {} -> sink {}", app_node_id, sink_id);
            return false;
        }

        for (output_port, input_port) in port_pairs {
            info!("Creating link: port {} -> port {}", output_port, input_port);
            self.send_pw_command(PwCommand::CreateLink { output_port, input_port });
        }

        // Add to assigned apps
        if let Some(identifier) = app_identifier {
            if let Some(channel) = self.state.channel_mut(channel_id) {
                if !channel.assigned_apps.contains(&identifier) {
                    channel.assigned_apps.push(identifier);
                }
            }
        }

        true
    }

    /// Unroute an app from a channel (standalone mode helper).
    fn unroute_app_from_channel_standalone(&mut self, app_node_id: u32, channel_id: Uuid) {
        let channel = match self.state.channel(channel_id) {
            Some(c) => c,
            None => return,
        };
        let sink_id = channel.pw_sink_id;

        // Get app identifier for tracking
        let app_identifier = self.state.available_apps.iter()
            .find(|a| a.node_id == app_node_id)
            .map(|a| a.identifier().to_string());

        // FIRST: Connect to default output (before destroying old links)
        // This ensures there's never a gap where the app has no audio output
        if let Some(default_output) = self.get_output_device_node_id() {
            info!("Reconnecting app {} to hardware sink {}", app_node_id, default_output);
            let port_pairs = self.state.pw_graph.find_port_pairs(app_node_id, default_output);
            for (output_port, input_port) in port_pairs {
                self.send_pw_command(PwCommand::CreateLink { output_port, input_port });
            }
        } else {
            warn!("No hardware sink found to reconnect app");
        }

        // THEN: Destroy links to our sink
        if let Some(sink_id) = sink_id {
            let links_to_destroy: Vec<u32> = self.state.pw_graph.links.values()
                .filter(|l| l.output_node == app_node_id && l.input_node == sink_id)
                .map(|l| l.id)
                .collect();
            for link_id in links_to_destroy {
                info!("Destroying link {} from app to channel sink", link_id);
                self.send_pw_command(PwCommand::DestroyLink { link_id });
            }
        }

        // Remove from assigned apps
        if let Some(identifier) = app_identifier {
            if let Some(channel) = self.state.channel_mut(channel_id) {
                channel.assigned_apps.retain(|a| a != &identifier);
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

    /// Pump sidechain levels from source channels to plugin parameters.
    ///
    /// For each plugin slot with a sidechain source configured, reads the source
    /// channel's meter level and sends it to any parameter with SidechainLevel hint.
    fn pump_sidechain_levels(&mut self) {
        use sootmix_plugin_api::ParameterHint;

        // Collect sidechain updates to avoid borrow conflicts
        let mut updates: Vec<(Uuid, Uuid, u32, f32)> = Vec::new();

        // Build a map of channel ID -> meter level
        let channel_levels: std::collections::HashMap<Uuid, f32> = self
            .state
            .channels
            .iter()
            .map(|ch| {
                let level = (ch.meter_display.level_left + ch.meter_display.level_right) / 2.0;
                (ch.id, level)
            })
            .collect();

        // Iterate over channels and their plugin chains
        for channel in &self.state.channels {
            for (slot_idx, slot_config) in channel.plugin_chain.iter().enumerate() {
                // Check if this slot has a sidechain source
                if let Some(source_id) = slot_config.sidechain_source {
                    // Get the source channel's meter level
                    if let Some(&source_level) = channel_levels.get(&source_id) {
                        // Get the plugin instance ID for this slot
                        if let Some(&instance_id) = channel.plugin_instances.get(slot_idx) {
                            // Find parameters with SidechainLevel hint
                            let params = self.plugin_manager.get_parameters(instance_id);
                            for param in params {
                                if param.hint == ParameterHint::SidechainLevel {
                                    updates.push((
                                        channel.id,
                                        instance_id,
                                        param.index,
                                        source_level,
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }

        // Apply the updates
        for (channel_id, instance_id, param_idx, value) in updates {
            // Update the plugin instance parameter
            self.plugin_manager.set_parameter(instance_id, param_idx, value);

            // Send to RT thread
            if let Some(ref pw) = self.pw_thread {
                let _ = pw.send(PwCommand::SendPluginParamUpdate {
                    channel_id,
                    instance_id,
                    param_index: param_idx,
                    value,
                });
            }
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

    /// Poll for tray messages and return a task if action needed.
    fn poll_tray_messages(&mut self) -> Option<Task<Message>> {
        let rx = self.tray_rx.as_ref()?;

        // Process all pending tray messages
        match rx.try_recv() {
            Ok(msg) => {
                debug!("Received tray message: {:?}", msg);
                match msg {
                    TrayMessage::ShowWindow => {
                        return Some(Task::done(Message::TrayShowWindow));
                    }
                    TrayMessage::ToggleMuteAll => {
                        return Some(Task::done(Message::TrayToggleMuteAll));
                    }
                    TrayMessage::Quit => {
                        return Some(Task::done(Message::TrayQuit));
                    }
                }
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                warn!("Tray message channel disconnected");
                self.tray_rx = None;
            }
        }

        None
    }

    /// Clean up resources before exiting.
    fn cleanup(&mut self) {
        info!("Cleaning up resources...");

        // Save current config
        self.save_config();

        // Destroy all virtual sinks
        crate::audio::virtual_sink::destroy_all_virtual_sinks();

        // The PipeWire thread will be dropped automatically
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
            PwEvent::VirtualSinkCreated { channel_id, node_id, loopback_output_node_id } => {
                info!("PwEvent: VirtualSinkCreated channel={} node={} loopback_output={:?}",
                    channel_id, node_id, loopback_output_node_id);

                // Get assigned apps and channel info before mutating
                let (assigned_apps, _channel_name, output_device_name) = self.state.channel(channel_id)
                    .map(|c| (c.assigned_apps.clone(), c.name.clone(), c.output_device_name.clone()))
                    .unwrap_or_default();

                // Get current volume settings before mutating
                let (volume_db, muted) = self.state.channel(channel_id)
                    .map(|c| (c.volume_db, c.muted))
                    .unwrap_or((0.0, false));

                if let Some(channel) = self.state.channel_mut(channel_id) {
                    channel.pw_sink_id = Some(node_id);
                    channel.pw_loopback_output_id = loopback_output_node_id;
                    info!("Updated channel '{}' pw_sink_id={}, loopback_output_id={:?}",
                          channel.name, node_id, loopback_output_node_id);
                }

                // Apply initial volume/mute to the loopback output node.
                // Request binding first to ensure native control works (falls back to CLI if not bound).
                // This is important because the registry listener might not have processed
                // the node yet when this event arrives.
                if let Some(loopback_id) = loopback_output_node_id {
                    let linear = db_to_linear(volume_db);
                    debug!("Applying initial volume to loopback output {}: db={:.1}, linear={:.3}",
                           loopback_id, volume_db, linear);
                    // Request bind to ensure node is ready for control
                    self.send_pw_command(PwCommand::BindNode { node_id: loopback_id });
                    // Set volume - will use CLI fallback if native binding not yet ready
                    self.send_pw_command(PwCommand::SetVolume { node_id: loopback_id, volume: linear });
                    self.send_pw_command(PwCommand::SetMute { node_id: loopback_id, muted });
                }

                // Route to saved output device if configured
                if let Some(loopback_id) = loopback_output_node_id {
                    let target_device_id = output_device_name.as_ref().and_then(|name| {
                        self.state.available_outputs.iter()
                            .find(|d| d.description == *name || d.name == *name)
                            .map(|d| d.node_id)
                    });

                    // If no saved device, route to default
                    self.send_pw_command(PwCommand::RouteChannelToDevice {
                        loopback_output_node: loopback_id,
                        target_device_id,
                    });
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
                // Note: Don't clear pending_reroute here - ports may not be discovered yet.
                // The retry logic in PortAdded will handle it once ports are available.
                if let Some((pending_channel_id, ref app_node_ids)) = self.state.pending_reroute.clone() {
                    if pending_channel_id == channel_id {
                        // Check if sink has ports yet
                        let sink_ports = self.state.pw_graph.input_ports_for_node(node_id);
                        if sink_ports.is_empty() {
                            debug!("Sink {} ports not ready yet, will retry re-routing in PortAdded", node_id);
                        } else {
                            info!("Re-routing {} apps to renamed sink {}", app_node_ids.len(), node_id);

                            let our_sink_ids: Vec<u32> = self.state.channels.iter()
                                .filter_map(|c| c.pw_sink_id)
                                .collect();
                            let default_sink_id = find_hardware_sink(&self.state.pw_graph, &our_sink_ids);

                            let mut any_routed = false;
                            for &app_node_id in app_node_ids.iter() {
                                let port_pairs = self.state.pw_graph.find_port_pairs(app_node_id, node_id);
                                if !port_pairs.is_empty() {
                                    any_routed = true;
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
                            }

                            // Only clear if we actually routed something
                            if any_routed {
                                self.state.pending_reroute = None;
                            }
                        }
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
            PwEvent::RecordingSourceCreated { name, node_id } => {
                info!("Recording source '{}' created with node_id={}", name, node_id);
                if name == "master" {
                    self.state.master_recording_source_id = Some(node_id);
                }
            }
            PwEvent::RecordingSourceDestroyed { node_id } => {
                info!("Recording source destroyed: node_id={}", node_id);
                if self.state.master_recording_source_id == Some(node_id) {
                    self.state.master_recording_source_id = None;
                    self.state.master_recording_enabled = false;
                }
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
        use crate::config::routing_rules::AppGrouping;

        let mut to_route = Vec::new();
        let group_by_app = self.state.routing_rules.app_grouping == AppGrouping::GroupByApp;

        for app in &self.state.available_apps {
            let app_id = app.identifier().to_string();

            // Skip if this node was already routed in this session
            if self.state.auto_routed_apps.contains(&app.node_id) {
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
                    if group_by_app {
                        // Route all nodes with the same identifier
                        info!("Reconnecting saved app '{}' (all streams) to its assigned channel", app_id);
                        let matching_nodes: Vec<_> = self.state.available_apps.iter()
                            .filter(|a| a.identifier() == app_id && !self.state.auto_routed_apps.contains(&a.node_id))
                            .map(|a| a.node_id)
                            .collect();
                        for node_id in matching_nodes {
                            to_route.push((node_id, app_id.clone(), channel_id));
                            self.state.auto_routed_apps.insert(node_id);
                        }
                    } else {
                        info!("Reconnecting saved app '{}' (node {}) to its assigned channel", app_id, app.node_id);
                        to_route.push((app.node_id, app_id.clone(), channel_id));
                        self.state.auto_routed_apps.insert(app.node_id);
                    }
                } else {
                    // Already connected, just mark as routed
                    self.state.auto_routed_apps.insert(app.node_id);
                }
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
                        let channel_id = channel.id;

                        if group_by_app {
                            // Route all nodes with the same identifier
                            info!("Auto-routing '{}' (all streams) to channel '{}' (rule: {})",
                                app.name, rule.target_channel, rule.name);
                            let matching_nodes: Vec<_> = self.state.available_apps.iter()
                                .filter(|a| a.identifier() == app_id && !self.state.auto_routed_apps.contains(&a.node_id))
                                .map(|a| a.node_id)
                                .collect();
                            for node_id in matching_nodes {
                                to_route.push((node_id, app_id.clone(), channel_id));
                                self.state.auto_routed_apps.insert(node_id);
                            }
                        } else {
                            info!("Auto-routing '{}' (node {}) to channel '{}' (rule: {})",
                                app.name, app.node_id, rule.target_channel, rule.name);
                            to_route.push((app.node_id, app_id.clone(), channel_id));
                            self.state.auto_routed_apps.insert(app.node_id);
                        }
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
                        output_device_name: c.output_device_name.clone(),
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
                    channel.output_device_name = saved.output_device_name;

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

                    // Create the virtual sink (uses channel name for node.name, readable in Helvum)
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

    /// Handle events received from the daemon via D-Bus.
    fn handle_daemon_event(&mut self, event: DaemonEvent) {
        use crate::daemon_client::DaemonEvent::*;

        match event {
            Connected => {
                info!("Connected to SootMix daemon - using daemon mode");
                self.daemon_connected = true;
                self.state.pw_connected = true;

                // Shutdown local PW thread if running (daemon handles audio)
                if let Some(pw) = self.pw_thread.take() {
                    info!("Shutting down local PipeWire thread (daemon takes over)");
                    pw.shutdown();
                }
                self.pw_event_rx = None;
            }
            Disconnected => {
                warn!("Disconnected from SootMix daemon");
                self.daemon_connected = false;
                self.state.pw_connected = false;

                // Spawn local PW thread as fallback
                if self.pw_thread.is_none() {
                    info!("Starting local PipeWire thread (standalone mode)");
                    let (event_tx, event_rx) = mpsc::channel();
                    match PwThread::spawn(event_tx) {
                        Ok(thread) => {
                            // Send shared plugin instances to PW thread
                            let shared_instances = self.plugin_manager.shared_instances();
                            if let Err(e) = thread.send(PwCommand::SetSharedPluginInstances(shared_instances)) {
                                error!("Failed to send shared plugin instances to PW thread: {:?}", e);
                            }
                            self.pw_thread = Some(thread);
                            self.pw_event_rx = Some(event_rx);
                            info!("PipeWire thread started in standalone mode");
                        }
                        Err(e) => {
                            error!("Failed to start PipeWire thread: {}", e);
                        }
                    }
                }
            }
            InitialState {
                channels,
                apps,
                outputs,
                master_volume,
                master_muted,
                master_output,
                connected,
                recording_enabled,
            } => {
                info!(
                    "Received initial state from daemon: {} channels, {} apps, {} outputs",
                    channels.len(),
                    apps.len(),
                    outputs.len()
                );

                // Update master state
                self.state.pw_connected = connected;
                self.state.master_volume_db = master_volume as f32;
                self.state.master_muted = master_muted;
                self.state.master_recording_enabled = recording_enabled;
                if !master_output.is_empty() {
                    self.state.output_device = Some(master_output);
                }

                // Sync channels from daemon
                self.state.channels.clear();
                for ch_info in channels {
                    if let Ok(id) = Uuid::parse_str(&ch_info.id) {
                        let channel = MixerChannel {
                            id,
                            name: ch_info.name,
                            volume_db: ch_info.volume_db as f32,
                            muted: ch_info.muted,
                            eq_enabled: ch_info.eq_enabled,
                            eq_preset: ch_info.eq_preset,
                            assigned_apps: ch_info.assigned_apps,
                            is_managed: true,
                            sink_name: None,
                            pw_sink_id: None,
                            pw_eq_node_id: None,
                            pw_loopback_output_id: None,
                            meter_display: crate::state::MeterDisplayState::default(),
                            plugin_chain: Vec::new(),
                            plugin_instances: Vec::new(),
                            meter_levels: Some(std::sync::Arc::new(crate::audio::meter_stream::AtomicMeterLevels::new())),
                            output_device_id: None,
                            output_device_name: if ch_info.output_device.is_empty() {
                                None
                            } else {
                                Some(ch_info.output_device)
                            },
                        };
                        self.state.channels.push(channel);
                    }
                }

                // Sync apps from daemon
                self.state.available_apps.clear();
                for app_info in apps {
                    self.state.available_apps.push(crate::state::AppInfo {
                        node_id: app_info.node_id,
                        name: app_info.name,
                        binary: if app_info.binary.is_empty() {
                            None
                        } else {
                            Some(app_info.binary)
                        },
                        icon: if app_info.icon.is_empty() {
                            None
                        } else {
                            Some(app_info.icon)
                        },
                    });
                }

                // Sync outputs from daemon
                self.state.available_outputs.clear();
                for output in outputs {
                    self.state.available_outputs.push(crate::audio::types::OutputDevice {
                        node_id: output.node_id,
                        name: output.name,
                        description: output.description,
                    });
                }

                self.state.startup_complete = true;
                info!("State synced from daemon - {} channels, {} apps",
                      self.state.channels.len(),
                      self.state.available_apps.len());
            }
            ChannelAdded(ch_info) => {
                info!("Daemon: Channel added: {}", ch_info.name);
                if let Ok(id) = Uuid::parse_str(&ch_info.id) {
                    // Check if channel already exists
                    if self.state.channel(id).is_none() {
                        let channel = MixerChannel {
                            id,
                            name: ch_info.name,
                            volume_db: ch_info.volume_db as f32,
                            muted: ch_info.muted,
                            eq_enabled: ch_info.eq_enabled,
                            eq_preset: ch_info.eq_preset,
                            assigned_apps: ch_info.assigned_apps,
                            is_managed: true,
                            sink_name: None,
                            pw_sink_id: None,
                            pw_eq_node_id: None,
                            pw_loopback_output_id: None,
                            meter_display: crate::state::MeterDisplayState::default(),
                            plugin_chain: Vec::new(),
                            plugin_instances: Vec::new(),
                            meter_levels: Some(std::sync::Arc::new(crate::audio::meter_stream::AtomicMeterLevels::new())),
                            output_device_id: None,
                            output_device_name: if ch_info.output_device.is_empty() {
                                None
                            } else {
                                Some(ch_info.output_device)
                            },
                        };
                        self.state.channels.push(channel);
                    }
                }
            }
            ChannelRemoved(channel_id) => {
                info!("Daemon: Channel removed: {}", channel_id);
                if let Ok(id) = Uuid::parse_str(&channel_id) {
                    self.state.channels.retain(|c| c.id != id);
                }
            }
            ChannelUpdated(ch_info) => {
                debug!("Daemon: Channel updated: {}", ch_info.name);
                if let Ok(id) = Uuid::parse_str(&ch_info.id) {
                    if let Some(channel) = self.state.channel_mut(id) {
                        channel.name = ch_info.name;
                        channel.volume_db = ch_info.volume_db as f32;
                        channel.muted = ch_info.muted;
                        channel.eq_enabled = ch_info.eq_enabled;
                        channel.eq_preset = ch_info.eq_preset;
                        channel.assigned_apps = ch_info.assigned_apps;
                        if !ch_info.output_device.is_empty() {
                            channel.output_device_name = Some(ch_info.output_device);
                        }
                    }
                }
            }
            VolumeChanged { channel_id, volume_db } => {
                if let Ok(id) = Uuid::parse_str(&channel_id) {
                    if let Some(channel) = self.state.channel_mut(id) {
                        channel.volume_db = volume_db as f32;
                    }
                }
            }
            MuteChanged { channel_id, muted } => {
                if let Ok(id) = Uuid::parse_str(&channel_id) {
                    if let Some(channel) = self.state.channel_mut(id) {
                        channel.muted = muted;
                    }
                }
            }
            AppDiscovered(app_info) => {
                debug!("Daemon: App discovered: {}", app_info.name);
                // Check if app already exists
                if !self.state.available_apps.iter().any(|a| a.node_id == app_info.node_id) {
                    self.state.available_apps.push(crate::state::AppInfo {
                        node_id: app_info.node_id,
                        name: app_info.name,
                        binary: if app_info.binary.is_empty() { None } else { Some(app_info.binary) },
                        icon: if app_info.icon.is_empty() { None } else { Some(app_info.icon) },
                    });
                }
            }
            AppRemoved(app_id) => {
                debug!("Daemon: App removed: {}", app_id);
                if let Ok(node_id) = app_id.parse::<u32>() {
                    self.state.available_apps.retain(|a| a.node_id != node_id);
                }
            }
            AppRouted { app_id, channel_id } => {
                debug!("Daemon: App {} routed to channel {}", app_id, channel_id);
                // The channel's assigned_apps should be updated via ChannelUpdated signal
            }
            AppUnrouted { app_id, channel_id } => {
                debug!("Daemon: App {} unrouted from channel {}", app_id, channel_id);
                // The channel's assigned_apps should be updated via ChannelUpdated signal
            }
            PipeWireConnectionChanged(connected) => {
                info!("PipeWire connection changed: {}", connected);
                self.state.pw_connected = connected;
            }
            MasterVolumeChanged(volume_db) => {
                self.state.master_volume_db = volume_db as f32;
            }
            MasterMuteChanged(muted) => {
                self.state.master_muted = muted;
            }
            Error(msg) => {
                error!("Daemon error: {}", msg);
            }
            MeterUpdate(data) => {
                // Update channel meter levels from daemon
                for meter in data {
                    let id = meter.channel_id();
                    if let Some(channel) = self.state.channel(id) {
                        // Meter data is already in dB from daemon
                        if let Some(ref meter_levels) = channel.meter_levels {
                            meter_levels.store(meter.level_left_db as f32, meter.level_right_db as f32);
                        }
                    }
                }
            }
            OutputsChanged => {
                debug!("Output devices changed - will refresh on next state query");
                // The full output list will be refreshed when needed
            }
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
