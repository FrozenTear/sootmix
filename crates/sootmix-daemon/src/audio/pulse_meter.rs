// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! PulseAudio-based metering for input channels.
//!
//! This module uses the PulseAudio API (via PipeWire's PA compatibility layer)
//! to perform reliable audio level metering. The key advantage is using
//! `PA_STREAM_PEAK_DETECT` which enables server-side peak calculation.
//!
//! # Why PulseAudio API?
//!
//! The native `pw_stream` approach has fundamental issues:
//! - Format mismatch: mono mics can't link to stereo streams
//! - Passive streams stay suspended without a driver
//! - No adapter loading for format conversion
//!
//! PulseAudio API solves all of this:
//! - PipeWire includes full PA compatibility by default
//! - `PEAK_DETECT` flag enables efficient server-side peak calculation
//! - Handles all format conversion automatically
//! - Works with any audio source (hardware, virtual, monitor ports)

use crate::audio::native_loopback::AtomicMeterLevels;
use libpulse_binding::context::{Context, FlagSet as ContextFlagSet, State as ContextState};
use libpulse_binding::mainloop::standard::Mainloop;
use libpulse_binding::sample::{Format, Spec};
use libpulse_binding::stream::{FlagSet as StreamFlagSet, State as StreamState, Stream};
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use tracing::{debug, error, info, trace, warn};
use uuid::Uuid;

/// PulseAudio-based meter for input channels.
///
/// Runs a dedicated thread with its own PA mainloop to capture peak levels
/// from a PulseAudio source (microphone).
pub struct PulseAudioMeter {
    /// Channel ID this meter belongs to.
    channel_id: Uuid,
    /// PulseAudio source name to monitor.
    source_name: String,
    /// Atomic levels shared with the main thread.
    levels: Arc<AtomicMeterLevels>,
    /// Flag to signal the meter thread to stop.
    running: Arc<AtomicBool>,
    /// Thread handle (if started).
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

    /// Start the meter thread.
    ///
    /// Spawns a background thread that runs the PulseAudio mainloop
    /// and captures peak levels from the configured source.
    pub fn start(&self) {
        if self.running.load(Ordering::Relaxed) {
            warn!(
                "PulseAudio meter for channel {} already running",
                self.channel_id
            );
            return;
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
            .expect("Failed to spawn PA meter thread");

        *self.thread_handle.borrow_mut() = Some(handle);
        info!("Started PulseAudio meter thread for channel {}", self.channel_id);
    }

    /// Stop the meter thread.
    pub fn stop(&self) {
        if !self.running.load(Ordering::Relaxed) {
            return;
        }

        info!("Stopping PulseAudio meter for channel {}", self.channel_id);
        self.running.store(false, Ordering::Relaxed);

        // Reset levels to zero
        self.levels.store(0.0, 0.0);

        // Wait for thread to finish (with timeout)
        if let Some(handle) = self.thread_handle.borrow_mut().take() {
            // Don't block indefinitely - the PA mainloop should exit quickly
            // once running is set to false
            let _ = handle.join();
        }
    }

    /// Get a reference to the atomic levels.
    pub fn levels(&self) -> &Arc<AtomicMeterLevels> {
        &self.levels
    }

    /// Check if the meter is running.
    #[cfg(test)]
    fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }
}

