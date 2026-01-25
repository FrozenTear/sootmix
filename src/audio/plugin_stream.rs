// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! PipeWire stream-based plugin audio processing.
//!
//! This module implements real-time audio processing through plugin chains
//! using PipeWire streams. It creates a capture stream (receiving audio from
//! the channel's virtual sink) and a playback stream (outputting to master).
//!
//! # Architecture
//!
//! ```text
//! [Virtual Sink] → [Capture Stream] → [Plugin Chain] → [Playback Stream] → [Master]
//!                        ↓                   ↑
//!                  process_cb          PluginProcessingContext
//! ```
//!
//! # Thread Safety
//!
//! - Streams run on PipeWire's main loop thread
//! - Plugin instances are accessed via try_lock() for RT safety
//! - Parameter updates flow via lock-free ring buffer

use crate::audio::plugin_filter::PluginProcessingContext;
use crate::plugins::SharedPluginInstances;
use crate::realtime::{PluginParamUpdate, RingBufferReader};
use pipewire::properties::properties;
use pipewire::stream::{Stream, StreamFlags, StreamListener, StreamRc};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::{debug, info, trace};
use uuid::Uuid;

/// Size of the audio buffer between capture and playback streams (in frames).
const AUDIO_BUFFER_FRAMES: usize = 2048;

/// Number of audio channels (stereo).
const NUM_CHANNELS: usize = 2;

/// User data passed to stream process callbacks.
struct StreamUserData {
    /// Processing context with plugin chain and param updates.
    context: Rc<RefCell<PluginProcessingContext>>,
    /// Shared audio buffer between capture and playback streams.
    audio_buffer: Rc<RefCell<AudioRingBuffer>>,
    /// Whether this is the capture (true) or playback (false) stream.
    is_capture: bool,
    /// Channel ID for logging.
    channel_id: Uuid,
}

/// Simple audio ring buffer for passing samples between streams.
///
/// This is a fixed-size buffer optimized for the typical case where
/// capture and playback are synchronized.
struct AudioRingBuffer {
    /// Interleaved stereo samples [L0, R0, L1, R1, ...].
    buffer: Vec<f32>,
    /// Write position.
    write_pos: usize,
    /// Read position.
    read_pos: usize,
    /// Capacity in frames.
    capacity_frames: usize,
}

impl AudioRingBuffer {
    fn new(capacity_frames: usize) -> Self {
        Self {
            buffer: vec![0.0; capacity_frames * NUM_CHANNELS],
            write_pos: 0,
            read_pos: 0,
            capacity_frames,
        }
    }

    /// Write interleaved stereo samples to the buffer.
    fn write(&mut self, samples: &[f32]) {
        let frames = samples.len() / NUM_CHANNELS;
        for i in 0..frames {
            let idx = (self.write_pos % self.capacity_frames) * NUM_CHANNELS;
            self.buffer[idx] = samples[i * NUM_CHANNELS];
            self.buffer[idx + 1] = samples[i * NUM_CHANNELS + 1];
            self.write_pos = self.write_pos.wrapping_add(1);
        }
    }

    /// Read interleaved stereo samples from the buffer.
    /// Returns the number of frames actually read.
    fn read(&mut self, samples: &mut [f32]) -> usize {
        let requested_frames = samples.len() / NUM_CHANNELS;
        let available = self.write_pos.wrapping_sub(self.read_pos);
        let frames_to_read = requested_frames.min(available);

        for i in 0..frames_to_read {
            let idx = (self.read_pos % self.capacity_frames) * NUM_CHANNELS;
            samples[i * NUM_CHANNELS] = self.buffer[idx];
            samples[i * NUM_CHANNELS + 1] = self.buffer[idx + 1];
            self.read_pos = self.read_pos.wrapping_add(1);
        }

        // Zero-fill if not enough data
        for i in frames_to_read..requested_frames {
            samples[i * NUM_CHANNELS] = 0.0;
            samples[i * NUM_CHANNELS + 1] = 0.0;
        }

        frames_to_read
    }

