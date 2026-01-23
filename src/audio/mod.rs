// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Audio subsystem - PipeWire integration.

pub mod control;
pub mod pipewire_thread;
pub mod routing;
pub mod types;
pub mod virtual_sink;
pub mod volume;

pub use control::{
    build_channel_volumes_pod, build_eq_band_pod, build_filter_control_pod, build_mute_pod,
    build_volume_mute_pod, build_volume_pod, db_to_linear, linear_to_db, ControlError,
};
pub use pipewire_thread::{PwCommand, PwEvent, PwThread};
pub use types::*;
