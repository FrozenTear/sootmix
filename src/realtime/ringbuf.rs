// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Lock-free single-producer single-consumer ring buffer.
//!
//! Used for passing data from the audio thread to the UI thread (e.g., meter data)
//! without blocking.
//!
//! # Example
//!
//! ```ignore
//! use sootmix::realtime::RingBuffer;
//!
//! // Create a ring buffer for meter data
//! let (mut writer, mut reader) = RingBuffer::<f32>::new(1024).split();
//!
//! // Audio thread writes peak values
//! writer.push(0.8);
//! writer.push(0.75);
//!
//! // UI thread reads them
//! while let Some(peak) = reader.pop() {
//!     update_meter(peak);
//! }
//! ```

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// A lock-free single-producer single-consumer ring buffer.
///
/// The buffer has a fixed capacity and will drop old data when full
/// (writer never blocks).
pub struct RingBuffer<T> {
    /// The actual buffer storage.
    buffer: Box<[UnsafeCell<Option<T>>]>,
    /// Write position (only modified by writer).
    write_pos: AtomicUsize,
    /// Read position (only modified by reader).
    read_pos: AtomicUsize,
    /// Capacity (power of 2 for efficient modulo).
    capacity: usize,
    /// Mask for efficient modulo operation.
    mask: usize,
}

// SAFETY: The ring buffer is designed for SPSC access.
// Only the writer modifies write_pos and writes to the buffer.
// Only the reader modifies read_pos and reads from the buffer.
unsafe impl<T: Send> Send for RingBuffer<T> {}
unsafe impl<T: Send> Sync for RingBuffer<T> {}

impl<T> RingBuffer<T> {
    /// Create a new ring buffer with the given capacity.
    ///
    /// The actual capacity will be rounded up to the next power of 2.
    pub fn new(capacity: usize) -> Self {
        // Round up to power of 2 for efficient modulo
        let capacity = capacity.next_power_of_two();
        let mask = capacity - 1;

        // Initialize buffer with None values
        let mut buffer = Vec::with_capacity(capacity);
        for _ in 0..capacity {
            buffer.push(UnsafeCell::new(None));
        }

        Self {
            buffer: buffer.into_boxed_slice(),
            write_pos: AtomicUsize::new(0),
            read_pos: AtomicUsize::new(0),
            capacity,
            mask,
        }
    }

    /// Get the capacity of the buffer.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Split into writer and reader handles.
    pub fn split(self) -> (RingBufferWriter<T>, RingBufferReader<T>) {
        let shared = Arc::new(self);
        (
            RingBufferWriter {
                inner: Arc::clone(&shared),
            },
            RingBufferReader { inner: shared },
        )
    }

    /// Get the number of items available to read.
    fn available(&self) -> usize {
        let write = self.write_pos.load(Ordering::Acquire);
        let read = self.read_pos.load(Ordering::Acquire);
        write.wrapping_sub(read)
    }

    /// Check if the buffer is empty.
    fn is_empty(&self) -> bool {
        self.available() == 0
    }

    /// Check if the buffer is full.
    fn is_full(&self) -> bool {
        self.available() >= self.capacity
    }
}

/// Writer handle for the ring buffer.
///
/// Only one writer should exist per buffer.
pub struct RingBufferWriter<T> {
    inner: Arc<RingBuffer<T>>,
}

impl<T> RingBufferWriter<T> {
    /// Push an item to the buffer.
    ///
    /// If the buffer is full, the oldest item is overwritten.
    /// Returns true if successful, false if item was dropped due to overflow.
    pub fn push(&mut self, item: T) -> bool {
        let write_pos = self.inner.write_pos.load(Ordering::Relaxed);
        let read_pos = self.inner.read_pos.load(Ordering::Acquire);

        // Check if buffer is full
        let is_full = write_pos.wrapping_sub(read_pos) >= self.inner.capacity;

        // Write the item
        let idx = write_pos & self.inner.mask;
        // SAFETY: We're the only writer, and we're writing to our current position
        unsafe {
            *self.inner.buffer[idx].get() = Some(item);
        }

        // Advance write position
        self.inner
            .write_pos
            .store(write_pos.wrapping_add(1), Ordering::Release);

        !is_full
    }

    /// Push multiple items to the buffer.
    ///
    /// Returns the number of items successfully written without overflow.
    pub fn push_slice(&mut self, items: &[T]) -> usize
    where
        T: Clone,
    {
        let mut successful = 0;
        for item in items {
            if self.push(item.clone()) {
                successful += 1;
            }
        }
        successful
    }

    /// Get the number of items available to read.
    pub fn available(&self) -> usize {
        self.inner.available()
    }

    /// Check if the buffer is full.
    pub fn is_full(&self) -> bool {
        self.inner.is_full()
    }
}

// Writer can be sent to another thread
unsafe impl<T: Send> Send for RingBufferWriter<T> {}

/// Reader handle for the ring buffer.
///
/// Only one reader should exist per buffer.
pub struct RingBufferReader<T> {
    inner: Arc<RingBuffer<T>>,
}

