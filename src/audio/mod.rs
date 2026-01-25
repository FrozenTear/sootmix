// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Audio subsystem - PipeWire integration.

pub mod control;
pub mod filter_chain;
pub mod meter;
pub mod pipewire_thread;
pub mod plugin_filter;
pub mod plugin_processor;
pub mod routing;
pub mod types;
pub mod virtual_sink;
pub mod volume;

pub use meter::MeterManager;
pub use pipewire_thread::{PwCommand, PwEvent, PwThread};
pub use plugin_filter::{PluginFilterManager, PluginProcessingContext};
pub use plugin_processor::PluginProcessorManager;
pub use types::{AudioChannel, MediaClass, PortDirection, PwLink, PwNode, PwPort};
