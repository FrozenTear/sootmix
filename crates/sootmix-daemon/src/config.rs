// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Configuration management for the daemon.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use thiserror::Error;
use tracing::{debug, info};
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("TOML parse error: {0}")]
    TomlParse(#[from] toml::de::Error),
    #[error("TOML serialize error: {0}")]
    TomlSerialize(#[from] toml::ser::Error),
    #[error("No config directory found")]
    NoConfigDir,
}

/// Plugin slot configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginSlotConfig {
    pub plugin_id: String,
    #[serde(default)]
    pub bypassed: bool,
    #[serde(default)]
    pub parameters: Vec<f32>,
    #[serde(default)]
    pub sidechain_source: Option<Uuid>,
}

/// Saved channel configuration for persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedChannel {
    pub id: Uuid,
    pub name: String,
    #[serde(default = "default_true")]
    pub is_managed: bool,
    pub sink_name: Option<String>,
    #[serde(default)]
    pub volume_db: f32,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub eq_enabled: bool,
    #[serde(default = "default_eq_preset")]
    pub eq_preset: String,
    #[serde(default)]
    pub assigned_apps: Vec<String>,
    #[serde(default)]
    pub plugin_chain: Vec<PluginSlotConfig>,
    #[serde(default)]
    pub output_device_name: Option<String>,
    /// Channel kind (Output or Input). Defaults to Output for backwards compatibility.
    #[serde(default)]
    pub kind: sootmix_ipc::ChannelKind,
    /// Input device name (for input channels - the microphone).
    #[serde(default)]
    pub input_device_name: Option<String>,
    /// Whether noise suppression is enabled for this channel.
    #[serde(default)]
    pub noise_suppression_enabled: bool,
    /// VAD threshold for noise suppression (0-100%). Higher = more aggressive noise gating.
    #[serde(default = "default_vad_threshold")]
    pub vad_threshold: f32,
    /// Hardware microphone gain in dB (-12.0 to +12.0). Controls the physical input device level.
    #[serde(default)]
    pub input_gain_db: f32,
}

fn default_vad_threshold() -> f32 {
    95.0
}

fn default_true() -> bool {
    true
}

fn default_eq_preset() -> String {
    "Flat".to_string()
}

/// Master output configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MasterConfig {
    #[serde(default)]
    pub volume_db: f32,
    #[serde(default)]
    pub muted: bool,
    pub output_device: Option<String>,
}

/// Complete mixer state configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MixerConfig {
    #[serde(default)]
    pub master: MasterConfig,
    #[serde(default)]
    pub channels: Vec<SavedChannel>,
}

impl MixerConfig {
    pub fn from_toml(s: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(s)
    }

    #[allow(dead_code)]
    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(self)
    }
}

/// Match target for routing rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MatchTarget {
    Name,
    Binary,
    Either,
}

impl Default for MatchTarget {
    fn default() -> Self {
        Self::Either
    }
}

/// Match type for routing rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MatchType {
    Contains(String),
    Exact(String),
    Regex(String),
    Glob(String),
}

impl MatchType {
    pub fn pattern(&self) -> &str {
        match self {
            MatchType::Contains(p) => p,
            MatchType::Exact(p) => p,
            MatchType::Regex(p) => p,
            MatchType::Glob(p) => p,
        }
    }

    pub fn type_name(&self) -> &str {
        match self {
            MatchType::Contains(_) => "contains",
            MatchType::Exact(_) => "exact",
            MatchType::Regex(_) => "regex",
            MatchType::Glob(_) => "glob",
        }
    }
}

/// A routing rule for auto-assigning apps to channels.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingRule {
    pub id: Uuid,
    pub name: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub match_target: MatchTarget,
    pub match_type: MatchType,
    pub target_channel: String,
    #[serde(default = "default_priority")]
    pub priority: u32,
}

fn default_priority() -> u32 {
    100
}

/// Collection of routing rules.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RoutingRulesConfig {
    #[serde(default)]
    pub rules: Vec<RoutingRule>,
}

impl RoutingRulesConfig {
    pub fn from_toml(s: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(s)
    }

    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(self)
    }

    #[allow(dead_code)]
    pub fn get_rule(&self, id: Uuid) -> Option<&RoutingRule> {
        self.rules.iter().find(|r| r.id == id)
    }

    #[allow(dead_code)]
    pub fn toggle_rule(&mut self, id: Uuid) {
        if let Some(rule) = self.rules.iter_mut().find(|r| r.id == id) {
            rule.enabled = !rule.enabled;
        }
    }

    #[allow(dead_code)]
    pub fn remove_rule(&mut self, id: Uuid) {
        self.rules.retain(|r| r.id != id);
    }
}

/// Configuration manager handles loading and saving config files.
pub struct ConfigManager {
    config_dir: PathBuf,
}

impl ConfigManager {
    /// Create a new config manager.
    pub fn new() -> Result<Self, ConfigError> {
        let config_dir = directories::ProjectDirs::from("com", "sootmix", "sootmix")
            .map(|d| d.config_dir().to_path_buf())
            .ok_or(ConfigError::NoConfigDir)?;

        // Ensure config directory exists
        fs::create_dir_all(&config_dir)?;

        debug!("Config directory: {:?}", config_dir);
        Ok(Self { config_dir })
    }

    /// Get the path to a config file.
    fn config_path(&self, name: &str) -> PathBuf {
        self.config_dir.join(name)
    }

    /// Load mixer configuration.
    pub fn load_mixer_config(&self) -> Result<MixerConfig, ConfigError> {
        let path = self.config_path("mixer.toml");
        if !path.exists() {
            debug!("No mixer config found, using defaults");
            return Ok(MixerConfig::default());
        }

        let content = fs::read_to_string(&path)?;
        let config = MixerConfig::from_toml(&content)?;
        info!("Loaded mixer config from {:?}", path);
        Ok(config)
    }

    /// Save mixer configuration.
    pub fn save_mixer_config(&self, config: &MixerConfig) -> Result<(), ConfigError> {
        let path = self.config_path("mixer.toml");
        let content = config.to_toml()?;
        fs::write(&path, content)?;
        debug!("Saved mixer config to {:?}", path);
        Ok(())
    }

    /// Load routing rules.
    pub fn load_routing_rules(&self) -> Result<RoutingRulesConfig, ConfigError> {
        let path = self.config_path("routing_rules.toml");
        if !path.exists() {
            debug!("No routing rules found, using defaults");
            return Ok(RoutingRulesConfig::default());
        }

        let content = fs::read_to_string(&path)?;
        let config = RoutingRulesConfig::from_toml(&content)?;
        info!("Loaded {} routing rules", config.rules.len());
        Ok(config)
    }

    /// Save routing rules.
    #[allow(dead_code)]
    pub fn save_routing_rules(&self, config: &RoutingRulesConfig) -> Result<(), ConfigError> {
        let path = self.config_path("routing_rules.toml");
        let content = config.to_toml()?;
        fs::write(&path, content)?;
        debug!("Saved routing rules to {:?}", path);
        Ok(())
    }
}