    /// Get available frames to read.
    fn available(&self) -> usize {
        self.write_pos.wrapping_sub(self.read_pos)
    }
}

/// A pair of PipeWire streams for plugin audio processing.
pub struct PluginFilterStreams {
    /// Channel ID this filter belongs to.
    pub channel_id: Uuid,
    /// Capture stream (receives audio from virtual sink).
    capture_stream: StreamRc,
    /// Playback stream (outputs to master sink).
    playback_stream: StreamRc,
    /// Capture stream listener (keeps callbacks alive).
    _capture_listener: StreamListener<StreamUserData>,
    /// Playback stream listener (keeps callbacks alive).
    _playback_listener: StreamListener<StreamUserData>,
    /// Shared processing context.
    context: Rc<RefCell<PluginProcessingContext>>,
    /// Whether the filter is active.
    active: AtomicBool,
}

impl PluginFilterStreams {
    /// Create a new plugin filter stream pair.
    ///
    /// # Arguments
    /// * `core` - PipeWire core connection
    /// * `channel_id` - Channel UUID
    /// * `channel_name` - Human-readable channel name
    /// * `plugin_instances` - Shared plugin instances for processing
    /// * `plugin_chain` - Plugin instance IDs in processing order
    /// * `param_reader` - Ring buffer reader for parameter updates
    /// * `sample_rate` - Audio sample rate
    /// * `block_size` - Processing block size
    pub fn new(
        core: &pipewire::core::CoreRc,
        channel_id: Uuid,
        channel_name: &str,
        plugin_instances: SharedPluginInstances,
        plugin_chain: Vec<Uuid>,
        param_reader: RingBufferReader<PluginParamUpdate>,
        sample_rate: f32,
        block_size: usize,
    ) -> Result<Self, pipewire::Error> {
        info!(
            "Creating plugin filter streams for channel '{}' ({})",
            channel_name, channel_id
        );

        // Create shared processing context
        let context = Rc::new(RefCell::new(PluginProcessingContext::new(
            plugin_instances,
            plugin_chain,
            param_reader,
            sample_rate,
            block_size,
        )));

        // Create shared audio buffer
        let audio_buffer = Rc::new(RefCell::new(AudioRingBuffer::new(AUDIO_BUFFER_FRAMES)));

        // Create capture stream (receives from virtual sink)
        let capture_name = format!("sootmix.plugins.{}.capture", channel_name);
        let capture_stream = StreamRc::new(
            core.clone(),
            &capture_name,
            properties! {
                "media.type" => "Audio",
                "media.category" => "Filter",
                "media.role" => "DSP",
                "node.name" => capture_name.clone(),
                "node.description" => format!("SootMix Plugins - {} (capture)", channel_name),
                "audio.channels" => "2",
                "audio.position" => "FL,FR"
            },
        )?;

        // Create playback stream (outputs to master)
        let playback_name = format!("sootmix.plugins.{}.playback", channel_name);
        let playback_stream = StreamRc::new(
            core.clone(),
            &playback_name,
            properties! {
                "media.type" => "Audio",
                "media.category" => "Filter",
                "media.role" => "DSP",
                "node.name" => playback_name.clone(),
                "node.description" => format!("SootMix Plugins - {} (playback)", channel_name),
                "audio.channels" => "2",
                "audio.position" => "FL,FR"
            },
        )?;

        // Set up capture stream listener
        let capture_user_data = StreamUserData {
            context: Rc::clone(&context),
            audio_buffer: Rc::clone(&audio_buffer),
            is_capture: true,
            channel_id,
        };

        let capture_listener = capture_stream
            .add_local_listener_with_user_data(capture_user_data)
            .state_changed(|_stream, user_data, old, new| {
                debug!(
                    "Capture stream state changed: {:?} -> {:?} (channel {})",
                    old, new, user_data.channel_id
                );
            })
            .process(process_callback)
            .register()?;

        // Set up playback stream listener
        let playback_user_data = StreamUserData {
            context: Rc::clone(&context),
            audio_buffer: Rc::clone(&audio_buffer),
            is_capture: false,
            channel_id,
        };

        let playback_listener = playback_stream
            .add_local_listener_with_user_data(playback_user_data)
            .state_changed(|_stream, user_data, old, new| {
                debug!(
                    "Playback stream state changed: {:?} -> {:?} (channel {})",
                    old, new, user_data.channel_id
                );
            })
            .process(process_callback)
            .register()?;

        Ok(Self {
            channel_id,
            capture_stream,
            playback_stream,
            _capture_listener: capture_listener,
            _playback_listener: playback_listener,
            context,
            active: AtomicBool::new(false),
        })
    }

