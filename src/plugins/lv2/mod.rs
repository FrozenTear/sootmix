// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! LV2 plugin support for SootMix.
//!
//! This module provides loading and hosting of LV2 audio effect plugins using
//! the lilv library for plugin discovery and instantiation.
//!
//! # Architecture
//!
//! - `Lv2World` - Global singleton managing the Lilv World instance
//! - `Lv2PluginMeta` - Metadata for discovered LV2 plugins
//! - `Lv2PluginAdapter` - Wraps LV2 instance to implement AudioEffect trait
//! - `Lv2PluginLoader` - Handles scanning and loading LV2 plugins

mod adapter;
mod scanner;
mod world;

pub use adapter::Lv2PluginAdapter;
pub use scanner::Lv2PluginMeta;
pub use world::Lv2World;

use super::{PluginLoadError, PluginResult};
use sootmix_plugin_api::PluginBox;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{debug, info, warn};

/// Standard LV2 search paths on Linux.
pub const LV2_SEARCH_PATHS: &[&str] = &[
    "~/.lv2",
    "/usr/lib/lv2",
    "/usr/local/lib/lv2",
    "/usr/lib64/lv2",
    "/usr/local/lib64/lv2",
];

/// LV2 plugin loader.
///
/// Handles scanning for LV2 plugins and loading them via the Lilv library.
pub struct Lv2PluginLoader {
    /// Discovered plugins by URI.
    plugins: HashMap<String, Lv2PluginMeta>,
    /// Reference to the LV2 world singleton.
    world: Arc<Lv2World>,
}

impl Lv2PluginLoader {
    /// Create a new LV2 plugin loader.
    ///
    /// This initializes the Lilv world and scans standard LV2 paths.
    pub fn new() -> PluginResult<Self> {
        let world = Lv2World::global()?;

        Ok(Self {
            plugins: HashMap::new(),
            world,
        })
    }

    /// Scan for available LV2 plugins.
    ///
    /// Returns the number of plugins found.
    pub fn scan(&mut self) -> usize {
        self.plugins.clear();

        let discovered = scanner::scan_plugins(&self.world);
        let count = discovered.len();

        for meta in discovered {
            debug!("Found LV2 plugin: {} ({})", meta.name, meta.uri);
            self.plugins.insert(meta.uri.clone(), meta);
        }

        info!("LV2 scan complete: {} plugins found", count);
        count
    }

    /// Get search paths for LV2 plugins.
    pub fn search_paths() -> Vec<PathBuf> {
        let mut paths = Vec::new();

        // Check LV2_PATH environment variable first
        if let Ok(lv2_path) = std::env::var("LV2_PATH") {
            for path in lv2_path.split(':') {
                paths.push(PathBuf::from(path));
            }
        }

        // Add standard paths
        for path in LV2_SEARCH_PATHS {
            let expanded = if path.starts_with("~/") {
                if let Some(home) = dirs::home_dir() {
                    home.join(&path[2..])
                } else {
                    PathBuf::from(path)
                }
            } else {
                PathBuf::from(path)
            };

            if expanded.exists() {
                paths.push(expanded);
            }
        }

        paths
    }

    /// Get all discovered plugins.
    pub fn plugins(&self) -> impl Iterator<Item = &Lv2PluginMeta> {
        self.plugins.values()
    }

    /// Get a plugin by URI.
    pub fn get_plugin(&self, uri: &str) -> Option<&Lv2PluginMeta> {
        self.plugins.get(uri)
    }

    /// Load a plugin by URI.
    pub fn load(&self, uri: &str) -> PluginResult<PluginBox> {
        let meta = self.plugins.get(uri).ok_or_else(|| {
            PluginLoadError::Lv2Error(format!("Plugin not found: {}", uri))
        })?;

        let adapter = Lv2PluginAdapter::new(&self.world, meta)?;

        // Convert to PluginBox
        use abi_stable::sabi_trait::TD_Opaque;
        use sootmix_plugin_api::AudioEffect_TO;

        Ok(AudioEffect_TO::from_value(adapter, TD_Opaque))
    }

    /// Get number of discovered plugins.
    pub fn plugin_count(&self) -> usize {
        self.plugins.len()
    }
}

impl Default for Lv2PluginLoader {
    fn default() -> Self {
        Self::new().expect("Failed to initialize LV2 loader")
    }
}

/// Helper module for home directory expansion.
mod dirs {
    use std::path::PathBuf;

    pub fn home_dir() -> Option<PathBuf> {
        std::env::var("HOME").ok().map(PathBuf::from)
    }
}
