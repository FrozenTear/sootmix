// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! SootMix Daemon - Background audio routing service.
//!
//! This daemon manages PipeWire virtual sinks, audio routing, and volume control.
//! It exposes a D-Bus interface that the UI client connects to.

mod audio;
mod config;
mod dbus;
mod service;

use dbus::DaemonDbusService;
use sootmix_ipc::{DBUS_NAME, DBUS_PATH};
use std::sync::{Arc, Mutex};
use tracing::{error, info};
use zbus::connection::Builder;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("sootmix_daemon=debug".parse().unwrap())
                .add_directive("zbus=warn".parse().unwrap()),
        )
        .init();

    info!("SootMix Daemon starting...");

    // Load configuration
    let config_manager = config::ConfigManager::new()?;
    let mixer_config = config_manager.load_mixer_config().unwrap_or_default();
    let routing_rules = config_manager.load_routing_rules().unwrap_or_default();

    info!(
        "Loaded config: {} channels, {} routing rules",
        mixer_config.channels.len(),
        routing_rules.rules.len()
    );

    // Create the daemon service
    let mut daemon_service = service::DaemonService::new(
        mixer_config,
        routing_rules,
        config_manager,
    );

    // Start PipeWire thread
    if let Err(e) = daemon_service.start_pipewire() {
        error!("Failed to start PipeWire: {}", e);
        return Err(e.into());
    }

    // Wait for startup discovery to complete
    daemon_service.wait_for_discovery();

    // Restore channels from config
    if let Err(e) = daemon_service.restore_channels() {
        error!("Failed to restore channels: {}", e);
    }

    // Wrap in Arc<Mutex> for D-Bus access
    let service = Arc::new(Mutex::new(daemon_service));

    // Create D-Bus interface
    let dbus_service = DaemonDbusService::new(service.clone());

    // Build D-Bus connection
    let _connection = Builder::session()?
        .name(DBUS_NAME)?
        .serve_at(DBUS_PATH, dbus_service)?
        .build()
        .await?;

    info!("D-Bus service registered at {}", DBUS_NAME);
    info!("SootMix Daemon ready");

    // Spawn event processing task
    let service_events = service.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            if let Ok(mut svc) = service_events.lock() {
                svc.process_pw_events();
            }
        }
    });

    // Handle shutdown signals
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;

    tokio::select! {
        _ = sigterm.recv() => {
            info!("Received SIGTERM, shutting down...");
        }
        _ = sigint.recv() => {
            info!("Received SIGINT, shutting down...");
        }
    }

    // Cleanup
    if let Ok(mut svc) = service.lock() {
        svc.shutdown();
    }

    info!("SootMix Daemon stopped");
    Ok(())
}
