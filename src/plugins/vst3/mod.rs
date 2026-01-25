// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! VST3 plugin support for SootMix.
//!
//! This module provides loading and hosting of VST3 audio effect plugins.
//!
//! Note: VST3 support is currently a work in progress. The module structure
//! is in place but full implementation requires additional work to match
//! the vst3 crate API.

mod factory;
mod scanner;

pub use scanner::Vst3PluginMeta;

use super::{PluginLoadError, PluginResult};
use sootmix_plugin_api::PluginBox;
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::{debug, info, warn};

/// Standard VST3 search paths on Linux.
pub const VST3_SEARCH_PATHS: &[&str] = &[
    "~/.vst3",
    "/usr/lib/vst3",
    "/usr/local/lib/vst3",
    "/usr/lib64/vst3",
    "/usr/local/lib64/vst3",
];

/// VST3 plugin loader.
///
/// Handles scanning for VST3 plugins and loading them.
pub struct Vst3PluginLoader {
    /// Discovered plugins by class ID.
    plugins: HashMap<String, Vst3PluginMeta>,
}

impl Vst3PluginLoader {
    /// Create a new VST3 plugin loader.
    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
        }
    }

    /// Scan for available VST3 plugins.
    ///
    /// Returns the number of plugins found.
    pub fn scan(&mut self) -> usize {
        self.plugins.clear();

        let paths = Self::search_paths();
        let mut count = 0;

        for search_path in paths {
            if !search_path.exists() {
                continue;
            }

            match self.scan_directory(&search_path) {
                Ok(n) => {
                    count += n;
                    debug!("Found {} VST3 plugins in {:?}", n, search_path);
                }
                Err(e) => {
                    warn!("Failed to scan VST3 directory {:?}: {}", search_path, e);
                }
            }
        }

        info!("VST3 scan complete: {} plugins found", count);
        count
    }

    /// Scan a single directory for VST3 bundles.
    fn scan_directory(&mut self, dir: &std::path::Path) -> std::io::Result<usize> {
        let mut count = 0;

        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            // VST3 bundles have .vst3 extension (directories)
            if path.extension().and_then(|e| e.to_str()) == Some("vst3") {
                match scanner::scan_bundle(&path) {
                    Ok(plugins) => {
                        for meta in plugins {
                            debug!("Found VST3 plugin: {} ({})", meta.name, meta.class_id);
                            self.plugins.insert(meta.class_id.clone(), meta);
                            count += 1;
                        }
                    }
                    Err(e) => {
                        warn!("Failed to scan VST3 bundle {:?}: {}", path, e);
                    }
                }
            }
        }

        Ok(count)
    }

    /// Get search paths for VST3 plugins.
    pub fn search_paths() -> Vec<PathBuf> {
        let mut paths = Vec::new();

        for path in VST3_SEARCH_PATHS {
            let expanded = if path.starts_with("~/") {
                if let Some(home) = std::env::var("HOME").ok().map(PathBuf::from) {
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
    pub fn plugins(&self) -> impl Iterator<Item = &Vst3PluginMeta> {
        self.plugins.values()
    }

    /// Get a plugin by class ID.
    pub fn get_plugin(&self, class_id: &str) -> Option<&Vst3PluginMeta> {
        self.plugins.get(class_id)
    }

    /// Load a plugin by class ID.
    ///
    /// Note: VST3 plugin loading is not yet fully implemented.
    pub fn load(&mut self, class_id: &str) -> PluginResult<PluginBox> {
        let _meta = self.plugins.get(class_id).ok_or_else(|| {
            PluginLoadError::Vst3Error(format!("Plugin not found: {}", class_id))
        })?;

        // VST3 loading is not yet implemented
        Err(PluginLoadError::Vst3Error(
            "VST3 plugin loading is not yet fully implemented".to_string(),
        ))
    }

    /// Get number of discovered plugins.
    pub fn plugin_count(&self) -> usize {
        self.plugins.len()
    }
}

impl Default for Vst3PluginLoader {
    fn default() -> Self {
        Self::new()
    }
}
