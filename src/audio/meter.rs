// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Audio level metering for VU meters.
//!
//! Display is driven only by real samples from the daemon/engine
//! (`AtomicMeterLevels`). Missing or inactive meters stay flat — the UI never
//! invents bounce from assignment, default device names, or mute state.

#![allow(dead_code)]

use crate::audio::meter_stream::AtomicMeterLevels;
use crate::state::{db_to_linear, MeterDisplayState, MixerChannel};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tracing::debug;
use uuid::Uuid;

/// Per-channel meter state tracking.
#[derive(Debug, Clone)]
pub struct ChannelMeterState {
    /// Real-time atomic levels from audio thread (if available).
    pub real_levels: Option<Arc<AtomicMeterLevels>>,
}

impl Default for ChannelMeterState {
    fn default() -> Self {
        Self { real_levels: None }
    }
}

/// Meter data manager that tracks levels across all channels.
///
/// Only real, active samples are shown. Idle/disconnected meters stay at zero.
#[derive(Debug, Default)]
pub struct MeterManager {
    /// Per-channel meter state.
    channel_states: HashMap<Uuid, ChannelMeterState>,
    /// Channels we've already logged an "inactive meter" warning for.
    logged_inactive: HashSet<Uuid>,
}

impl MeterManager {
    /// Create a new meter manager.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register real-time levels for a channel.
    pub fn register_real_levels(&mut self, channel_id: Uuid, levels: Arc<AtomicMeterLevels>) {
        let state = self.channel_states.entry(channel_id).or_default();
        state.real_levels = Some(levels);
    }

    /// Unregister real-time levels for a channel.
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

    /// Snap every channel and the master meter to inactive/zero.
    ///
    /// Used on daemon disconnect so leftover peaks do not keep decaying as if
    /// audio were still flowing.
    pub fn reset_all_inactive(
        &mut self,
        channels: &mut [MixerChannel],
        master_meter: &mut MeterDisplayState,
    ) {
        for channel in channels.iter_mut() {
            if let Some(ref levels) = channel.meter_levels {
                levels.reset();
            }
            channel.meter_display.reset();
        }
        master_meter.reset();
        self.logged_inactive.clear();
    }

    /// Update meters for all channels.
    ///
    /// `dt` is delta time in seconds since last update.
    ///
    /// Levels come only from an active `AtomicMeterLevels` sample. Assignment
    /// (`assigned_apps`) and default input names (`system-default`) never
    /// invent motion. Master is the post-fader max of real output-channel
    /// samples only — never a mix of simulated values.
    pub fn update_meters(
        &mut self,
        channels: &mut [MixerChannel],
        master_meter: &mut MeterDisplayState,
        master_volume_db: f32,
        master_muted: bool,
        dt: f32,
    ) {
        let mut total_left = 0.0f32;
        let mut total_right = 0.0f32;
        let mut master_has_real = false;

        for channel in channels.iter_mut() {
            let _ = self.channel_states.entry(channel.id).or_default();

            let (level_left, level_right, from_real) =
                real_meter_levels(channel, &mut self.logged_inactive);

            channel.meter_display.update(level_left, level_right, dt);

            // Master is output-bus only, and only from real samples.
            if from_real && !channel.is_input() && (level_left > 0.0 || level_right > 0.0) {
                total_left = total_left.max(level_left);
                total_right = total_right.max(level_right);
                master_has_real = true;
            }
        }

        if master_has_real {
            let master_scale = if master_muted {
                0.0
            } else {
                db_to_linear(master_volume_db)
            };
            master_meter.update(total_left * master_scale, total_right * master_scale, dt);
        } else {
            // No real output samples this tick — idle, do not invent a mix.
            master_meter.update(0.0, 0.0, dt);
        }

        let channel_ids: std::collections::HashSet<Uuid> =
            channels.iter().map(|c| c.id).collect();
        self.channel_states
            .retain(|id, _| channel_ids.contains(id));
    }
}

