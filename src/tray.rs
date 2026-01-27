// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! System tray integration using ksni (StatusNotifierItem).
//!
//! Provides a tray icon that allows the app to run in the background
//! while still being accessible for quick actions.

use ksni::{menu::StandardItem, Handle, MenuItem, Tray, TrayMethods};
use std::sync::mpsc;
use tracing::{debug, error, info};

/// Messages sent from the tray to the main application.
#[derive(Debug, Clone)]
pub enum TrayMessage {
    /// Show the main window.
    ShowWindow,
    /// Toggle mute all channels.
    ToggleMuteAll,
    /// Quit the application.
    Quit,
}

/// State shared with the tray icon.
struct SootMixTray {
    /// Channel to send messages to the main app.
    tx: mpsc::Sender<TrayMessage>,
    /// Whether all channels are currently muted.
    muted: bool,
}

impl Tray for SootMixTray {
    fn id(&self) -> String {
        "sootmix".to_string()
    }

    fn title(&self) -> String {
        "SootMix".to_string()
    }

    fn icon_name(&self) -> String {
        // Use our installed icon from the hicolor theme
        "sootmix".to_string()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "SootMix".to_string(),
            description: "Audio routing and mixing".to_string(),
            icon_name: String::new(),
            icon_pixmap: vec![],
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            MenuItem::Standard(StandardItem {
                label: "Show SootMix".to_string(),
                activate: Box::new(|tray: &mut Self| {
                    debug!("Tray: Show clicked");
                    let _ = tray.tx.send(TrayMessage::ShowWindow);
                }),
                ..Default::default()
            }),
            MenuItem::Separator,
            MenuItem::Standard(StandardItem {
                label: if self.muted {
                    "Unmute All".to_string()
                } else {
                    "Mute All".to_string()
                },
                activate: Box::new(|tray: &mut Self| {
                    debug!("Tray: Mute toggle clicked");
                    let _ = tray.tx.send(TrayMessage::ToggleMuteAll);
                }),
                ..Default::default()
            }),
            MenuItem::Separator,
            MenuItem::Standard(StandardItem {
                label: "Quit".to_string(),
                activate: Box::new(|tray: &mut Self| {
                    debug!("Tray: Quit clicked");
                    let _ = tray.tx.send(TrayMessage::Quit);
                }),
                ..Default::default()
            }),
        ]
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        // Left-click on tray icon shows the window
        debug!("Tray: Activated (left-click)");
        let _ = self.tx.send(TrayMessage::ShowWindow);
    }
}

/// Handle to the running tray service.
pub struct TrayHandle {
    /// Handle to update the tray state.
    handle: Handle<SootMixTray>,
}

impl TrayHandle {
    /// Update the muted state shown in the tray menu.
    pub fn set_muted(&self, muted: bool) {
        let handle = self.handle.clone();
        tokio::spawn(async move {
            handle.update(move |tray| {
                tray.muted = muted;
            }).await;
        });
    }

    /// Shut down the tray icon, removing it from the system tray.
    pub fn shutdown(&self) {
        info!("Shutting down system tray");
        self.handle.shutdown();
    }
}

/// Start the system tray icon.
///
/// Returns a receiver for tray messages and a handle to update tray state.
/// The tray runs in a background tokio task.
pub fn start_tray() -> Option<(mpsc::Receiver<TrayMessage>, TrayHandle)> {
    let (tx, rx) = mpsc::channel();

    let tray = SootMixTray { tx, muted: false };

    // Spawn the tray in a background task
    // We use a oneshot channel to get the handle back
    let (handle_tx, handle_rx) = std::sync::mpsc::channel();

    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to create tokio runtime for tray");

        rt.block_on(async {
            match tray.spawn().await {
                Ok(handle) => {
                    info!("System tray started");
                    let _ = handle_tx.send(Some(handle));
                    // Keep the runtime alive
                    std::future::pending::<()>().await;
                }
                Err(e) => {
                    error!("Failed to start system tray: {}", e);
                    let _ = handle_tx.send(None);
                }
            }
        });
    });

    // Wait for the handle (with timeout)
    match handle_rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(Some(handle)) => Some((rx, TrayHandle { handle })),
        Ok(None) => None,
        Err(_) => {
            error!("Timeout waiting for tray to start");
            None
        }
    }
}
