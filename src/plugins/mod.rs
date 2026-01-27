// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Plugin system for SootMix.
//!
//! This module provides infrastructure for loading, managing, and running
//! audio effect plugins. Supports native plugins (via abi_stable), sandboxed
//! WASM plugins (via wasmtime), LV2 plugins (via lv2), and VST3 plugins.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────┐
//! │                    PluginManager                         │
//! ├─────────────────────────────────────────────────────────┤
//! │  native_loader    lv2_loader       vst3_loader          │
//! │       │               │                 │               │
//! │       ▼               ▼                 ▼               │
//! │   NativePlugin   LV2 Instance    VST3 Component         │
//! │       │               │                 │               │
//! │       └───────────────┴─────────────────┘               │
//! │                       │                                 │
//! │                       ▼                                 │
//! │              Adapter (impl AudioEffect)                 │
//! │                       │                                 │
//! │                       ▼                                 │
//! │                   PluginBox                             │
//! └─────────────────────────────────────────────────────────┘
//! ```

pub mod host;
pub mod manager;
pub mod native;

#[cfg(feature = "wasm-plugins")]
pub mod wasm;

#[cfg(feature = "lv2-plugins")]
pub mod lv2;

#[cfg(feature = "vst3-plugins")]
pub mod vst3;

pub use host::PluginHost;
pub use manager::{PluginInstance, PluginManager, PluginRegistry, SharedPluginInstances};

#[cfg(feature = "lv2-plugins")]
pub use lv2::{Lv2PluginLoader, Lv2PluginMeta};

#[cfg(feature = "vst3-plugins")]
pub use vst3::{Vst3PluginLoader, Vst3PluginMeta};

use serde::{Deserialize, Serialize};
use sootmix_plugin_api::{PluginCategory, PluginInfo};
use std::collections::HashMap;
use std::path::PathBuf;
use uuid::Uuid;

/// Metadata about a discovered plugin (before loading).
#[derive(Debug, Clone)]
pub struct PluginMetadata {
    /// Path to the plugin file.
    pub path: PathBuf,
    /// Plugin type.
    pub plugin_type: PluginType,
    /// Plugin info (if available from manifest/cache).
    pub info: Option<PluginInfo>,
    /// Whether the plugin is enabled.
    pub enabled: bool,
}

/// Type of plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PluginType {
    /// Native shared library (.so on Linux).
    Native,
    /// WebAssembly module (.wasm).
    Wasm,
    /// Built-in plugin (compiled into SootMix).
    Builtin,
    /// LV2 plugin.
    #[cfg(feature = "lv2-plugins")]
    Lv2,
    /// VST3 plugin.
    #[cfg(feature = "vst3-plugins")]
    Vst3,
}

impl PluginType {
    /// Get the file extension for this plugin type.
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Native => "so",
            Self::Wasm => "wasm",
            Self::Builtin => "",
            #[cfg(feature = "lv2-plugins")]
            Self::Lv2 => "lv2",
            #[cfg(feature = "vst3-plugins")]
            Self::Vst3 => "vst3",
        }
    }
}

/// Error type for plugin operations.
#[derive(Debug, thiserror::Error)]
pub enum PluginLoadError {
    /// Plugin file not found.
    #[error("plugin not found: {0}")]
    NotFound(PathBuf),

    /// Failed to load shared library.
    #[error("failed to load library: {0}")]
    LibraryLoad(String),

    /// Plugin entry point not found.
    #[error("entry point not found: {0}")]
    EntryPointNotFound(String),

    /// API version mismatch.
    #[error("API version mismatch: plugin {plugin_major}.{plugin_minor}, host {host_major}.{host_minor}")]
    VersionMismatch {
        plugin_major: u32,
        plugin_minor: u32,
        host_major: u32,
        host_minor: u32,
    },

    /// WASM instantiation failed.
    #[error("WASM instantiation failed: {0}")]
    WasmInstantiation(String),

