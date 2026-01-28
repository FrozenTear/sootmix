// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Real-time audio level metering via PipeWire streams.
//!
//! This module provides actual audio level monitoring by creating lightweight
//! PipeWire capture streams that connect to the monitor ports of virtual sinks.
//!
//! # Architecture
//!
//! ```text
//! [Virtual Sink] → [monitor_FL/FR ports]
//!                          ↓
//!                  [MeterCaptureStream]
//!                          ↓
//!                  [Peak calculation in RT callback]
//!                          ↓
//!                  [AtomicMeterLevels (lock-free)]
//!                          ↓
//!                  [UI reads for display]
//! ```

use pipewire::properties::properties;
use pipewire::stream::{Stream, StreamFlags, StreamListener, StreamRc};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tracing::{debug, info, trace, warn};
use uuid::Uuid;

// ============================================================================
// ATOMIC METER LEVELS
// ============================================================================

/// Thread-safe meter levels using atomic operations.
///
/// Uses AtomicU32 with f32 bit patterns for lock-free updates from the RT thread.
#[derive(Debug, Default)]
pub struct AtomicMeterLevels {
    /// Left channel peak level (f32 bits stored as u32).
    peak_left: AtomicU32,
    /// Right channel peak level (f32 bits stored as u32).
    peak_right: AtomicU32,
    /// Whether this meter has received any audio data.
    active: AtomicU32,
}

impl AtomicMeterLevels {
    /// Create new meter levels initialized to zero.
    pub fn new() -> Self {
        Self {
            peak_left: AtomicU32::new(0),
            peak_right: AtomicU32::new(0),
            active: AtomicU32::new(0),
        }
    }

    /// Store peak levels (called from RT thread).
    #[inline]
    pub fn store(&self, left: f32, right: f32) {
        self.peak_left.store(left.to_bits(), Ordering::Relaxed);
        self.peak_right.store(right.to_bits(), Ordering::Relaxed);
        self.active.store(1, Ordering::Relaxed);
    }

    /// Load peak levels (called from UI thread).
    #[inline]
    pub fn load(&self) -> (f32, f32) {
        let left = f32::from_bits(self.peak_left.load(Ordering::Relaxed));
        let right = f32::from_bits(self.peak_right.load(Ordering::Relaxed));
        (left, right)
    }

    /// Check if this meter is active (receiving audio).
    #[inline]
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Relaxed) != 0
    }

    /// Reset to inactive state.
    pub fn reset(&self) {
        self.peak_left.store(0, Ordering::Relaxed);
        self.peak_right.store(0, Ordering::Relaxed);
        self.active.store(0, Ordering::Relaxed);
    }
}

// ============================================================================
// PEAK CALCULATION
// ============================================================================

/// Calculate peak level from audio samples.
///
/// This is RT-safe - no allocations or blocking.
#[inline]
pub fn calculate_peak(samples: &[f32]) -> f32 {
    let mut peak: f32 = 0.0;
    for &sample in samples {
        let abs = sample.abs();
        if abs > peak {
            peak = abs;
        }
    }
    peak
}

