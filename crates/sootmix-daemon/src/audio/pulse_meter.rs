// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! PulseAudio-based metering for input/output channels.
//!
//! This module uses the PulseAudio API (via PipeWire's PA compatibility layer)
//! to perform reliable audio level metering. The key advantage is using
//! `PA_STREAM_PEAK_DETECT` which enables server-side peak calculation.
//!
//! Honest metering rules (Engine "meters first" slice):
//! - Max-hold peaks (`store_max`); the poller uses `load_and_reset`.
//! - Prefer a stereo peak stream when the source is stereo. Do not copy a
//!   mono peak onto both L/R when real channel data exists.
//! - Never invent / simulate bounce. Silence is silence.
//! - `request_stop()` never `join()`s — joining a meter thread on the
//!   PipeWire thread would stall the graph.

use crate::audio::native_loopback::AtomicMeterLevels;
use libpulse_binding::context::{Context, FlagSet as ContextFlagSet, State as ContextState};
use libpulse_binding::mainloop::standard::Mainloop;
use libpulse_binding::sample::{Format, Spec};
use libpulse_binding::stream::{FlagSet as StreamFlagSet, State as StreamState, Stream};
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use thiserror::Error;
use tracing::{debug, error, info, trace, warn};
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum PulseMeterError {
    #[error("Failed to spawn PA meter thread: {0}")]
    SpawnFailed(String),
}

/// PulseAudio-based meter for input/output channels.
///
/// Runs a dedicated thread with its own PA mainloop to capture peak levels
/// from a PulseAudio source (microphone or sink monitor).
pub struct PulseAudioMeter {
    /// Channel ID this meter belongs to.
    channel_id: Uuid,
    /// PulseAudio source name to monitor.
    source_name: String,
    /// Atomic levels shared with the main thread.
    levels: Arc<AtomicMeterLevels>,
    /// Flag to signal the meter thread to stop.
    running: Arc<AtomicBool>,
    /// Thread handle (if started). Never joined on the PipeWire thread.
    thread_handle: RefCell<Option<JoinHandle<()>>>,
}

impl PulseAudioMeter {
    /// Create a new PulseAudio meter.
    ///
    /// # Arguments
    /// * `channel_id` - Channel UUID for logging
    /// * `source_name` - PulseAudio source name (or empty for default)
    /// * `levels` - Atomic levels to store peaks (shared with meter polling)
    pub fn new(channel_id: Uuid, source_name: &str, levels: Arc<AtomicMeterLevels>) -> Self {
        let source = if source_name.is_empty() {
            "@DEFAULT_SOURCE@".to_string()
        } else {
            source_name.to_string()
        };

        info!(
            "Creating PulseAudio meter for channel {} targeting '{}'",
            channel_id, source
        );

        Self {
            channel_id,
            source_name: source,
            levels,
            running: Arc::new(AtomicBool::new(false)),
            thread_handle: RefCell::new(None),
        }
    }

    /// PulseAudio source this meter is attached to (for remount matching).
    pub fn source_name(&self) -> &str {
        &self.source_name
    }

    /// Start the meter thread.
    ///
    /// Spawns a background thread that runs the PulseAudio mainloop
    /// and captures peak levels from the configured source.
    ///
    /// Returns an error if the OS cannot spawn the thread — callers must
    /// surface this (do not `.expect`).
    pub fn start(&self) -> Result<(), PulseMeterError> {
        if self.running.load(Ordering::Relaxed) {
            warn!(
                "PulseAudio meter for channel {} already running",
                self.channel_id
            );
            return Ok(());
        }

        // Reap a finished handle off-thread before spawning a replacement.
        if let Some(handle) = self.thread_handle.borrow_mut().take() {
            detach_join(handle);
        }

        self.running.store(true, Ordering::Relaxed);

        let channel_id = self.channel_id;
        let source_name = self.source_name.clone();
        let levels = Arc::clone(&self.levels);
        let running = Arc::clone(&self.running);

        let handle = thread::Builder::new()
            .name(format!("pa-meter-{}", channel_id))
            .spawn(move || {
                meter_thread(channel_id, source_name, levels, running);
            })
            .map_err(|e| {
                self.running.store(false, Ordering::Relaxed);
                PulseMeterError::SpawnFailed(e.to_string())
            })?;

        *self.thread_handle.borrow_mut() = Some(handle);
        info!(
            "Started PulseAudio meter thread for channel {}",
            self.channel_id
        );
        Ok(())
    }