    /// Connect the streams and start processing.
    ///
    /// # Arguments
    /// * `capture_target` - Node ID to capture from (loopback output), or None for auto-connect
    /// * `playback_target` - Node ID to play to (master sink), or None for default sink
    pub fn connect(
        &self,
        capture_target: Option<u32>,
        playback_target: Option<u32>,
    ) -> Result<(), pipewire::Error> {
        info!(
            "Connecting plugin filter streams for channel {} (capture={:?}, playback={:?})",
            self.channel_id, capture_target, playback_target
        );

        // Base flags for all streams
        let base_flags = StreamFlags::MAP_BUFFERS | StreamFlags::RT_PROCESS;

        // Connect capture stream (input direction = receiving audio)
        // Use AUTOCONNECT only if no specific target is provided
        let capture_flags = if capture_target.is_some() {
            base_flags
        } else {
            base_flags | StreamFlags::AUTOCONNECT
        };

        self.capture_stream.connect(
            libspa::utils::Direction::Input,
            capture_target,
            capture_flags,
            &mut [],
        )?;

        // Connect playback stream (output direction = sending audio)
        // Always use AUTOCONNECT for playback to connect to default sink
        self.playback_stream.connect(
            libspa::utils::Direction::Output,
            playback_target,
            base_flags | StreamFlags::AUTOCONNECT,
            &mut [],
        )?;

        self.active.store(true, Ordering::Relaxed);
        Ok(())
    }

    /// Disconnect the streams.
    pub fn disconnect(&self) -> Result<(), pipewire::Error> {
        info!(
            "Disconnecting plugin filter streams for channel {}",
            self.channel_id
        );

        self.active.store(false, Ordering::Relaxed);
        self.capture_stream.disconnect()?;
        self.playback_stream.disconnect()?;
        Ok(())
    }

    /// Update the plugin chain.
    pub fn update_plugin_chain(&self, plugin_chain: Vec<Uuid>) {
        self.context.borrow_mut().plugin_chain = plugin_chain;
    }

    /// Set bypass state.
    pub fn set_bypassed(&self, bypassed: bool) {
        self.context.borrow().set_bypassed(bypassed);
    }

    /// Get the capture stream's node ID.
    pub fn capture_node_id(&self) -> u32 {
        self.capture_stream.node_id()
    }

    /// Get the playback stream's node ID.
    pub fn playback_node_id(&self) -> u32 {
        self.playback_stream.node_id()
    }

    /// Check if active.
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }
}

