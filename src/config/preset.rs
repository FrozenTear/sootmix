// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Global preset definitions (entire mixer state).

use serde::{Deserialize, Serialize};

/// Master channel settings in a preset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MasterPreset {
    pub volume_db: f32,
    pub muted: bool,
    pub output_device: Option<String>,
}

impl Default for MasterPreset {
    fn default() -> Self {
        Self {
            volume_db: 0.0,
            muted: false,
            output_device: None,
        }
    }
}

/// A single channel's configuration in a preset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelPreset {
    pub name: String,
    pub volume_db: f32,
    pub muted: bool,
    pub eq_enabled: bool,
    pub eq_preset: String,
    /// Patterns to match app names (regex or glob).
    pub app_patterns: Vec<String>,
}

impl ChannelPreset {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            volume_db: 0.0,
            muted: false,
            eq_enabled: false,
            eq_preset: "Flat".to_string(),
            app_patterns: Vec::new(),
        }
    }

    /// Check if an app name matches any pattern.
    pub fn matches_app(&self, app_name: &str) -> bool {
        let app_lower = app_name.to_lowercase();
        self.app_patterns
            .iter()
            .any(|p| app_lower.contains(&p.to_lowercase()))
    }
}

/// A complete mixer preset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalPreset {
    pub name: String,
    #[serde(default)]
    pub master: MasterPreset,
    #[serde(default)]
    pub channels: Vec<ChannelPreset>,
}

impl Default for GlobalPreset {
    fn default() -> Self {
        Self::empty("Default")
    }
}

impl GlobalPreset {
    /// Create an empty preset.
    pub fn empty(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            master: MasterPreset::default(),
            channels: Vec::new(),
        }
    }

    /// Create a gaming preset.
    pub fn gaming() -> Self {
        Self {
            name: "Gaming".to_string(),
            master: MasterPreset::default(),
            channels: vec![
                ChannelPreset {
                    name: "Game".to_string(),
                    volume_db: -6.0,
                    muted: false,
                    eq_enabled: true,
                    eq_preset: "Bass Boost".to_string(),
                    app_patterns: vec![
                        "steam".to_string(),
                        "wine".to_string(),
                        "gamescope".to_string(),
                        "lutris".to_string(),
                        "proton".to_string(),
                    ],
                },
                ChannelPreset {
                    name: "Voice".to_string(),
                    volume_db: 0.0,
                    muted: false,
                    eq_enabled: true,
                    eq_preset: "Vocal Clarity".to_string(),
                    app_patterns: vec![
                        "discord".to_string(),
                        "mumble".to_string(),
                        "teamspeak".to_string(),
                    ],
                },
                ChannelPreset {
                    name: "Music".to_string(),
                    volume_db: -12.0,
                    muted: false,
                    eq_enabled: false,
                    eq_preset: "Flat".to_string(),
                    app_patterns: vec![
                        "spotify".to_string(),
                        "rhythmbox".to_string(),
                        "clementine".to_string(),
                    ],
                },
                ChannelPreset {
                    name: "System".to_string(),
                    volume_db: -18.0,
                    muted: false,
                    eq_enabled: false,
                    eq_preset: "Flat".to_string(),
                    app_patterns: vec![], // Catch-all
                },
            ],
        }
    }

    /// Create a streaming preset.
    pub fn streaming() -> Self {
        Self {
            name: "Streaming".to_string(),
            master: MasterPreset::default(),
            channels: vec![
                ChannelPreset {
                    name: "Game".to_string(),
                    volume_db: -3.0,
                    muted: false,
                    eq_enabled: false,
                    eq_preset: "Flat".to_string(),
                    app_patterns: vec!["steam".to_string(), "wine".to_string()],
                },
                ChannelPreset {
                    name: "Browser".to_string(),
                    volume_db: -6.0,
                    muted: false,
                    eq_enabled: false,
                    eq_preset: "Flat".to_string(),
                    app_patterns: vec![
                        "firefox".to_string(),
                        "chrome".to_string(),
                        "chromium".to_string(),
                    ],
                },
                ChannelPreset {
                    name: "Alerts".to_string(),
                    volume_db: -12.0,
                    muted: false,
                    eq_enabled: false,
                    eq_preset: "Flat".to_string(),
                    app_patterns: vec!["obs".to_string(), "streamlabs".to_string()],
                },
            ],
        }
    }

    /// Get all built-in presets.
    pub fn builtin_presets() -> Vec<Self> {
        vec![Self::default(), Self::gaming(), Self::streaming()]
    }

    /// Load from TOML string.
    pub fn from_toml(s: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(s)
    }

    /// Serialize to TOML string.
    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(self)
    }
}
