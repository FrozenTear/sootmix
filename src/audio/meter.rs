// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Audio level metering for VU meters.
//!
//! This module provides audio level information for display in VU meters.
//! It supports both real audio levels (from PipeWire streams) and simulated
//! levels (as fallback when real metering isn't available).

#![allow(dead_code)]

use crate::audio::meter_stream::AtomicMeterLevels;
use crate::state::{db_to_linear, MeterDisplayState, MixerChannel};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

/// Per-channel meter state tracking.
#[derive(Debug, Clone)]
pub struct ChannelMeterState {
    /// Real-time atomic levels from audio thread (if available).
    pub real_levels: Option<Arc<AtomicMeterLevels>>,
    /// Simulated activity level for fallback (0.0 to 1.0).
    pub simulated_activity: f32,
    /// Random variation seed for realistic simulated movement.
    pub variation_phase: f32,
}

impl Default for ChannelMeterState {
    fn default() -> Self {
        Self {
            real_levels: None,
            simulated_activity: 0.0,
            variation_phase: rand_phase(),
        }
    }
}

/// Meter data manager that tracks levels across all channels.
///
/// Supports both real audio levels (when available) and simulated levels.
#[derive(Debug, Default)]
pub struct MeterManager {
    /// Per-channel meter state.
    channel_states: HashMap<Uuid, ChannelMeterState>,
    /// Phase counter for simulated animation.
    phase: f32,
}

impl MeterManager {
    /// Create a new meter manager.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register real-time levels for a channel.
    ///
    /// Once registered, the manager will use these atomic levels instead of simulation.
    pub fn register_real_levels(&mut self, channel_id: Uuid, levels: Arc<AtomicMeterLevels>) {
        let state = self.channel_states.entry(channel_id).or_default();
        state.real_levels = Some(levels);
    }

    /// Unregister real-time levels for a channel (falls back to simulation).
    pub fn unregister_real_levels(&mut self, channel_id: Uuid) {
        if let Some(state) = self.channel_states.get_mut(&channel_id) {
            state.real_levels = None;
        }
    }

    /// Check if a channel has real metering enabled.
    pub fn has_real_metering(&self, channel_id: Uuid) -> bool {
        self.channel_states
            .get(&channel_id)
            .map(|s| s.real_levels.is_some())
            .unwrap_or(false)
    }

    /// Update meters for all channels.
    ///
    /// `dt` is delta time in seconds since last update.
    pub fn update_meters(
        &mut self,
        channels: &mut [MixerChannel],
        master_meter: &mut MeterDisplayState,
        master_volume_db: f32,
        master_muted: bool,
        dt: f32,
    ) {
        // Update animation phase for simulated meters
        self.phase += dt * 3.0; // 3 Hz base frequency
        if self.phase > std::f32::consts::TAU {
            self.phase -= std::f32::consts::TAU;
        }

        let mut total_left = 0.0f32;
        let mut total_right = 0.0f32;

        for channel in channels.iter_mut() {
            // Get or create channel state for simulated levels
            let state = self.channel_states.entry(channel.id).or_default();

            // Try to get real levels from the channel's meter_levels first
            let phase = self.phase;
            let (level_left, level_right) = if let Some(ref real_levels) = channel.meter_levels {
                if real_levels.is_active() {
                    // Use real audio levels from the plugin processing chain
                    let (raw_left, raw_right) = real_levels.load();
                    if raw_left > 0.01 || raw_right > 0.01 {
                        tracing::trace!(
                            "Meter read: ch={} raw=({:.4},{:.4}) is_input={}",
                            channel.name, raw_left, raw_right, channel.is_input()
                        );
                    }

                    // For INPUT channels: show PRE-FADER levels (raw input, not affected by volume)
                    // This is industry standard - input meters show what's coming in, not what's going out.
                    // For OUTPUT channels: show POST-FADER levels (affected by volume/mute)
                    if channel.is_input() {
                        // Pre-fader metering for inputs - show raw input level
                        // Only apply mute (user still wants to see "muted" state visually)
                        if channel.muted {
                            (0.0, 0.0)
                        } else {
                            (raw_left, raw_right)
                        }
                    } else {
                        // Post-fader metering for outputs - apply channel volume
                        let volume_scale = if channel.muted {
                            0.0
                        } else {
                            db_to_linear(channel.volume_db)
                        };
                        (raw_left * volume_scale, raw_right * volume_scale)
                    }
                } else {
                    // Real metering available but no audio data yet - use simulated
                    calculate_simulated_level(phase, channel, state)
                }
            } else {
                // No real metering - fall back to simulated levels
                calculate_simulated_level(phase, channel, state)
            };

            // Update channel meter display
            channel.meter_display.update(level_left, level_right, dt);

            // Accumulate for master meter (only output channels contribute)
            // Input channels don't route to master output directly
            if !channel.is_input() && (level_left > 0.0 || level_right > 0.0) {
                total_left = total_left.max(level_left);
                total_right = total_right.max(level_right);
            }
        }

        // Update master meter
        let master_scale = if master_muted {
            0.0
        } else {
            db_to_linear(master_volume_db)
        };
        let master_left = total_left * master_scale;
        let master_right = total_right * master_scale;
        master_meter.update(master_left, master_right, dt);

        // Clean up states for removed channels
        let channel_ids: std::collections::HashSet<Uuid> =
            channels.iter().map(|c| c.id).collect();
        self.channel_states
            .retain(|id, _| channel_ids.contains(id));
    }

}