    /// Signal the meter thread to exit without joining it.
    ///
    /// Safe to call from the PipeWire thread. The OS thread is detached
    /// (joined on a helper thread) so the PW loop never blocks.
    pub fn request_stop(&self) {
        self.running.store(false, Ordering::Relaxed);
        self.levels.reset();

        if let Some(handle) = self.thread_handle.borrow_mut().take() {
            info!(
                "Requesting PulseAudio meter stop for channel {}",
                self.channel_id
            );
            detach_join(handle);
        }
    }

    /// Get a reference to the atomic levels.
    pub fn levels(&self) -> &Arc<AtomicMeterLevels> {
        &self.levels
    }

    /// Check if the meter is running.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }
}

impl Drop for PulseAudioMeter {
    fn drop(&mut self) {
        // Never join() here — Drop can run on the PipeWire thread when
        // `pulse_meters` is cleared (destroy / reconnect).
        self.running.store(false, Ordering::Relaxed);
        if let Some(handle) = self.thread_handle.borrow_mut().take() {
            detach_join(handle);
        }
    }
}

/// Join a meter thread on a helper thread so the caller never blocks.
fn detach_join(handle: JoinHandle<()>) {
    // If the helper spawn fails, drop the handle without joining — the
    // thread will exit on its own when `running` is false. Still never
    // join on the caller (may be the PW thread).
    let _ = thread::Builder::new()
        .name("pa-meter-join".to_string())
        .spawn(move || {
            let _ = handle.join();
        });
}

/// Parse PEAK_DETECT payload into (left, right) linear peaks.
///
/// `channels` is the stream channel count we requested. If the payload
/// contains a real stereo frame (8+ bytes), L/R are used independently.
/// A single float is treated as true mono and copied to both sides.
pub(crate) fn peaks_from_peak_detect_bytes(data: &[u8], channels: u8) -> (f32, f32) {
    if channels >= 2 && data.len() >= 8 {
        let left = f32::from_ne_bytes([data[0], data[1], data[2], data[3]]).abs();
        let right = f32::from_ne_bytes([data[4], data[5], data[6], data[7]]).abs();
        (left, right)
    } else if data.len() >= 4 {
        let peak = f32::from_ne_bytes([data[0], data[1], data[2], data[3]]).abs();
        (peak, peak)
    } else {
        (0.0, 0.0)
    }
}

