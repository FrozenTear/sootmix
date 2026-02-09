// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! PipeWire filter stream for plugin audio processing.
//!
//! This module provides in-process audio filtering using PipeWire streams.
//! Each channel with plugins gets a filter stream that routes audio through
//! the plugin chain.
//!
//! # Architecture
//!
//! ```text
//! [App Audio] → [Virtual Sink] → [Plugin Filter Stream] → [Master Sink]
//!                                         │
//!                                   RT Callback:
//!                                   - Drain param updates
//!                                   - Process through plugins
//! ```
//!
//! # Thread Safety
//!
//! The filter streams run on PipeWire's real-time thread. Communication with
//! the UI thread is done via:
//! - Lock-free ring buffer for parameter updates (UI → RT)
//! - try_lock() on plugin instances to avoid blocking RT thread

#![allow(dead_code, unused_imports)]

use crate::plugins::SharedPluginInstances;
use crate::realtime::{PluginParamUpdate, RingBuffer, RingBufferReader, RingBufferWriter};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use thiserror::Error;
use tracing::{debug, info, trace};
use uuid::Uuid;

/// Number of audio channels (stereo).
const NUM_CHANNELS: usize = 2;

#[derive(Debug, Error)]
pub enum PluginFilterError {
    #[error("Failed to create filter stream: {0}")]
    StreamCreationFailed(String),
    #[error("PipeWire error: {0}")]
    PipeWireError(String),
    #[error("Channel not found: {0}")]
    ChannelNotFound(Uuid),
}

/// Shared context for plugin audio processing in the RT callback.
///
/// This struct is accessed from both the UI thread (for setup/updates)
/// and the RT thread (for audio processing).
pub struct PluginProcessingContext {
    /// Shared plugin instances (use try_lock for RT safety).
    pub plugin_instances: SharedPluginInstances,
    /// Plugin chain for this channel (instance IDs in processing order).
    pub plugin_chain: Vec<Uuid>,
    /// Ring buffer reader for parameter updates (RT thread drains this).
    pub param_reader: RingBufferReader<PluginParamUpdate>,
    /// Whether processing is bypassed.
    pub bypassed: AtomicBool,
    /// Sample rate for processing.
    pub sample_rate: f32,
    /// Block size for processing.
    pub block_size: usize,
    /// Pre-allocated ping-pong buffer A for RT-safe plugin chain processing.
    temp_a: Vec<Vec<f32>>,
    /// Pre-allocated ping-pong buffer B for RT-safe plugin chain processing.
    temp_b: Vec<Vec<f32>>,
}

impl PluginProcessingContext {
    /// Create a new processing context.
    pub fn new(
        plugin_instances: SharedPluginInstances,
        plugin_chain: Vec<Uuid>,
        param_reader: RingBufferReader<PluginParamUpdate>,
        sample_rate: f32,
        block_size: usize,
    ) -> Self {
        let temp_a = vec![vec![0.0f32; block_size]; NUM_CHANNELS];
        let temp_b = vec![vec![0.0f32; block_size]; NUM_CHANNELS];

        Self {
            plugin_instances,
            plugin_chain,
            param_reader,
            bypassed: AtomicBool::new(false),
            sample_rate,
            block_size,
            temp_a,
            temp_b,
        }
    }

    /// Set bypass state.
    pub fn set_bypassed(&self, bypassed: bool) {
        self.bypassed.store(bypassed, Ordering::Relaxed);
    }

    /// Check if bypassed.
    pub fn is_bypassed(&self) -> bool {
        self.bypassed.load(Ordering::Relaxed)
    }

    /// Drain parameter updates from the ring buffer and apply them.
    ///
    /// This is RT-safe as it uses try_lock and lock-free ring buffer reads.
    /// Returns the number of updates applied.
    pub fn drain_param_updates(&mut self) -> usize {
        let mut count = 0;

        // Try to acquire the lock - if contended, skip updates this block
        let mut instances = match self.plugin_instances.try_lock() {
            Some(guard) => guard,
            None => return 0,
        };

        // Drain all pending updates
        while let Some(update) = self.param_reader.pop() {
            if let Some(instance) = instances.get_mut(&update.instance_id) {
                instance.set_parameter(update.param_index, update.value);
                count += 1;
                trace!(
                    "Applied param update: plugin={}, param={}, value={}",
                    update.instance_id,
                    update.param_index,
                    update.value
                );
            }
        }

        count
    }

