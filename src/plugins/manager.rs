// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Plugin manager - discovery, lifecycle, and registry.

use super::{native::NativePluginLoader, PluginFilter, PluginLoadError, PluginMetadata, PluginResult, PluginType};
use sootmix_plugin_api::{ActivationContext, PluginBox, PluginInfo};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Registry of all discovered plugins.
#[derive(Debug, Default)]
pub struct PluginRegistry {
    /// All discovered plugins by ID.
    plugins: HashMap<String, PluginMetadata>,
    /// Plugin directories to scan.
    search_paths: Vec<PathBuf>,
}

impl PluginRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a search path for plugin discovery.
    pub fn add_search_path(&mut self, path: impl Into<PathBuf>) {
        self.search_paths.push(path.into());
    }

    /// Get default plugin directories.
    pub fn default_search_paths() -> Vec<PathBuf> {
        let mut paths = Vec::new();

        // User plugins
        if let Some(data_dir) = directories::BaseDirs::new().map(|d| d.data_local_dir().to_path_buf()) {
            paths.push(data_dir.join("sootmix").join("plugins").join("native"));
            paths.push(data_dir.join("sootmix").join("plugins").join("wasm"));
        }

        // System plugins
        paths.push(PathBuf::from("/usr/share/sootmix/plugins"));
        paths.push(PathBuf::from("/usr/local/share/sootmix/plugins"));

        paths
    }

    /// Scan all search paths for plugins.
    pub fn scan(&mut self) -> usize {
        let mut count = 0;

        for path in &self.search_paths.clone() {
            if !path.exists() {
                debug!("Plugin path does not exist: {:?}", path);
                continue;
            }

            match self.scan_directory(path) {
                Ok(n) => {
                    count += n;
                    debug!("Found {} plugins in {:?}", n, path);
                }
                Err(e) => {
                    warn!("Failed to scan plugin directory {:?}: {}", path, e);
                }
            }
        }

        info!("Plugin scan complete: {} plugins found", count);
        count
    }

    /// Scan a single directory for plugins.
    fn scan_directory(&mut self, dir: &Path) -> std::io::Result<usize> {
        let mut count = 0;

        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                let plugin_type = match ext {
                    "so" => PluginType::Native,
                    "wasm" => PluginType::Wasm,
                    _ => continue,
                };

                let metadata = PluginMetadata {
                    path: path.clone(),
                    plugin_type,
                    info: None, // Will be loaded lazily
                    enabled: true,
                };

                // Use filename as temporary ID until we load the plugin
                if let Some(name) = path.file_stem().and_then(|n| n.to_str()) {
                    self.plugins.insert(name.to_string(), metadata);
                    count += 1;
                }
            }
        }

        Ok(count)
    }

    /// Get all plugins matching a filter.
    pub fn list(&self, filter: &PluginFilter) -> Vec<&PluginMetadata> {
        self.plugins
            .values()
            .filter(|m| filter.matches(m))
            .collect()
    }

    /// Get a plugin by ID.
    pub fn get(&self, id: &str) -> Option<&PluginMetadata> {
        self.plugins.get(id)
    }

    /// Get a mutable plugin by ID.
    pub fn get_mut(&mut self, id: &str) -> Option<&mut PluginMetadata> {
        self.plugins.get_mut(id)
    }

    /// Get number of registered plugins.
    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    /// Check if registry is empty.
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }
}

/// A loaded and active plugin instance.
pub struct PluginInstance {
    /// Unique instance ID.
    pub id: Uuid,
    /// Plugin metadata.
    pub metadata: PluginMetadata,
    /// The loaded plugin.
    plugin: PluginBox,
    /// Whether the plugin is activated.
    activated: bool,
    /// Current sample rate.
    sample_rate: f32,
}

impl PluginInstance {
    /// Create a new plugin instance.
    pub(crate) fn new(metadata: PluginMetadata, plugin: PluginBox) -> Self {
        Self {
            id: Uuid::new_v4(),
            metadata,
            plugin,
            activated: false,
            sample_rate: 48000.0,
        }
    }

    /// Get plugin info.
    pub fn info(&self) -> PluginInfo {
        self.plugin.info()
    }

    /// Activate the plugin for processing.
    pub fn activate(&mut self, sample_rate: f32, max_block_size: usize) {
        if self.activated {
            self.deactivate();
        }

        let context = ActivationContext {
            sample_rate,
            max_block_size: max_block_size as u32,
        };

        self.plugin.activate(context);
        self.sample_rate = sample_rate;
        self.activated = true;

        debug!(
            "Plugin {} activated (sr={}, block={})",
            self.info().name,
            sample_rate,
            max_block_size
        );
    }

    /// Deactivate the plugin.
    pub fn deactivate(&mut self) {
        if self.activated {
            self.plugin.deactivate();
            self.activated = false;
            debug!("Plugin {} deactivated", self.info().name);
        }
    }

