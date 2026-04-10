// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Shared DSP primitives for built-in plugins.

use std::f32::consts::PI;

/// Convert a time constant (seconds) to a one-pole smoothing coefficient.
///
/// Returns the `a` coefficient for: `y[n] = a * y[n-1] + (1-a) * x[n]`
/// where the output reaches ~63% of the target after `time_sec`.
#[inline]
pub fn time_to_coeff(time_sec: f32, sample_rate: f32) -> f32 {
    if time_sec <= 0.0 {
        return 0.0;
    }
    (-1.0 / (time_sec * sample_rate)).exp()
}

/// One-pole parameter smoother to avoid zipper noise.
#[derive(Debug, Clone)]
pub struct OnePoleSmooth {
    current: f32,
    coeff: f32,
}

impl OnePoleSmooth {
    /// Create a new smoother with a given time constant.
    pub fn new(time_sec: f32, sample_rate: f32) -> Self {
        Self {
            current: 0.0,
            coeff: time_to_coeff(time_sec, sample_rate),
        }
    }

    /// Set the smoothing time.
    pub fn set_time(&mut self, time_sec: f32, sample_rate: f32) {
        self.coeff = time_to_coeff(time_sec, sample_rate);
    }

    /// Process one sample toward the target value.
    #[inline]
    pub fn process(&mut self, target: f32) -> f32 {
        self.current = self.coeff * self.current + (1.0 - self.coeff) * target;
        self.current
    }

    /// Jump immediately to a value (no smoothing).
    pub fn set(&mut self, value: f32) {
        self.current = value;
    }

    /// Get the current smoothed value.
    #[inline]
    pub fn value(&self) -> f32 {
        self.current
    }
}

/// Biquad filter coefficients (normalized).
#[derive(Debug, Clone, Copy)]
pub struct BiquadCoeffs {
    pub b0: f32,
    pub b1: f32,
    pub b2: f32,
    pub a1: f32,
    pub a2: f32,
}

impl Default for BiquadCoeffs {
    fn default() -> Self {
        // Unity pass-through
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
        }
    }
}

impl BiquadCoeffs {
    /// Compute Butterworth (Q=0.7071) highpass coefficients.
    ///
    /// Based on Robert Bristow-Johnson Audio EQ Cookbook.
    pub fn highpass(cutoff_hz: f32, sample_rate: f32) -> Self {
        let w0 = 2.0 * PI * cutoff_hz / sample_rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        // Q = 1/sqrt(2) for Butterworth
        let alpha = sin_w0 / (2.0 * std::f32::consts::FRAC_1_SQRT_2.recip());

        let b0 = (1.0 + cos_w0) / 2.0;
        let b1 = -(1.0 + cos_w0);
        let b2 = (1.0 + cos_w0) / 2.0;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha;

        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        }
    }
}

/// Biquad filter state — Direct Form II Transposed.
///
/// This form has better numerical properties for floating-point arithmetic.
#[derive(Debug, Clone, Default)]
pub struct BiquadState {
    s1: f32,
    s2: f32,
}

impl BiquadState {
    /// Process one sample through the filter.
    #[inline]
    pub fn process(&mut self, input: f32, coeffs: &BiquadCoeffs) -> f32 {
        let output = coeffs.b0 * input + self.s1;
        self.s1 = coeffs.b1 * input - coeffs.a1 * output + self.s2;
        self.s2 = coeffs.b2 * input - coeffs.a2 * output;
        output
    }

    /// Reset the filter state.
    pub fn reset(&mut self) {
        self.s1 = 0.0;
        self.s2 = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_time_to_coeff_zero() {
        assert_eq!(time_to_coeff(0.0, 48000.0), 0.0);
    }

    #[test]
    fn test_time_to_coeff_range() {
        let c = time_to_coeff(0.01, 48000.0);
        assert!(c > 0.0 && c < 1.0);
    }

    #[test]
    fn test_one_pole_convergence() {
        let mut smoother = OnePoleSmooth::new(0.005, 48000.0);
        smoother.set(0.0);
        // After many samples toward 1.0, should converge
        for _ in 0..48000 {
            smoother.process(1.0);
        }
        assert!((smoother.value() - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_biquad_dc_rejection() {
        let coeffs = BiquadCoeffs::highpass(100.0, 48000.0);
        let mut state = BiquadState::default();
        // Feed DC (constant 1.0) through HPF — output should settle near 0
        let mut last = 0.0;
        for _ in 0..48000 {
            last = state.process(1.0, &coeffs);
        }
        assert!(last.abs() < 0.001, "DC should be rejected, got {}", last);
    }

    #[test]
    fn test_biquad_passband() {
        let sample_rate = 48000.0;
        let cutoff = 100.0;
        let coeffs = BiquadCoeffs::highpass(cutoff, sample_rate);
        let mut state = BiquadState::default();

        // Feed a 1kHz sine (well above 100 Hz cutoff), measure output amplitude
        let freq = 1000.0;
        let mut max_out = 0.0_f32;
        for i in 0..4800 {
            let input = (2.0 * PI * freq * i as f32 / sample_rate).sin();
            let out = state.process(input, &coeffs);
            if i > 480 {
                // skip transient
                max_out = max_out.max(out.abs());
            }
        }
        // Should pass through ~1.0 amplitude
        assert!(max_out > 0.9, "Passband signal attenuated too much: {}", max_out);
    }
}
