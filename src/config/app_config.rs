// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Application configuration (window settings, behavior).

use serde::{Deserialize, Serialize};

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
