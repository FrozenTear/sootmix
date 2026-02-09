// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Real-time safe utilities for audio processing.
//!
//! This module provides lock-free data structures and patterns for
//! communicating between the audio thread and other threads.
//!
//! # Real-Time Safety
//!
//! The audio thread has strict requirements:
//! - No memory allocation
//! - No locks (mutexes, RwLocks)
//! - No system calls (file I/O, network)
//! - Bounded execution time
//!
//! All utilities in this module are designed to be called from the audio thread.

pub mod atomic_params;
pub mod ringbuf;

pub use ringbuf::{PluginParamUpdate, RingBuffer, RingBufferReader, RingBufferWriter};
