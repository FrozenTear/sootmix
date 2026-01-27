// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! SootMix - Audio routing and mixing for Linux.
//!
//! A PipeWire-based audio mixer inspired by SteelSeries Sonar and VoiceMeeter.

mod app;
mod audio;
mod config;
mod daemon_client;
mod message;
mod plugins;
mod realtime;
mod single_instance;
mod state;
mod tray;
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

    // Single-instance check: if another UI is already running, activate it and exit
    if single_instance::try_activate_existing() {
        info!("Another SootMix UI instance is already running — activated it");
        return Ok(());
    }

    // Run as daemon so closing windows doesn't exit the app (for tray support)
    iced::daemon(SootMix::new, SootMix::update, SootMix::view)
        .title("SootMix")
        .subscription(SootMix::subscription)
        .theme(crate::ui::theme::sootmix_theme())
        .run()
}
