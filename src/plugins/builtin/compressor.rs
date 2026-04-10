// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Feed-forward compressor with soft knee.
//!
//! Based on the Giannoulis et al. (JAES 2012) design:
//! - Log-domain gain computation
//! - Soft knee via quadratic interpolation
//! - Smooth branching envelope follower

use super::dsp::time_to_coeff;
use abi_stable::std_types::{ROption, RResult, RSlice, RSliceMut, RVec};
use serde::{Deserialize, Serialize};
use sootmix_plugin_api::{
    ActivationContext, ParameterCurve, ParameterInfo, PluginCategory, PluginError, PluginInfo,
    db_to_linear, linear_to_db,
};

const PARAM_THRESHOLD: u32 = 0;
const PARAM_RATIO: u32 = 1;
const PARAM_ATTACK: u32 = 2;
const PARAM_RELEASE: u32 = 3;
const PARAM_KNEE: u32 = 4;
const PARAM_MAKEUP: u32 = 5;
const NUM_PARAMS: usize = 6;

const THRESH_MIN: f32 = -60.0;
const THRESH_MAX: f32 = 0.0;
const THRESH_DEFAULT: f32 = -20.0;

const RATIO_MIN: f32 = 1.0;
const RATIO_MAX: f32 = 20.0;
const RATIO_DEFAULT: f32 = 4.0;

const ATTACK_MIN: f32 = 0.1;
const ATTACK_MAX: f32 = 200.0;
const ATTACK_DEFAULT: f32 = 10.0;

const RELEASE_MIN: f32 = 10.0;
const RELEASE_MAX: f32 = 2000.0;
const RELEASE_DEFAULT: f32 = 100.0;

const KNEE_MIN: f32 = 0.0;
const KNEE_MAX: f32 = 20.0;
const KNEE_DEFAULT: f32 = 6.0;

const MAKEUP_MIN: f32 = -12.0;
const MAKEUP_MAX: f32 = 24.0;
const MAKEUP_DEFAULT: f32 = 0.0;

#[derive(Serialize, Deserialize)]
struct CompressorPreset {
    params: [f32; NUM_PARAMS],
}

/// Feed-forward compressor with soft knee.
pub struct CompressorPlugin {
    params: [f32; NUM_PARAMS],
    sample_rate: f32,

    // Envelope state (dB domain)
    envelope_db: f32,

    // Cached coefficients
    attack_coeff: f32,
    release_coeff: f32,
}

impl Default for CompressorPlugin {
    fn default() -> Self {
        Self {
            params: [
                sootmix_plugin_api::normalize(THRESH_DEFAULT, THRESH_MIN, THRESH_MAX, ParameterCurve::Linear),
                sootmix_plugin_api::normalize(RATIO_DEFAULT, RATIO_MIN, RATIO_MAX, ParameterCurve::Logarithmic),
                sootmix_plugin_api::normalize(ATTACK_DEFAULT, ATTACK_MIN, ATTACK_MAX, ParameterCurve::Logarithmic),
                sootmix_plugin_api::normalize(RELEASE_DEFAULT, RELEASE_MIN, RELEASE_MAX, ParameterCurve::Logarithmic),
                sootmix_plugin_api::normalize(KNEE_DEFAULT, KNEE_MIN, KNEE_MAX, ParameterCurve::Linear),
                sootmix_plugin_api::normalize(MAKEUP_DEFAULT, MAKEUP_MIN, MAKEUP_MAX, ParameterCurve::Linear),
            ],
            sample_rate: 48000.0,
            envelope_db: 0.0,
            attack_coeff: 0.0,
            release_coeff: 0.0,
        }
    }
}

impl CompressorPlugin {
    pub fn plugin_info() -> PluginInfo {
        PluginInfo::new("com.sootmix.compressor", "Compressor")
            .with_vendor("SootMix")
            .with_version("1.0.0")
            .with_category(PluginCategory::Dynamics)
            .with_channels(2, 2)
    }

    fn denorm(&self, index: u32) -> f32 {
        let (min, max, curve) = match index {
            PARAM_THRESHOLD => (THRESH_MIN, THRESH_MAX, ParameterCurve::Linear),
            PARAM_RATIO => (RATIO_MIN, RATIO_MAX, ParameterCurve::Logarithmic),
            PARAM_ATTACK => (ATTACK_MIN, ATTACK_MAX, ParameterCurve::Logarithmic),
            PARAM_RELEASE => (RELEASE_MIN, RELEASE_MAX, ParameterCurve::Logarithmic),
            PARAM_KNEE => (KNEE_MIN, KNEE_MAX, ParameterCurve::Linear),
            PARAM_MAKEUP => (MAKEUP_MIN, MAKEUP_MAX, ParameterCurve::Linear),
            _ => return 0.0,
        };
        sootmix_plugin_api::denormalize(self.params[index as usize], min, max, curve)
    }