    /// Process audio through the plugin chain.
    ///
    /// This is RT-safe: uses try_lock, pre-allocated ping-pong buffers,
    /// and returns passthrough if lock is contended.
    pub fn process_audio(&mut self, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) -> bool {
        // Check bypass
        if self.bypassed.load(Ordering::Relaxed) {
            Self::copy_passthrough(inputs, outputs);
            return false;
        }

        // Try to acquire lock
        let mut instances = match self.plugin_instances.try_lock() {
            Some(guard) => guard,
            None => {
                // Lock contended, passthrough
                Self::copy_passthrough(inputs, outputs);
                return false;
            }
        };

        // If no plugins, passthrough
        if self.plugin_chain.is_empty() {
            Self::copy_passthrough(inputs, outputs);
            return false;
        }

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
        for &instance_id in &self.plugin_chain {
            let instance = match instances.get_mut(&instance_id) {
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

        true
    }

    /// Copy input to output (passthrough).
    fn copy_passthrough(inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        for (input, output) in inputs.iter().zip(outputs.iter_mut()) {
            output.copy_from_slice(input);
        }
    }
}

/// Information about a plugin filter stream.
pub struct PluginFilterStream {
    /// Channel ID this filter belongs to.
    pub channel_id: Uuid,
    /// Channel name for PipeWire node naming.
    pub channel_name: String,
    /// PipeWire node ID of the filter sink (input).
    pub sink_node_id: Option<u32>,
    /// PipeWire node ID of the filter output.
    pub output_node_id: Option<u32>,
    /// Ring buffer writer for sending parameter updates to RT thread.
    pub param_writer: RingBufferWriter<PluginParamUpdate>,
    /// Whether the filter is active.
    pub active: bool,
}

impl PluginFilterStream {
    /// Create a new filter stream info (before PipeWire stream creation).
    pub fn new(channel_id: Uuid, channel_name: String) -> (Self, RingBufferReader<PluginParamUpdate>) {
        let (writer, reader) = RingBuffer::new(256).split();

        let filter = Self {
            channel_id,
            channel_name,
            sink_node_id: None,
            output_node_id: None,
            param_writer: writer,
            active: false,
        };

        (filter, reader)
    }

    /// Send a parameter update to the RT thread.
    pub fn send_param_update(&mut self, instance_id: Uuid, param_index: u32, value: f32) {
        let update = PluginParamUpdate::new(instance_id, param_index, value);
        self.param_writer.push(update);
    }
}

/// Manages plugin filter streams for all channels.
pub struct PluginFilterManager {
    /// Filter streams by channel ID.
    filters: HashMap<Uuid, PluginFilterStream>,
    /// Shared plugin instances for RT processing.
    plugin_instances: Option<SharedPluginInstances>,
    /// Sample rate for audio processing.
    sample_rate: f32,
    /// Block size for audio processing.
    block_size: usize,
}

impl PluginFilterManager {
    /// Create a new filter manager.
    pub fn new() -> Self {
        Self {
            filters: HashMap::new(),
            plugin_instances: None,
            sample_rate: 48000.0,
            block_size: 512,
        }
    }

    /// Set the shared plugin instances.
    pub fn set_plugin_instances(&mut self, instances: SharedPluginInstances) {
        self.plugin_instances = Some(instances);
    }

    /// Set audio parameters.
    pub fn set_audio_params(&mut self, sample_rate: f32, block_size: usize) {
        self.sample_rate = sample_rate;
        self.block_size = block_size;
    }

    /// Create a filter stream for a channel.
    ///
    /// Returns the filter info. The plugin chain is stored separately
    /// and communicated to the RT thread via the processing context.
    pub fn create_filter(
        &mut self,
        channel_id: Uuid,
        channel_name: &str,
        num_plugins: usize,
    ) -> Result<&PluginFilterStream, PluginFilterError> {
        // Check if filter already exists
        if self.filters.contains_key(&channel_id) {
            debug!("Filter already exists for channel {}", channel_id);
            return self
                .filters
                .get(&channel_id)
                .ok_or(PluginFilterError::ChannelNotFound(channel_id));
        }

        let (filter, _param_reader) = PluginFilterStream::new(channel_id, channel_name.to_string());

        info!(
            "Created plugin filter for channel '{}' with {} plugins",
            channel_name, num_plugins
        );

        self.filters.insert(channel_id, filter);

        self.filters
            .get(&channel_id)
            .ok_or(PluginFilterError::ChannelNotFound(channel_id))
    }

    /// Destroy a filter stream for a channel.
    pub fn destroy_filter(&mut self, channel_id: Uuid) -> bool {
        if let Some(filter) = self.filters.remove(&channel_id) {
            info!(
                "Destroyed plugin filter for channel '{}'",
                filter.channel_name
            );
            true
        } else {
            false
        }
    }

    /// Get a filter by channel ID.
    pub fn get(&self, channel_id: Uuid) -> Option<&PluginFilterStream> {
        self.filters.get(&channel_id)
    }

    /// Get a mutable filter by channel ID.
    pub fn get_mut(&mut self, channel_id: Uuid) -> Option<&mut PluginFilterStream> {
        self.filters.get_mut(&channel_id)
    }

    /// Send a parameter update to a channel's filter.
    pub fn send_param_update(
        &mut self,
        channel_id: Uuid,
        instance_id: Uuid,
        param_index: u32,
        value: f32,
    ) -> bool {
        if let Some(filter) = self.filters.get_mut(&channel_id) {
            filter.send_param_update(instance_id, param_index, value);
            true
        } else {
            false
        }
    }

    /// Check if a channel has a filter.
    pub fn has_filter(&self, channel_id: Uuid) -> bool {
        self.filters.contains_key(&channel_id)
    }

    /// Get all channel IDs with filters.
    pub fn filter_channels(&self) -> Vec<Uuid> {
        self.filters.keys().copied().collect()
    }

    /// Update the plugin chain for a filter.
    pub fn update_plugin_chain(&mut self, channel_id: Uuid, plugin_chain: Vec<Uuid>) -> bool {
        // The plugin chain is stored in the RT context, not in the filter info.
        // This method just validates the filter exists.
        // The actual chain update happens via PipeWire commands.
        self.filters.contains_key(&channel_id)
    }
}

impl Default for PluginFilterManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use parking_lot::Mutex;

    #[test]
    fn test_filter_stream_creation() {
        let channel_id = Uuid::new_v4();
        let (filter, _reader) = PluginFilterStream::new(channel_id, "Test".to_string());
        assert_eq!(filter.channel_id, channel_id);
        assert!(!filter.active);
    }

    #[test]
    fn test_filter_manager() {
        let mut manager = PluginFilterManager::new();
        let channel_id = Uuid::new_v4();

        // Create filter
        manager
            .create_filter(channel_id, "Test", 0)
            .expect("Failed to create filter");
        assert!(manager.has_filter(channel_id));

        // Destroy filter
        assert!(manager.destroy_filter(channel_id));
        assert!(!manager.has_filter(channel_id));
    }

    #[test]
    fn test_processing_context_passthrough() {
        let instances = Arc::new(Mutex::new(HashMap::new()));
        let (_, reader) = RingBuffer::new(16).split();

        let mut ctx = PluginProcessingContext::new(
            instances,
            vec![],
            reader,
            48000.0,
            512,
        );

        let input = vec![vec![1.0, 2.0, 3.0]; 2];
        let input_refs: Vec<&[f32]> = input.iter().map(|v| v.as_slice()).collect();

        let mut output = vec![vec![0.0; 3]; 2];
        let mut output_refs: Vec<&mut [f32]> = output.iter_mut().map(|v| v.as_mut_slice()).collect();

        let processed = ctx.process_audio(&input_refs, &mut output_refs);
        assert!(!processed); // No plugins = passthrough

        // Verify passthrough
        assert_eq!(output[0], vec![1.0, 2.0, 3.0]);
        assert_eq!(output[1], vec![1.0, 2.0, 3.0]);
    }
}