impl Drop for PulseAudioMeter {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The meter thread function.
///
/// Creates a PA mainloop and context, then connects a peak detection stream
/// to the specified source. Runs until `running` is set to false.
fn meter_thread(
    channel_id: Uuid,
    source_name: String,
    levels: Arc<AtomicMeterLevels>,
    running: Arc<AtomicBool>,
) {
    debug!("PA meter thread starting for channel {}", channel_id);

    // Create standard mainloop (simpler than threaded for our use case)
    let mut mainloop = match Mainloop::new() {
        Some(ml) => ml,
        None => {
            error!("Failed to create PA mainloop for channel {}", channel_id);
            return;
        }
    };

    // Create context
    let mut context = match Context::new(&mainloop, "sootmix-meter") {
        Some(ctx) => ctx,
        None => {
            error!("Failed to create PA context for channel {}", channel_id);
            return;
        }
    };

    // Connect to the PA server
    if context.connect(None, ContextFlagSet::NOFLAGS, None).is_err() {
        error!("Failed to connect PA context for channel {}", channel_id);
        return;
    }

    debug!("PA context connecting for channel {}", channel_id);

    // Wait for context to be ready (with iteration)
    loop {
        match mainloop.iterate(true) {
            libpulse_binding::mainloop::standard::IterateResult::Quit(_) |
            libpulse_binding::mainloop::standard::IterateResult::Err(_) => {
                error!("PA mainloop iteration failed for channel {}", channel_id);
                return;
            }
            libpulse_binding::mainloop::standard::IterateResult::Success(_) => {}
        }

        match context.get_state() {
            ContextState::Ready => break,
            ContextState::Failed | ContextState::Terminated => {
                error!("PA context failed for channel {}", channel_id);
                return;
            }
            _ => continue,
        }
    }

    info!(
        "PA context ready for channel {}, creating peak stream for '{}'",
        channel_id, source_name
    );

    // Create sample spec for peak detection
    // Higher rate for responsive meters
    let spec = Spec {
        format: Format::FLOAT32NE,
        rate: 60, // 60 samples/sec for responsive VU meters (~16ms)
        channels: 1, // Mono peak value
    };

    if !spec.is_valid() {
        error!("Invalid sample spec for channel {}", channel_id);
        return;
    }

    // Create the stream
    let mut stream = match Stream::new(&mut context, "peak-meter", &spec, None) {
        Some(s) => s,
        None => {
            error!("Failed to create PA stream for channel {}", channel_id);
            return;
        }
    };

    // Connect the stream with PEAK_DETECT flag
    let flags = StreamFlagSet::PEAK_DETECT
        | StreamFlagSet::ADJUST_LATENCY
        | StreamFlagSet::DONT_MOVE;

    // Use the source_name - PA will resolve @DEFAULT_SOURCE@ automatically
    let source = if source_name == "@DEFAULT_SOURCE@" {
        None
    } else {
        Some(source_name.as_str())
    };

    // Retry connection with exponential backoff - source may not exist yet
    // (e.g., sink monitor sources are created asynchronously)
    let mut retry_count = 0;
    const MAX_RETRIES: u32 = 20; // ~10 seconds total with backoff
    loop {
        if !running.load(Ordering::Relaxed) {
            debug!("PA meter stopped during connection retry for channel {}", channel_id);
            return;
        }

        if stream.connect_record(source, None, flags).is_ok() {
            break;
        }

        retry_count += 1;
        if retry_count >= MAX_RETRIES {
            error!(
                "Failed to connect PA stream to source '{}' for channel {} after {} retries",
                source_name, channel_id, retry_count
            );
            return;
        }

        // Exponential backoff: 100ms, 200ms, 400ms, ... capped at 1s
        let delay_ms = std::cmp::min(100 * (1 << retry_count.min(4)), 1000);
        debug!(
            "PA stream connect failed for channel {}, retry {}/{} in {}ms",
            channel_id, retry_count, MAX_RETRIES, delay_ms
        );
        std::thread::sleep(std::time::Duration::from_millis(delay_ms));

        // Need to recreate stream after failed connect attempt
        drop(stream);
        stream = match Stream::new(&mut context, "peak-meter", &spec, None) {
            Some(s) => s,
            None => {
                error!("Failed to recreate PA stream for channel {}", channel_id);
                return;
            }
        };
    }

    debug!("PA stream connecting for channel {}", channel_id);

    // Wait for stream to be ready
    loop {
        match mainloop.iterate(true) {
            libpulse_binding::mainloop::standard::IterateResult::Quit(_) |
            libpulse_binding::mainloop::standard::IterateResult::Err(_) => {
                error!("PA mainloop iteration failed waiting for stream {}", channel_id);
                return;
            }
            libpulse_binding::mainloop::standard::IterateResult::Success(_) => {}
        }

        match stream.get_state() {
            StreamState::Ready => break,
            StreamState::Failed | StreamState::Terminated => {
                error!("PA stream failed for channel {}", channel_id);
                return;
            }
            _ => continue,
        }
    }

    info!(
        "PA peak stream ready for channel {}, entering main loop",
        channel_id
    );

    // Main loop - read peaks until stopped
    while running.load(Ordering::Relaxed) {
        // Iterate mainloop (non-blocking to avoid deadlocks)
        match mainloop.iterate(false) {
            libpulse_binding::mainloop::standard::IterateResult::Quit(_) |
            libpulse_binding::mainloop::standard::IterateResult::Err(_) => {
                warn!("PA mainloop error for channel {}", channel_id);
                break;
            }
            libpulse_binding::mainloop::standard::IterateResult::Success(_) => {}
        }

        // Check if stream is still valid
        if stream.get_state() != StreamState::Ready {
            warn!("PA stream no longer ready for channel {}", channel_id);
            break;
        }

        // Read ALL available data (drain the buffer for this iteration)
        while let Some(readable) = stream.readable_size() {
            if readable == 0 {
                break;
            }
            match stream.peek() {
                Ok(res) => {
                    match res {
                        libpulse_binding::stream::PeekResult::Data(data) => {
                            // With PEAK_DETECT, we get a single f32 peak value
                            if data.len() >= 4 {
                                let peak = f32::from_ne_bytes([data[0], data[1], data[2], data[3]]);
                                let peak_abs = peak.abs();
                                trace!("Peak for channel {}: {:.4}", channel_id, peak_abs);
                                levels.store(peak_abs, peak_abs);
                            }
                            let _ = stream.discard();
                        }
                        libpulse_binding::stream::PeekResult::Hole(_) => {
                            let _ = stream.discard();
                        }
                        libpulse_binding::stream::PeekResult::Empty => {
                            break;
                        }
                    }
                }
                Err(e) => {
                    warn!("PA stream peek error for channel {}: {:?}", channel_id, e);
                    break;
                }
            }
        }

        // Short sleep - 5ms gives ~200Hz polling which should catch all 60Hz PA samples
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    // Cleanup
    stream.disconnect().ok();

    debug!("PA meter thread exiting for channel {}", channel_id);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_meter_creation() {
        let levels = Arc::new(AtomicMeterLevels::new());
        let meter = PulseAudioMeter::new(
            Uuid::new_v4(),
            "@DEFAULT_SOURCE@",
            Arc::clone(&levels),
        );
        assert!(!meter.is_running());
    }

    #[test]
    fn test_empty_source_defaults() {
        let levels = Arc::new(AtomicMeterLevels::new());
        let meter = PulseAudioMeter::new(Uuid::new_v4(), "", Arc::clone(&levels));
        assert_eq!(meter.source_name, "@DEFAULT_SOURCE@");
    }
}