    /// Plugin initialization failed.
    #[error("plugin initialization failed: {0}")]
    Initialization(String),

    /// Plugin file has insecure permissions.
    #[error("insecure permissions on plugin: {0}")]
    InsecurePermissions(PathBuf),

    /// I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// LV2 plugin error.
    #[cfg(feature = "lv2-plugins")]
    #[error("LV2 error: {0}")]
    Lv2Error(String),

    /// VST3 plugin error.
    #[cfg(feature = "vst3-plugins")]
    #[error("VST3 error: {0}")]
    Vst3Error(String),
}

/// Result type for plugin operations.
pub type PluginResult<T> = Result<T, PluginLoadError>;

/// Filter criteria for plugin discovery.
#[derive(Debug, Clone, Default)]
pub struct PluginFilter {
    /// Filter by category.
    pub category: Option<PluginCategory>,
    /// Filter by plugin type.
    pub plugin_type: Option<PluginType>,
    /// Search term for name/vendor.
    pub search: Option<String>,
}

impl PluginFilter {
    /// Create a new empty filter.
    pub fn new() -> Self {
        Self::default()
    }

    /// Filter by category.
    pub fn with_category(mut self, category: PluginCategory) -> Self {
        self.category = Some(category);
        self
    }

    /// Filter by plugin type.
    pub fn with_type(mut self, plugin_type: PluginType) -> Self {
        self.plugin_type = Some(plugin_type);
        self
    }

    /// Filter by search term.
    pub fn with_search(mut self, search: impl Into<String>) -> Self {
        self.search = Some(search.into());
        self
    }

    /// Check if a plugin matches this filter.
    pub fn matches(&self, metadata: &PluginMetadata) -> bool {
        // Check plugin type
        if let Some(pt) = self.plugin_type {
            if metadata.plugin_type != pt {
                return false;
            }
        }

        // Check category and search term (require info)
        if let Some(ref info) = metadata.info {
            if let Some(category) = self.category {
                if info.category != category {
                    return false;
                }
            }

            if let Some(ref search) = self.search {
                let search_lower = search.to_lowercase();
                let name_match = info.name.to_lowercase().contains(&search_lower);
                let vendor_match = info.vendor.to_lowercase().contains(&search_lower);
                if !name_match && !vendor_match {
                    return false;
                }
            }
        }

        true
    }
}

/// Configuration for a plugin slot in a channel's plugin chain.
/// Used for serialization/persistence of plugin state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginSlotConfig {
    /// Plugin identifier (filename stem, e.g., "sootmix-eq").
    pub plugin_id: String,
    /// Type of plugin (Native, WASM, Builtin, Lv2, Vst3).
    pub plugin_type: PluginType,
    /// Whether this plugin slot is bypassed.
    pub bypassed: bool,
    /// Saved parameter values (param_index -> value).
    #[serde(default)]
    pub parameters: HashMap<u32, f32>,
    /// External identifier for LV2 URI or VST3 class ID.
    /// Used to locate and load external plugins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
    /// Sidechain source channel ID for sidechain-aware plugins.
    /// When set, the host will send the source channel's meter level
    /// to any parameter with SidechainLevel hint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidechain_source: Option<Uuid>,
}

impl PluginSlotConfig {
    /// Create a new plugin slot config.
    pub fn new(plugin_id: impl Into<String>, plugin_type: PluginType) -> Self {
        Self {
            plugin_id: plugin_id.into(),
            plugin_type,
            bypassed: false,
            parameters: HashMap::new(),
            external_id: None,
            sidechain_source: None,
        }
    }

    /// Create a new plugin slot config for an external plugin (LV2/VST3).
    pub fn new_external(
        plugin_id: impl Into<String>,
        plugin_type: PluginType,
        external_id: impl Into<String>,
    ) -> Self {
        Self {
            plugin_id: plugin_id.into(),
            plugin_type,
            bypassed: false,
            parameters: HashMap::new(),
            external_id: Some(external_id.into()),
            sidechain_source: None,
        }
    }
}