impl<T> RingBufferReader<T> {
    /// Pop an item from the buffer.
    ///
    /// Returns None if the buffer is empty.
    pub fn pop(&mut self) -> Option<T> {
        let read_pos = self.inner.read_pos.load(Ordering::Relaxed);
        let write_pos = self.inner.write_pos.load(Ordering::Acquire);

        // Check if empty
        if read_pos == write_pos {
            return None;
        }

        // Read the item
        let idx = read_pos & self.inner.mask;
        // SAFETY: We're the only reader, and writer has finished writing this position
        let item = unsafe { (*self.inner.buffer[idx].get()).take() };

        // Advance read position
        self.inner
            .read_pos
            .store(read_pos.wrapping_add(1), Ordering::Release);

        item
    }

    /// Pop all available items into a vector.
    pub fn pop_all(&mut self) -> Vec<T> {
        let mut items = Vec::new();
        while let Some(item) = self.pop() {
            items.push(item);
        }
        items
    }

    /// Peek at the next item without removing it.
    pub fn peek(&self) -> Option<&T> {
        let read_pos = self.inner.read_pos.load(Ordering::Relaxed);
        let write_pos = self.inner.write_pos.load(Ordering::Acquire);

        if read_pos == write_pos {
            return None;
        }

        let idx = read_pos & self.inner.mask;
        // SAFETY: We're the only reader
        unsafe { (*self.inner.buffer[idx].get()).as_ref() }
    }

    /// Get the number of items available to read.
    pub fn available(&self) -> usize {
        self.inner.available()
    }

    /// Check if the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Skip n items (discard without processing).
    pub fn skip(&mut self, n: usize) {
        for _ in 0..n {
            if self.pop().is_none() {
                break;
            }
        }
    }

    /// Clear all items from the buffer.
    pub fn clear(&mut self) {
        while self.pop().is_some() {}
    }
}

// Reader can be sent to another thread
unsafe impl<T: Send> Send for RingBufferReader<T> {}

/// Meter data sent from audio thread to UI.
#[derive(Debug, Clone, Copy, Default)]
pub struct MeterData {
    /// Peak level (0.0 to 1.0+).
    pub peak: f32,
    /// RMS level (0.0 to 1.0+).
    pub rms: f32,
    /// Whether clipping occurred.
    pub clipping: bool,
}

impl MeterData {
    /// Create new meter data.
    pub fn new(peak: f32, rms: f32) -> Self {
        Self {
            peak,
            rms,
            clipping: peak >= 1.0,
        }
    }

    /// Convert to decibels.
    pub fn peak_db(&self) -> f32 {
        if self.peak <= 0.0 {
            -80.0
        } else {
            20.0 * self.peak.log10()
        }
    }

    /// Convert RMS to decibels.
    pub fn rms_db(&self) -> f32 {
        if self.rms <= 0.0 {
            -80.0
        } else {
            20.0 * self.rms.log10()
        }
    }
}

/// Stereo meter data.
#[derive(Debug, Clone, Copy, Default)]
pub struct StereoMeterData {
    /// Left channel.
    pub left: MeterData,
    /// Right channel.
    pub right: MeterData,
}

/// Plugin parameter update sent from UI thread to RT audio thread.
///
/// Used to update plugin parameters without locking in the audio callback.
#[derive(Debug, Clone, Copy)]
pub struct PluginParamUpdate {
    /// Plugin instance ID.
    pub instance_id: uuid::Uuid,
    /// Parameter index.
    pub param_index: u32,
    /// New parameter value.
    pub value: f32,
}

impl PluginParamUpdate {
    /// Create a new parameter update.
    pub fn new(instance_id: uuid::Uuid, param_index: u32, value: f32) -> Self {
        Self {
            instance_id,
            param_index,
            value,
        }
    }
}

impl StereoMeterData {
    /// Create new stereo meter data.
    pub fn new(left: MeterData, right: MeterData) -> Self {
        Self { left, right }
    }

    /// Check if either channel is clipping.
    pub fn is_clipping(&self) -> bool {
        self.left.clipping || self.right.clipping
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_push_pop() {
        let (mut writer, mut reader) = RingBuffer::<i32>::new(4).split();

        assert!(reader.is_empty());

        writer.push(1);
        writer.push(2);
        writer.push(3);

        assert_eq!(reader.available(), 3);
        assert_eq!(reader.pop(), Some(1));
        assert_eq!(reader.pop(), Some(2));
        assert_eq!(reader.pop(), Some(3));
        assert_eq!(reader.pop(), None);
    }

    #[test]
    fn test_overflow() {
        let (mut writer, mut reader) = RingBuffer::<i32>::new(2).split();

        // Fill buffer (capacity is rounded to 2)
        writer.push(1);
        writer.push(2);

        // Overflow - oldest gets overwritten
        writer.push(3);

        // Should have 2 and 3 (1 was overwritten)
        let items = reader.pop_all();
        assert!(items.contains(&3));
    }

    #[test]
    fn test_peek() {
        let (mut writer, reader) = RingBuffer::<i32>::new(4).split();

        writer.push(42);

        assert_eq!(reader.peek(), Some(&42));
        assert_eq!(reader.peek(), Some(&42)); // Still there
        assert_eq!(reader.available(), 1);
    }

    #[test]
    fn test_meter_data() {
        let meter = MeterData::new(0.5, 0.3);
        assert!(!meter.clipping);
        assert!((meter.peak_db() - (-6.02)).abs() < 0.1);

        let clipping = MeterData::new(1.5, 0.8);
        assert!(clipping.clipping);
    }
}
