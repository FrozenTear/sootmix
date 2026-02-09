// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Native PipeWire loopback implementation.
//!
//! This module replaces the CLI-based `pw-loopback` with native PipeWire streams.
//! It creates stream pairs for virtual sinks (output channels) and virtual sources
//! (input channels).
//!
//! # Architecture
//!
//! ## Virtual Sink (Output Channel)
//! ```text
//! [Apps] → [Audio/Sink] → [internal buffer] → [Stream/Output/Audio] → [Master]
//!              ↑                                      ↓
//!         capture_stream                        playback_stream
//! ```
//!
//! ## Virtual Source (Input Channel)
//! ```text
//! [Mic] → [Stream/Input/Audio] → [internal buffer] → [Audio/Source] → [Apps]
//!               ↑                                          ↓
//!         capture_stream                            playback_stream
//! ```

#![allow(dead_code)]

use pipewire::properties::properties;
use pipewire::stream::{Stream, StreamFlags, StreamListener, StreamRc};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use tracing::{debug, info, trace, warn};
use uuid::Uuid;

/// Size of the audio buffer between capture and playback streams (in frames).
const AUDIO_BUFFER_FRAMES: usize = 2048;

/// Number of audio channels (stereo).
const NUM_CHANNELS: usize = 2;

// ============================================================================
// AUDIO RING BUFFER
// ============================================================================

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
}

// ============================================================================
// ATOMIC METER LEVELS
// ============================================================================

/// Thread-safe meter levels using atomic operations.
#[derive(Debug, Default)]
pub struct AtomicMeterLevels {
    peak_left: AtomicU32,
    peak_right: AtomicU32,
}

impl AtomicMeterLevels {
    pub fn new() -> Self {
        Self {
            peak_left: AtomicU32::new(0),
            peak_right: AtomicU32::new(0),
        }
    }

    #[inline]
    pub fn store(&self, left: f32, right: f32) {
        self.peak_left.store(left.to_bits(), Ordering::Relaxed);
        self.peak_right.store(right.to_bits(), Ordering::Relaxed);
    }

    #[inline]
    pub fn load(&self) -> (f32, f32) {
        let left = f32::from_bits(self.peak_left.load(Ordering::Relaxed));
        let right = f32::from_bits(self.peak_right.load(Ordering::Relaxed));
        (left, right)
    }
}

// ============================================================================
// STREAM USER DATA
// ============================================================================

/// User data passed to stream process callbacks.
struct LoopbackUserData {
    /// Shared audio buffer between capture and playback streams.
    audio_buffer: Rc<RefCell<AudioRingBuffer>>,
    /// Atomic meter levels for real-time level reporting.
    meter_levels: Arc<AtomicMeterLevels>,
    /// Whether this is the capture (true) or playback (false) stream.
    is_capture: bool,
    /// Channel ID for logging.
    channel_id: Uuid,
}

// ============================================================================
// NATIVE LOOPBACK
// ============================================================================

/// A native PipeWire loopback - replaces pw-loopback CLI.
///
/// Creates a pair of streams that route audio internally:
/// - For sinks: Apps play to Audio/Sink, output routes to master
/// - For sources: Mic captures to stream, output as Audio/Source for apps
pub struct NativeLoopback {
    /// Channel ID this loopback belongs to.
    pub channel_id: Uuid,
    /// Whether this is a sink (output) or source (input) loopback.
    pub is_source: bool,
    /// Capture stream (Audio/Sink for output channels, Stream/Input/Audio for input).
    capture_stream: StreamRc,
    /// Playback stream (Stream/Output/Audio for output channels, Audio/Source for input).
    playback_stream: StreamRc,
    /// Capture stream listener.
    _capture_listener: StreamListener<LoopbackUserData>,
    /// Playback stream listener.
    _playback_listener: StreamListener<LoopbackUserData>,
    /// Shared meter levels.
    meter_levels: Arc<AtomicMeterLevels>,
    /// Whether connected.
    connected: AtomicBool,
}

