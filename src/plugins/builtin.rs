// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Built-in plugins compiled directly into SootMix.
//!
//! Unlike external plugins (Native, LV2, VST3), builtin plugins are part of the
//! SootMix binary. They are created directly rather than loaded from external files.

#![allow(dead_code, unused_imports)]

use sootmix_plugin_api::{PluginBox, PluginInfo};

/// Registry of available built-in plugins.
#[derive(Debug, Default)]
pub struct BuiltinRegistry {
    plugins: Vec<BuiltinPluginMeta>,
}

/// Metadata for a built-in plugin.
#[derive(Debug, Clone)]
pub struct BuiltinPluginMeta {
    /// Unique identifier for the plugin.
    pub id: String,
    /// Plugin info.
    pub info: PluginInfo,
}

impl BuiltinRegistry {
    /// Create a new builtin plugin registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Get all available builtin plugins.
    pub fn plugins(&self) -> &[BuiltinPluginMeta] {
        &self.plugins
    }

    /// Find a builtin plugin by ID.
    pub fn get(&self, id: &str) -> Option<&BuiltinPluginMeta> {
        self.plugins.iter().find(|p| p.id == id)
    }

    /// Check if the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    /// Get number of builtin plugins.
    pub fn len(&self) -> usize {
        self.plugins.len()
    }
}

/// Create a builtin plugin instance by ID.
///
/// Returns `None` if the plugin ID is not recognized.
pub fn create_builtin(_id: &str) -> Option<PluginBox> {
    // No builtin plugins are currently implemented.
    // Future plugins (e.g., gain, simple EQ) would be matched here.
    None
}