/// Read a channel's real meter sample, or `(0, 0)` when missing/inactive.
///
/// Returns `(left, right, from_real)` where `from_real` is true only when an
/// active atomic sample was consumed.
fn real_meter_levels(
    channel: &MixerChannel,
    logged_inactive: &mut HashSet<Uuid>,
) -> (f32, f32, bool) {
    let Some(ref real_levels) = channel.meter_levels else {
        return (0.0, 0.0, false);
    };

    if !real_levels.is_active() {
        if logged_inactive.insert(channel.id) {
            debug!(
                "Channel '{}' meter stream not active, showing idle levels",
                channel.name
            );
        }
        return (0.0, 0.0, false);
    }

    logged_inactive.remove(&channel.id);
    let (raw_left, raw_right) = real_levels.load_and_reset();
    if raw_left > 0.01 || raw_right > 0.01 {
        tracing::trace!(
            "Meter read: ch={} raw=({:.4},{:.4}) is_input={}",
            channel.name,
            raw_left,
            raw_right,
            channel.is_input()
        );
    }

    // INPUT: pre-fader (mute still zeros the display). OUTPUT: post-fader.
    let (left, right) = if channel.is_input() {
        if channel.muted {
            (0.0, 0.0)
        } else {
            (raw_left, raw_right)
        }
    } else {
        let volume_scale = if channel.muted {
            0.0
        } else {
            db_to_linear(channel.volume_db)
        };
        (raw_left * volume_scale, raw_right * volume_scale)
    };
    (left, right, true)
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
        meter.update(1.0, 1.0, 0.05);
        let high_level = meter.level_left;
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

    #[test]
    fn inactive_output_with_assigned_apps_stays_flat() {
        let mut manager = MeterManager::new();
        let mut channel = MixerChannel::new("Games");
        channel.assigned_apps.push("firefox".into());
        assert!(
            !channel.meter_levels.as_ref().unwrap().is_active(),
            "fresh AtomicMeterLevels must start inactive"
        );

        let mut master = MeterDisplayState::default();
        manager.update_meters(
            std::slice::from_mut(&mut channel),
            &mut master,
            0.0,
            false,
            0.016,
        );

        assert_eq!(channel.meter_display.level_left, 0.0);
        assert_eq!(channel.meter_display.level_right, 0.0);
        assert_eq!(master.level_left, 0.0);
        assert_eq!(master.level_right, 0.0);
    }

    #[test]
    fn inactive_input_with_system_default_stays_flat() {
        let mut manager = MeterManager::new();
        let mut channel = MixerChannel::new_input("Mic");
        assert_eq!(
            channel.input_device_name.as_deref(),
            Some("system-default")
        );
        assert!(!channel.meter_levels.as_ref().unwrap().is_active());

        let mut master = MeterDisplayState::default();
        manager.update_meters(
            std::slice::from_mut(&mut channel),
            &mut master,
            0.0,
            false,
            0.016,
        );

        assert_eq!(channel.meter_display.level_left, 0.0);
        assert_eq!(channel.meter_display.level_right, 0.0);
    }

    #[test]
    fn missing_atomic_levels_stay_flat() {
        let mut manager = MeterManager::new();
        let mut channel = MixerChannel::new("Bare");
        channel.assigned_apps.push("discord".into());
        channel.meter_levels = None;

        let mut master = MeterDisplayState::default();
        manager.update_meters(
            std::slice::from_mut(&mut channel),
            &mut master,
            0.0,
            false,
            0.016,
        );

        assert_eq!(channel.meter_display.level_left, 0.0);
        assert_eq!(master.level_left, 0.0);
    }

    #[test]
    fn master_is_not_derived_from_inactive_channels() {
        let mut manager = MeterManager::new();
        let mut channel = MixerChannel::new("Out");
        channel.assigned_apps.push("spotify".into());
        channel.volume_db = 0.0;

        let mut master = MeterDisplayState::default();
        // Several ticks: old simulated path would ease toward ~0.7.
        for _ in 0..20 {
            manager.update_meters(
                std::slice::from_mut(&mut channel),
                &mut master,
                0.0,
                false,
                0.016,
            );
        }

        assert_eq!(channel.meter_display.level_left, 0.0);
        assert_eq!(master.level_left, 0.0);
        assert_eq!(master.peak_hold_left, 0.0);
    }

    #[test]
    fn active_real_sample_is_displayed() {
        let mut manager = MeterManager::new();
        let mut channel = MixerChannel::new("Live");
        channel.volume_db = 0.0;
        {
            let levels = channel.meter_levels.as_ref().unwrap();
            levels.store(0.4, 0.5);
            assert!(levels.is_active());
        }

        let mut master = MeterDisplayState::default();
        manager.update_meters(
            std::slice::from_mut(&mut channel),
            &mut master,
            0.0,
            false,
            0.016,
        );

        assert!(channel.meter_display.level_left > 0.0);
        assert!(channel.meter_display.level_right > 0.0);
        assert!(master.level_left > 0.0);
        assert!(master.level_right > 0.0);
    }

    #[test]
    fn reset_all_inactive_clears_display_and_atomics() {
        let mut manager = MeterManager::new();
        let mut channel = MixerChannel::new("Live");
        channel.meter_levels.as_ref().unwrap().store(0.8, 0.8);
        let mut master = MeterDisplayState::default();
        manager.update_meters(
            std::slice::from_mut(&mut channel),
            &mut master,
            0.0,
            false,
            0.016,
        );
        assert!(channel.meter_display.level_left > 0.0);

        manager.reset_all_inactive(std::slice::from_mut(&mut channel), &mut master);
        assert!(!channel.meter_levels.as_ref().unwrap().is_active());
        assert_eq!(channel.meter_display.level_left, 0.0);
        assert_eq!(master.level_left, 0.0);
    }
}