impl NativeLoopback {
    /// Create a new virtual sink (for output channels).
    ///
    /// Apps will see an Audio/Sink they can play to.
    /// Audio is routed internally to a Stream/Output/Audio that connects to master.
    ///
    /// # Arguments
    /// * `target_device` - Optional target device name. If provided, the playback stream
    ///   will be routed to this device. If None, uses system default.
    pub fn new_sink(
        core: &pipewire::core::CoreRc,
        channel_id: Uuid,
        name: &str,
        description: &str,
        target_device: Option<&str>,
    ) -> Result<Self, pipewire::Error> {
        let sink_name = format!("sootmix.{}", name);
        let output_name = format!("sootmix.{}.output", name);

        info!(
            "Creating native virtual sink: {} ({}) target={:?}",
            sink_name, channel_id, target_device
        );

        // Shared state
        let audio_buffer = Rc::new(RefCell::new(AudioRingBuffer::new(AUDIO_BUFFER_FRAMES)));
        let meter_levels = Arc::new(AtomicMeterLevels::new());

        // Create capture stream (Audio/Sink - apps play to this)
        let capture_stream = StreamRc::new(
            core.clone(),
            &sink_name,
            properties! {
                "media.type" => "Audio",
                "media.class" => "Audio/Sink",
                "node.name" => sink_name.clone(),
                "node.description" => description,
                "audio.channels" => "2",
                "audio.position" => "FL,FR",
                "priority.session" => "2000"
            },
        )?;

        // Build playback stream properties - include target.object if specified
        // Use factory.mode=merge to get adapter wrapping for proper FL/FR ports
        let mut playback_props = properties! {
            "media.type" => "Audio",
            "media.class" => "Stream/Output/Audio",
            "node.name" => output_name.clone(),
            "node.description" => format!("{} Output", description),
            "audio.channels" => "2",
            "audio.position" => "FL,FR",
            "stream.autoconnect" => "true",
            "factory.mode" => "merge",
            "audio.adapt.follower" => ""
        };

        // Set target.object to route to specific device
        // WirePlumber will handle format conversion automatically
        // Skip if "system-default" - that's our sentinel for WirePlumber's default
        if let Some(device) = target_device {
            if device != "system-default" {
                playback_props.insert("target.object", device);
            }
        }

        // Create playback stream (Stream/Output/Audio - routes to master)
        let playback_stream = StreamRc::new(core.clone(), &output_name, playback_props)?;

        // Set up capture listener
        let capture_user_data = LoopbackUserData {
            audio_buffer: Rc::clone(&audio_buffer),
            meter_levels: Arc::clone(&meter_levels),
            is_capture: true,
            channel_id,
        };

        let capture_listener = capture_stream
            .add_local_listener_with_user_data(capture_user_data)
            .state_changed(|_stream, user_data, old, new| {
                debug!(
                    "Sink capture stream state: {:?} -> {:?} ({})",
                    old, new, user_data.channel_id
                );
            })
            .process(process_callback)
            .register()?;

        // Set up playback listener
        let playback_user_data = LoopbackUserData {
            audio_buffer: Rc::clone(&audio_buffer),
            meter_levels: Arc::clone(&meter_levels),
            is_capture: false,
            channel_id,
        };

        let playback_listener = playback_stream
            .add_local_listener_with_user_data(playback_user_data)
            .state_changed(|_stream, user_data, old, new| {
                debug!(
                    "Sink playback stream state: {:?} -> {:?} ({})",
                    old, new, user_data.channel_id
                );
            })
            .process(process_callback)
            .register()?;

        Ok(Self {
            channel_id,
            is_source: false,
            capture_stream,
            playback_stream,
            _capture_listener: capture_listener,
            _playback_listener: playback_listener,
            meter_levels,
            connected: AtomicBool::new(false),
        })
    }

