// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Noise gate plugin with envelope-following and state machine.

use super::dsp::{time_to_coeff, OnePoleSmooth};
use abi_stable::std_types::{ROption, RResult, RSlice, RSliceMut, RVec};
use serde::{Deserialize, Serialize};
use sootmix_plugin_api::{
    ActivationContext, ParameterCurve, ParameterInfo, PluginCategory, PluginError, PluginInfo,
    db_to_linear, linear_to_db,
};

const PARAM_THRESHOLD: u32 = 0;
const PARAM_HYSTERESIS: u32 = 1;
const PARAM_ATTACK: u32 = 2;
const PARAM_HOLD: u32 = 3;
const PARAM_RELEASE: u32 = 4;
const PARAM_RANGE: u32 = 5;
const NUM_PARAMS: usize = 6;

// Parameter ranges
const THRESH_MIN: f32 = -80.0;
const THRESH_MAX: f32 = 0.0;
const THRESH_DEFAULT: f32 = -40.0;

const HYST_MIN: f32 = 0.0;
const HYST_MAX: f32 = 12.0;
const HYST_DEFAULT: f32 = 6.0;

const ATTACK_MIN: f32 = 0.01;
const ATTACK_MAX: f32 = 50.0;
const ATTACK_DEFAULT: f32 = 0.5;

const HOLD_MIN: f32 = 0.01;
const HOLD_MAX: f32 = 500.0;
const HOLD_DEFAULT: f32 = 50.0;

const RELEASE_MIN: f32 = 5.0;
const RELEASE_MAX: f32 = 2000.0;
const RELEASE_DEFAULT: f32 = 100.0;

const RANGE_MIN: f32 = -80.0;
const RANGE_MAX: f32 = 0.0;
const RANGE_DEFAULT: f32 = -80.0;

#[derive(Debug, Clone, Copy, PartialEq)]
enum GateState {
    Closed,
    Opening,
    Open,
    Holding,
    Closing,
}

#[derive(Serialize, Deserialize)]
struct GatePreset {
    params: [f32; NUM_PARAMS],
}

/// Noise gate with attack/hold/release envelope and hysteresis.
pub struct NoiseGatePlugin {
    params: [f32; NUM_PARAMS],
    sample_rate: f32,

    // Envelope state
    state: GateState,
    gain: f32,          // current gain (linear, 0..1)
    hold_counter: u32,  // samples remaining in hold phase

    // Coefficients (recalculated on parameter change)
    attack_coeff: f32,
    release_coeff: f32,

    // Peak envelope follower
    envelope: OnePoleSmooth,
}

impl Default for NoiseGatePlugin {
    fn default() -> Self {
        Self {
            params: [
                sootmix_plugin_api::normalize(THRESH_DEFAULT, THRESH_MIN, THRESH_MAX, ParameterCurve::Linear),
                sootmix_plugin_api::normalize(HYST_DEFAULT, HYST_MIN, HYST_MAX, ParameterCurve::Linear),
                sootmix_plugin_api::normalize(ATTACK_DEFAULT, ATTACK_MIN, ATTACK_MAX, ParameterCurve::Logarithmic),
                sootmix_plugin_api::normalize(HOLD_DEFAULT, HOLD_MIN, HOLD_MAX, ParameterCurve::Logarithmic),
                sootmix_plugin_api::normalize(RELEASE_DEFAULT, RELEASE_MIN, RELEASE_MAX, ParameterCurve::Logarithmic),
                sootmix_plugin_api::normalize(RANGE_DEFAULT, RANGE_MIN, RANGE_MAX, ParameterCurve::Linear),
            ],
            sample_rate: 48000.0,
            state: GateState::Closed,
            gain: 0.0,
            hold_counter: 0,
            attack_coeff: 0.0,
            release_coeff: 0.0,
            envelope: OnePoleSmooth::new(0.001, 48000.0),
        }
    }
}

impl NoiseGatePlugin {
    pub fn plugin_info() -> PluginInfo {
        PluginInfo::new("com.sootmix.gate", "Noise Gate")
            .with_vendor("SootMix")
            .with_version("1.0.0")
            .with_category(PluginCategory::Dynamics)
            .with_channels(2, 2)
    }

