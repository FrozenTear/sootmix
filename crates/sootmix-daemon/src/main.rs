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
use service::SignalEvent;
use sootmix_ipc::{DBUS_NAME, DBUS_PATH};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc as tokio_mpsc;
use tracing::{debug, error, info, warn};
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

    // Create signal channel for D-Bus signal events
    let (signal_tx, signal_rx) = tokio_mpsc::unbounded_channel::<SignalEvent>();

    // Create the daemon service
    let mut daemon_service =
        service::DaemonService::new(mixer_config, routing_rules, config_manager);
    daemon_service.set_signal_sender(signal_tx);

    // Start PipeWire thread
    if let Err(e) = daemon_service.start_pipewire() {
        error!("Failed to start PipeWire: {}", e);
        return Err(e.into());
    }

    // Wait for startup discovery to complete
    daemon_service.wait_for_discovery();

    // Clean up orphaned sootmix nodes from previous runs before restoring channels
    crate::audio::virtual_sink::cleanup_orphaned_nodes();

    // Restore channels from config
    if let Err(e) = daemon_service.restore_channels() {
        error!("Failed to restore channels: {}", e);
    }

    // Wrap in Arc<Mutex> for D-Bus access
    let service = Arc::new(Mutex::new(daemon_service));

    // Create D-Bus interface
    let dbus_service = DaemonDbusService::new(service.clone());

    // Build D-Bus connection
    let connection = Builder::session()?
        .name(DBUS_NAME)?
        .serve_at(DBUS_PATH, dbus_service)?
        .build()
        .await?;

    info!("D-Bus service registered at {}", DBUS_NAME);
    info!("SootMix Daemon ready");

    // Shutdown flag for graceful termination
    let shutdown_flag = Arc::new(AtomicBool::new(false));

    // Spawn event processing task
    let service_events = service.clone();
    let shutdown_flag_events = shutdown_flag.clone();
    let event_task = tokio::spawn(async move {
        while !shutdown_flag_events.load(Ordering::Relaxed) {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            if let Ok(mut svc) = service_events.lock() {
                svc.process_pw_events();
            }
        }
    });

    // Spawn task to emit D-Bus signals from the signal channel
    let shutdown_flag_signals = shutdown_flag.clone();
    let signal_task = tokio::spawn(async move {
        let mut signal_rx = signal_rx;
        loop {
            tokio::select! {
                Some(event) = signal_rx.recv() => {
                    let object_server = connection.object_server();
                    let iface_ref = match object_server.interface::<_, DaemonDbusService>(DBUS_PATH).await {
                        Ok(iface) => iface,
                        Err(e) => {
                            warn!("Failed to get D-Bus interface for signal: {}", e);
                            continue;
                        }
                    };
                    let ctx = iface_ref.signal_context();
                    match event {
                        SignalEvent::AppDiscovered(app) => {
                            debug!("Emitting D-Bus AppDiscovered signal: {}", app.name);
                            if let Err(e) = dbus::emit_app_discovered(ctx, app).await {
                                warn!("Failed to emit AppDiscovered signal: {}", e);
                            }
                        }
                        SignalEvent::AppRemoved(app_id) => {
                            debug!("Emitting D-Bus AppRemoved signal: {}", app_id);
                            if let Err(e) = dbus::emit_app_removed(ctx, &app_id).await {
                                warn!("Failed to emit AppRemoved signal: {}", e);
                            }
                        }
                        SignalEvent::OutputsChanged => {
                            debug!("Emitting D-Bus OutputsChanged signal");
                            if let Err(e) = dbus::emit_outputs_changed(ctx).await {
                                warn!("Failed to emit OutputsChanged signal: {}", e);
                            }
                        }
                    }
                }
                _ = tokio::time::sleep(tokio::time::Duration::from_millis(100)) => {
                    if shutdown_flag_signals.load(Ordering::Relaxed) {
                        break;
                    }
                }
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

    // Signal the tasks to stop
    shutdown_flag.store(true, Ordering::Relaxed);

    // Wait for tasks to finish (with timeout)
    let _ = tokio::time::timeout(tokio::time::Duration::from_secs(2), event_task).await;
    let _ = tokio::time::timeout(tokio::time::Duration::from_secs(1), signal_task).await;

    // Cleanup
    if let Ok(mut svc) = service.lock() {
        svc.shutdown();
    }

    info!("SootMix Daemon stopped");
    Ok(())
}
