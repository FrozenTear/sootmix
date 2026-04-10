// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Built-in plugins compiled directly into SootMix.

mod compressor;
mod dsp;
mod gate;
mod hpf;

use sootmix_plugin_api::{AudioEffect_TO, PluginBox, PluginInfo};

/// Metadata for a built-in plugin.
#[derive(Debug, Clone)]
pub struct BuiltinPluginMeta {
    /// Unique identifier for the plugin.
    pub id: String,
    /// Plugin info.
    pub info: PluginInfo,
}

/// Registry of available built-in plugins.
#[derive(Debug, Default)]
pub struct BuiltinRegistry {
    plugins: Vec<BuiltinPluginMeta>,
}

impl BuiltinRegistry {
    /// Create a new builtin plugin registry populated with all built-in plugins.
    pub fn new() -> Self {
        let plugins = vec![
            BuiltinPluginMeta {
                id: "com.sootmix.hpf".to_string(),
                info: hpf::HpfPlugin::plugin_info(),
            },
            BuiltinPluginMeta {
                id: "com.sootmix.gate".to_string(),
                info: gate::NoiseGatePlugin::plugin_info(),
            },
            BuiltinPluginMeta {
                id: "com.sootmix.compressor".to_string(),
                info: compressor::CompressorPlugin::plugin_info(),
            },
        ];
        Self { plugins }
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
pub fn create_builtin(id: &str) -> Option<PluginBox> {
    match id {
        "com.sootmix.hpf" => {
            let plugin = hpf::HpfPlugin::default();
            Some(AudioEffect_TO::from_value(plugin, abi_stable::sabi_trait::TD_Opaque))
        }
        "com.sootmix.gate" => {
            let plugin = gate::NoiseGatePlugin::default();
            Some(AudioEffect_TO::from_value(plugin, abi_stable::sabi_trait::TD_Opaque))
        }
        "com.sootmix.compressor" => {
            let plugin = compressor::CompressorPlugin::default();
            Some(AudioEffect_TO::from_value(plugin, abi_stable::sabi_trait::TD_Opaque))
        }
        _ => None,
    }
}