/// Calculate simulated levels for a channel (fallback when no real metering).
/// This is a free function to avoid borrow issues with &mut self.
fn calculate_simulated_level(
    phase: f32,
    channel: &MixerChannel,
    state: &mut ChannelMeterState,
) -> (f32, f32) {
    // For input channels, activity is based on having an input device selected
    // For output channels, activity is based on having apps assigned
    let has_activity = if channel.is_input() {
        channel.input_device_name.is_some() && !channel.muted
    } else {
        !channel.assigned_apps.is_empty() && !channel.muted
    };
    let target_activity = if has_activity { 0.7 } else { 0.0 };
    state.simulated_activity += 0.1 * (target_activity - state.simulated_activity);

    if state.simulated_activity > 0.01 {
        let base_level = state.simulated_activity;
        let phase_offset = state.variation_phase;
        let variation_l = 0.15 * (phase + phase_offset).sin();
        let variation_r = 0.15 * (phase * 1.1 + phase_offset + 0.5).sin();

        // For INPUT channels: show PRE-FADER levels (not affected by channel volume)
        // For OUTPUT channels: show POST-FADER levels (affected by volume)
        if channel.is_input() {
            // Pre-fader simulated metering for inputs
            let left = (base_level + variation_l).clamp(0.0, 1.0);
            let right = (base_level + variation_r).clamp(0.0, 1.0);
            (left, right)
        } else {
            // Post-fader simulated metering for outputs
            let volume_scale = db_to_linear(channel.volume_db);
            let left = (base_level + variation_l).clamp(0.0, 1.0) * volume_scale;
            let right = (base_level + variation_r).clamp(0.0, 1.0) * volume_scale;
            (left, right)
        }
    } else {
        (0.0, 0.0)
    }
}

/// Generate a random phase offset for variation.
fn rand_phase() -> f32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    (nanos as f32 / 1_000_000_000.0) * std::f32::consts::TAU
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_meter_manager_creation() {
        let manager = MeterManager::new();
        assert!(manager.channel_states.is_empty());
    }

    #[test]
    fn test_meter_display_update() {
        let mut meter = MeterDisplayState::default();
        meter.update(0.5, 0.5, 0.05);
        assert!(meter.level_left > 0.0);
        assert!(meter.level_right > 0.0);
    }

    #[test]
    fn test_meter_decay() {
        let mut meter = MeterDisplayState::default();
        // Set high level
        meter.update(1.0, 1.0, 0.05);
        let high_level = meter.level_left;

        // Update with zero level - should decay
        meter.update(0.0, 0.0, 0.05);
        assert!(meter.level_left < high_level);
    }

    #[test]
    fn test_real_levels_registration() {
        let mut manager = MeterManager::new();
        let channel_id = Uuid::new_v4();
        let levels = Arc::new(AtomicMeterLevels::new());

        assert!(!manager.has_real_metering(channel_id));

        manager.register_real_levels(channel_id, levels);
        assert!(manager.has_real_metering(channel_id));

        manager.unregister_real_levels(channel_id);
        assert!(!manager.has_real_metering(channel_id));
    }
}