    /// Create a new virtual source (for input channels).
    ///
    /// Audio is captured from a physical mic (or default source).
    /// Apps will see an Audio/Source they can capture from.
    pub fn new_source(
        core: &pipewire::core::CoreRc,
        channel_id: Uuid,
        name: &str,
        target_device: Option<&str>,
    ) -> Result<Self, pipewire::Error> {
        let source_name = format!("sootmix.{}", name);
        let input_name = format!("sootmix.{}.input", name);

        info!(
            "Creating native virtual source: {} ({}) target={:?}",
            source_name, channel_id, target_device
        );

        // Shared state
        let audio_buffer = Rc::new(RefCell::new(AudioRingBuffer::new(AUDIO_BUFFER_FRAMES)));
        let meter_levels = Arc::new(AtomicMeterLevels::new());

        // Build capture properties - target specific device if provided
        let mut capture_props = properties! {
            "media.type" => "Audio",
            "media.class" => "Stream/Input/Audio",
            "node.name" => input_name.clone(),
            "node.description" => format!("{} Input", name),
            "audio.channels" => "2",
            "audio.position" => "FL,FR",
            "node.passive" => "true"
        };

        if let Some(device) = target_device {
            capture_props.insert("target.object", device);
        }

        // Create capture stream (Stream/Input/Audio - captures from mic)
        let capture_stream = StreamRc::new(core.clone(), &input_name, capture_props)?;

        // Create playback stream (Audio/Source - apps capture from this)
        let playback_stream = StreamRc::new(
            core.clone(),
            &source_name,
            properties! {
                "media.type" => "Audio",
                "media.class" => "Audio/Source",
                "node.name" => source_name.clone(),
                "node.description" => name,
                "audio.channels" => "2",
                "audio.position" => "FL,FR",
                "node.virtual" => "false",
                "device.class" => "audio-input",
                "priority.session" => "2000"
            },
        )?;

        // Set up capture listener
        let capture_user_data = LoopbackUserData {
            audio_buffer: Rc::clone(&audio_buffer),
            meter_levels: Arc::clone(&meter_levels),
            is_capture: true,
            channel_id,
        };

        let capture_listener = capture_stream
            .add_local_listener_with_user_data(capture_user_data)
            .state_changed(|_stream, user_data, old, new| {
                debug!(
                    "Source capture stream state: {:?} -> {:?} ({})",
                    old, new, user_data.channel_id
                );
            })
            .process(process_callback)
            .register()?;

        // Set up playback listener
        let playback_user_data = LoopbackUserData {
            audio_buffer: Rc::clone(&audio_buffer),
            meter_levels: Arc::clone(&meter_levels),
            is_capture: false,
            channel_id,
        };

        let playback_listener = playback_stream
            .add_local_listener_with_user_data(playback_user_data)
            .state_changed(|_stream, user_data, old, new| {
                debug!(
                    "Source playback stream state: {:?} -> {:?} ({})",
                    old, new, user_data.channel_id
                );
            })
            .process(process_callback)
            .register()?;

        Ok(Self {
            channel_id,
            is_source: true,
            capture_stream,
            playback_stream,
            _capture_listener: capture_listener,
            _playback_listener: playback_listener,
            meter_levels,
            connected: AtomicBool::new(false),
        })
    }

