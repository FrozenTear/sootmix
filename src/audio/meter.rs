// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Audio level metering for VU meters.
//!
//! This module provides audio level information for display in VU meters.
//! Currently uses simulated levels based on channel activity.
//! Future versions can hook into PipeWire streams for real peak detection.

use crate::state::{db_to_linear, MeterDisplayState, MixerChannel};
use std::collections::HashMap;
use uuid::Uuid;

/// Simulated audio activity state for a channel.
#[derive(Debug, Clone, Default)]
pub struct ChannelMeterState {
    /// Base activity level (0.0 to 1.0) - simulates whether apps are playing.
    pub activity: f32,
    /// Random variation seed for realistic meter movement.
    pub variation_phase: f32,
}

/// Meter data manager that tracks levels across all channels.
#[derive(Debug, Default)]
pub struct MeterManager {
    /// Per-channel activity state.
    channel_states: HashMap<Uuid, ChannelMeterState>,
    /// Phase counter for animation.
    phase: f32,
}

impl MeterManager {
    /// Create a new meter manager.
    pub fn new() -> Self {
        Self::default()
    }

    /// Update meters for all channels.
    ///
    /// `dt` is delta time in seconds since last update.
    /// Returns updated meter display states for each channel and master.
    pub fn update_meters(
        &mut self,
        channels: &mut [MixerChannel],
        master_meter: &mut MeterDisplayState,
        master_volume_db: f32,
        master_muted: bool,
        dt: f32,
    ) {
        // Update animation phase
        self.phase += dt * 3.0; // 3 Hz base frequency
        if self.phase > std::f32::consts::TAU {
            self.phase -= std::f32::consts::TAU;
        }

        let mut total_left = 0.0f32;
        let mut total_right = 0.0f32;
        let mut active_channels = 0;

        for channel in channels.iter_mut() {
            // Get or create channel state
            let state = self.channel_states.entry(channel.id).or_insert_with(|| {
                ChannelMeterState {
                    activity: 0.0,
                    variation_phase: rand_phase(),
                }
            });

            // Determine if channel has active audio (apps assigned and not muted)
            let has_activity = !channel.assigned_apps.is_empty() && !channel.muted;

            // Smoothly adjust activity level
            let target_activity = if has_activity { 0.7 } else { 0.0 };
            state.activity += 0.1 * (target_activity - state.activity);

            // Calculate simulated levels with variation
            let (level_left, level_right) = if state.activity > 0.01 {
                let base_level = state.activity;
                let volume_scale = db_to_linear(channel.volume_db);

                // Add some variation for realistic movement
                let phase_offset = state.variation_phase;
                let variation_l = 0.15 * (self.phase + phase_offset).sin();
                let variation_r = 0.15 * (self.phase * 1.1 + phase_offset + 0.5).sin();

                let left = (base_level + variation_l).clamp(0.0, 1.0) * volume_scale;
                let right = (base_level + variation_r).clamp(0.0, 1.0) * volume_scale;

                (left, right)
            } else {
                (0.0, 0.0)
            };

            // Update channel meter display
            channel.meter_display.update(level_left, level_right, dt);

            // Accumulate for master meter
            if level_left > 0.0 || level_right > 0.0 {
                total_left = total_left.max(level_left);
                total_right = total_right.max(level_right);
                active_channels += 1;
            }
        }

        // Update master meter
        let master_scale = if master_muted { 0.0 } else { db_to_linear(master_volume_db) };
        let master_left = total_left * master_scale;
        let master_right = total_right * master_scale;
        master_meter.update(master_left, master_right, dt);

        // Clean up states for removed channels
        let channel_ids: std::collections::HashSet<Uuid> = channels.iter().map(|c| c.id).collect();
        self.channel_states.retain(|id, _| channel_ids.contains(id));
    }

    /// Get simulated level for a specific node (for future PipeWire integration).
    #[allow(dead_code)]
    pub fn get_node_level(&self, _node_id: u32) -> (f32, f32) {
        // Placeholder for future PipeWire level monitoring
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
}
