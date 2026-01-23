// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Application state management.

use crate::audio::types::{MediaClass, OutputDevice, PwLink, PwNode, PwPort};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// A virtual mixer channel created by SootMix.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MixerChannel {
    /// Unique identifier for this channel.
    pub id: Uuid,
    /// Display name.
    pub name: String,
    /// Volume in decibels (-60.0 to +12.0).
    pub volume_db: f32,
    /// Whether the channel is muted.
    pub muted: bool,
    /// Whether EQ is enabled for this channel.
    pub eq_enabled: bool,
    /// Name of the EQ preset applied.
    pub eq_preset: String,
    /// App identifiers assigned to this channel.
    pub assigned_apps: Vec<String>,
    /// Runtime PipeWire node ID for the virtual sink (not serialized).
    #[serde(skip)]
    pub pw_sink_id: Option<u32>,
    /// Runtime PipeWire node ID for the EQ filter-chain (not serialized).
    #[serde(skip)]
    pub pw_eq_node_id: Option<u32>,
}

impl MixerChannel {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            volume_db: 0.0,
            muted: false,
            eq_enabled: false,
            eq_preset: "Flat".to_string(),
            assigned_apps: Vec::new(),
            pw_sink_id: None,
            pw_eq_node_id: None,
        }
    }

    /// Convert volume in dB to linear scale (0.0 to ~4.0 for +12dB).
    pub fn volume_linear(&self) -> f32 {
        if self.muted {
            0.0
        } else {
            db_to_linear(self.volume_db)
        }
    }
}

/// Convert decibels to linear volume.
pub fn db_to_linear(db: f32) -> f32 {
    if db <= -60.0 {
        0.0
    } else {
        10.0_f32.powf(db / 20.0)
    }
}

/// Convert linear volume to decibels.
pub fn linear_to_db(linear: f32) -> f32 {
    if linear <= 0.0 {
        -60.0
    } else {
        20.0 * linear.log10()
    }
}

/// Information about an audio application.
#[derive(Debug, Clone)]
pub struct AppInfo {
    /// PipeWire node ID.
    pub node_id: u32,
    /// Application name (from PipeWire properties).
    pub name: String,
    /// Binary name for pattern matching.
    pub binary: Option<String>,
    /// Icon name hint (if available).
    pub icon: Option<String>,
}

impl AppInfo {
    /// Get identifier used for matching and assignment.
    pub fn identifier(&self) -> &str {
        self.binary.as_deref().unwrap_or(&self.name)
    }
}

/// Current PipeWire graph state.
#[derive(Debug, Default)]
pub struct PwGraphState {
    /// All known nodes by ID.
    pub nodes: HashMap<u32, PwNode>,
    /// All known ports by ID.
    pub ports: HashMap<u32, PwPort>,
    /// All known links by ID.
    pub links: HashMap<u32, PwLink>,
}

impl PwGraphState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get all audio playback streams (apps playing audio).
    pub fn playback_streams(&self) -> Vec<&PwNode> {
        self.nodes
            .values()
            .filter(|n| n.is_playback_stream())
            .collect()
    }

    /// Get all audio sinks (output devices and virtual sinks).
    pub fn audio_sinks(&self) -> Vec<&PwNode> {
        self.nodes.values().filter(|n| n.is_sink()).collect()
    }

    /// Get all output devices (hardware sinks, excluding our virtual ones).
    pub fn output_devices(&self, exclude_names: &[&str]) -> Vec<OutputDevice> {
        self.nodes
            .values()
            .filter(|n| {
                n.media_class == MediaClass::AudioSink
                    && !exclude_names.iter().any(|ex| n.name.contains(ex))
            })
            .map(|n| OutputDevice {
                node_id: n.id,
                name: n.name.clone(),
                description: n.description.clone(),
            })
            .collect()
    }

    /// Get ports for a specific node.
    pub fn ports_for_node(&self, node_id: u32) -> Vec<&PwPort> {
        self.ports.values().filter(|p| p.node_id == node_id).collect()
    }

    /// Find a link between two nodes.
    pub fn find_link(&self, output_node: u32, input_node: u32) -> Option<&PwLink> {
        self.links
            .values()
            .find(|l| l.output_node == output_node && l.input_node == input_node)
    }
}

/// Main application state.
#[derive(Debug)]
pub struct AppState {
    /// User-created mixer channels.
    pub channels: Vec<MixerChannel>,
    /// Master volume in dB.
    pub master_volume_db: f32,
    /// Master mute state.
    pub master_muted: bool,
    /// Selected output device name.
    pub output_device: Option<String>,
    /// Current preset name.
    pub current_preset: String,
    /// Available apps (populated from PipeWire).
    pub available_apps: Vec<AppInfo>,
    /// Available output devices (populated from PipeWire).
    pub available_outputs: Vec<OutputDevice>,
    /// Current PipeWire graph state.
    pub pw_graph: PwGraphState,
    /// Whether connected to PipeWire.
    pub pw_connected: bool,
    /// Currently open EQ panel (channel ID).
    pub eq_panel_channel: Option<Uuid>,
    /// Settings modal open.
    pub settings_open: bool,
    /// Last error message.
    pub last_error: Option<String>,
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

impl AppState {
    pub fn new() -> Self {
        Self {
            channels: Vec::new(),
            master_volume_db: 0.0,
            master_muted: false,
            output_device: None,
            current_preset: "Default".to_string(),
            available_apps: Vec::new(),
            available_outputs: Vec::new(),
            pw_graph: PwGraphState::new(),
            pw_connected: false,
            eq_panel_channel: None,
            settings_open: false,
            last_error: None,
        }
    }

    /// Find a channel by ID.
    pub fn channel(&self, id: Uuid) -> Option<&MixerChannel> {
        self.channels.iter().find(|c| c.id == id)
    }

    /// Find a channel by ID (mutable).
    pub fn channel_mut(&mut self, id: Uuid) -> Option<&mut MixerChannel> {
        self.channels.iter_mut().find(|c| c.id == id)
    }

    /// Get channel that has an app assigned.
    pub fn channel_for_app(&self, app_identifier: &str) -> Option<&MixerChannel> {
        self.channels
            .iter()
            .find(|c| c.assigned_apps.iter().any(|a| a == app_identifier))
    }

    /// Update available apps from PipeWire graph.
    pub fn update_available_apps(&mut self) {
        self.available_apps = self
            .pw_graph
            .playback_streams()
            .iter()
            .map(|node| AppInfo {
                node_id: node.id,
                name: node.app_name.clone().unwrap_or_else(|| node.name.clone()),
                binary: node.binary_name.clone(),
                icon: None,
            })
            .collect();
    }

    /// Update available outputs from PipeWire graph.
    pub fn update_available_outputs(&mut self) {
        // Exclude our virtual sinks from output device list
        let virtual_sink_names: Vec<&str> = self
            .channels
            .iter()
            .filter_map(|c| c.pw_sink_id.map(|_| c.name.as_str()))
            .collect();

        self.available_outputs = self.pw_graph.output_devices(&virtual_sink_names);
    }
}