/// Process callback for both capture and playback streams.
fn process_callback(stream: &Stream, user_data: &mut StreamUserData) {
    // Dequeue the buffer
    let mut buffer = match stream.dequeue_buffer() {
        Some(b) => b,
        None => {
            trace!("No buffer available");
            return;
        }
    };

    let datas = buffer.datas_mut();
    if datas.is_empty() {
        return;
    }

    let data = &mut datas[0];
    let chunk = data.chunk();
    let n_frames = chunk.size() as usize / (NUM_CHANNELS * std::mem::size_of::<f32>());

    if n_frames == 0 {
        return;
    }

    // Get the raw audio data
    let audio_data = match data.data() {
        Some(d) => d,
        None => return,
    };

    // Interpret as f32 samples
    let samples: &mut [f32] = unsafe {
        std::slice::from_raw_parts_mut(
            audio_data.as_mut_ptr() as *mut f32,
            n_frames * NUM_CHANNELS,
        )
    };

    if user_data.is_capture {
        // Capture stream: process audio and write to shared buffer
        process_capture(user_data, samples, n_frames);
    } else {
        // Playback stream: read from shared buffer
        process_playback(user_data, samples, n_frames);
    }

    // Update chunk size for output
    let chunk_mut = data.chunk_mut();
    *chunk_mut.size_mut() = (n_frames * NUM_CHANNELS * std::mem::size_of::<f32>()) as u32;
    *chunk_mut.offset_mut() = 0;
    *chunk_mut.stride_mut() = (NUM_CHANNELS * std::mem::size_of::<f32>()) as i32;

    // Buffer is automatically queued when dropped
}

/// Process capture stream: receive audio, process through plugins, write to buffer.
fn process_capture(user_data: &mut StreamUserData, samples: &mut [f32], n_frames: usize) {
    let mut context = user_data.context.borrow_mut();

    // Drain parameter updates
    let updates = context.drain_param_updates();
    if updates > 0 {
        trace!("Applied {} parameter updates", updates);
    }

    // Prepare input/output buffers for processing
    // Split interleaved into separate channels
    let mut left_in = vec![0.0f32; n_frames];
    let mut right_in = vec![0.0f32; n_frames];
    let mut left_out = vec![0.0f32; n_frames];
    let mut right_out = vec![0.0f32; n_frames];

    // Deinterleave input
    for i in 0..n_frames {
        left_in[i] = samples[i * NUM_CHANNELS];
        right_in[i] = samples[i * NUM_CHANNELS + 1];
    }

    // Process through plugin chain
    let inputs: Vec<&[f32]> = vec![&left_in, &right_in];
    let mut outputs: Vec<&mut [f32]> = vec![&mut left_out, &mut right_out];

    context.process_audio(&inputs, &mut outputs);

    // Interleave output back into samples
    for i in 0..n_frames {
        samples[i * NUM_CHANNELS] = left_out[i];
        samples[i * NUM_CHANNELS + 1] = right_out[i];
    }

    // Write processed audio to shared buffer for playback stream
    user_data.audio_buffer.borrow_mut().write(samples);
}

/// Process playback stream: read from shared buffer and output.
fn process_playback(user_data: &mut StreamUserData, samples: &mut [f32], n_frames: usize) {
    let mut audio_buffer = user_data.audio_buffer.borrow_mut();
    let frames_read = audio_buffer.read(samples);

    if frames_read < n_frames {
        trace!(
            "Playback underrun: got {} frames, needed {}",
            frames_read,
            n_frames
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_ring_buffer() {
        let mut buffer = AudioRingBuffer::new(16);

        // Write some samples
        let input = vec![1.0, 2.0, 3.0, 4.0]; // 2 frames
        buffer.write(&input);
        assert_eq!(buffer.available(), 2);

        // Read them back
        let mut output = vec![0.0; 4];
        let frames = buffer.read(&mut output);
        assert_eq!(frames, 2);
        assert_eq!(output, vec![1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn test_audio_ring_buffer_underrun() {
        let mut buffer = AudioRingBuffer::new(16);

        // Write 1 frame
        buffer.write(&[1.0, 2.0]);

        // Try to read 2 frames
        let mut output = vec![0.0; 4];
        let frames = buffer.read(&mut output);
        assert_eq!(frames, 1);
        // First frame is data, second is zeros
        assert_eq!(output[0], 1.0);
        assert_eq!(output[1], 2.0);
        assert_eq!(output[2], 0.0);
        assert_eq!(output[3], 0.0);
    }
}