    fn update_coeffs(&mut self) {
        let attack_ms = self.denorm(PARAM_ATTACK);
        let release_ms = self.denorm(PARAM_RELEASE);
        self.attack_coeff = time_to_coeff(attack_ms / 1000.0, self.sample_rate);
        self.release_coeff = time_to_coeff(release_ms / 1000.0, self.sample_rate);
    }

    /// Gain computer with soft knee (Giannoulis et al.)
    ///
    /// Returns the gain reduction in dB for a given input level in dB.
    #[inline]
    fn gain_computer(input_db: f32, threshold: f32, ratio: f32, knee_width: f32) -> f32 {
        let half_knee = knee_width / 2.0;

        if knee_width > 0.0 && (input_db - threshold).abs() < half_knee {
            // Soft knee region: quadratic interpolation
            let x = input_db - threshold + half_knee;
            input_db + ((1.0 / ratio) - 1.0) * x * x / (2.0 * knee_width)
        } else if input_db >= threshold + half_knee {
            // Above knee: apply ratio
            threshold + (input_db - threshold) / ratio
        } else {
            // Below threshold: no compression
            input_db
        }
    }
}

impl sootmix_plugin_api::AudioEffect for CompressorPlugin {
    fn info(&self) -> PluginInfo {
        Self::plugin_info()
    }

    fn activate(&mut self, context: ActivationContext) {
        self.sample_rate = context.sample_rate;
        self.envelope_db = 0.0;
        self.update_coeffs();
    }

    fn deactivate(&mut self) {}

    fn process(&mut self, inputs: RSlice<RSlice<f32>>, mut outputs: RSliceMut<RSliceMut<f32>>) {
        let threshold = self.denorm(PARAM_THRESHOLD);
        let ratio = self.denorm(PARAM_RATIO);
        let knee_width = self.denorm(PARAM_KNEE);
        let makeup_db = self.denorm(PARAM_MAKEUP);
        let makeup_linear = db_to_linear(makeup_db);

        self.update_coeffs();
        let attack_coeff = self.attack_coeff;
        let release_coeff = self.release_coeff;

        let num_channels = inputs.len().min(outputs.len());
        if num_channels == 0 {
            return;
        }

        let num_samples = inputs[0].len();

        for i in 0..num_samples {
            // Stereo-linked peak detection
            let mut peak = 0.0_f32;
            for ch in 0..num_channels {
                if i < inputs[ch].len() {
                    peak = peak.max(inputs[ch][i].abs());
                }
            }

            let input_db = linear_to_db(peak);

            // Gain computer: compute desired output level
            let output_db = Self::gain_computer(input_db, threshold, ratio, knee_width);
            let gain_reduction_db = output_db - input_db; // always <= 0

            // Smooth branching envelope follower
            // Attack when compressing more, release when compressing less
            let coeff = if gain_reduction_db < self.envelope_db {
                attack_coeff
            } else {
                release_coeff
            };
            self.envelope_db = coeff * self.envelope_db + (1.0 - coeff) * gain_reduction_db;

            // Convert to linear gain and apply makeup
            let gain = db_to_linear(self.envelope_db) * makeup_linear;

            // Apply gain to all channels
            for ch in 0..num_channels {
                if i < inputs[ch].len() && i < outputs[ch].len() {
                    outputs[ch][i] = inputs[ch][i] * gain;
                }
            }
        }
    }

    fn parameter_count(&self) -> u32 {
        NUM_PARAMS as u32
    }

    fn parameter_info(&self, index: u32) -> ROption<ParameterInfo> {
        match index {
            PARAM_THRESHOLD => ROption::RSome(
                ParameterInfo::new(0, "threshold", "Threshold", THRESH_MIN, THRESH_MAX, THRESH_DEFAULT)
                    .with_unit("dB"),
            ),
            PARAM_RATIO => ROption::RSome(
                ParameterInfo::new(1, "ratio", "Ratio", RATIO_MIN, RATIO_MAX, RATIO_DEFAULT)
                    .with_unit(":1")
                    .with_curve(ParameterCurve::Logarithmic),
            ),
            PARAM_ATTACK => ROption::RSome(
                ParameterInfo::new(2, "attack", "Attack", ATTACK_MIN, ATTACK_MAX, ATTACK_DEFAULT)
                    .with_unit("ms")
                    .with_curve(ParameterCurve::Logarithmic),
            ),
            PARAM_RELEASE => ROption::RSome(
                ParameterInfo::new(3, "release", "Release", RELEASE_MIN, RELEASE_MAX, RELEASE_DEFAULT)
                    .with_unit("ms")
                    .with_curve(ParameterCurve::Logarithmic),
            ),
            PARAM_KNEE => ROption::RSome(
                ParameterInfo::new(4, "knee", "Knee Width", KNEE_MIN, KNEE_MAX, KNEE_DEFAULT)
                    .with_unit("dB"),
            ),
            PARAM_MAKEUP => ROption::RSome(
                ParameterInfo::new(5, "makeup", "Makeup Gain", MAKEUP_MIN, MAKEUP_MAX, MAKEUP_DEFAULT)
                    .with_unit("dB"),
            ),
            _ => ROption::RNone,
        }
    }

