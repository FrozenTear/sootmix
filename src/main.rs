// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! SootMix - Audio routing and mixing for Linux.
//!
//! A PipeWire-based audio mixer inspired by SteelSeries Sonar and VoiceMeeter.

mod app;
mod audio;
mod config;
mod message;
mod plugins;
mod realtime;
mod state;
mod ui;

use app::SootMix;
use tracing::info;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

fn main() -> iced::Result {
    // Initialize logging
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env().add_directive("sootmix=debug".parse().unwrap()))
        .init();

    info!("Starting SootMix");

    // Run the Iced application
    iced::application("SootMix", SootMix::update, SootMix::view)
        .subscription(SootMix::subscription)
        .theme(SootMix::theme)
        .window_size((900.0, 600.0))
        .run_with(SootMix::new)
}
