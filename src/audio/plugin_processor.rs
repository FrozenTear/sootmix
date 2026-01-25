// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Plugin audio processor using PipeWire filter streams.
//!
//! This module handles routing audio through per-channel plugin chains.
//! It tracks which channels have active plugin chains and provides the
//! processing infrastructure.
//!
//! # Architecture
//!
//! ```text
//! [App] -> [Channel Virtual Sink] -> [Plugin Filter] -> [Master Sink]
//!                                          |
//!                                    Plugin Chain:
//!                                    - Plugin 1
//!                                    - Plugin 2
//!                                    - ...
//! ```
//!
//! # Current Status
//!
//! The plugin processor infrastructure is implemented:
//! - Tracks per-channel plugin chains
//! - Provides audio processing through plugin instances
//! - Syncs with app state when plugins are added/removed
//!
//! # TODO: PipeWire Audio Routing
//!
//! To complete the audio integration, we need to create PipeWire filter
//! streams that route audio through the plugins. This involves:
//!
//! 1. **Create a filter stream** for each channel with plugins
//!    - Use pipewire::stream API with both input and output
//!    - Register process callback for real-time audio handling
//!
//! 2. **Route audio through the filter**
//!    - Disconnect channel virtual sink from master
//!    - Connect: virtual sink output → filter input
//!    - Connect: filter output → master sink
//!
//! 3. **Process callback implementation**
//!    - Dequeue input buffer from stream
//!    - Call process_audio() with plugin chain
//!    - Queue processed buffer to output
//!
//! 4. **Thread safety considerations**
//!    - PipeWire runs on its own real-time thread
//!    - Plugin instances need to be accessed from callback
//!    - Use lock-free communication for parameter updates

use crate::plugins::PluginManager;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use thiserror::Error;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum PluginProcessorError {
    #[error("Failed to create filter: {0}")]
    FilterCreationFailed(String),
    #[error("Channel not found: {0}")]
    ChannelNotFound(Uuid),
    #[error("Plugin not loaded: {0}")]
    PluginNotLoaded(Uuid),
    #[error("PipeWire error: {0}")]
    PipeWireError(String),
}

/// State for a channel's plugin processing chain.
pub struct ChannelPluginProcessor {
    /// Channel ID.
    pub channel_id: Uuid,
    /// Plugin instance IDs in processing order.
    pub plugin_instances: Vec<Uuid>,
    /// Whether processing is active.
    pub active: bool,
    /// Input node ID (channel's virtual sink output).
    pub input_node_id: Option<u32>,
    /// Output node ID (where processed audio goes).
    pub output_node_id: Option<u32>,
}

impl ChannelPluginProcessor {
    pub fn new(channel_id: Uuid) -> Self {
        Self {
            channel_id,
            plugin_instances: Vec::new(),
            active: false,
            input_node_id: None,
            output_node_id: None,
        }
    }
}

/// Manages plugin audio processing for all channels.
pub struct PluginProcessorManager {
    /// Per-channel processors.
    processors: HashMap<Uuid, ChannelPluginProcessor>,
    /// Sample rate for audio processing.
    sample_rate: f32,
    /// Block size for audio processing.
    block_size: usize,
}

impl PluginProcessorManager {
    pub fn new() -> Self {
        Self {
            processors: HashMap::new(),
            sample_rate: 48000.0,
            block_size: 512,
        }
    }

    /// Set audio parameters for processing.
    pub fn set_audio_params(&mut self, sample_rate: f32, block_size: usize) {
        self.sample_rate = sample_rate;
        self.block_size = block_size;
        debug!(
            "Plugin processor audio params: {} Hz, {} samples",
            sample_rate, block_size
        );
    }

    /// Create or update a processor for a channel.
    pub fn setup_channel(
        &mut self,
        channel_id: Uuid,
        plugin_instances: Vec<Uuid>,
    ) -> Result<(), PluginProcessorError> {
        let processor = self
            .processors
            .entry(channel_id)
            .or_insert_with(|| ChannelPluginProcessor::new(channel_id));

        processor.plugin_instances = plugin_instances;

        if processor.plugin_instances.is_empty() {
            // No plugins, disable processing
            processor.active = false;
            info!(
                "Channel {} has no plugins, bypassing processing",
                channel_id
            );
        } else {
            processor.active = true;
            info!(
                "Channel {} setup with {} plugins",
                channel_id,
                processor.plugin_instances.len()
            );
        }

        Ok(())
    }