    /// Check if the plugin is activated.
    pub fn is_activated(&self) -> bool {
        self.activated
    }

    /// Process audio through the plugin.
    ///
    /// # Safety
    /// This method must only be called from the audio thread.
    /// The plugin must be activated before processing.
    pub fn process(&mut self, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        if !self.activated {
            // Pass-through if not activated
            for (input, output) in inputs.iter().zip(outputs.iter_mut()) {
                output.copy_from_slice(input);
            }
            return;
        }

        // Convert to abi_stable types
        use abi_stable::std_types::{RSlice, RSliceMut};

        let inputs_r: Vec<RSlice<f32>> = inputs.iter().map(|s| RSlice::from_slice(s)).collect();
        let inputs_slice = RSlice::from_slice(&inputs_r);

        // For outputs, we need mutable slices
        let mut outputs_r: Vec<RSliceMut<f32>> = outputs
            .iter_mut()
            .map(|s| RSliceMut::from_mut_slice(s))
            .collect();
        let outputs_slice = RSliceMut::from_mut_slice(&mut outputs_r);

        self.plugin.process(inputs_slice, outputs_slice);
    }

    /// Get parameter count.
    pub fn parameter_count(&self) -> u32 {
        self.plugin.parameter_count()
    }

    /// Get parameter info by index.
    pub fn parameter_info(&self, index: u32) -> Option<sootmix_plugin_api::ParameterInfo> {
        self.plugin.parameter_info(index).into()
    }

    /// Get parameter value.
    pub fn get_parameter(&self, index: u32) -> f32 {
        self.plugin.get_parameter(index)
    }

    /// Set parameter value.
    pub fn set_parameter(&mut self, index: u32, value: f32) {
        self.plugin.set_parameter(index, value);
    }

    /// Reset the plugin state.
    pub fn reset(&mut self) {
        self.plugin.reset();
    }

    /// Get plugin latency in samples.
    pub fn latency(&self) -> u32 {
        self.plugin.latency()
    }
}

impl Drop for PluginInstance {
    fn drop(&mut self) {
        self.deactivate();
    }
}

/// Thread-safe plugin instances storage.
///
/// Used to share plugin instances between UI thread and RT audio thread.
/// The UI thread holds the PluginManager and uses regular lock access.
/// The RT audio thread uses try_lock() to avoid blocking.
pub type SharedPluginInstances = Arc<Mutex<HashMap<Uuid, PluginInstance>>>;

/// Plugin manager - handles loading, instantiation, and lifecycle.
///
/// The manager provides thread-safe access to plugin instances through
/// the `shared_instances()` method, which returns an Arc<Mutex<>> that
/// can be shared with the RT audio thread.
pub struct PluginManager {
    /// Plugin registry.
    registry: Arc<RwLock<PluginRegistry>>,
    /// Native plugin loader.
    native_loader: NativePluginLoader,
    /// Active plugin instances (thread-safe).
    instances: SharedPluginInstances,
    /// Default sample rate for activation.
    sample_rate: f32,
    /// Default block size for activation.
    block_size: usize,
}

impl PluginManager {
    /// Create a new plugin manager.
    pub fn new() -> Self {
        let mut registry = PluginRegistry::new();

        // Add default search paths
        for path in PluginRegistry::default_search_paths() {
            registry.add_search_path(path);
        }

        Self {
            registry: Arc::new(RwLock::new(registry)),
            native_loader: NativePluginLoader::new(),
            instances: Arc::new(Mutex::new(HashMap::new())),
            sample_rate: 48000.0,
            block_size: 512,
        }
    }

    /// Get a shared reference to the plugin instances.
    ///
    /// Use this to share instances with the RT audio thread.
    /// The RT thread should use `try_lock()` to avoid blocking.
    pub fn shared_instances(&self) -> SharedPluginInstances {
        Arc::clone(&self.instances)
    }

    /// Set default audio parameters for plugin activation.
    pub fn set_audio_params(&mut self, sample_rate: f32, block_size: usize) {
        self.sample_rate = sample_rate;
        self.block_size = block_size;
    }

    /// Scan for available plugins.
    pub fn scan(&self) -> usize {
        let mut registry = self.registry.write().unwrap();
        registry.scan()
    }

    /// Add a custom search path.
    pub fn add_search_path(&self, path: impl Into<PathBuf>) {
        let mut registry = self.registry.write().unwrap();
        registry.add_search_path(path);
    }

    /// Get the plugin registry (read-only).
    pub fn registry(&self) -> Arc<RwLock<PluginRegistry>> {
        Arc::clone(&self.registry)
    }