/// The meter thread function.
///
/// Creates a PA mainloop and context, then connects a peak detection stream
/// to the specified source. If the source disappears the thread remounts
/// (retry loop) until `running` is cleared.
fn meter_thread(
    channel_id: Uuid,
    source_name: String,
    levels: Arc<AtomicMeterLevels>,
    running: Arc<AtomicBool>,
) {
    debug!("PA meter thread starting for channel {}", channel_id);

    while running.load(Ordering::Relaxed) {
        match run_meter_session(&channel_id, &source_name, &levels, &running) {
            SessionEnd::Stopped => break,
            SessionEnd::Disconnected => {
                if !running.load(Ordering::Relaxed) {
                    break;
                }
                warn!(
                    "PA meter session ended for channel {} (source '{}'); remounting",
                    channel_id, source_name
                );
                levels.reset();
                // Brief pause so we don't spin if the source is gone.
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
        }
    }

    debug!("PA meter thread exiting for channel {}", channel_id);
}

enum SessionEnd {
    Stopped,
    Disconnected,
}

fn run_meter_session(
    channel_id: &Uuid,
    source_name: &str,
    levels: &Arc<AtomicMeterLevels>,
    running: &Arc<AtomicBool>,
) -> SessionEnd {
    let mut mainloop = match Mainloop::new() {
        Some(ml) => ml,
        None => {
            error!("Failed to create PA mainloop for channel {}", channel_id);
            return SessionEnd::Disconnected;
        }
    };

    let mut context = match Context::new(&mainloop, "sootmix-meter") {
        Some(ctx) => ctx,
        None => {
            error!("Failed to create PA context for channel {}", channel_id);
            return SessionEnd::Disconnected;
        }
    };

    if context
        .connect(None, ContextFlagSet::NOFLAGS, None)
        .is_err()
    {
        error!("Failed to connect PA context for channel {}", channel_id);
        return SessionEnd::Disconnected;
    }

    debug!("PA context connecting for channel {}", channel_id);

    loop {
        if !running.load(Ordering::Relaxed) {
            return SessionEnd::Stopped;
        }
        match mainloop.iterate(true) {
            libpulse_binding::mainloop::standard::IterateResult::Quit(_)
            | libpulse_binding::mainloop::standard::IterateResult::Err(_) => {
                error!("PA mainloop iteration failed for channel {}", channel_id);
                return SessionEnd::Disconnected;
            }
            libpulse_binding::mainloop::standard::IterateResult::Success(_) => {}
        }

        match context.get_state() {
            ContextState::Ready => break,
            ContextState::Failed | ContextState::Terminated => {
                error!("PA context failed for channel {}", channel_id);
                return SessionEnd::Disconnected;
            }
            _ => continue,
        }
    }

    info!(
        "PA context ready for channel {}, creating peak stream for '{}'",
        channel_id, source_name
    );

    // Prefer stereo when the source has real L/R. Fall back to mono only
    // if a 2-channel peak stream cannot be created/connected.
    let stereo_spec = Spec {
        format: Format::FLOAT32NE,
        rate: 60,
        channels: 2,
    };
    let mono_spec = Spec {
        format: Format::FLOAT32NE,
        rate: 60,
        channels: 1,
    };

    let (mut stream, channels) = match connect_peak_stream(
        &mut context,
        &mut mainloop,
        source_name,
        running,
        *channel_id,
        &stereo_spec,
        2,
    ) {
        StreamConnect::Ready(s, ch) => (s, ch),
        StreamConnect::Stopped => return SessionEnd::Stopped,
        StreamConnect::Failed => {
            debug!(
                "Stereo PA peak stream failed for channel {}, falling back to mono",
                channel_id
            );
            match connect_peak_stream(
                &mut context,
                &mut mainloop,
                source_name,
                running,
                *channel_id,
                &mono_spec,
                1,
            ) {
                StreamConnect::Ready(s, ch) => (s, ch),
                StreamConnect::Stopped => return SessionEnd::Stopped,
                StreamConnect::Failed => {
                    error!(
                        "Failed to connect PA peak stream to '{}' for channel {}",
                        source_name, channel_id
                    );
                    return SessionEnd::Disconnected;
                }
            }
        }
    };

    info!(
        "PA peak stream ready for channel {} ({} ch), entering main loop",
        channel_id, channels
    );

    while running.load(Ordering::Relaxed) {
        match mainloop.iterate(false) {
            libpulse_binding::mainloop::standard::IterateResult::Quit(_)
            | libpulse_binding::mainloop::standard::IterateResult::Err(_) => {
                warn!("PA mainloop error for channel {}", channel_id);
                let _ = stream.disconnect();
                return SessionEnd::Disconnected;
            }
            libpulse_binding::mainloop::standard::IterateResult::Success(_) => {}
        }

        if stream.get_state() != StreamState::Ready {
            warn!("PA stream no longer ready for channel {}", channel_id);
            let _ = stream.disconnect();
            return SessionEnd::Disconnected;
        }

        while let Some(readable) = stream.readable_size() {
            if readable == 0 {
                break;
            }
            match stream.peek() {
                Ok(res) => match res {
                    libpulse_binding::stream::PeekResult::Data(data) => {
                        let (left, right) = peaks_from_peak_detect_bytes(data, channels);
                        trace!(
                            "Peak for channel {}: L={:.4} R={:.4} ({} ch)",
                            channel_id, left, right, channels
                        );
                        levels.store_max(left, right);
                        let _ = stream.discard();
                    }
                    libpulse_binding::stream::PeekResult::Hole(_) => {
                        let _ = stream.discard();
                    }
                    libpulse_binding::stream::PeekResult::Empty => {
                        break;
                    }
                },
                Err(e) => {
                    warn!("PA stream peek error for channel {}: {:?}", channel_id, e);
                    let _ = stream.disconnect();
                    return SessionEnd::Disconnected;
                }
            }
        }

        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    stream.disconnect().ok();
    SessionEnd::Stopped
}

enum StreamConnect {
    Ready(Stream, u8),
    Stopped,
    Failed,
}

fn connect_peak_stream(
    context: &mut Context,
    mainloop: &mut Mainloop,
    source_name: &str,
    running: &Arc<AtomicBool>,
    channel_id: Uuid,
    spec: &Spec,
    channels: u8,
) -> StreamConnect {
    if !spec.is_valid() {
        return StreamConnect::Failed;
    }

    let mut stream = match Stream::new(context, "peak-meter", spec, None) {
        Some(s) => s,
        None => return StreamConnect::Failed,
    };

    let flags =
        StreamFlagSet::PEAK_DETECT | StreamFlagSet::ADJUST_LATENCY | StreamFlagSet::DONT_MOVE;

    let source = if source_name == "@DEFAULT_SOURCE@" {
        None
    } else {
        Some(source_name)
    };

    // Retry connection with exponential backoff — source may not exist yet
    // (sink monitor sources are created asynchronously; PW reconnects).
    let mut retry_count = 0;
    const MAX_RETRIES: u32 = 20;
    loop {
        if !running.load(Ordering::Relaxed) {
            debug!(
                "PA meter stopped during connection retry for channel {}",
                channel_id
            );
            return StreamConnect::Stopped;
        }

        if stream.connect_record(source, None, flags).is_ok() {
            break;
        }

        retry_count += 1;
        if retry_count >= MAX_RETRIES {
            debug!(
                "PA stream connect to '{}' for channel {} failed after {} retries",
                source_name, channel_id, retry_count
            );
            return StreamConnect::Failed;
        }

        let delay_ms = std::cmp::min(100 * (1 << retry_count.min(4)), 1000);
        debug!(
            "PA stream connect failed for channel {}, retry {}/{} in {}ms",
            channel_id, retry_count, MAX_RETRIES, delay_ms
        );
        std::thread::sleep(std::time::Duration::from_millis(delay_ms));

        drop(stream);
        stream = match Stream::new(context, "peak-meter", spec, None) {
            Some(s) => s,
            None => return StreamConnect::Failed,
        };
    }

    loop {
        if !running.load(Ordering::Relaxed) {
            let _ = stream.disconnect();
            return StreamConnect::Stopped;
        }
        match mainloop.iterate(true) {
            libpulse_binding::mainloop::standard::IterateResult::Quit(_)
            | libpulse_binding::mainloop::standard::IterateResult::Err(_) => {
                return StreamConnect::Failed;
            }
            libpulse_binding::mainloop::standard::IterateResult::Success(_) => {}
        }

        match stream.get_state() {
            StreamState::Ready => return StreamConnect::Ready(stream, channels),
            StreamState::Failed | StreamState::Terminated => {
                let _ = stream.disconnect();
                return StreamConnect::Failed;
            }
            _ => continue,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_meter_creation() {
        let levels = Arc::new(AtomicMeterLevels::new());
        let meter = PulseAudioMeter::new(Uuid::new_v4(), "@DEFAULT_SOURCE@", Arc::clone(&levels));
        assert!(!meter.is_running());
    }

    #[test]
    fn test_empty_source_defaults() {
        let levels = Arc::new(AtomicMeterLevels::new());
        let meter = PulseAudioMeter::new(Uuid::new_v4(), "", Arc::clone(&levels));
        assert_eq!(meter.source_name(), "@DEFAULT_SOURCE@");
    }

    #[test]
    fn test_request_stop_is_non_blocking_without_thread() {
        let levels = Arc::new(AtomicMeterLevels::new());
        let meter = PulseAudioMeter::new(Uuid::new_v4(), "dummy", Arc::clone(&levels));
        levels.store_max(0.9, 0.8);
        meter.request_stop();
        assert!(!meter.is_running());
        let (l, r) = levels.load();
        assert_eq!(l, 0.0);
        assert_eq!(r, 0.0);
    }

    #[test]
    fn test_stereo_peak_bytes_not_copied_to_both() {
        let mut data = [0u8; 8];
        data[0..4].copy_from_slice(&0.25f32.to_ne_bytes());
        data[4..8].copy_from_slice(&0.75f32.to_ne_bytes());
        let (l, r) = peaks_from_peak_detect_bytes(&data, 2);
        assert!((l - 0.25).abs() < 0.0001);
        assert!((r - 0.75).abs() < 0.0001);
    }

    #[test]
    fn test_mono_peak_bytes_copied_only_when_mono() {
        let data = 0.4f32.to_ne_bytes();
        let (l, r) = peaks_from_peak_detect_bytes(&data, 1);
        assert!((l - 0.4).abs() < 0.0001);
        assert!((r - 0.4).abs() < 0.0001);
        // Even if we asked for stereo, a 4-byte payload is true mono.
        let (l2, r2) = peaks_from_peak_detect_bytes(&data, 2);
        assert!((l2 - 0.4).abs() < 0.0001);
        assert!((r2 - 0.4).abs() < 0.0001);
    }

    #[test]
    fn test_max_hold_used_by_meter_levels() {
        let levels = Arc::new(AtomicMeterLevels::new());
        levels.store_max(0.1, 0.2);
        levels.store_max(0.3, 0.05);
        let (l, r) = levels.load_and_reset();
        assert!((l - 0.3).abs() < 0.0001);
        assert!((r - 0.2).abs() < 0.0001);
    }
}
