// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! High-pass filter plugin using cascaded biquad stages.

use super::dsp::{BiquadCoeffs, BiquadState};
use abi_stable::std_types::{ROption, RResult, RSlice, RSliceMut, RVec};
use serde::{Deserialize, Serialize};
use sootmix_plugin_api::{
    ActivationContext, ParameterCurve, ParameterInfo, PluginCategory, PluginError, PluginInfo,
};

const PARAM_CUTOFF: u32 = 0;
const PARAM_SLOPE: u32 = 1;

const CUTOFF_MIN: f32 = 20.0;
const CUTOFF_MAX: f32 = 1000.0;
const CUTOFF_DEFAULT: f32 = 80.0;

/// Per-channel biquad state (2 stages for optional 24 dB/oct).
#[derive(Debug, Clone, Default)]
struct ChannelState {
    stage1: BiquadState,
    stage2: BiquadState,
}

/// Saved state for serialization.
#[derive(Serialize, Deserialize)]
struct HpfState {
    params: [f32; 2],
}

/// High-pass filter plugin.
///
/// - 12 dB/oct (slope=0): single Butterworth biquad
/// - 24 dB/oct (slope=1): two cascaded Butterworth biquads
pub struct HpfPlugin {
    params: [f32; 2], // normalized 0.0-1.0
    sample_rate: f32,
    coeffs: BiquadCoeffs,
    channels: Vec<ChannelState>,
    last_cutoff: f32,
}

impl Default for HpfPlugin {
    fn default() -> Self {
        Self {
            params: [
                sootmix_plugin_api::normalize(CUTOFF_DEFAULT, CUTOFF_MIN, CUTOFF_MAX, ParameterCurve::Logarithmic),
                0.0, // 12 dB/oct
            ],
            sample_rate: 48000.0,
            coeffs: BiquadCoeffs::default(),
            channels: Vec::new(),
            last_cutoff: 0.0,
        }
    }
}

impl HpfPlugin {
    pub fn plugin_info() -> PluginInfo {
        PluginInfo::new("com.sootmix.hpf", "High-Pass Filter")
            .with_vendor("SootMix")
            .with_version("1.0.0")
            .with_category(PluginCategory::Filter)
            .with_channels(2, 2)
    }

    fn cutoff(&self) -> f32 {
        sootmix_plugin_api::denormalize(self.params[0], CUTOFF_MIN, CUTOFF_MAX, ParameterCurve::Logarithmic)
    }

    fn is_24db(&self) -> bool {
        self.params[1] >= 0.5
    }

    fn update_coeffs(&mut self) {
        let cutoff = self.cutoff();
        if (cutoff - self.last_cutoff).abs() > 0.01 {
            self.coeffs = BiquadCoeffs::highpass(cutoff, self.sample_rate);
            self.last_cutoff = cutoff;
        }
    }
}

impl sootmix_plugin_api::AudioEffect for HpfPlugin {
    fn info(&self) -> PluginInfo {
        Self::plugin_info()
    }

    fn activate(&mut self, context: ActivationContext) {
        self.sample_rate = context.sample_rate;
        self.channels = vec![ChannelState::default(); 2];
        self.last_cutoff = 0.0;
        self.update_coeffs();
    }

    fn deactivate(&mut self) {}

    fn process(&mut self, inputs: RSlice<RSlice<f32>>, mut outputs: RSliceMut<RSliceMut<f32>>) {
        self.update_coeffs();
        let use_24db = self.is_24db();
        let num_channels = inputs.len().min(outputs.len()).min(self.channels.len());

        for ch in 0..num_channels {
            let input = &inputs[ch];
            let output = &mut outputs[ch];
            let state = &mut self.channels[ch];

            for i in 0..input.len().min(output.len()) {
                let mut sample = state.stage1.process(input[i], &self.coeffs);
                if use_24db {
                    sample = state.stage2.process(sample, &self.coeffs);
                }
                output[i] = sample;
            }
        }
    }

    fn parameter_count(&self) -> u32 {
        2
    }

    fn parameter_info(&self, index: u32) -> ROption<ParameterInfo> {
        match index {
            PARAM_CUTOFF => ROption::RSome(
                ParameterInfo::new(0, "cutoff", "Cutoff", CUTOFF_MIN, CUTOFF_MAX, CUTOFF_DEFAULT)
                    .with_unit("Hz")
                    .with_curve(ParameterCurve::Logarithmic),
            ),
            PARAM_SLOPE => ROption::RSome(
                ParameterInfo::new(1, "slope", "Slope", 0.0, 1.0, 0.0)
                    .with_unit("dB/oct")
                    .with_step(1.0),
            ),
            _ => ROption::RNone,
        }
    }

    fn get_parameter(&self, index: u32) -> f32 {
        match index {
            0 | 1 => self.params[index as usize],
            _ => 0.0,
        }
    }

    fn set_parameter(&mut self, index: u32, value: f32) {
        if (index as usize) < self.params.len() {
            self.params[index as usize] = value.clamp(0.0, 1.0);
        }
    }

    fn save_state(&self) -> RVec<u8> {
        let state = HpfState { params: self.params };
        serde_json::to_vec(&state).unwrap_or_default().into()
    }

    fn load_state(&mut self, data: RSlice<u8>) -> RResult<(), PluginError> {
        match serde_json::from_slice::<HpfState>(&data) {
            Ok(state) => {
                self.params = state.params;
                self.last_cutoff = 0.0;
                RResult::ROk(())
            }
            Err(e) => RResult::RErr(PluginError::StateLoadFailed(e.to_string().into())),
        }
    }

    fn reset(&mut self) {
        for ch in &mut self.channels {
            ch.stage1.reset();
            ch.stage2.reset();
        }
    }
}