    /// Connect the streams and start audio flow.
    ///
    /// For sinks: capture connects as input (receiving), playback as output (sending)
    /// For sources: capture connects as input (from mic), playback as output (to apps)
    pub fn connect(&self, playback_target: Option<u32>) -> Result<(), pipewire::Error> {
        info!(
            "Connecting native loopback {} (source={}, target={:?})",
            self.channel_id, self.is_source, playback_target
        );

        let base_flags = StreamFlags::MAP_BUFFERS | StreamFlags::RT_PROCESS;

        if self.is_source {
            // Source: capture from mic (input), output as Audio/Source
            // Capture needs AUTOCONNECT to find the default/target mic
            self.capture_stream.connect(
                libspa::utils::Direction::Input,
                None, // Let WirePlumber handle mic routing
                base_flags | StreamFlags::AUTOCONNECT,
                &mut [],
            )?;

            // Playback (Audio/Source) - apps will connect to this
            // Use DRIVER to make this node drive the graph when apps capture
            self.playback_stream.connect(
                libspa::utils::Direction::Output,
                None,
                base_flags | StreamFlags::DRIVER,
                &mut [],
            )?;
        } else {
            // Sink: apps play to Audio/Sink, output routes to master
            // Capture (Audio/Sink) - apps will connect to this
            self.capture_stream.connect(
                libspa::utils::Direction::Input,
                None,
                base_flags | StreamFlags::DRIVER,
                &mut [],
            )?;

            // Playback connects to master (or specified target)
            self.playback_stream.connect(
                libspa::utils::Direction::Output,
                playback_target,
                base_flags | StreamFlags::AUTOCONNECT,
                &mut [],
            )?;
        }

        // Activate streams
        self.capture_stream.set_active(true)?;
        self.playback_stream.set_active(true)?;

        self.connected.store(true, Ordering::Relaxed);
        Ok(())
    }

    /// Disconnect the streams.
    pub fn disconnect(&self) -> Result<(), pipewire::Error> {
        info!("Disconnecting native loopback {}", self.channel_id);
        self.connected.store(false, Ordering::Relaxed);
        self.capture_stream.disconnect()?;
        self.playback_stream.disconnect()?;
        Ok(())
    }

    /// Get the main node ID (sink node for output channels, source node for input).
    pub fn main_node_id(&self) -> u32 {
        if self.is_source {
            self.playback_stream.node_id()
        } else {
            self.capture_stream.node_id()
        }
    }

    /// Get the sink node ID (for output channels).
    pub fn sink_node_id(&self) -> Option<u32> {
        if self.is_source {
            None
        } else {
            Some(self.capture_stream.node_id())
        }
    }

    /// Get the source node ID (for input channels).
    pub fn source_node_id(&self) -> Option<u32> {
        if self.is_source {
            Some(self.playback_stream.node_id())
        } else {
            None
        }
    }

    /// Get the loopback output node ID (Stream/Output/Audio for sinks).
    pub fn loopback_output_node_id(&self) -> Option<u32> {
        if self.is_source {
            None
        } else {
            Some(self.playback_stream.node_id())
        }
    }

    /// Get the loopback capture node ID (Stream/Input/Audio for sources).
    pub fn loopback_capture_node_id(&self) -> Option<u32> {
        if self.is_source {
            Some(self.capture_stream.node_id())
        } else {
            None
        }
    }

    /// Get meter levels.
    pub fn meter_levels(&self) -> &Arc<AtomicMeterLevels> {
        &self.meter_levels
    }

