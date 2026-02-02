// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Single-instance mechanism using D-Bus.
//!
//! Prevents multiple UI instances from running simultaneously.
//! When a second instance is launched, it signals the existing one
//! to show its window and then exits.

use std::sync::mpsc;
use tracing::{debug, error, info, warn};
use zbus::blocking;

/// D-Bus well-known name for the SootMix UI.
const UI_DBUS_NAME: &str = "com.sootmix.UI";
/// D-Bus object path for the UI activation interface.
const UI_DBUS_PATH: &str = "/com/sootmix/UI";

/// Try to activate an already-running SootMix UI instance.
///
/// Returns `true` if an existing instance was found and activated
/// (the caller should exit). Returns `false` if no existing instance
/// was found (the caller should proceed with startup).
///
/// This uses D-Bus name ownership as a lock - we try to acquire the name
/// with DO_NOT_QUEUE, and if it fails, another instance has it.
pub fn try_activate_existing() -> bool {
    let conn = match blocking::Connection::session() {
        Ok(c) => c,
        Err(e) => {
            warn!("Could not connect to session D-Bus: {}", e);
            return false;
        }
    };

    // Try to request the D-Bus name with DO_NOT_QUEUE flag.
    // If another instance has it, this will fail immediately.
    // We use the blocking API to check ownership.
    let reply = conn.call_method(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        Some("org.freedesktop.DBus"),
        "RequestName",
        &(UI_DBUS_NAME, 4u32), // 4 = DBUS_NAME_FLAG_DO_NOT_QUEUE
    );

    match reply {
        Ok(msg) => {
            // RequestName returns: 1=PRIMARY_OWNER, 2=IN_QUEUE, 3=EXISTS, 4=ALREADY_OWNER
            let result: u32 = msg.body().deserialize().unwrap_or(0);
            if result == 1 || result == 4 {
                // We got the name - no other instance running
                // Release it so start_activation_listener can properly acquire it
                let _ = conn.call_method(
                    Some("org.freedesktop.DBus"),
                    "/org/freedesktop/DBus",
                    Some("org.freedesktop.DBus"),
                    "ReleaseName",
                    &(UI_DBUS_NAME,),
                );
                debug!("No existing instance found, proceeding with startup");
                false
            } else {
                // Another instance owns the name - try to activate it
                info!("Another instance owns D-Bus name, attempting activation");
                let _ = conn.call_method(
                    Some(UI_DBUS_NAME),
                    UI_DBUS_PATH,
                    Some("com.sootmix.UI"),
                    "Activate",
                    &(),
                );
                true
            }
        }
        Err(e) => {
            // D-Bus error - try the old method as fallback
            debug!("RequestName failed ({}), trying direct activation", e);
            match conn.call_method(
                Some(UI_DBUS_NAME),
                UI_DBUS_PATH,
                Some("com.sootmix.UI"),
                "Activate",
                &(),
            ) {
                Ok(_) => {
                    info!("Activated existing SootMix instance");
                    true
                }
                Err(_) => {
                    debug!("No existing instance found, proceeding with startup");
                    false
                }
            }
        }
    }
}

/// D-Bus interface served by the running UI instance.
struct UiActivation {
    tx: mpsc::Sender<()>,
}

#[zbus::interface(name = "com.sootmix.UI")]
impl UiActivation {
    /// Called by new instances to activate (show) the existing window.
    fn activate(&self) {
        info!("Received activation request from another instance");
        let _ = self.tx.send(());
    }
}

/// Start the D-Bus activation listener in a background thread.
///
/// Returns a receiver that emits `()` each time another instance
/// requests activation (i.e., wants us to show our window).
pub fn start_activation_listener() -> Option<mpsc::Receiver<()>> {
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                error!("Failed to create runtime for activation listener: {}", e);
                return;
            }
        };

        rt.block_on(async {
            let activation = UiActivation { tx };

            let builder = match zbus::connection::Builder::session() {
                Ok(b) => b,
                Err(e) => {
                    warn!("Failed to create D-Bus session builder: {}", e);
                    return;
                }
            };

            let builder = match builder.name(UI_DBUS_NAME) {
                Ok(b) => b,
                Err(e) => {
                    warn!("Failed to request D-Bus name {}: {}", UI_DBUS_NAME, e);
                    return;
                }
            };

            let builder = match builder.serve_at(UI_DBUS_PATH, activation) {
                Ok(b) => b,
                Err(e) => {
                    warn!("Failed to serve at {}: {}", UI_DBUS_PATH, e);
                    return;
                }
            };

            match builder.build().await {
                Ok(_conn) => {
                    info!(
                        "UI activation listener registered on D-Bus as {}",
                        UI_DBUS_NAME
                    );
                    // Keep the connection alive indefinitely
                    std::future::pending::<()>().await;
                }
                Err(e) => {
                    warn!("Failed to register UI on D-Bus: {}", e);
                }
            }
        });
    });

    Some(rx)
}
