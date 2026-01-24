// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Iced Application implementation for SootMix.

use crate::audio::{PwCommand, PwEvent, PwThread};
use crate::message::Message;
use crate::state::{AppState, MixerChannel};
use crate::ui::apps_panel::apps_panel;
use crate::ui::channel_strip::{channel_strip, master_strip};
use crate::ui::theme::{self, *};
use iced::widget::{button, column, container, row, scrollable, text, Space};
use iced::{Alignment, Background, Element, Fill, Subscription, Task, Theme};
use std::sync::mpsc;
use tracing::{debug, error, info, warn};

/// Main application state.
pub struct SootMix {
    /// Application state.
    state: AppState,
    /// PipeWire thread handle.
    pw_thread: Option<PwThread>,
    /// Receiver for PipeWire events.
    pw_event_rx: Option<mpsc::Receiver<PwEvent>>,
}

impl SootMix {
    /// Create a new application instance.
    pub fn new() -> (Self, Task<Message>) {
        let state = AppState::new();

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

        let app = Self {
            state,
            pw_thread,
            pw_event_rx: Some(event_rx),
        };

        (app, Task::none())
    }

    /// Application title.
    pub fn title(&self) -> String {
        "SootMix".to_string()
    }

    /// Handle messages.
    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            // ==================== Channel Actions ====================
            Message::ChannelVolumeChanged(id, volume) => {
                if let Some(channel) = self.state.channel_mut(id) {
                    channel.volume_db = volume;
                    // Send volume update to PipeWire if we have a sink
                    if let Some(node_id) = channel.pw_sink_id {
                        let linear_vol = channel.volume_linear();
                        self.send_pw_command(PwCommand::SetVolume {
                            node_id,
                            volume: linear_vol,
                        });
                    }
                }
            }
            Message::ChannelVolumeReleased(id) => {
                // Could trigger save here
                debug!("Volume released for channel {}", id);
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
                if let Some(channel) = self.state.channel_mut(id) {
                    channel.eq_enabled = !channel.eq_enabled;
                    // TODO: Load/unload EQ filter
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

                            // Find default sink for temporary routing
                            let our_sink_ids: Vec<u32> = self.state.channels.iter()
                                .filter_map(|c| c.pw_sink_id)
                                .collect();
                            let default_sink_id = self.state.pw_graph.nodes.values()
                                .find(|n| n.media_class == crate::audio::types::MediaClass::AudioSink
                                      && !our_sink_ids.contains(&n.id)
                                      && !n.name.starts_with("sootmix."))
                                .map(|n| n.id);

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
                // First, reconnect all assigned apps to default sink before destroying
                if let Some(channel) = self.state.channel(id) {
                    let sink_node_id = channel.pw_sink_id;
                    let assigned_apps = channel.assigned_apps.clone();

                    // Find default sink
                    let our_sink_ids: Vec<u32> = self.state.channels.iter()
                        .filter_map(|c| c.pw_sink_id)
                        .collect();
                    let default_sink_id = self.state.pw_graph.nodes.values()
                        .find(|n| n.media_class == crate::audio::types::MediaClass::AudioSink
                              && !our_sink_ids.contains(&n.id)
                              && !n.name.starts_with("sootmix."))
                        .map(|n| n.id);

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

                    // Wait a bit more before destroying sink
                    std::thread::sleep(std::time::Duration::from_millis(50));

                    // Now destroy the virtual sink
                    if let Some(node_id) = sink_node_id {
                        self.send_pw_command(PwCommand::DestroyVirtualSink { node_id });
                    }
                }
                self.state.channels.retain(|c| c.id != id);
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

                        // Debug: show available ports
                        let app_out_ports = self.state.pw_graph.output_ports_for_node(app_node_id);
                        let sink_in_ports = self.state.pw_graph.input_ports_for_node(sink_node_id);
                        debug!("App {} output ports: {:?}", app_node_id,
                            app_out_ports.iter().map(|p| (p.id, &p.name)).collect::<Vec<_>>());
                        debug!("Sink {} input ports: {:?}", sink_node_id,
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

                // Find a default sink to reconnect to (any Audio/Sink that isn't ours)
                let our_sink_ids: Vec<u32> = self.state.channels.iter()
                    .filter_map(|c| c.pw_sink_id)
                    .collect();
                let default_sink = self.state.pw_graph.nodes.values()
                    .find(|n| n.media_class == crate::audio::types::MediaClass::AudioSink
                          && !our_sink_ids.contains(&n.id)
                          && !n.name.starts_with("sootmix."));

                if let (Some(app_node_id), Some(sink_node_id)) = (app_node_id, sink_node_id) {
                    // FIRST: Connect to default sink (before destroying old links)
                    // This ensures there's never a gap where the app has no audio output
                    if let Some(default_sink) = default_sink {
                        info!("Reconnecting app {} to default sink {} ({})", app_node_id, default_sink.id, default_sink.name);
                        let port_pairs = self.state.pw_graph.find_port_pairs(app_node_id, default_sink.id);
                        for (output_port, input_port) in port_pairs {
                            self.send_pw_command(PwCommand::CreateLink {
                                output_port,
                                input_port,
                            });
                        }
                    } else {
                        warn!("No default sink found to reconnect app");
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
                // TODO: Apply to actual output
            }
            Message::MasterVolumeReleased => {
                debug!("Master volume released");
            }
            Message::MasterMuteToggled => {
                self.state.master_muted = !self.state.master_muted;
            }
            Message::OutputDeviceChanged(device) => {
                self.state.output_device = Some(device);
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
            Message::PwVirtualSinkCreated(channel_id, node_id) => {
                info!("Virtual sink created for channel {}: node {}", channel_id, node_id);
                if let Some(channel) = self.state.channel_mut(channel_id) {
                    channel.pw_sink_id = Some(node_id);
                }

                // Check if there are apps waiting to be re-routed to this channel
                if let Some((pending_channel_id, ref app_node_ids)) = self.state.pending_reroute.clone() {
                    if pending_channel_id == channel_id {
                        info!("Re-routing {} apps to renamed sink {}", app_node_ids.len(), node_id);

                        // Find default sink to disconnect from
                        let our_sink_ids: Vec<u32> = self.state.channels.iter()
                            .filter_map(|c| c.pw_sink_id)
                            .collect();
                        let default_sink_id = self.state.pw_graph.nodes.values()
                            .find(|n| n.media_class == crate::audio::types::MediaClass::AudioSink
                                  && !our_sink_ids.contains(&n.id)
                                  && !n.name.starts_with("sootmix."))
                            .map(|n| n.id);

                        for &app_node_id in app_node_ids.iter() {
                            // Connect to new sink
                            let port_pairs = self.state.pw_graph.find_port_pairs(app_node_id, node_id);
                            if port_pairs.is_empty() {
                                // Ports not available yet, will retry on next tick
                                debug!("Ports not available yet for re-routing, will retry");
                                return Task::none();
                            }
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
            Message::PwError(err) => {
                error!("PipeWire error: {}", err);
                self.state.last_error = Some(err);
            }

            // ==================== Other ====================
            Message::Tick => {
                // Check for PipeWire events
                self.poll_pw_events();

                // Retry pending re-routing if sink ports are now available
                if let Some((channel_id, ref app_node_ids)) = self.state.pending_reroute.clone() {
                    if let Some(sink_node_id) = self.state.channel(channel_id).and_then(|c| c.pw_sink_id) {
                        // Check if sink has ports now
                        let sink_ports = self.state.pw_graph.input_ports_for_node(sink_node_id);

                        if !sink_ports.is_empty() {
                            // Find default sink to disconnect from
                            let our_sink_ids: Vec<u32> = self.state.channels.iter()
                                .filter_map(|c| c.pw_sink_id)
                                .collect();
                            let default_sink_id = self.state.pw_graph.nodes.values()
                                .find(|n| n.media_class == crate::audio::types::MediaClass::AudioSink
                                      && !our_sink_ids.contains(&n.id)
                                      && !n.name.starts_with("sootmix."))
                                .map(|n| n.id);

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

        // Main content
        let content = column![
            header,
            Space::new().height(SPACING),
            channel_strips,
            Space::new().height(SPACING),
            apps,
            Space::new().height(SPACING),
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

        let settings_button = button(text("Settings").size(12))
            .padding([6, 12])
            .style(|theme: &Theme, status| button::Style {
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
            settings_button,
        ]
        .align_y(Alignment::Center)
        .into()
    }

    /// View the channel strips area.
    fn view_channel_strips(&self) -> Element<Message> {
        let dragging = self.state.dragging_app.as_ref();
        let editing = self.state.editing_channel.as_ref();

        // Build channel strip widgets
        let mut strips: Vec<Element<Message>> = self
            .state
            .channels
            .iter()
            .map(|c| channel_strip(c, dragging, editing))
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
            self.state.output_device.as_deref(),
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

    /// View the footer with add channel button.
    fn view_footer(&self) -> Element<Message> {
        let add_button = button(text("+ New Channel").size(14))
            .padding([10, 20])
            .style(|theme: &Theme, status| button::Style {
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
                if let Some(channel) = self.state.channel_mut(channel_id) {
                    channel.pw_sink_id = Some(node_id);
                }
            }
            PwEvent::VirtualSinkDestroyed { node_id } => {
                for channel in &mut self.state.channels {
                    if channel.pw_sink_id == Some(node_id) {
                        channel.pw_sink_id = None;
                    }
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
}

impl Drop for SootMix {
    fn drop(&mut self) {
        info!("SootMix shutting down, reconnecting apps to default sink...");

        // Find default sink (any Audio/Sink that isn't ours)
        let our_sink_ids: Vec<u32> = self.state.channels.iter()
            .filter_map(|c| c.pw_sink_id)
            .collect();
        let default_sink_id = self.state.pw_graph.nodes.values()
            .find(|n| n.media_class == crate::audio::types::MediaClass::AudioSink
                  && !our_sink_ids.contains(&n.id)
                  && !n.name.starts_with("sootmix."))
            .map(|n| n.id);

        // Reconnect all assigned apps to default sink before destroying
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

        // Clean up virtual sinks
        crate::audio::virtual_sink::destroy_all_virtual_sinks();

        // Shutdown PipeWire thread
        if let Some(thread) = self.pw_thread.take() {
            thread.shutdown();
        }
    }
}
