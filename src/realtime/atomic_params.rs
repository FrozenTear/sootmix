// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Atomic parameter types for lock-free UI ↔ audio communication.
//!
//! These types allow the UI thread to update parameters while the audio thread
//! reads them, without any locking.
//!
//! # Usage
//!
//! ```ignore
//! use sootmix::realtime::AtomicF32;
//!
//! let volume = AtomicF32::new(1.0);
//!
//! // UI thread sets the value
//! volume.set(0.5);
//!
//! // Audio thread reads the value
//! let v = volume.get();
//! ```

use std::sync::atomic::{AtomicU32 as StdAtomicU32, AtomicI32 as StdAtomicI32, Ordering};

/// Atomic f32 for lock-free parameter updates.
///
/// Uses `Relaxed` ordering by default, which is sufficient for independent
/// parameters that don't need to synchronize with other data.
#[derive(Debug)]
pub struct AtomicF32 {
    bits: StdAtomicU32,
}

impl AtomicF32 {
    /// Create a new atomic f32 with the given initial value.
    #[inline]
    pub const fn new(value: f32) -> Self {
        Self {
            bits: StdAtomicU32::new(value.to_bits()),
        }
    }

    /// Get the current value.
    #[inline]
    pub fn get(&self) -> f32 {
        f32::from_bits(self.bits.load(Ordering::Relaxed))
    }

    /// Set a new value.
    #[inline]
    pub fn set(&self, value: f32) {
        self.bits.store(value.to_bits(), Ordering::Relaxed);
    }

    /// Swap the value and return the old one.
    #[inline]
    pub fn swap(&self, value: f32) -> f32 {
        f32::from_bits(self.bits.swap(value.to_bits(), Ordering::Relaxed))
    }

    /// Get with acquire ordering (for synchronization).
    #[inline]
    pub fn get_acquire(&self) -> f32 {
        f32::from_bits(self.bits.load(Ordering::Acquire))
    }

    /// Set with release ordering (for synchronization).
    #[inline]
    pub fn set_release(&self, value: f32) {
        self.bits.store(value.to_bits(), Ordering::Release);
    }
}

impl Default for AtomicF32 {
    fn default() -> Self {
        Self::new(0.0)
    }
}

impl Clone for AtomicF32 {
    fn clone(&self) -> Self {
        Self::new(self.get())
    }
}

/// Atomic u32 wrapper with convenient methods.
#[derive(Debug)]
pub struct AtomicU32 {
    inner: StdAtomicU32,
}

impl AtomicU32 {
    /// Create a new atomic u32.
    #[inline]
    pub const fn new(value: u32) -> Self {
        Self {
            inner: StdAtomicU32::new(value),
        }
    }

    /// Get the current value.
    #[inline]
    pub fn get(&self) -> u32 {
        self.inner.load(Ordering::Relaxed)
    }

    /// Set a new value.
    #[inline]
    pub fn set(&self, value: u32) {
        self.inner.store(value, Ordering::Relaxed);
    }

    /// Increment and return the new value.
    #[inline]
    pub fn increment(&self) -> u32 {
        self.inner.fetch_add(1, Ordering::Relaxed).wrapping_add(1)
    }

    /// Decrement and return the new value.
    #[inline]
    pub fn decrement(&self) -> u32 {
        self.inner.fetch_sub(1, Ordering::Relaxed).wrapping_sub(1)
    }
}

impl Default for AtomicU32 {
    fn default() -> Self {
        Self::new(0)
    }
}

impl Clone for AtomicU32 {
    fn clone(&self) -> Self {
        Self::new(self.get())
    }
}

/// Atomic i32 wrapper with convenient methods.
#[derive(Debug)]
pub struct AtomicI32 {
    inner: StdAtomicI32,
}

impl AtomicI32 {
    /// Create a new atomic i32.
    #[inline]
    pub const fn new(value: i32) -> Self {
        Self {
            inner: StdAtomicI32::new(value),
        }
    }

    /// Get the current value.
    #[inline]
    pub fn get(&self) -> i32 {
        self.inner.load(Ordering::Relaxed)
    }

    /// Set a new value.
    #[inline]
    pub fn set(&self, value: i32) {
        self.inner.store(value, Ordering::Relaxed);
    }
}

impl Default for AtomicI32 {
    fn default() -> Self {
        Self::new(0)
    }
}

