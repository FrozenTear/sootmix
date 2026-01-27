// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Native plugin loader using abi_stable and libloading.
//!
//! Native plugins are shared libraries (.so on Linux) that export a
//! `sootmix_plugin_entry` function returning a `PluginEntry` struct.

use super::{PluginLoadError, PluginResult};
use libloading::{Library, Symbol};
use sootmix_plugin_api::{PluginBox, PluginEntry, API_VERSION_MAJOR, API_VERSION_MINOR};
use std::collections::HashMap;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Entry point function name that plugins must export.
const ENTRY_POINT_NAME: &[u8] = b"sootmix_plugin_entry\0";

/// Check that a plugin file and its parent directory do not have insecure permissions.
/// Rejects world-writable files or files in world-writable directories.
pub(super) fn check_plugin_permissions(path: &Path) -> PluginResult<()> {
    let metadata = std::fs::metadata(path)?;
    let mode = metadata.mode();

    // Reject world-writable plugin files
    if mode & 0o002 != 0 {
        warn!(
            "Rejecting plugin with world-writable permissions: {:?} (mode {:o})",
            path, mode
        );
        return Err(PluginLoadError::InsecurePermissions(path.to_path_buf()));
    }

    // Reject plugins in world-writable directories
    if let Some(parent) = path.parent() {
        if let Ok(dir_meta) = std::fs::metadata(parent) {
            let dir_mode = dir_meta.mode();
            if dir_mode & 0o002 != 0 {
                warn!(
                    "Rejecting plugin in world-writable directory: {:?} (dir mode {:o})",
                    path, dir_mode
                );
                return Err(PluginLoadError::InsecurePermissions(path.to_path_buf()));
            }
        }
    }

    Ok(())
}

/// Native plugin loader.
///
/// Handles loading shared libraries and managing their lifetimes.
pub struct NativePluginLoader {
    /// Loaded libraries (kept alive to prevent unloading).
    libraries: HashMap<PathBuf, Library>,
}

impl NativePluginLoader {
    /// Create a new native plugin loader.
    pub fn new() -> Self {
        Self {
            libraries: HashMap::new(),
        }
    }

    /// Load a plugin from a shared library.
    pub fn load(&mut self, path: &Path) -> PluginResult<PluginBox> {
        // Check if file exists
        if !path.exists() {
            return Err(PluginLoadError::NotFound(path.to_path_buf()));
        }

        // Check file permissions before loading
        check_plugin_permissions(path)?;

        debug!("Loading native plugin: {:?}", path);

        // Load the library
        // SAFETY: We trust that plugins in the plugin directory are safe to load.
        // Users should only install plugins from trusted sources.
        let library = unsafe {
            Library::new(path).map_err(|e| PluginLoadError::LibraryLoad(e.to_string()))?
        };

        // Look up the entry point
        let entry: PluginEntry = unsafe {
            let entry_fn: Symbol<extern "C" fn() -> PluginEntry> =
                library.get(ENTRY_POINT_NAME).map_err(|e| {
                    PluginLoadError::EntryPointNotFound(format!(
                        "sootmix_plugin_entry: {}",
                        e
                    ))
                })?;

            entry_fn()
        };

        // Check API version compatibility
        if entry.api_version_major != API_VERSION_MAJOR {
            return Err(PluginLoadError::VersionMismatch {
                plugin_major: entry.api_version_major,
                plugin_minor: entry.api_version_minor,
                host_major: API_VERSION_MAJOR,
                host_minor: API_VERSION_MINOR,
            });
        }

        // Minor version: plugin must be <= host (host is backwards compatible)
        if entry.api_version_minor > API_VERSION_MINOR {
            warn!(
                "Plugin API minor version ({}) is newer than host ({}), some features may not work",
                entry.api_version_minor, API_VERSION_MINOR
            );
        }

        // Create the plugin instance
        let plugin = (entry.create)();

        // Store the library to keep it loaded
        self.libraries.insert(path.to_path_buf(), library);

        let info = plugin.info();
        info!(
            "Loaded native plugin: {} v{} by {}",
            info.name, info.version, info.vendor
        );

        Ok(plugin)
    }

    /// Unload a plugin library.
    ///
    /// Note: The plugin instances must be dropped first!
    pub fn unload(&mut self, path: &Path) -> bool {
        if self.libraries.remove(path).is_some() {
            debug!("Unloaded library: {:?}", path);
            true
        } else {
            false
        }
    }

    /// Check if a library is loaded.
    pub fn is_loaded(&self, path: &Path) -> bool {
        self.libraries.contains_key(path)
    }

    /// Get number of loaded libraries.
    pub fn loaded_count(&self) -> usize {
        self.libraries.len()
    }

    /// Validate a plugin file without fully loading it.
    ///
    /// Returns plugin info if valid, or an error.
    pub fn validate(&self, path: &Path) -> PluginResult<sootmix_plugin_api::PluginInfo> {
        if !path.exists() {
            return Err(PluginLoadError::NotFound(path.to_path_buf()));
        }

        // Check file permissions before loading
        check_plugin_permissions(path)?;

        // Temporarily load to check
        let library = unsafe {
            Library::new(path).map_err(|e| PluginLoadError::LibraryLoad(e.to_string()))?
        };

        let entry: PluginEntry = unsafe {
            let entry_fn: Symbol<extern "C" fn() -> PluginEntry> =
                library.get(ENTRY_POINT_NAME).map_err(|e| {
                    PluginLoadError::EntryPointNotFound(format!(
                        "sootmix_plugin_entry: {}",
                        e
                    ))
                })?;

            entry_fn()
        };

        // Check version
        if entry.api_version_major != API_VERSION_MAJOR {
            return Err(PluginLoadError::VersionMismatch {
                plugin_major: entry.api_version_major,
                plugin_minor: entry.api_version_minor,
                host_major: API_VERSION_MAJOR,
                host_minor: API_VERSION_MINOR,
            });
        }

        // Get info
        let plugin = (entry.create)();
        let info = plugin.info();

        // Library will be unloaded when dropped
        Ok(info)
    }
}

impl Default for NativePluginLoader {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for NativePluginLoader {
    fn drop(&mut self) {
        // Libraries will be unloaded when the HashMap is dropped
        if !self.libraries.is_empty() {
            debug!("Unloading {} plugin libraries", self.libraries.len());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_loader_creation() {
        let loader = NativePluginLoader::new();
        assert_eq!(loader.loaded_count(), 0);
    }

    #[test]
    fn test_load_nonexistent() {
        let mut loader = NativePluginLoader::new();
        let result = loader.load(Path::new("/nonexistent/plugin.so"));
        assert!(matches!(result, Err(PluginLoadError::NotFound(_))));
    }
}