    fn get_parameter(&self, index: u32) -> f32 {
        if (index as usize) < NUM_PARAMS {
            self.params[index as usize]
        } else {
            0.0
        }
    }

    fn set_parameter(&mut self, index: u32, value: f32) {
        if (index as usize) < NUM_PARAMS {
            self.params[index as usize] = value.clamp(0.0, 1.0);
        }
    }

    fn save_state(&self) -> RVec<u8> {
        let state = CompressorPreset { params: self.params };
        serde_json::to_vec(&state).unwrap_or_default().into()
    }

    fn load_state(&mut self, data: RSlice<u8>) -> RResult<(), PluginError> {
        match serde_json::from_slice::<CompressorPreset>(&data) {
            Ok(state) => {
                self.params = state.params;
                RResult::ROk(())
            }
            Err(e) => RResult::RErr(PluginError::StateLoadFailed(e.to_string().into())),
        }
    }

    fn reset(&mut self) {
        self.envelope_db = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gain_computer_below_threshold() {
        // Below threshold: no gain reduction
        let out = CompressorPlugin::gain_computer(-30.0, -20.0, 4.0, 0.0);
        assert!((out - (-30.0)).abs() < 0.001);
    }

    #[test]
    fn test_gain_computer_above_threshold_hard_knee() {
        // 10 dB above threshold with 4:1 ratio, hard knee
        let out = CompressorPlugin::gain_computer(-10.0, -20.0, 4.0, 0.0);
        // Should be: -20 + (10 / 4) = -17.5
        assert!((out - (-17.5)).abs() < 0.001);
    }

    #[test]
    fn test_gain_computer_soft_knee() {
        // Below threshold: hard knee has no compression, soft knee has gradual onset
        let hard = CompressorPlugin::gain_computer(-24.0, -20.0, 4.0, 0.0);
        let soft = CompressorPlugin::gain_computer(-24.0, -20.0, 4.0, 12.0);
        // Hard knee: no compression below threshold
        assert!((hard - (-24.0)).abs() < 0.001);
        // Soft knee: slight compression within knee region
        assert!(soft < hard, "Soft knee should begin compressing below threshold");

        // Well above knee: both converge
        let hard_high = CompressorPlugin::gain_computer(0.0, -20.0, 4.0, 0.0);
        let soft_high = CompressorPlugin::gain_computer(0.0, -20.0, 4.0, 12.0);
        assert!((hard_high - soft_high).abs() < 0.1);
    }

    #[test]
    fn test_unity_passthrough_below_threshold() {
        use abi_stable::std_types::{RSlice, RSliceMut};
        use sootmix_plugin_api::AudioEffect;

        let mut plugin = CompressorPlugin::default();
        plugin.activate(ActivationContext { sample_rate: 48000.0, max_block_size: 512 });

        // Set threshold high so signal is below
        plugin.set_parameter(PARAM_THRESHOLD,
            sootmix_plugin_api::normalize(0.0, THRESH_MIN, THRESH_MAX, ParameterCurve::Linear));
        plugin.set_parameter(PARAM_MAKEUP,
            sootmix_plugin_api::normalize(0.0, MAKEUP_MIN, MAKEUP_MAX, ParameterCurve::Linear));

        // Quiet signal at -40 dB
        let signal: Vec<f32> = (0..512).map(|i| 0.01 * (i as f32 * 0.1).sin()).collect();
        let mut out_l = vec![0.0_f32; 512];
        let mut out_r = vec![0.0_f32; 512];

        let inputs_r = [RSlice::from_slice(&signal), RSlice::from_slice(&signal)];
        let inputs = RSlice::from_slice(&inputs_r);
        let mut outputs_r = [RSliceMut::from_mut_slice(&mut out_l), RSliceMut::from_mut_slice(&mut out_r)];
        let outputs = RSliceMut::from_mut_slice(&mut outputs_r);

        plugin.process(inputs, outputs);

        // Output should be very close to input (no compression applied)
        for i in 100..512 {
            let diff = (out_l[i] - signal[i]).abs();
            assert!(diff < 0.01, "Sample {} differs too much: {} vs {}", i, out_l[i], signal[i]);
        }
    }
}
