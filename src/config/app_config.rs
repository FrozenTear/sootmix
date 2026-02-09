// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Application configuration (window settings, behavior).

#![allow(dead_code, unused_imports)]

use crate::plugins::PluginSlotConfig;
use crate::state::ChannelKind;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Window position and size settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowConfig {
    pub width: u32,
    pub height: u32,
    pub x: Option<i32>,
    pub y: Option<i32>,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            width: 900,
            height: 600,
            x: None,
            y: None,
        }
    }
}

/// General application settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    /// Start minimized to tray.
    pub start_minimized: bool,
    /// Minimize to tray instead of closing.
    pub minimize_to_tray: bool,
    /// Restore previous session on startup.
    pub restore_on_startup: bool,
    /// Last used preset name.
    pub current_preset: String,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            start_minimized: false,
            minimize_to_tray: false,
            restore_on_startup: true,
            current_preset: "Default".to_string(),
        }
    }
}

/// Appearance settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppearanceConfig {
    /// Theme name (currently only "dark").
    pub theme: String,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            theme: "dark".to_string(),
        }
    }
}

/// Complete application configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub window: WindowConfig,
    #[serde(default)]
    pub general: GeneralConfig,
    #[serde(default)]
    pub appearance: AppearanceConfig,
}

impl AppConfig {
    /// Load config from TOML string.
    pub fn from_toml(s: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(s)
    }

    /// Serialize to TOML string.
    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(self)
    }
}

/// Saved channel configuration for persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedChannel {
    /// Unique identifier.
    pub id: Uuid,
    /// Display name.
    pub name: String,
    /// Whether this is a managed (SootMix-created) or adopted (existing) sink.
    pub is_managed: bool,
    /// PipeWire sink name for matching on startup.
    pub sink_name: Option<String>,
    /// Volume in decibels.
    pub volume_db: f32,
    /// Whether muted.
    pub muted: bool,
    /// Whether EQ is enabled.
    pub eq_enabled: bool,
    /// EQ preset name.
    pub eq_preset: String,
    /// Assigned app identifiers.
    pub assigned_apps: Vec<String>,
    /// Plugin chain configuration.
    #[serde(default)]
    pub plugin_chain: Vec<PluginSlotConfig>,
    /// Output device name for per-channel routing (None = default output).
    #[serde(default)]
    pub output_device_name: Option<String>,
    /// Channel kind (Output or Input). Defaults to Output for backwards compatibility.
    #[serde(default)]
    pub kind: ChannelKind,
    /// Input device name (for input channels).
    #[serde(default)]
    pub input_device_name: Option<String>,
    /// Whether sidetone (input monitoring) is enabled.
    #[serde(default)]
    pub sidetone_enabled: bool,
    /// Sidetone volume in dB.
    #[serde(default = "default_sidetone_db")]
    pub sidetone_volume_db: f32,
}

fn default_sidetone_db() -> f32 {
    -20.0
}

/// Master output configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MasterConfig {
    /// Master volume in decibels.
    #[serde(default)]
    pub volume_db: f32,
    /// Master muted state.
    #[serde(default)]
    pub muted: bool,
    /// Selected output device name.
    pub output_device: Option<String>,
}

/// Complete mixer state configuration for persistence.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MixerConfig {
    /// Master output settings.
    #[serde(default)]
    pub master: MasterConfig,
    /// Saved channel configurations.
    #[serde(default)]
    pub channels: Vec<SavedChannel>,
}

impl MixerConfig {
    /// Load config from TOML string.
    pub fn from_toml(s: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(s)
    }

    /// Serialize to TOML string.
    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(self)
    }
}