/// Calculate stereo peak levels from interleaved samples.
///
/// Assumes stereo interleaved format: [L0, R0, L1, R1, ...]
#[inline]
pub fn calculate_stereo_peaks(samples: &[f32]) -> (f32, f32) {
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

// ============================================================================
// METER CAPTURE STREAM
// ============================================================================

const NUM_CHANNELS: usize = 2;

/// User data for meter stream callback.
struct MeterUserData {
    /// Atomic levels to store peaks.
    levels: Arc<AtomicMeterLevels>,
    /// Channel ID for logging.
    channel_id: Uuid,
}

/// A lightweight PipeWire stream that captures audio for metering.
///
/// This stream connects to a virtual sink's monitor ports and calculates
/// peak levels in real-time without affecting the audio path.
pub struct MeterCaptureStream {
    /// Channel ID this meter belongs to.
    pub channel_id: Uuid,
    /// The capture stream.
    stream: StreamRc,
    /// Stream listener (keeps callback alive).
    _listener: StreamListener<MeterUserData>,
    /// Shared atomic levels.
    levels: Arc<AtomicMeterLevels>,
}

impl MeterCaptureStream {
    /// Create a new meter capture stream.
    ///
    /// # Arguments
    /// * `core` - PipeWire core connection
    /// * `channel_id` - Channel UUID
    /// * `channel_name` - Human-readable channel name
    /// * `target_node_name` - Name of the node to capture from (the virtual sink)
    /// * `levels` - Atomic levels to store peaks (shared with UI)
    pub fn new(
        core: &pipewire::core::CoreRc,
        channel_id: Uuid,
        channel_name: &str,
        target_node_name: &str,
        levels: Arc<AtomicMeterLevels>,
    ) -> Result<Self, pipewire::Error> {
        let stream_name = format!("sootmix.meter.{}", channel_name);

        info!(
            "Creating meter capture stream for channel '{}' ({}) targeting '{}'",
            channel_name, channel_id, target_node_name
        );

        let stream = StreamRc::new(
            core.clone(),
            &stream_name,
            properties! {
                "media.type" => "Audio",
                "media.class" => "Stream/Input/Audio",
                "media.name" => "Peak detect",
                "media.role" => "DSP",
                "node.name" => stream_name.clone(),
                "node.description" => format!("SootMix Meter - {}", channel_name),
                "node.passive" => "true",
                "node.rate" => "1/30",
                "node.latency" => "1/30",
                "stream.monitor" => "true",
                "resample.peaks" => "true",
                "target.object" => target_node_name,
                "audio.channels" => "2",
                "audio.position" => "FL,FR"
            },
        )?;

        let user_data = MeterUserData {
            levels: Arc::clone(&levels),
            channel_id,
        };

        let listener = stream
            .add_local_listener_with_user_data(user_data)
            .state_changed(|_stream, user_data, old, new| {
                debug!(
                    "Meter stream state changed: {:?} -> {:?} (channel {})",
                    old, new, user_data.channel_id
                );
            })
            .process(meter_process_callback)
            .register()?;

        Ok(Self {
            channel_id,
            stream,
            _listener: listener,
            levels,
        })
    }

    /// Connect the stream to a target node (the virtual sink).
    ///
    /// # Arguments
    /// * `target_node_id` - The virtual sink node ID to capture from
    pub fn connect(&self, target_node_id: u32) -> Result<(), pipewire::Error> {
        info!(
            "Connecting meter stream for channel {} to sink {}",
            self.channel_id, target_node_id
        );

        // Connect as input (capturing audio from sink's monitor ports)
        // AUTOCONNECT lets PipeWire/WirePlumber handle the actual connection
        let flags = StreamFlags::MAP_BUFFERS | StreamFlags::RT_PROCESS | StreamFlags::AUTOCONNECT;

        self.stream.connect(
            libspa::utils::Direction::Input,
            Some(target_node_id),
            flags,
            &mut [],
        )?;

        Ok(())
    }

    /// Disconnect the stream.
    pub fn disconnect(&self) -> Result<(), pipewire::Error> {
        info!(
            "Disconnecting meter stream for channel {}",
            self.channel_id
        );
        self.levels.reset();
        self.stream.disconnect()
    }

    /// Get the stream's node ID.
    pub fn node_id(&self) -> u32 {
        self.stream.node_id()
    }

    /// Get a reference to the atomic levels.
    pub fn levels(&self) -> &Arc<AtomicMeterLevels> {
        &self.levels
    }
}

/// Process callback for meter capture stream.
///
/// With `resample.peaks = true`, PipeWire outputs peak amplitude values directly
/// (one per channel per update period).
fn meter_process_callback(stream: &Stream, user_data: &mut MeterUserData) {
    // Dequeue the buffer
    let mut buffer = match stream.dequeue_buffer() {
        Some(b) => b,
        None => return,
    };

    let datas = buffer.datas_mut();
    if datas.is_empty() {
        return;
    }

    let data = &mut datas[0];
    let chunk = data.chunk();
    let size = chunk.size() as usize;

    if size == 0 {
        return;
    }

    // Get the raw data
    let raw_data = match data.data() {
        Some(d) => d,
        None => return,
    };

    // With resample.peaks=true, we get peak values as f32 samples
    // The number of samples depends on node.rate and how many channels
    let n_samples = size / std::mem::size_of::<f32>();

    if n_samples == 0 {
        return;
    }

    let samples: &[f32] = unsafe {
        std::slice::from_raw_parts(raw_data.as_ptr() as *const f32, n_samples)
    };

    // For stereo, take the peak from available samples
    // With low rates, we might get 1-2 samples per channel
    let (peak_left, peak_right) = if n_samples >= 2 {
        // Interleaved stereo: [L, R, ...]
        let mut left_peak: f32 = 0.0;
        let mut right_peak: f32 = 0.0;
        for i in (0..n_samples).step_by(2) {
            left_peak = left_peak.max(samples[i].abs());
            if i + 1 < n_samples {
                right_peak = right_peak.max(samples[i + 1].abs());
            }
        }
        (left_peak, right_peak)
    } else {
        // Mono or single sample - use for both channels
        let peak = samples[0].abs();
        (peak, peak)
    };

    // Clamp to valid range
    let peak_left = peak_left.clamp(0.0, 1.0);
    let peak_right = peak_right.clamp(0.0, 1.0);

    // Store in atomic levels
    user_data.levels.store(peak_left, peak_right);

    trace!(
        "Meter {}: L={:.3} R={:.3} (samples={})",
        user_data.channel_id,
        peak_left,
        peak_right,
        n_samples
    );
}

// ============================================================================
// METER STREAM MANAGER
// ============================================================================

/// Manages meter capture streams for all channels.
pub struct MeterStreamManager {
    /// Active meter streams, keyed by channel ID.
    streams: HashMap<Uuid, MeterCaptureStream>,
}

impl MeterStreamManager {
    /// Create a new meter stream manager.
    pub fn new() -> Self {
        Self {
            streams: HashMap::new(),
        }
    }

    /// Create a meter stream for a channel.
    pub fn create_stream(
        &mut self,
        core: &pipewire::core::CoreRc,
        channel_id: Uuid,
        channel_name: &str,
        target_node_name: &str,
        levels: Arc<AtomicMeterLevels>,
    ) -> Result<(), pipewire::Error> {
        // Remove existing stream if any
        self.destroy_stream(channel_id);

        let stream = MeterCaptureStream::new(core, channel_id, channel_name, target_node_name, levels)?;
        self.streams.insert(channel_id, stream);

        Ok(())
    }

    /// Connect a meter stream to a sink node.
    pub fn connect_stream(
        &self,
        channel_id: Uuid,
        sink_node_id: u32,
    ) -> Result<(), pipewire::Error> {
        if let Some(stream) = self.streams.get(&channel_id) {
            stream.connect(sink_node_id)?;
        } else {
            warn!(
                "Cannot connect meter stream: channel {} not found",
                channel_id
            );
        }
        Ok(())
    }

    /// Destroy a meter stream.
    pub fn destroy_stream(&mut self, channel_id: Uuid) {
        if let Some(stream) = self.streams.remove(&channel_id) {
            let _ = stream.disconnect();
            info!("Destroyed meter stream for channel {}", channel_id);
        }
    }

    /// Check if a channel has a meter stream.
    pub fn has_stream(&self, channel_id: Uuid) -> bool {
        self.streams.contains_key(&channel_id)
    }

    /// Get a stream's node ID.
    pub fn stream_node_id(&self, channel_id: Uuid) -> Option<u32> {
        self.streams.get(&channel_id).map(|s| s.node_id())
    }
}

impl Default for MeterStreamManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_atomic_levels() {
        let levels = AtomicMeterLevels::new();

        // Initially zero
        let (l, r) = levels.load();
        assert_eq!(l, 0.0);
        assert_eq!(r, 0.0);
        assert!(!levels.is_active());

        // Store values
        levels.store(0.5, 0.75);
        let (l, r) = levels.load();
        assert!((l - 0.5).abs() < 0.001);
        assert!((r - 0.75).abs() < 0.001);
        assert!(levels.is_active());

        // Reset
        levels.reset();
        assert!(!levels.is_active());
    }

    #[test]
    fn test_peak_calculation() {
        let samples = vec![0.1, -0.5, 0.3, -0.2, 0.8, -0.1];
        let peak = calculate_peak(&samples);
        assert!((peak - 0.8).abs() < 0.001);
    }

    #[test]
    fn test_stereo_peaks() {
        // Interleaved: [L0, R0, L1, R1, ...]
        let samples = vec![0.1, 0.2, -0.5, 0.3, 0.4, -0.8];
        let (left, right) = calculate_stereo_peaks(&samples);
        assert!((left - 0.5).abs() < 0.001);
        assert!((right - 0.8).abs() < 0.001);
    }
}
