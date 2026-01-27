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
//! # Implementation Status
//!
//! The plugin processor infrastructure is fully implemented:
//! - Thread-safe PluginManager with `Arc<Mutex<HashMap>>` for RT access
//! - Lock-free parameter ring buffer for UI → RT thread updates
//! - `PluginFilterManager` for managing per-channel filter streams
//! - `PluginProcessingContext` for RT-safe audio processing
//! - PipeWire commands for filter creation/destruction/updates
//!
//! See also:
//! - `plugin_filter.rs` - PipeWire filter stream implementation
//! - `pipewire_thread.rs` - PwCommand::CreatePluginFilter, etc.
//!
//! # Thread Safety
//!
//! - PluginManager uses `Arc<Mutex<>>` for thread-safe instance access
//! - RT thread uses `try_lock()` to avoid blocking
//! - Parameter updates flow via lock-free ring buffer
//! - If lock is contended, RT thread does passthrough (no audio glitches)

use crate::plugins::SharedPluginInstances;
use std::collections::HashMap;
use thiserror::Error;
use tracing::{debug, info};
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

/// Number of audio channels (stereo).
const NUM_CHANNELS: usize = 2;

/// Manages plugin audio processing for all channels.
pub struct PluginProcessorManager {
    /// Per-channel processors.
    processors: HashMap<Uuid, ChannelPluginProcessor>,
    /// Sample rate for audio processing.
    sample_rate: f32,
    /// Block size for audio processing.
    block_size: usize,
    /// Pre-allocated ping-pong buffer A for RT-safe plugin chain processing.
    temp_a: Vec<Vec<f32>>,
    /// Pre-allocated ping-pong buffer B for RT-safe plugin chain processing.
    temp_b: Vec<Vec<f32>>,
}

impl PluginProcessorManager {
    pub fn new() -> Self {
        let block_size = 512;
        Self {
            processors: HashMap::new(),
            sample_rate: 48000.0,
            block_size,
            temp_a: vec![vec![0.0f32; block_size]; NUM_CHANNELS],
            temp_b: vec![vec![0.0f32; block_size]; NUM_CHANNELS],
        }
    }

    /// Set audio parameters for processing.
    pub fn set_audio_params(&mut self, sample_rate: f32, block_size: usize) {
        self.sample_rate = sample_rate;
        self.block_size = block_size;
        self.temp_a = vec![vec![0.0f32; block_size]; NUM_CHANNELS];
        self.temp_b = vec![vec![0.0f32; block_size]; NUM_CHANNELS];
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

    /// Process audio through a channel's plugin chain (RT-safe version).
    ///
    /// This is called from the audio processing callback.
    /// It processes the input buffers through each plugin in sequence.
    /// Uses try_lock() for RT-safety - returns passthrough if lock is contended.
    ///
    /// # Arguments
    /// * `channel_id` - The channel to process
    /// * `instances` - Shared plugin instances (use try_lock for RT safety)
    /// * `inputs` - Input audio buffers (stereo: [left, right])
    /// * `outputs` - Output audio buffers (stereo: [left, right])
    ///
    /// # Returns
    /// * `Ok(true)` if audio was processed through plugins
    /// * `Ok(false)` if channel has no active plugins or lock was contended (passthrough)
    /// * `Err` on processing error
    pub fn process_audio(
        &mut self,
        channel_id: Uuid,
        instances: &SharedPluginInstances,
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

        // Try to acquire lock - RT-safe: don't block if contended
        let mut instances_guard = match instances.try_lock() {
            Some(guard) => guard,
            None => {
                // Lock contended, passthrough to avoid blocking audio thread
                for (input, output) in inputs.iter().zip(outputs.iter_mut()) {
                    output.copy_from_slice(input);
                }
                return Ok(false);
            }
        };

        let num_samples = inputs.first().map(|b| b.len()).unwrap_or(0);

        // Ensure pre-allocated buffers are large enough (rare resize for unexpected block sizes)
        for buf in self.temp_a.iter_mut().chain(self.temp_b.iter_mut()) {
            if buf.len() < num_samples {
                buf.resize(num_samples, 0.0);
            }
        }

        // Copy input into temp_a (current read buffer)
        for (src, dst) in inputs.iter().zip(self.temp_a.iter_mut()) {
            dst[..num_samples].copy_from_slice(src);
        }

        // Ping-pong: read from temp_a, write to temp_b, then swap
        let mut read_a = true;
        for &instance_id in &processor.plugin_instances {
            let instance = match instances_guard.get_mut(&instance_id) {
                Some(i) => i,
                None => continue,
            };

            let (read_bufs, write_bufs) = if read_a {
                (&mut self.temp_a as *mut Vec<Vec<f32>>, &mut self.temp_b as *mut Vec<Vec<f32>>)
            } else {
                (&mut self.temp_b as *mut Vec<Vec<f32>>, &mut self.temp_a as *mut Vec<Vec<f32>>)
            };

            // SAFETY: temp_a and temp_b are distinct fields; we need simultaneous
            // read and write access to different buffers for plugin processing.
            let (read_bufs, write_bufs) = unsafe { (&*read_bufs, &mut *write_bufs) };

            let input_slices: [&[f32]; 2] = [
                &read_bufs[0][..num_samples],
                &read_bufs[1][..num_samples],
            ];
            let (wb0, wb1) = write_bufs.split_at_mut(1);
            let mut output_slices: [&mut [f32]; 2] = [
                &mut wb0[0][..num_samples],
                &mut wb1[0][..num_samples],
            ];

            instance.process(&input_slices, &mut output_slices);
            read_a = !read_a;
        }

        // Copy from the last-written buffer to output
        let final_bufs = if read_a { &self.temp_a } else { &self.temp_b };
        for (temp, output) in final_bufs.iter().zip(outputs.iter_mut()) {
            output.copy_from_slice(&temp[..num_samples]);
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