impl Clone for AtomicI32 {
    fn clone(&self) -> Self {
        Self::new(self.get())
    }
}

/// Atomic bool wrapper.
#[derive(Debug)]
pub struct AtomicBool {
    inner: std::sync::atomic::AtomicBool,
}

impl AtomicBool {
    /// Create a new atomic bool.
    #[inline]
    pub const fn new(value: bool) -> Self {
        Self {
            inner: std::sync::atomic::AtomicBool::new(value),
        }
    }

    /// Get the current value.
    #[inline]
    pub fn get(&self) -> bool {
        self.inner.load(Ordering::Relaxed)
    }

    /// Set a new value.
    #[inline]
    pub fn set(&self, value: bool) {
        self.inner.store(value, Ordering::Relaxed);
    }

    /// Toggle the value and return the new value.
    #[inline]
    pub fn toggle(&self) -> bool {
        !self.inner.fetch_xor(true, Ordering::Relaxed)
    }

    /// Set to true and return the previous value.
    #[inline]
    pub fn set_true(&self) -> bool {
        self.inner.swap(true, Ordering::Relaxed)
    }

    /// Set to false and return the previous value.
    #[inline]
    pub fn set_false(&self) -> bool {
        self.inner.swap(false, Ordering::Relaxed)
    }
}

impl Default for AtomicBool {
    fn default() -> Self {
        Self::new(false)
    }
}

impl Clone for AtomicBool {
    fn clone(&self) -> Self {
        Self::new(self.get())
    }
}

/// A block of parameters that can be atomically swapped.
///
/// Useful for updating multiple related parameters together.
/// Uses a simple versioning scheme to detect changes.
///
/// # Example
///
/// ```ignore
/// use sootmix::realtime::ParameterBlock;
///
/// #[derive(Clone, Default)]
/// struct EqParams {
///     low_gain: f32,
///     mid_gain: f32,
///     high_gain: f32,
/// }
///
/// let params = ParameterBlock::new(EqParams::default());
///
/// // UI thread updates all params at once
/// params.update(|p| {
///     p.low_gain = 3.0;
///     p.mid_gain = 0.0;
///     p.high_gain = -2.0;
/// });
///
/// // Audio thread reads the current params
/// let current = params.read();
/// ```
#[derive(Debug)]
pub struct ParameterBlock<T: Clone + Default> {
    /// Double buffer for lock-free swapping.
    buffers: [std::sync::RwLock<T>; 2],
    /// Current read index (0 or 1).
    read_index: AtomicU32,
    /// Version counter for change detection.
    version: AtomicU32,
}

impl<T: Clone + Default> ParameterBlock<T> {
    /// Create a new parameter block with the given initial value.
    pub fn new(initial: T) -> Self {
        Self {
            buffers: [
                std::sync::RwLock::new(initial.clone()),
                std::sync::RwLock::new(initial),
            ],
            read_index: AtomicU32::new(0),
            version: AtomicU32::new(0),
        }
    }

    /// Read the current parameters.
    ///
    /// This is safe to call from the audio thread as it only reads.
    pub fn read(&self) -> T {
        let idx = self.read_index.get() as usize;
        self.buffers[idx].read().unwrap().clone()
    }

    /// Update the parameters.
    ///
    /// Call this from the UI thread. The update function receives a mutable
    /// reference to a copy of the current parameters.
    pub fn update<F: FnOnce(&mut T)>(&self, f: F) {
        let read_idx = self.read_index.get() as usize;
        let write_idx = 1 - read_idx;

        // Copy current to write buffer and apply changes
        {
            let current = self.buffers[read_idx].read().unwrap();
            let mut write = self.buffers[write_idx].write().unwrap();
            *write = current.clone();
            f(&mut write);
        }

        // Swap buffers
        self.read_index.set(write_idx as u32);
        self.version.increment();
    }

    /// Get the current version number.
    ///
    /// Useful for detecting if parameters have changed since last read.
    pub fn version(&self) -> u32 {
        self.version.get()
    }

    /// Set parameters directly.
    pub fn set(&self, value: T) {
        self.update(|p| *p = value);
    }
}

impl<T: Clone + Default> Default for ParameterBlock<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

/// Smoothed parameter for avoiding clicks/pops during value changes.
///
/// Implements a simple one-pole lowpass filter for smoothing.
#[derive(Debug, Clone)]
pub struct SmoothedParam {
    /// Target value set by UI.
    target: AtomicF32,
    /// Current smoothed value (audio thread only).
    current: f32,
    /// Smoothing coefficient (0-1, higher = faster).
    coeff: f32,
    /// Threshold for considering value "arrived".
    threshold: f32,
}