    fn denorm(&self, index: u32) -> f32 {
        let (min, max, curve) = match index {
            PARAM_THRESHOLD => (THRESH_MIN, THRESH_MAX, ParameterCurve::Linear),
            PARAM_HYSTERESIS => (HYST_MIN, HYST_MAX, ParameterCurve::Linear),
            PARAM_ATTACK => (ATTACK_MIN, ATTACK_MAX, ParameterCurve::Logarithmic),
            PARAM_HOLD => (HOLD_MIN, HOLD_MAX, ParameterCurve::Logarithmic),
            PARAM_RELEASE => (RELEASE_MIN, RELEASE_MAX, ParameterCurve::Logarithmic),
            PARAM_RANGE => (RANGE_MIN, RANGE_MAX, ParameterCurve::Linear),
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
}

impl sootmix_plugin_api::AudioEffect for NoiseGatePlugin {
    fn info(&self) -> PluginInfo {
        Self::plugin_info()
    }

    fn activate(&mut self, context: ActivationContext) {
        self.sample_rate = context.sample_rate;
        self.envelope = OnePoleSmooth::new(0.001, context.sample_rate);
        self.update_coeffs();
        self.state = GateState::Closed;
        self.gain = 0.0;
        self.hold_counter = 0;
    }

    fn deactivate(&mut self) {}

    fn process(&mut self, inputs: RSlice<RSlice<f32>>, mut outputs: RSliceMut<RSliceMut<f32>>) {
        let threshold_db = self.denorm(PARAM_THRESHOLD);
        let hysteresis_db = self.denorm(PARAM_HYSTERESIS);
        let hold_ms = self.denorm(PARAM_HOLD);
        let range_db = self.denorm(PARAM_RANGE);

        let open_thresh = db_to_linear(threshold_db);
        let close_thresh = db_to_linear(threshold_db - hysteresis_db);
        let range_gain = db_to_linear(range_db);
        let hold_samples = ((hold_ms / 1000.0) * self.sample_rate) as u32;

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
            let level = self.envelope.process(peak);

            // State machine
            match self.state {
                GateState::Closed => {
                    if level >= open_thresh {
                        self.state = GateState::Opening;
                    }
                }
                GateState::Opening => {
                    // Ramp gain up using attack coefficient
                    self.gain = attack_coeff * self.gain + (1.0 - attack_coeff) * 1.0;
                    if self.gain >= 0.999 {
                        self.gain = 1.0;
                        self.state = GateState::Open;
                    }
                }
                GateState::Open => {
                    self.gain = 1.0;
                    if level < close_thresh {
                        self.state = GateState::Holding;
                        self.hold_counter = hold_samples;
                    }
                }
                GateState::Holding => {
                    self.gain = 1.0;
                    if level >= open_thresh {
                        self.state = GateState::Open;
                    } else if self.hold_counter == 0 {
                        self.state = GateState::Closing;
                    } else {
                        self.hold_counter -= 1;
                    }
                }
                GateState::Closing => {
                    // Ramp gain down using release coefficient
                    self.gain = release_coeff * self.gain;
                    if level >= open_thresh {
                        self.state = GateState::Opening;
                    } else if self.gain <= range_gain + 0.001 {
                        self.gain = range_gain;
                        self.state = GateState::Closed;
                    }
                }
            }

            // Apply gain (with range floor)
            let effective_gain = range_gain + (1.0 - range_gain) * self.gain;
            for ch in 0..num_channels {
                if i < inputs[ch].len() && i < outputs[ch].len() {
                    outputs[ch][i] = inputs[ch][i] * effective_gain;
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
            PARAM_HYSTERESIS => ROption::RSome(
                ParameterInfo::new(1, "hysteresis", "Hysteresis", HYST_MIN, HYST_MAX, HYST_DEFAULT)
                    .with_unit("dB"),
            ),
            PARAM_ATTACK => ROption::RSome(
                ParameterInfo::new(2, "attack", "Attack", ATTACK_MIN, ATTACK_MAX, ATTACK_DEFAULT)
                    .with_unit("ms")
                    .with_curve(ParameterCurve::Logarithmic),
            ),
            PARAM_HOLD => ROption::RSome(
                ParameterInfo::new(3, "hold", "Hold", HOLD_MIN, HOLD_MAX, HOLD_DEFAULT)
                    .with_unit("ms")
                    .with_curve(ParameterCurve::Logarithmic),
            ),
            PARAM_RELEASE => ROption::RSome(
                ParameterInfo::new(4, "release", "Release", RELEASE_MIN, RELEASE_MAX, RELEASE_DEFAULT)
                    .with_unit("ms")
                    .with_curve(ParameterCurve::Logarithmic),
            ),
            PARAM_RANGE => ROption::RSome(
                ParameterInfo::new(5, "range", "Range", RANGE_MIN, RANGE_MAX, RANGE_DEFAULT)
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
        let state = GatePreset { params: self.params };
        serde_json::to_vec(&state).unwrap_or_default().into()
    }

    fn load_state(&mut self, data: RSlice<u8>) -> RResult<(), PluginError> {
        match serde_json::from_slice::<GatePreset>(&data) {
            Ok(state) => {
                self.params = state.params;
                RResult::ROk(())
            }
            Err(e) => RResult::RErr(PluginError::StateLoadFailed(e.to_string().into())),
        }
    }

    fn reset(&mut self) {
        self.state = GateState::Closed;
        self.gain = 0.0;
        self.hold_counter = 0;
        self.envelope.set(0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use abi_stable::std_types::{RSlice, RSliceMut};

    fn make_plugin() -> NoiseGatePlugin {
        use sootmix_plugin_api::AudioEffect;
        let mut p = NoiseGatePlugin::default();
        p.activate(ActivationContext { sample_rate: 48000.0, max_block_size: 512 });
        p
    }

    #[test]
    fn test_silence_is_gated() {
        use sootmix_plugin_api::AudioEffect;
        let mut plugin = make_plugin();
        // Set threshold to -20 dB so silence is gated
        plugin.set_parameter(PARAM_THRESHOLD,
            sootmix_plugin_api::normalize(-20.0, THRESH_MIN, THRESH_MAX, ParameterCurve::Linear));
        // Range = -80 dB (full gate)
        plugin.set_parameter(PARAM_RANGE,
            sootmix_plugin_api::normalize(-80.0, RANGE_MIN, RANGE_MAX, ParameterCurve::Linear));

        let silence = vec![0.0_f32; 256];
        let mut out_l = vec![0.0_f32; 256];
        let mut out_r = vec![0.0_f32; 256];

        let inputs_r = [RSlice::from_slice(&silence), RSlice::from_slice(&silence)];
        let inputs = RSlice::from_slice(&inputs_r);
        let mut outputs_r = [RSliceMut::from_mut_slice(&mut out_l), RSliceMut::from_mut_slice(&mut out_r)];
        let outputs = RSliceMut::from_mut_slice(&mut outputs_r);

        plugin.process(inputs, outputs);

        let max: f32 = out_l.iter().map(|s| s.abs()).fold(0.0, f32::max);
        assert!(max < 0.001, "Silence should be gated, got max={}", max);
    }

    #[test]
    fn test_loud_signal_passes() {
        use sootmix_plugin_api::AudioEffect;
        let mut plugin = make_plugin();
        plugin.set_parameter(PARAM_THRESHOLD,
            sootmix_plugin_api::normalize(-40.0, THRESH_MIN, THRESH_MAX, ParameterCurve::Linear));

        // Feed a loud signal (0.5 = ~-6 dB, well above -40 dB threshold)
        let loud: Vec<f32> = (0..1024).map(|i| 0.5 * (i as f32 * 0.1).sin()).collect();
        let mut out_l = vec![0.0_f32; 1024];
        let mut out_r = vec![0.0_f32; 1024];

        let inputs_r = [RSlice::from_slice(&loud), RSlice::from_slice(&loud)];
        let inputs = RSlice::from_slice(&inputs_r);
        let mut outputs_r = [RSliceMut::from_mut_slice(&mut out_l), RSliceMut::from_mut_slice(&mut out_r)];
        let outputs = RSliceMut::from_mut_slice(&mut outputs_r);

        plugin.process(inputs, outputs);

        // After attack time, output should be close to input
        let late_max: f32 = out_l[512..].iter().map(|s| s.abs()).fold(0.0, f32::max);
        assert!(late_max > 0.3, "Loud signal should pass through gate, got max={}", late_max);
    }
}
