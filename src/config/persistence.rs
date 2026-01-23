// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Configuration persistence (save/load).

use crate::config::{AppConfig, EqPreset, GlobalPreset};
use directories::ProjectDirs;
use std::fs;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Failed to determine config directory")]
    NoConfigDir,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("TOML parse error: {0}")]
    TomlParse(#[from] toml::de::Error),
    #[error("TOML serialize error: {0}")]
    TomlSerialize(#[from] toml::ser::Error),
}

/// Manages configuration file persistence.
pub struct ConfigManager {
    config_dir: PathBuf,
    presets_dir: PathBuf,
    eq_presets_dir: PathBuf,
    state_dir: PathBuf,
}

impl ConfigManager {
    /// Create a new config manager, initializing directories.
    pub fn new() -> Result<Self, ConfigError> {
        let project_dirs =
            ProjectDirs::from("", "", "sootmix").ok_or(ConfigError::NoConfigDir)?;

        let config_dir = project_dirs.config_dir().to_path_buf();
        let presets_dir = config_dir.join("presets");
        let eq_presets_dir = config_dir.join("eq_presets");

        // State dir for runtime data (created node IDs for crash recovery)
        let state_dir = project_dirs
            .state_dir()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| config_dir.join("state"));

        // Ensure directories exist
        fs::create_dir_all(&config_dir)?;
        fs::create_dir_all(&presets_dir)?;
        fs::create_dir_all(&eq_presets_dir)?;
        fs::create_dir_all(&state_dir)?;

        Ok(Self {
            config_dir,
            presets_dir,
            eq_presets_dir,
            state_dir,
        })
    }

    /// Get the path to the main config file.
    pub fn config_path(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    /// Load the application config.
    pub fn load_config(&self) -> Result<AppConfig, ConfigError> {
        let path = self.config_path();
        if path.exists() {
            let content = fs::read_to_string(&path)?;
            Ok(AppConfig::from_toml(&content)?)
        } else {
            Ok(AppConfig::default())
        }
    }

    /// Save the application config.
    pub fn save_config(&self, config: &AppConfig) -> Result<(), ConfigError> {
        let content = config.to_toml()?;
        fs::write(self.config_path(), content)?;
        Ok(())
    }

    /// List available global presets.
    pub fn list_presets(&self) -> Result<Vec<String>, ConfigError> {
        let mut presets = Vec::new();

        // Add built-in preset names
        for preset in GlobalPreset::builtin_presets() {
            presets.push(preset.name);
        }

        // Add user presets
        if self.presets_dir.exists() {
            for entry in fs::read_dir(&self.presets_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension().map(|e| e == "toml").unwrap_or(false) {
                    if let Some(name) = path.file_stem() {
                        let name = name.to_string_lossy().to_string();
                        if !presets.contains(&name) {
                            presets.push(name);
                        }
                    }
                }
            }
        }

        Ok(presets)
    }

    /// Load a global preset by name.
    pub fn load_preset(&self, name: &str) -> Result<GlobalPreset, ConfigError> {
        // Check built-in presets first
        for preset in GlobalPreset::builtin_presets() {
            if preset.name.eq_ignore_ascii_case(name) {
                return Ok(preset);
            }
        }

        // Try to load from file
        let path = self.presets_dir.join(format!("{}.toml", name.to_lowercase()));
        if path.exists() {
            let content = fs::read_to_string(&path)?;
            Ok(GlobalPreset::from_toml(&content)?)
        } else {
            Ok(GlobalPreset::default())
        }
    }

    /// Save a global preset.
    pub fn save_preset(&self, preset: &GlobalPreset) -> Result<(), ConfigError> {
        let path = self
            .presets_dir
            .join(format!("{}.toml", preset.name.to_lowercase()));
        let content = preset.to_toml()?;
        fs::write(path, content)?;
        Ok(())
    }

    /// Delete a user preset.
    pub fn delete_preset(&self, name: &str) -> Result<(), ConfigError> {
        let path = self.presets_dir.join(format!("{}.toml", name.to_lowercase()));
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }

    /// List available EQ presets.
    pub fn list_eq_presets(&self) -> Result<Vec<String>, ConfigError> {
        let mut presets = Vec::new();

        // Add built-in preset names
        for preset in EqPreset::builtin_presets() {
            presets.push(preset.name);
        }

        // Add user presets
        if self.eq_presets_dir.exists() {
            for entry in fs::read_dir(&self.eq_presets_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension().map(|e| e == "toml").unwrap_or(false) {
                    if let Some(name) = path.file_stem() {
                        let name = name.to_string_lossy().to_string();
                        if !presets.contains(&name) {
                            presets.push(name);
                        }
                    }
                }
            }
        }

        Ok(presets)
    }

    /// Load an EQ preset by name.
    pub fn load_eq_preset(&self, name: &str) -> Result<EqPreset, ConfigError> {
        // Check built-in presets first
        for preset in EqPreset::builtin_presets() {
            if preset.name.eq_ignore_ascii_case(name) {
                return Ok(preset);
            }
        }

        // Try to load from file
        let path = self
            .eq_presets_dir
            .join(format!("{}.toml", name.to_lowercase()));
        if path.exists() {
            let content = fs::read_to_string(&path)?;
            Ok(EqPreset::from_toml(&content)?)
        } else {
            Ok(EqPreset::default())
        }
    }

    /// Save an EQ preset.
    pub fn save_eq_preset(&self, preset: &EqPreset) -> Result<(), ConfigError> {
        let path = self
            .eq_presets_dir
            .join(format!("{}.toml", preset.name.to_lowercase()));
        let content = preset.to_toml()?;
        fs::write(path, content)?;
        Ok(())
    }

    /// Path to state file for crash recovery (tracking created PW nodes).
    pub fn state_file_path(&self) -> PathBuf {
        self.state_dir.join("runtime_nodes.json")
    }

    /// Save runtime node IDs for crash recovery.
    pub fn save_runtime_nodes(&self, node_ids: &[u32]) -> Result<(), ConfigError> {
        let content = serde_json::to_string(node_ids).unwrap_or_else(|_| "[]".to_string());
        fs::write(self.state_file_path(), content)?;
        Ok(())
    }

    /// Load runtime node IDs from previous session (for cleanup).
    pub fn load_runtime_nodes(&self) -> Result<Vec<u32>, ConfigError> {
        let path = self.state_file_path();
        if path.exists() {
            let content = fs::read_to_string(&path)?;
            Ok(serde_json::from_str(&content).unwrap_or_default())
        } else {
            Ok(Vec::new())
        }
    }

    /// Clear runtime node state file.
    pub fn clear_runtime_nodes(&self) -> Result<(), ConfigError> {
        let path = self.state_file_path();
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }
}
