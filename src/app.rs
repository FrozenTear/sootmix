// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Iced Application implementation for SootMix.

use crate::audio::{PwCommand, PwEvent, PwThread};
use crate::message::Message;
use crate::state::{AppState, MixerChannel};
use crate::ui::channel_strip::{channel_strip, master_strip};
use crate::ui::theme::{self, *};
use iced::widget::{button, column, container, row, scrollable, text, Space};
use iced::{Alignment, Background, Element, Fill, Length, Subscription, Task, Theme};
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
            Message::ChannelDeleted(id) => {
                // Destroy virtual sink if exists
                if let Some(channel) = self.state.channel(id) {
                    if let Some(node_id) = channel.pw_sink_id {
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
            }
            Message::PwError(err) => {
                error!("PipeWire error: {}", err);
                self.state.last_error = Some(err);
            }

            // ==================== Other ====================
            Message::Tick => {
                // Check for PipeWire events
                self.poll_pw_events();
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

        // Main content
        let content = column![
            header,
            Space::new().height(SPACING),
            channel_strips,
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
        // Build channel strip widgets
        let mut strips: Vec<Element<Message>> = self
            .state
            .channels
            .iter()
            .map(|c| channel_strip(c))
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
        // Clean up virtual sinks
        crate::audio::virtual_sink::destroy_all_virtual_sinks();

        // Shutdown PipeWire thread
        if let Some(thread) = self.pw_thread.take() {
            thread.shutdown();
        }
    }
}
