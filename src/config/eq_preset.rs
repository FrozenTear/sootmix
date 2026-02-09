// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! EQ preset definitions.

#![allow(dead_code, unused_imports)]

use serde::{Deserialize, Serialize};

/// Standard EQ frequency bands.
pub const EQ_FREQUENCIES: [u32; 5] = [60, 250, 1000, 4000, 16000];

/// A single EQ band.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct EqBand {
    /// Center frequency in Hz.
    pub freq: u32,
    /// Gain in dB (-12.0 to +12.0).
    pub gain: f32,
    /// Q factor (0.5 to 5.0).
    pub q: f32,
}

impl EqBand {
    pub fn new(freq: u32) -> Self {
        Self {
            freq,
            gain: 0.0,
            q: 1.0,
        }
    }

    pub fn with_gain(mut self, gain: f32) -> Self {
        self.gain = gain.clamp(-12.0, 12.0);
        self
    }

    pub fn with_q(mut self, q: f32) -> Self {
        self.q = q.clamp(0.5, 5.0);
        self
    }
}

/// A 5-band parametric EQ preset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EqPreset {
    pub name: String,
    pub bands: [EqBand; 5],
}

impl Default for EqPreset {
    fn default() -> Self {
        Self::flat()
    }
}

impl EqPreset {
    /// Create a flat EQ preset.
    pub fn flat() -> Self {
        Self {
            name: "Flat".to_string(),
            bands: [
                EqBand::new(60),
                EqBand::new(250),
                EqBand::new(1000),
                EqBand::new(4000),
                EqBand::new(16000),
            ],
        }
    }

    /// Bass boost preset.
    pub fn bass_boost() -> Self {
        Self {
            name: "Bass Boost".to_string(),
            bands: [
                EqBand::new(60).with_gain(6.0).with_q(0.8),
                EqBand::new(250).with_gain(3.0),
                EqBand::new(1000),
                EqBand::new(4000),
                EqBand::new(16000).with_gain(-2.0),
            ],
        }
    }

    /// Vocal clarity preset.
    pub fn vocal_clarity() -> Self {
        Self {
            name: "Vocal Clarity".to_string(),
            bands: [
                EqBand::new(60).with_gain(-2.0),
                EqBand::new(250).with_gain(-1.0),
                EqBand::new(1000).with_gain(2.0).with_q(1.5),
                EqBand::new(4000).with_gain(4.0).with_q(1.2),
                EqBand::new(16000).with_gain(1.0),
            ],
        }
    }

    /// Treble boost preset.
    pub fn treble_boost() -> Self {
        Self {
            name: "Treble Boost".to_string(),
            bands: [
                EqBand::new(60).with_gain(-2.0),
                EqBand::new(250),
                EqBand::new(1000),
                EqBand::new(4000).with_gain(3.0),
                EqBand::new(16000).with_gain(5.0),
            ],
        }
    }

    /// Cinema/movie preset.
    pub fn cinema() -> Self {
        Self {
            name: "Cinema".to_string(),
            bands: [
                EqBand::new(60).with_gain(4.0).with_q(0.7),
                EqBand::new(250).with_gain(1.0),
                EqBand::new(1000).with_gain(-1.0),
                EqBand::new(4000).with_gain(2.0),
                EqBand::new(16000).with_gain(3.0),
            ],
        }
    }

    /// Get all built-in presets.
    pub fn builtin_presets() -> Vec<Self> {
        vec![
            Self::flat(),
            Self::bass_boost(),
            Self::vocal_clarity(),
            Self::treble_boost(),
            Self::cinema(),
        ]
    }

    /// Load from TOML string.
    pub fn from_toml(s: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(s)
    }

    /// Serialize to TOML string.
    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(self)
    }

    /// Check if this preset is effectively flat (all gains near zero).
    pub fn is_flat(&self) -> bool {
        self.bands.iter().all(|b| b.gain.abs() < 0.1)
    }
}