    /// Remove processor for a channel.
    pub fn remove_channel(&mut self, channel_id: Uuid) {
        if self.processors.remove(&channel_id).is_some() {
            info!("Removed plugin processor for channel {}", channel_id);
        }
    }

    /// Get the processor for a channel.
    pub fn get(&self, channel_id: Uuid) -> Option<&ChannelPluginProcessor> {
        self.processors.get(&channel_id)
    }

    /// Get mutable reference to processor.
    pub fn get_mut(&mut self, channel_id: Uuid) -> Option<&mut ChannelPluginProcessor> {
        self.processors.get_mut(&channel_id)
    }

    /// Check if a channel has active plugin processing.
    pub fn is_active(&self, channel_id: Uuid) -> bool {
        self.processors
            .get(&channel_id)
            .map(|p| p.active)
            .unwrap_or(false)
    }

    /// Process audio through a channel's plugin chain.
    ///
    /// This is called from the audio processing callback.
    /// It processes the input buffers through each plugin in sequence.
    ///
    /// # Arguments
    /// * `channel_id` - The channel to process
    /// * `plugin_manager` - Reference to the plugin manager
    /// * `inputs` - Input audio buffers (stereo: [left, right])
    /// * `outputs` - Output audio buffers (stereo: [left, right])
    ///
    /// # Returns
    /// * `Ok(true)` if audio was processed through plugins
    /// * `Ok(false)` if channel has no active plugins (passthrough)
    /// * `Err` on processing error
    pub fn process_audio(
        &self,
        channel_id: Uuid,
        plugin_manager: &mut PluginManager,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
    ) -> Result<bool, PluginProcessorError> {
        let processor = match self.processors.get(&channel_id) {
            Some(p) if p.active => p,
            _ => {
                // No active processor, copy input to output (passthrough)
                for (input, output) in inputs.iter().zip(outputs.iter_mut()) {
                    output.copy_from_slice(input);
                }
                return Ok(false);
            }
        };

        // Process through plugin chain
        let num_samples = inputs.first().map(|b| b.len()).unwrap_or(0);

        // We need intermediate buffers for chaining plugins
        // For now, use a simple approach with temp buffers
        let mut temp_buffers: Vec<Vec<f32>> = inputs.iter().map(|b| b.to_vec()).collect();

        for &instance_id in &processor.plugin_instances {
            let instance = plugin_manager
                .get_mut(instance_id)
                .ok_or(PluginProcessorError::PluginNotLoaded(instance_id))?;

            // Check bypass state (would need to track this per-instance)
            // For now, always process

            // Create input/output slices for this plugin
            let input_slices: Vec<&[f32]> = temp_buffers.iter().map(|b| b.as_slice()).collect();

            // Create output buffers
            let mut output_vecs: Vec<Vec<f32>> = vec![vec![0.0; num_samples]; outputs.len()];
            let mut output_slices: Vec<&mut [f32]> =
                output_vecs.iter_mut().map(|b| b.as_mut_slice()).collect();

            // Process through plugin
            instance.process(&input_slices, &mut output_slices);

            // Swap buffers for next plugin in chain
            temp_buffers = output_vecs;
        }

        // Copy final output
        for (temp, output) in temp_buffers.iter().zip(outputs.iter_mut()) {
            output.copy_from_slice(temp);
        }

        Ok(true)
    }
}

impl Default for PluginProcessorManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_processor_manager_new() {
        let manager = PluginProcessorManager::new();
        assert!(manager.processors.is_empty());
    }

    #[test]
    fn test_setup_channel_empty() {
        let mut manager = PluginProcessorManager::new();
        let channel_id = Uuid::new_v4();

        manager.setup_channel(channel_id, vec![]).unwrap();

        assert!(!manager.is_active(channel_id));
    }

    #[test]
    fn test_setup_channel_with_plugins() {
        let mut manager = PluginProcessorManager::new();
        let channel_id = Uuid::new_v4();
        let plugin_id = Uuid::new_v4();

        manager.setup_channel(channel_id, vec![plugin_id]).unwrap();

        assert!(manager.is_active(channel_id));
        assert_eq!(
            manager.get(channel_id).unwrap().plugin_instances,
            vec![plugin_id]
        );
    }

    #[test]
    fn test_remove_channel() {
        let mut manager = PluginProcessorManager::new();
        let channel_id = Uuid::new_v4();

        manager.setup_channel(channel_id, vec![Uuid::new_v4()]).unwrap();
        assert!(manager.get(channel_id).is_some());

        manager.remove_channel(channel_id);
        assert!(manager.get(channel_id).is_none());
    }
}