    /// List available plugins.
    pub fn list_plugins(&self, filter: &PluginFilter) -> Vec<PluginMetadata> {
        let registry = self.registry.read().unwrap();
        registry.list(filter).into_iter().cloned().collect()
    }

    /// Load and instantiate a plugin.
    pub fn load(&mut self, plugin_id: &str) -> PluginResult<Uuid> {
        let metadata = {
            let registry = self.registry.read().unwrap();
            registry
                .get(plugin_id)
                .cloned()
                .ok_or_else(|| PluginLoadError::NotFound(PathBuf::from(plugin_id)))?
        };

        self.load_from_path(&metadata.path, metadata.plugin_type)
    }

    /// Load a plugin from a specific path.
    pub fn load_from_path(&mut self, path: &Path, plugin_type: PluginType) -> PluginResult<Uuid> {
        let metadata = PluginMetadata {
            path: path.to_path_buf(),
            plugin_type,
            info: None,
            enabled: true,
        };

        let plugin = match plugin_type {
            PluginType::Native => self.native_loader.load(path)?,
            PluginType::Wasm => {
                return Err(PluginLoadError::Initialization(
                    "WASM plugins not yet implemented".to_string(),
                ));
            }
            PluginType::Builtin => {
                return Err(PluginLoadError::Initialization(
                    "Builtin plugins should be created directly".to_string(),
                ));
            }
        };

        let mut instance = PluginInstance::new(metadata, plugin);

        // Activate with current audio parameters
        instance.activate(self.sample_rate, self.block_size);

        let id = instance.id;
        self.instances.lock().unwrap().insert(id, instance);

        info!("Loaded plugin: {} (id={})", path.display(), id);
        Ok(id)
    }

    /// Unload a plugin instance.
    pub fn unload(&mut self, id: Uuid) -> bool {
        let mut instances = self.instances.lock().unwrap();
        if let Some(mut instance) = instances.remove(&id) {
            instance.deactivate();
            info!("Unloaded plugin: {}", id);
            true
        } else {
            false
        }
    }

    /// Get plugin info by instance ID.
    ///
    /// This acquires a lock on the instances map. For RT-safe access,
    /// use `shared_instances()` with `try_lock()` instead.
    pub fn get_info(&self, id: Uuid) -> Option<PluginInfo> {
        let instances = self.instances.lock().unwrap();
        instances.get(&id).map(|i| i.info())
    }

    /// Get parameter count for a plugin instance.
    pub fn get_parameter_count(&self, id: Uuid) -> Option<u32> {
        let instances = self.instances.lock().unwrap();
        instances.get(&id).map(|i| i.parameter_count())
    }

    /// Get parameter info for a plugin instance.
    pub fn get_parameter_info(&self, id: Uuid, index: u32) -> Option<sootmix_plugin_api::ParameterInfo> {
        let instances = self.instances.lock().unwrap();
        instances.get(&id).and_then(|i| i.parameter_info(index))
    }

    /// Get parameter value for a plugin instance.
    pub fn get_parameter(&self, id: Uuid, index: u32) -> Option<f32> {
        let instances = self.instances.lock().unwrap();
        instances.get(&id).map(|i| i.get_parameter(index))
    }

    /// Set parameter value for a plugin instance.
    ///
    /// This acquires a lock. For RT-safe parameter updates, use
    /// the parameter ring buffer and apply updates in the audio callback.
    pub fn set_parameter(&self, id: Uuid, index: u32, value: f32) {
        let mut instances = self.instances.lock().unwrap();
        if let Some(instance) = instances.get_mut(&id) {
            instance.set_parameter(index, value);
        }
    }

    /// Execute a function with access to a plugin instance.
    ///
    /// This provides safe access without exposing references outside the lock scope.
    pub fn with_instance<F, R>(&self, id: Uuid, f: F) -> Option<R>
    where
        F: FnOnce(&PluginInstance) -> R,
    {
        let instances = self.instances.lock().unwrap();
        instances.get(&id).map(f)
    }

    /// Execute a function with mutable access to a plugin instance.
    pub fn with_instance_mut<F, R>(&self, id: Uuid, f: F) -> Option<R>
    where
        F: FnOnce(&mut PluginInstance) -> R,
    {
        let mut instances = self.instances.lock().unwrap();
        instances.get_mut(&id).map(f)
    }

    /// Get all active plugin instance IDs.
    pub fn active_instance_ids(&self) -> Vec<Uuid> {
        let instances = self.instances.lock().unwrap();
        instances.keys().copied().collect()
    }

    /// Unload all plugin instances.
    pub fn unload_all(&mut self) {
        let mut instances = self.instances.lock().unwrap();
        for (_, mut instance) in instances.drain() {
            instance.deactivate();
        }
        info!("Unloaded all plugins");
    }
}

impl Default for PluginManager {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for PluginManager {
    fn drop(&mut self) {
        self.unload_all();
    }
}