    /// Check if connected.
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    /// Re-route the loopback output to a different target device.
    ///
    /// For output channels (sinks), this changes where the playback stream routes to.
    /// Uses WirePlumber metadata to change the target without disconnecting.
    ///
    /// # Arguments
    /// * `target_node_id` - The node ID of the target device (or None for system default)
    pub fn reroute_to_device(&self, target_node_id: Option<u32>) -> Result<(), String> {
        if self.is_source {
            // For input channels, we'd need to change the capture target
            // This is more complex - for now, just log
            warn!(
                "Re-routing input channel {} not yet implemented via native API",
                self.channel_id
            );
            return Err("Input channel re-routing not implemented".to_string());
        }

        let playback_node_id = self.playback_stream.node_id();
        if playback_node_id == u32::MAX {
            return Err("Playback stream has no valid node ID".to_string());
        }

        info!(
            "Re-routing loopback {} (node {}) to device {:?}",
            self.channel_id, playback_node_id, target_node_id
        );

        // Use pw-metadata to set target.node - this is the WirePlumber-native way
        // WirePlumber will handle the actual link creation with proper format conversion
        let args = if let Some(target_id) = target_node_id {
            vec![
                "-n".to_string(),
                "default".to_string(),
                playback_node_id.to_string(),
                "target.node".to_string(),
                format!("{{ \"name\": \"target.node\", \"value\": {} }}", target_id),
            ]
        } else {
            // Clear the target to use system default
            vec![
                "-n".to_string(),
                "default".to_string(),
                "-d".to_string(),
                playback_node_id.to_string(),
                "target.node".to_string(),
            ]
        };

        let output = std::process::Command::new("pw-metadata")
            .args(&args)
            .output()
            .map_err(|e| format!("Failed to run pw-metadata: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Not fatal - WirePlumber may still route correctly via AUTOCONNECT
            debug!("pw-metadata returned non-zero: {}", stderr);
        }

        Ok(())
    }

    /// Get the playback stream node ID (for routing operations).
    pub fn playback_node_id(&self) -> u32 {
        self.playback_stream.node_id()
    }
}

// ============================================================================
// PROCESS CALLBACK
// ============================================================================

/// Process callback for loopback streams.
fn process_callback(stream: &Stream, user_data: &mut LoopbackUserData) {
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

    let audio_data = match data.data() {
        Some(d) => d,
        None => return,
    };

    let samples: &mut [f32] = unsafe {
        std::slice::from_raw_parts_mut(audio_data.as_mut_ptr() as *mut f32, n_frames * NUM_CHANNELS)
    };

    if user_data.is_capture {
        // Capture: calculate peaks and write to buffer
        let (peak_left, peak_right) = calculate_stereo_peaks(samples);
        user_data.meter_levels.store(peak_left, peak_right);
        user_data.audio_buffer.borrow_mut().write(samples);
    } else {
        // Playback: read from buffer
        let frames_read = user_data.audio_buffer.borrow_mut().read(samples);
        if frames_read < n_frames {
            trace!(
                "Playback underrun: got {} frames, needed {}",
                frames_read,
                n_frames
            );
        }
    }

    // Update chunk for output
    let chunk_mut = data.chunk_mut();
    *chunk_mut.size_mut() = (n_frames * NUM_CHANNELS * std::mem::size_of::<f32>()) as u32;
    *chunk_mut.offset_mut() = 0;
    *chunk_mut.stride_mut() = (NUM_CHANNELS * std::mem::size_of::<f32>()) as i32;
}

/// Calculate stereo peak levels from interleaved samples.
#[inline]
fn calculate_stereo_peaks(samples: &[f32]) -> (f32, f32) {
    let mut peak_left: f32 = 0.0;
    let mut peak_right: f32 = 0.0;

    let mut i = 0;
    while i + 1 < samples.len() {
        let left_abs = samples[i].abs();
        let right_abs = samples[i + 1].abs();

        if left_abs > peak_left {
            peak_left = left_abs;
        }
        if right_abs > peak_right {
            peak_right = right_abs;
        }

        i += 2;
    }

    (peak_left, peak_right)
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
        assert_eq!(output[0], 1.0);
        assert_eq!(output[1], 2.0);
        assert_eq!(output[2], 0.0);
        assert_eq!(output[3], 0.0);
    }

    #[test]
    fn test_stereo_peaks() {
        let samples = vec![0.1, 0.2, -0.5, 0.3, 0.4, -0.8];
        let (left, right) = calculate_stereo_peaks(&samples);
        assert!((left - 0.5).abs() < 0.001);
        assert!((right - 0.8).abs() < 0.001);
    }

    #[test]
    fn test_atomic_meter_levels() {
        let levels = AtomicMeterLevels::new();
        levels.store(0.5, 0.75);
        let (l, r) = levels.load();
        assert!((l - 0.5).abs() < 0.001);
        assert!((r - 0.75).abs() < 0.001);
    }
}