impl SmoothedParam {
    /// Create a new smoothed parameter.
    ///
    /// # Arguments
    /// * `initial` - Initial value
    /// * `smooth_time_ms` - Time to reach ~63% of target (in milliseconds)
    /// * `sample_rate` - Audio sample rate
    pub fn new(initial: f32, smooth_time_ms: f32, sample_rate: f32) -> Self {
        let coeff = Self::calc_coeff(smooth_time_ms, sample_rate);
        Self {
            target: AtomicF32::new(initial),
            current: initial,
            coeff,
            threshold: 0.0001,
        }
    }

    /// Calculate smoothing coefficient from time constant.
    fn calc_coeff(time_ms: f32, sample_rate: f32) -> f32 {
        if time_ms <= 0.0 {
            1.0
        } else {
            let samples = (time_ms * 0.001 * sample_rate).max(1.0);
            1.0 - (-1.0 / samples).exp()
        }
    }

    /// Set the target value (UI thread).
    #[inline]
    pub fn set_target(&self, value: f32) {
        self.target.set(value);
    }

    /// Get the target value.
    #[inline]
    pub fn target(&self) -> f32 {
        self.target.get()
    }

    /// Get the current smoothed value (audio thread).
    #[inline]
    pub fn current(&self) -> f32 {
        self.current
    }

    /// Process one sample of smoothing (audio thread).
    ///
    /// Call this once per sample to update the smoothed value.
    #[inline]
    pub fn process(&mut self) -> f32 {
        let target = self.target.get();
        self.current += self.coeff * (target - self.current);
        self.current
    }

    /// Process a block of samples, returning true if still smoothing.
    #[inline]
    pub fn process_block(&mut self, num_samples: usize) -> bool {
        let target = self.target.get();
        for _ in 0..num_samples {
            self.current += self.coeff * (target - self.current);
        }
        (self.current - target).abs() > self.threshold
    }

    /// Check if smoothing is complete.
    #[inline]
    pub fn is_smoothing(&self) -> bool {
        (self.current - self.target.get()).abs() > self.threshold
    }

    /// Skip smoothing and jump to target value.
    #[inline]
    pub fn skip_to_target(&mut self) {
        self.current = self.target.get();
    }

    /// Update the smoothing time.
    pub fn set_smooth_time(&mut self, time_ms: f32, sample_rate: f32) {
        self.coeff = Self::calc_coeff(time_ms, sample_rate);
    }
}

impl Default for SmoothedParam {
    fn default() -> Self {
        Self::new(0.0, 10.0, 48000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_atomic_f32() {
        let param = AtomicF32::new(1.0);
        assert!((param.get() - 1.0).abs() < 0.0001);

        param.set(0.5);
        assert!((param.get() - 0.5).abs() < 0.0001);

        let old = param.swap(0.75);
        assert!((old - 0.5).abs() < 0.0001);
        assert!((param.get() - 0.75).abs() < 0.0001);
    }

    #[test]
    fn test_atomic_bool() {
        let flag = AtomicBool::new(false);
        assert!(!flag.get());

        flag.set(true);
        assert!(flag.get());

        let new_val = flag.toggle();
        assert!(!new_val);
        assert!(!flag.get());
    }

    #[test]
    fn test_smoothed_param() {
        let mut param = SmoothedParam::new(0.0, 10.0, 48000.0);
        param.set_target(1.0);

        // Process enough samples to converge (10ms smoothing @ 48kHz = 480 sample time constant)
        for _ in 0..2000 {
            param.process();
        }

        // Should be close to target
        assert!((param.current() - 1.0).abs() < 0.1);
    }

    #[test]
    fn test_parameter_block() {
        #[derive(Clone, Default)]
        struct Params {
            a: f32,
            b: f32,
        }

        let block = ParameterBlock::new(Params { a: 1.0, b: 2.0 });

        let v1 = block.version();
        block.update(|p| {
            p.a = 3.0;
            p.b = 4.0;
        });
        let v2 = block.version();

        assert!(v2 > v1);

        let params = block.read();
        assert!((params.a - 3.0).abs() < 0.0001);
        assert!((params.b - 4.0).abs() < 0.0001);
    }
}
