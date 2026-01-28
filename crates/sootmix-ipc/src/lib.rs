// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Shared IPC types and D-Bus interface definitions for SootMix.
//!
//! This crate defines the communication protocol between the SootMix daemon
//! and UI client via D-Bus.

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zbus::zvariant::Type;

/// D-Bus service name for the SootMix daemon.
pub const DBUS_NAME: &str = "com.sootmix.Daemon";

/// D-Bus object path for the main daemon interface.
pub const DBUS_PATH: &str = "/com/sootmix/Daemon";

/// D-Bus interface name.
pub const DBUS_INTERFACE: &str = "com.sootmix.Daemon";

/// Information about a mixer channel.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ChannelInfo {
    /// Unique identifier (UUID as string).
    pub id: String,
    /// Display name.
    pub name: String,
    /// Volume in decibels (-60.0 to +12.0).
    pub volume_db: f64,
    /// Whether the channel is muted.
    pub muted: bool,
    /// Whether EQ is enabled for this channel.
    pub eq_enabled: bool,
    /// Name of the EQ preset applied.
    pub eq_preset: String,
    /// App identifiers assigned to this channel.
    pub assigned_apps: Vec<String>,
    /// Output device name for this channel (empty string = default).
    pub output_device: String,
    /// Current meter levels (left, right) in dB.
    pub meter_levels: (f64, f64),
}

impl ChannelInfo {
    /// Create a new ChannelInfo with default values.
    pub fn new(id: Uuid, name: String) -> Self {
        Self {
            id: id.to_string(),
            name,
            volume_db: 0.0,
            muted: false,
            eq_enabled: false,
            eq_preset: "Flat".to_string(),
            assigned_apps: Vec::new(),
            output_device: String::new(),
            meter_levels: (-60.0, -60.0),
        }
    }

    /// Get the UUID from the string ID.
    pub fn uuid(&self) -> Option<Uuid> {
        Uuid::parse_str(&self.id).ok()
    }
}

/// Information about an audio application.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct AppInfo {
    /// Unique identifier (typically node_id as string for stability).
    pub id: String,
    /// Application name (from PipeWire properties).
    pub name: String,
    /// Binary name for pattern matching.
    pub binary: String,
    /// Icon name hint (if available).
    pub icon: String,
    /// PipeWire node ID.
    pub node_id: u32,
}

impl AppInfo {
    /// Get identifier used for matching and assignment.
    pub fn identifier(&self) -> &str {
        if !self.binary.is_empty() {
            &self.binary
        } else {
            &self.name
        }
    }
}

/// Information about an audio output device.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct OutputInfo {
    /// Device name (node.name from PipeWire).
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// PipeWire node ID.
    pub node_id: u32,
}

impl OutputInfo {
    /// Get the display name for the device.
    pub fn display_name(&self) -> &str {
        if !self.description.is_empty() {
            &self.description
        } else {
            &self.name
        }
    }
}

/// Information about an audio input device (microphone, line-in, etc).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct InputInfo {
    /// Device name (node.name from PipeWire).
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// PipeWire node ID.
    pub node_id: u32,
}

impl InputInfo {
    /// Get the display name for the device.
    pub fn display_name(&self) -> &str {
        if !self.description.is_empty() {
            &self.description
        } else {
            &self.name
        }
    }
}

/// Plugin slot configuration for a channel.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct PluginSlotInfo {
    /// Plugin identifier (e.g., "builtin:gain", "lv2:uri").
    pub plugin_id: String,
    /// Instance identifier (UUID as string).
    pub instance_id: String,
    /// Whether this plugin slot is bypassed.
    pub bypassed: bool,
    /// Sidechain source channel ID (empty string if none).
    pub sidechain_source: String,
}

/// Meter data for real-time level display.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type)]
pub struct MeterData {
    /// Channel ID (UUID as string - high 64 bits).
    pub channel_id_high: u64,
    /// Channel ID (UUID as string - low 64 bits).
    pub channel_id_low: u64,
    /// Left channel level in dB.
    pub level_left_db: f64,
    /// Right channel level in dB.
    pub level_right_db: f64,
    /// Left peak hold level in dB.
    pub peak_left_db: f64,
    /// Right peak hold level in dB.
    pub peak_right_db: f64,
}

impl MeterData {
    pub fn new(channel_id: Uuid, left: f64, right: f64, peak_left: f64, peak_right: f64) -> Self {
        let bytes = channel_id.as_u128();
        Self {
            channel_id_high: (bytes >> 64) as u64,
            channel_id_low: bytes as u64,
            level_left_db: left,
            level_right_db: right,
            peak_left_db: peak_left,
            peak_right_db: peak_right,
        }
    }

    pub fn channel_id(&self) -> Uuid {
        let high = (self.channel_id_high as u128) << 64;
        let low = self.channel_id_low as u128;
        Uuid::from_u128(high | low)
    }
}

/// Routing rule information.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct RoutingRuleInfo {
    /// Rule ID (UUID as string).
    pub id: String,
    /// Rule name.
    pub name: String,
    /// Whether the rule is enabled.
    pub enabled: bool,
    /// Match target: "name", "binary", or "either".
    pub match_target: String,
    /// Match type: "contains", "exact", "regex", or "glob".
    pub match_type: String,
    /// Pattern string.
    pub pattern: String,
    /// Target channel name.
    pub target_channel: String,
    /// Priority (lower = higher priority).
    pub priority: u32,
}

/// Error types for daemon operations.
#[derive(Debug, Clone, thiserror::Error)]
pub enum DaemonError {
    #[error("Channel not found: {0}")]
    ChannelNotFound(String),
    #[error("App not found: {0}")]
    AppNotFound(String),
    #[error("Output device not found: {0}")]
    OutputNotFound(String),
    #[error("PipeWire error: {0}")]
    PipeWireError(String),
    #[error("Invalid argument: {0}")]
    InvalidArgument(String),
    #[error("Internal error: {0}")]
    Internal(String),
}

impl From<DaemonError> for zbus::fdo::Error {
    fn from(e: DaemonError) -> Self {
        zbus::fdo::Error::Failed(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_channel_info_uuid() {
        let id = Uuid::new_v4();
        let channel = ChannelInfo::new(id, "Test".to_string());
        assert_eq!(channel.uuid(), Some(id));
    }

    #[test]
    fn test_meter_data_round_trip() {
        let id = Uuid::new_v4();
        let data = MeterData::new(id, -20.0, -18.0, -10.0, -8.0);
        assert_eq!(data.channel_id(), id);
        assert_eq!(data.level_left_db, -20.0);
        assert_eq!(data.level_right_db, -18.0);
    }
}
