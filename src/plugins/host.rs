// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Plugin host - provides services to plugins.
//!
//! The plugin host exposes functionality that plugins can use, such as:
//! - Parameter change notifications
//! - Logging
//! - Sample rate queries
//! - (Future) File access, preset management, etc.

use sootmix_plugin_api::PluginInfo;
use std::sync::Arc;
use tracing::{debug, info, warn};

/// Callback for parameter changes from the UI.
pub type ParameterCallback = Arc<dyn Fn(u32, f32) + Send + Sync>;

/// Callback for plugin state changes.
pub type StateCallback = Arc<dyn Fn(PluginStateChange) + Send + Sync>;

/// Plugin state changes that the host can receive.
#[derive(Debug, Clone)]
pub enum PluginStateChange {
    /// Plugin requested a resize.
    ResizeRequested { width: u32, height: u32 },
    /// Plugin latency changed.
    LatencyChanged(u32),
    /// Plugin wants to save its state.
    StateDirty,
}

/// Host context provided to plugins.
///
/// This allows plugins to communicate back to the host application.
pub struct PluginHost {
    /// Current sample rate.
    sample_rate: f32,
    /// Maximum block size.
    max_block_size: usize,
    /// Parameter change callback.
    on_param_change: Option<ParameterCallback>,
    /// State change callback.
    on_state_change: Option<StateCallback>,
}

impl PluginHost {
    /// Create a new plugin host context.
    pub fn new(sample_rate: f32, max_block_size: usize) -> Self {
        Self {
            sample_rate,
            max_block_size,
            on_param_change: None,
            on_state_change: None,
        }
    }

    /// Get the current sample rate.
    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// Get the maximum block size.
    pub fn max_block_size(&self) -> usize {
        self.max_block_size
    }

    /// Update audio parameters.
    pub fn update_audio_params(&mut self, sample_rate: f32, max_block_size: usize) {
        self.sample_rate = sample_rate;
        self.max_block_size = max_block_size;
    }

    /// Set callback for parameter changes.
    pub fn set_param_callback(&mut self, callback: ParameterCallback) {
        self.on_param_change = Some(callback);
    }

    /// Set callback for state changes.
    pub fn set_state_callback(&mut self, callback: StateCallback) {
        self.on_state_change = Some(callback);
    }

    /// Notify host of a parameter change (called by plugin).
    pub fn notify_param_change(&self, index: u32, value: f32) {
        if let Some(ref callback) = self.on_param_change {
            callback(index, value);
        }
    }

    /// Notify host of a state change (called by plugin).
    pub fn notify_state_change(&self, change: PluginStateChange) {
        debug!("Plugin state change: {:?}", change);
        if let Some(ref callback) = self.on_state_change {
            callback(change);
        }
    }

    /// Log a message from a plugin (for debugging).
    pub fn log(&self, plugin: &PluginInfo, level: LogLevel, message: &str) {
        match level {
            LogLevel::Debug => debug!("[{}] {}", plugin.name, message),
            LogLevel::Info => info!("[{}] {}", plugin.name, message),
            LogLevel::Warning => warn!("[{}] {}", plugin.name, message),
            LogLevel::Error => tracing::error!("[{}] {}", plugin.name, message),
        }
    }
}

impl Default for PluginHost {
    fn default() -> Self {
        Self::new(48000.0, 512)
    }
}

/// Log levels for plugin messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    /// Debug information.
    Debug,
    /// Informational message.
    Info,
    /// Warning (non-fatal issue).
    Warning,
    /// Error (operation failed).
    Error,
}

/// Builder for creating plugin processing chains.
///
/// A chain is an ordered list of plugins that audio passes through.
pub struct PluginChainBuilder {
    plugins: Vec<uuid::Uuid>,
}

impl PluginChainBuilder {
    /// Create a new empty chain builder.
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
        }
    }

    /// Add a plugin to the chain.
    pub fn add(mut self, plugin_id: uuid::Uuid) -> Self {
        self.plugins.push(plugin_id);
        self
    }

    /// Get the plugin IDs in order.
    pub fn build(self) -> Vec<uuid::Uuid> {
        self.plugins
    }
}

impl Default for PluginChainBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Represents an active plugin chain for a channel.
pub struct PluginChain {
    /// Plugin instance IDs in processing order.
    pub plugins: Vec<uuid::Uuid>,
    /// Intermediate buffers for chain processing.
    buffers: Vec<Vec<f32>>,
    /// Number of channels.
    num_channels: usize,
    /// Buffer size.
    buffer_size: usize,
}

impl PluginChain {
    /// Create a new plugin chain.
    pub fn new(plugins: Vec<uuid::Uuid>, num_channels: usize, buffer_size: usize) -> Self {
        // Allocate intermediate buffers
        let buffers = (0..num_channels)
            .map(|_| vec![0.0f32; buffer_size])
            .collect();

        Self {
            plugins,
            buffers,
            num_channels,
            buffer_size,
        }
    }

    /// Check if the chain is empty.
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    /// Get the number of plugins in the chain.
    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    /// Ensure buffers are large enough for the given size.
    pub fn ensure_buffer_size(&mut self, size: usize) {
        if size > self.buffer_size {
            for buf in &mut self.buffers {
                buf.resize(size, 0.0);
            }
            self.buffer_size = size;
        }
    }

    /// Get mutable access to internal buffers for processing.
    pub fn buffers_mut(&mut self) -> &mut [Vec<f32>] {
        &mut self.buffers
    }
}

impl Default for PluginChain {
    fn default() -> Self {
        Self::new(Vec::new(), 2, 512)
    }
}
