// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! D-Bus client for communicating with the SootMix daemon.

#![allow(dead_code, unused_imports)]

use sootmix_ipc::{AppInfo, ChannelInfo, InputInfo, MeterData, OutputInfo, RoutingRuleInfo};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};
use zbus::{proxy, Connection, Result as ZbusResult};

/// D-Bus proxy for the daemon interface.
#[proxy(
    interface = "com.sootmix.Daemon",
    default_service = "com.sootmix.Daemon",
    default_path = "/com/sootmix/Daemon"
)]
trait Daemon {
    // Methods
    fn create_channel(&self, name: &str) -> ZbusResult<String>;
    fn create_input_channel(&self, name: &str) -> ZbusResult<String>;
    fn delete_channel(&self, channel_id: &str) -> ZbusResult<()>;
    fn rename_channel(&self, channel_id: &str, name: &str) -> ZbusResult<()>;
    fn move_channel(&self, channel_id: &str, direction: i32) -> ZbusResult<()>;
    fn set_channel_volume(&self, channel_id: &str, volume_db: f64) -> ZbusResult<()>;
    fn set_channel_mute(&self, channel_id: &str, muted: bool) -> ZbusResult<()>;
    fn set_channel_noise_suppression(&self, channel_id: &str, enabled: bool) -> ZbusResult<()>;
    fn set_channel_vad_threshold(&self, channel_id: &str, threshold: f64) -> ZbusResult<()>;
    fn set_channel_input_gain(&self, channel_id: &str, gain_db: f64) -> ZbusResult<()>;
    fn set_master_volume(&self, volume_db: f64) -> ZbusResult<()>;
    fn set_master_mute(&self, muted: bool) -> ZbusResult<()>;
    fn assign_app(&self, app_id: &str, channel_id: &str) -> ZbusResult<()>;
    fn unassign_app(&self, app_id: &str, channel_id: &str) -> ZbusResult<()>;
    fn set_channel_output(&self, channel_id: &str, device_name: &str) -> ZbusResult<()>;
    fn set_master_output(&self, device_name: &str) -> ZbusResult<()>;
    fn set_master_recording(&self, enabled: bool) -> ZbusResult<()>;
    fn get_channels(&self) -> ZbusResult<Vec<ChannelInfo>>;
    fn get_apps(&self) -> ZbusResult<Vec<AppInfo>>;
    fn get_outputs(&self) -> ZbusResult<Vec<OutputInfo>>;
    fn get_inputs(&self) -> ZbusResult<Vec<InputInfo>>;
    fn get_master_volume(&self) -> ZbusResult<f64>;
    fn get_master_muted(&self) -> ZbusResult<bool>;
    fn get_master_output(&self) -> ZbusResult<String>;
    fn get_connected(&self) -> ZbusResult<bool>;
    fn get_master_recording_enabled(&self) -> ZbusResult<bool>;
    fn get_routing_rules(&self) -> ZbusResult<Vec<RoutingRuleInfo>>;
    fn set_routing_rule(&self, rule: RoutingRuleInfo) -> ZbusResult<()>;
    fn delete_routing_rule(&self, rule_id: &str) -> ZbusResult<()>;
    fn toggle_routing_rule(&self, rule_id: &str) -> ZbusResult<()>;

    // Signals
    #[zbus(signal)]
    fn channel_added(&self, channel: ChannelInfo) -> ZbusResult<()>;
    #[zbus(signal)]
    fn channel_removed(&self, channel_id: &str) -> ZbusResult<()>;
    #[zbus(signal)]
    fn channel_updated(&self, channel: ChannelInfo) -> ZbusResult<()>;
    #[zbus(signal)]
    fn volume_changed(&self, channel_id: &str, volume_db: f64) -> ZbusResult<()>;
    #[zbus(signal)]
    fn mute_changed(&self, channel_id: &str, muted: bool) -> ZbusResult<()>;
    #[zbus(signal)]
    fn app_discovered(&self, app: AppInfo) -> ZbusResult<()>;
    #[zbus(signal)]
    fn app_removed(&self, app_id: &str) -> ZbusResult<()>;
    #[zbus(signal)]
    fn app_routed(&self, app_id: &str, channel_id: &str) -> ZbusResult<()>;
    #[zbus(signal)]
    fn app_unrouted(&self, app_id: &str, channel_id: &str) -> ZbusResult<()>;
    #[zbus(signal)]
    fn connection_changed(&self, connected: bool) -> ZbusResult<()>;
    #[zbus(signal)]
    fn error_occurred(&self, message: &str) -> ZbusResult<()>;
    #[zbus(signal)]
    fn meter_update(&self, data: Vec<MeterData>) -> ZbusResult<()>;
    #[zbus(signal)]
    fn master_volume_changed(&self, volume_db: f64) -> ZbusResult<()>;
    #[zbus(signal)]
    fn master_mute_changed(&self, muted: bool) -> ZbusResult<()>;
    #[zbus(signal)]
    fn outputs_changed(&self) -> ZbusResult<()>;
    #[zbus(signal)]
    fn inputs_changed(&self) -> ZbusResult<()>;
}

/// Events received from the daemon.
#[derive(Debug, Clone)]
pub enum DaemonEvent {
    Connected,
    Disconnected,
    ChannelAdded(ChannelInfo),
    ChannelRemoved(String),
    ChannelUpdated(ChannelInfo),
    VolumeChanged { channel_id: String, volume_db: f64 },
    MuteChanged { channel_id: String, muted: bool },
    AppDiscovered(AppInfo),
    AppRemoved(String),
    AppRouted { app_id: String, channel_id: String },
    AppUnrouted { app_id: String, channel_id: String },
    PipeWireConnectionChanged(bool),
    Error(String),
    MeterUpdate(Vec<MeterData>),
    MasterVolumeChanged(f64),
    MasterMuteChanged(bool),
    OutputsChanged,
    InputsChanged,
    /// Initial state snapshot after connection
    InitialState {
        channels: Vec<ChannelInfo>,
        apps: Vec<AppInfo>,
        outputs: Vec<OutputInfo>,
        inputs: Vec<InputInfo>,
        master_volume: f64,
        master_muted: bool,
        master_output: String,
        connected: bool,
        recording_enabled: bool,
    },
}

/// Client for communicating with the SootMix daemon.
pub struct DaemonClient {
    proxy: DaemonProxy<'static>,
    connection: Connection,
}

impl DaemonClient {
    /// Connect to the daemon.
    pub async fn connect() -> Result<Self, DaemonClientError> {
        info!("Connecting to SootMix daemon...");

        let connection = Connection::session().await
            .map_err(|e| DaemonClientError::ConnectionFailed(e.to_string()))?;

        let proxy = DaemonProxy::new(&connection).await
            .map_err(|e| DaemonClientError::ProxyCreationFailed(e.to_string()))?;

        // Verify the daemon is running by calling a method
        match proxy.get_connected().await {
            Ok(_) => {
                info!("Connected to SootMix daemon");
            }
            Err(e) => {
                return Err(DaemonClientError::DaemonNotRunning(e.to_string()));
            }
        }

        Ok(Self { proxy, connection })
    }

    /// Get the initial state from the daemon.
    pub async fn get_initial_state(&self) -> Result<DaemonEvent, DaemonClientError> {
        let channels = self.proxy.get_channels().await
            .map_err(|e| DaemonClientError::MethodCallFailed(e.to_string()))?;
        let apps = self.proxy.get_apps().await
            .map_err(|e| DaemonClientError::MethodCallFailed(e.to_string()))?;
        let outputs = self.proxy.get_outputs().await
            .map_err(|e| DaemonClientError::MethodCallFailed(e.to_string()))?;
        let inputs = self.proxy.get_inputs().await
            .map_err(|e| DaemonClientError::MethodCallFailed(e.to_string()))?;
        let master_volume = self.proxy.get_master_volume().await
            .map_err(|e| DaemonClientError::MethodCallFailed(e.to_string()))?;
        let master_muted = self.proxy.get_master_muted().await
            .map_err(|e| DaemonClientError::MethodCallFailed(e.to_string()))?;
        let master_output = self.proxy.get_master_output().await
            .map_err(|e| DaemonClientError::MethodCallFailed(e.to_string()))?;
        let connected = self.proxy.get_connected().await
            .map_err(|e| DaemonClientError::MethodCallFailed(e.to_string()))?;
        let recording_enabled = self.proxy.get_master_recording_enabled().await
            .map_err(|e| DaemonClientError::MethodCallFailed(e.to_string()))?;

        Ok(DaemonEvent::InitialState {
            channels,
            apps,
            outputs,
            inputs,
            master_volume,
            master_muted,
            master_output,
            connected,
            recording_enabled,
        })
    }

    // ==================== Channel Management ====================

    /// Create a new mixer channel.
    pub async fn create_channel(&self, name: &str) -> Result<String, DaemonClientError> {
        debug!("Creating channel: {}", name);
        self.proxy.create_channel(name).await
            .map_err(|e| DaemonClientError::MethodCallFailed(e.to_string()))
    }

    /// Create a new input (microphone) channel.
    pub async fn create_input_channel(&self, name: &str) -> Result<String, DaemonClientError> {
        debug!("Creating input channel: {}", name);
        self.proxy.create_input_channel(name).await
            .map_err(|e| DaemonClientError::MethodCallFailed(e.to_string()))
    }

    /// Delete a mixer channel.
    pub async fn delete_channel(&self, channel_id: &str) -> Result<(), DaemonClientError> {
        debug!("Deleting channel: {}", channel_id);
        self.proxy.delete_channel(channel_id).await
            .map_err(|e| DaemonClientError::MethodCallFailed(e.to_string()))
    }

    /// Rename a mixer channel.
    pub async fn rename_channel(&self, channel_id: &str, name: &str) -> Result<(), DaemonClientError> {
        debug!("Renaming channel {} to {}", channel_id, name);
        self.proxy.rename_channel(channel_id, name).await
            .map_err(|e| DaemonClientError::MethodCallFailed(e.to_string()))
    }

    /// Move a channel left or right within its kind group.
    pub async fn move_channel(&self, channel_id: &str, direction: i32) -> Result<(), DaemonClientError> {
        debug!("Moving channel {} by {}", channel_id, direction);
        self.proxy.move_channel(channel_id, direction).await
            .map_err(|e| DaemonClientError::MethodCallFailed(e.to_string()))
    }

    // ==================== Volume/Mute ====================

    /// Set channel volume in dB.
    pub async fn set_channel_volume(&self, channel_id: &str, volume_db: f64) -> Result<(), DaemonClientError> {
        self.proxy.set_channel_volume(channel_id, volume_db).await
            .map_err(|e| DaemonClientError::MethodCallFailed(e.to_string()))
    }

    /// Set channel mute state.
    pub async fn set_channel_mute(&self, channel_id: &str, muted: bool) -> Result<(), DaemonClientError> {
        debug!("Setting channel {} mute to {}", channel_id, muted);
        self.proxy.set_channel_mute(channel_id, muted).await
            .map_err(|e| DaemonClientError::MethodCallFailed(e.to_string()))
    }

    /// Enable or disable noise suppression on an input channel.
    pub async fn set_channel_noise_suppression(&self, channel_id: &str, enabled: bool) -> Result<(), DaemonClientError> {
        debug!("Setting channel {} noise suppression to {}", channel_id, enabled);
        self.proxy.set_channel_noise_suppression(channel_id, enabled).await
            .map_err(|e| DaemonClientError::MethodCallFailed(e.to_string()))
    }

    /// Set the VAD threshold for noise suppression on an input channel.
    pub async fn set_channel_vad_threshold(&self, channel_id: &str, threshold: f64) -> Result<(), DaemonClientError> {
        debug!("Setting channel {} VAD threshold to {}%", channel_id, threshold);
        self.proxy.set_channel_vad_threshold(channel_id, threshold).await
            .map_err(|e| DaemonClientError::MethodCallFailed(e.to_string()))
    }

    /// Set the hardware microphone gain for an input channel.
    pub async fn set_channel_input_gain(&self, channel_id: &str, gain_db: f64) -> Result<(), DaemonClientError> {
        debug!("Setting channel {} input gain to {} dB", channel_id, gain_db);
        self.proxy.set_channel_input_gain(channel_id, gain_db).await
            .map_err(|e| DaemonClientError::MethodCallFailed(e.to_string()))
    }

    /// Set master volume in dB.
    pub async fn set_master_volume(&self, volume_db: f64) -> Result<(), DaemonClientError> {
        self.proxy.set_master_volume(volume_db).await
            .map_err(|e| DaemonClientError::MethodCallFailed(e.to_string()))
    }

    /// Set master mute state.
    pub async fn set_master_mute(&self, muted: bool) -> Result<(), DaemonClientError> {
        debug!("Setting master mute to {}", muted);
        self.proxy.set_master_mute(muted).await
            .map_err(|e| DaemonClientError::MethodCallFailed(e.to_string()))
    }

    // ==================== App Routing ====================

    /// Assign an app to a channel.
    pub async fn assign_app(&self, app_id: &str, channel_id: &str) -> Result<(), DaemonClientError> {
        debug!("Assigning app {} to channel {}", app_id, channel_id);
        self.proxy.assign_app(app_id, channel_id).await
            .map_err(|e| DaemonClientError::MethodCallFailed(e.to_string()))
    }

    /// Unassign an app from a channel.
    pub async fn unassign_app(&self, app_id: &str, channel_id: &str) -> Result<(), DaemonClientError> {
        debug!("Unassigning app {} from channel {}", app_id, channel_id);
        self.proxy.unassign_app(app_id, channel_id).await
            .map_err(|e| DaemonClientError::MethodCallFailed(e.to_string()))
    }

    // ==================== Output Routing ====================

    /// Set the output device for a channel.
    pub async fn set_channel_output(&self, channel_id: &str, device_name: &str) -> Result<(), DaemonClientError> {
        debug!("Setting channel {} output to {}", channel_id, device_name);
        self.proxy.set_channel_output(channel_id, device_name).await
            .map_err(|e| DaemonClientError::MethodCallFailed(e.to_string()))
    }

    /// Set the master output device.
    pub async fn set_master_output(&self, device_name: &str) -> Result<(), DaemonClientError> {
        debug!("Setting master output to {}", device_name);
        self.proxy.set_master_output(device_name).await
            .map_err(|e| DaemonClientError::MethodCallFailed(e.to_string()))
    }

    // ==================== Recording ====================

    /// Enable or disable master recording output.
    pub async fn set_master_recording(&self, enabled: bool) -> Result<(), DaemonClientError> {
        debug!("Setting master recording to {}", enabled);
        self.proxy.set_master_recording(enabled).await
            .map_err(|e| DaemonClientError::MethodCallFailed(e.to_string()))
    }

    // ==================== Getters ====================

    /// Get all channels.
    pub async fn get_channels(&self) -> Result<Vec<ChannelInfo>, DaemonClientError> {
        self.proxy.get_channels().await
            .map_err(|e| DaemonClientError::MethodCallFailed(e.to_string()))
    }

    /// Get all discovered apps.
    pub async fn get_apps(&self) -> Result<Vec<AppInfo>, DaemonClientError> {
        self.proxy.get_apps().await
            .map_err(|e| DaemonClientError::MethodCallFailed(e.to_string()))
    }

    /// Get all output devices.
    pub async fn get_outputs(&self) -> Result<Vec<OutputInfo>, DaemonClientError> {
        self.proxy.get_outputs().await
            .map_err(|e| DaemonClientError::MethodCallFailed(e.to_string()))
    }

    /// Get whether connected to PipeWire.
    pub async fn get_connected(&self) -> Result<bool, DaemonClientError> {
        self.proxy.get_connected().await
            .map_err(|e| DaemonClientError::MethodCallFailed(e.to_string()))
    }

    // ==================== Routing Rules ====================

    /// Get all routing rules.
    pub async fn get_routing_rules(&self) -> Result<Vec<RoutingRuleInfo>, DaemonClientError> {
        self.proxy.get_routing_rules().await
            .map_err(|e| DaemonClientError::MethodCallFailed(e.to_string()))
    }

    /// Add or update a routing rule.
    pub async fn set_routing_rule(&self, rule: RoutingRuleInfo) -> Result<(), DaemonClientError> {
        debug!("Setting routing rule: {:?}", rule);
        self.proxy.set_routing_rule(rule).await
            .map_err(|e| DaemonClientError::MethodCallFailed(e.to_string()))
    }

    /// Delete a routing rule.
    pub async fn delete_routing_rule(&self, rule_id: &str) -> Result<(), DaemonClientError> {
        debug!("Deleting routing rule: {}", rule_id);
        self.proxy.delete_routing_rule(rule_id).await
            .map_err(|e| DaemonClientError::MethodCallFailed(e.to_string()))
    }

    /// Toggle a routing rule's enabled state.
    pub async fn toggle_routing_rule(&self, rule_id: &str) -> Result<(), DaemonClientError> {
        debug!("Toggling routing rule: {}", rule_id);
        self.proxy.toggle_routing_rule(rule_id).await
            .map_err(|e| DaemonClientError::MethodCallFailed(e.to_string()))
    }
}

/// Errors that can occur when communicating with the daemon.
#[derive(Debug, Clone, thiserror::Error)]
pub enum DaemonClientError {
    #[error("Failed to connect to D-Bus: {0}")]
    ConnectionFailed(String),
    #[error("Failed to create D-Bus proxy: {0}")]
    ProxyCreationFailed(String),
    #[error("Daemon is not running: {0}")]
    DaemonNotRunning(String),
    #[error("Method call failed: {0}")]
    MethodCallFailed(String),
    #[error("Signal subscription failed: {0}")]
    SignalSubscriptionFailed(String),
}

/// Shared client wrapped in Arc<Mutex> for use in async contexts.
pub type SharedDaemonClient = Arc<Mutex<Option<DaemonClient>>>;

/// Create a new shared daemon client (initially disconnected).
pub fn new_shared_client() -> SharedDaemonClient {
    Arc::new(Mutex::new(None))
}

/// Command to send to the daemon.
#[derive(Debug, Clone)]
pub enum DaemonCommand {
    CreateChannel(String),
    CreateInputChannel(String),
    DeleteChannel(String),
    RenameChannel { id: String, name: String },
    SetChannelVolume { id: String, volume_db: f64 },
    SetChannelMute { id: String, muted: bool },
    SetMasterVolume(f64),
    SetMasterMute(bool),
    AssignApp { app_id: String, channel_id: String },
    UnassignApp { app_id: String, channel_id: String },
    SetChannelOutput { channel_id: String, device_name: String },
    SetMasterOutput(String),
    SetMasterRecording(bool),
    SetChannelNoiseSuppression { channel_id: String, enabled: bool },
    SetChannelVadThreshold { channel_id: String, threshold: f64 },
    SetChannelInputGain { channel_id: String, gain_db: f64 },
    MoveChannel { channel_id: String, direction: i32 },
}

/// Global command sender for the daemon subscription.
/// Uses RwLock instead of OnceLock to allow updating the sender on reconnection.
static DAEMON_CMD_TX: std::sync::RwLock<Option<tokio::sync::mpsc::UnboundedSender<DaemonCommand>>> = std::sync::RwLock::new(None);

/// Send a command to the daemon.
pub fn send_daemon_command(cmd: DaemonCommand) -> Result<(), String> {
    let guard = DAEMON_CMD_TX.read().map_err(|e| format!("Lock poisoned: {}", e))?;
    if let Some(tx) = guard.as_ref() {
        tx.send(cmd).map_err(|e| e.to_string())
    } else {
        Err("Daemon command channel not initialized".to_string())
    }
}

// ==================== Systemd Service Controls ====================

const SERVICE_NAME: &str = "sootmix-daemon.service";

/// Start the daemon via systemd.
pub async fn systemctl_start() -> Result<String, String> {
    let output = tokio::process::Command::new("systemctl")
        .args(["--user", "start", SERVICE_NAME])
        .output()
        .await
        .map_err(|e| format!("Failed to run systemctl: {}", e))?;
    if output.status.success() {
        Ok("Daemon started".to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// Stop the daemon via systemd.
pub async fn systemctl_stop() -> Result<String, String> {
    let output = tokio::process::Command::new("systemctl")
        .args(["--user", "stop", SERVICE_NAME])
        .output()
        .await
        .map_err(|e| format!("Failed to run systemctl: {}", e))?;
    if output.status.success() {
        Ok("Daemon stopped".to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// Restart the daemon via systemd.
pub async fn systemctl_restart() -> Result<String, String> {
    let output = tokio::process::Command::new("systemctl")
        .args(["--user", "restart", SERVICE_NAME])
        .output()
        .await
        .map_err(|e| format!("Failed to run systemctl: {}", e))?;
    if output.status.success() {
        Ok("Daemon restarted".to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// Check if the daemon systemd service is enabled (autostart).
pub async fn systemctl_is_enabled() -> Result<bool, String> {
    let output = tokio::process::Command::new("systemctl")
        .args(["--user", "is-enabled", SERVICE_NAME])
        .output()
        .await
        .map_err(|e| format!("Failed to run systemctl: {}", e))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(stdout == "enabled")
}

/// Enable or disable the daemon systemd service (autostart).
pub async fn systemctl_set_enabled(enable: bool) -> Result<String, String> {
    let action = if enable { "enable" } else { "disable" };
    let output = tokio::process::Command::new("systemctl")
        .args(["--user", action, SERVICE_NAME])
        .output()
        .await
        .map_err(|e| format!("Failed to run systemctl: {}", e))?;
    if output.status.success() {
        Ok(format!("Daemon {}", if enable { "enabled" } else { "disabled" }))
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// Iced subscription for daemon events.
pub fn daemon_subscription() -> iced::Subscription<DaemonEvent> {
    iced::Subscription::run(daemon_event_stream)
}

/// Stream of daemon events for the Iced subscription.
fn daemon_event_stream() -> impl futures::Stream<Item = DaemonEvent> + Send {
    // Create an async channel for events
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

    // Create command channel and store sender globally
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::unbounded_channel::<DaemonCommand>();
    if let Ok(mut guard) = DAEMON_CMD_TX.write() {
        *guard = Some(cmd_tx);
    }

    // Spawn the connection and signal listener task
    tokio::spawn(async move {
        let mut start_attempted = false;

        loop {
            // Try to connect
            match DaemonClient::connect().await {
                Ok(client) => {
                    start_attempted = false; // Reset on successful connection
                    let _ = tx.send(DaemonEvent::Connected);

                    // Get initial state
                    match client.get_initial_state().await {
                        Ok(state) => {
                            let _ = tx.send(state);
                        }
                        Err(e) => {
                            error!("Failed to get initial state: {}", e);
                        }
                    }

                    // Listen to signals and handle commands concurrently
                    let client = std::sync::Arc::new(client);
                    let client_for_signals = client.clone();
                    let client_for_commands = client.clone();
                    let tx_clone = tx.clone();

                    // Spawn command handler
                    let cmd_handle = tokio::spawn(async move {
                        while let Some(cmd) = cmd_rx.recv().await {
                            if let Err(e) = handle_daemon_command(&client_for_commands, cmd).await {
                                error!("Daemon command failed: {}", e);
                            }
                        }
                    });

                    // Subscribe to signals (blocks until disconnection)
                    if let Err(e) = listen_to_signals_arc(&client_for_signals, tx_clone).await {
                        error!("Signal listener error: {}", e);
                    }

                    cmd_handle.abort();

                    // If we get here, the connection was lost
                    let _ = tx.send(DaemonEvent::Disconnected);

                    // Recreate command channel for next connection and update the global sender
                    let (new_cmd_tx, new_cmd_rx) = tokio::sync::mpsc::unbounded_channel::<DaemonCommand>();
                    cmd_rx = new_cmd_rx;
                    if let Ok(mut guard) = DAEMON_CMD_TX.write() {
                        *guard = Some(new_cmd_tx);
                    }
                }
                Err(e) => {
                    warn!("Failed to connect to daemon: {}", e);

                    // Try to start the daemon if we haven't already
                    if !start_attempted {
                        start_attempted = true;
                        info!("Attempting to start sootmix-daemon...");
                        if let Err(start_err) = try_start_daemon().await {
                            warn!("Failed to start daemon: {}", start_err);
                        } else {
                            // Give daemon time to start
                            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                            continue; // Try connecting again immediately
                        }
                    }

                    let _ = tx.send(DaemonEvent::Disconnected);
                }
            }

            // Wait before retry
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        }
    });

    // Convert the mpsc receiver to a stream
    tokio_stream::wrappers::UnboundedReceiverStream::new(rx)
}

/// Try to start the daemon, first via systemd, then directly.
async fn try_start_daemon() -> Result<(), String> {
    // First, try systemd user service
    let systemd_result = tokio::process::Command::new("systemctl")
        .args(["--user", "start", "sootmix-daemon.service"])
        .output()
        .await;

    if let Ok(output) = systemd_result {
        if output.status.success() {
            info!("Started sootmix-daemon via systemd");
            return Ok(());
        }
        debug!(
            "systemctl start failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // Try starting the daemon directly
    // Look for the binary in common locations
    let mut binary_paths: Vec<std::path::PathBuf> = vec![
        // System locations
        std::path::PathBuf::from("/usr/bin/sootmix-daemon"),
        std::path::PathBuf::from("/usr/local/bin/sootmix-daemon"),
    ];

    // Development build (same directory as current exe)
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(parent) = exe_path.parent() {
            binary_paths.insert(0, parent.join("sootmix-daemon"));
        }
    }

    // Cargo install location
    if let Some(home) = std::env::var_os("HOME") {
        binary_paths.insert(1, std::path::PathBuf::from(home).join(".cargo/bin/sootmix-daemon"));
    }

    for path in binary_paths.iter() {
        if path.exists() {
            info!("Starting daemon from: {}", path.display());
            match tokio::process::Command::new(path)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
            {
                Ok(_) => {
                    info!("Started sootmix-daemon directly");
                    return Ok(());
                }
                Err(e) => {
                    debug!("Failed to start {}: {}", path.display(), e);
                }
            }
        }
    }

    Err("Could not find or start sootmix-daemon".to_string())
}

/// Listen to daemon signals and forward them to the channel.
async fn listen_to_signals(
    client: &DaemonClient,
    tx: tokio::sync::mpsc::UnboundedSender<DaemonEvent>,
) -> Result<(), DaemonClientError> {
    use futures::StreamExt;

    // Get signal streams
    let mut channel_added = client.proxy.receive_channel_added().await
        .map_err(|e| DaemonClientError::SignalSubscriptionFailed(e.to_string()))?;
    let mut channel_removed = client.proxy.receive_channel_removed().await
        .map_err(|e| DaemonClientError::SignalSubscriptionFailed(e.to_string()))?;
    let mut channel_updated = client.proxy.receive_channel_updated().await
        .map_err(|e| DaemonClientError::SignalSubscriptionFailed(e.to_string()))?;
    let mut volume_changed = client.proxy.receive_volume_changed().await
        .map_err(|e| DaemonClientError::SignalSubscriptionFailed(e.to_string()))?;
    let mut mute_changed = client.proxy.receive_mute_changed().await
        .map_err(|e| DaemonClientError::SignalSubscriptionFailed(e.to_string()))?;
    let mut app_discovered = client.proxy.receive_app_discovered().await
        .map_err(|e| DaemonClientError::SignalSubscriptionFailed(e.to_string()))?;
    let mut app_removed = client.proxy.receive_app_removed().await
        .map_err(|e| DaemonClientError::SignalSubscriptionFailed(e.to_string()))?;
    let mut app_routed = client.proxy.receive_app_routed().await
        .map_err(|e| DaemonClientError::SignalSubscriptionFailed(e.to_string()))?;
    let mut app_unrouted = client.proxy.receive_app_unrouted().await
        .map_err(|e| DaemonClientError::SignalSubscriptionFailed(e.to_string()))?;
    let mut connection_changed = client.proxy.receive_connection_changed().await
        .map_err(|e| DaemonClientError::SignalSubscriptionFailed(e.to_string()))?;
    let mut error_occurred = client.proxy.receive_error_occurred().await
        .map_err(|e| DaemonClientError::SignalSubscriptionFailed(e.to_string()))?;
    let mut meter_update = client.proxy.receive_meter_update().await
        .map_err(|e| DaemonClientError::SignalSubscriptionFailed(e.to_string()))?;
    let mut master_volume_changed = client.proxy.receive_master_volume_changed().await
        .map_err(|e| DaemonClientError::SignalSubscriptionFailed(e.to_string()))?;
    let mut master_mute_changed = client.proxy.receive_master_mute_changed().await
        .map_err(|e| DaemonClientError::SignalSubscriptionFailed(e.to_string()))?;
    let mut outputs_changed = client.proxy.receive_outputs_changed().await
        .map_err(|e| DaemonClientError::SignalSubscriptionFailed(e.to_string()))?;
    let mut inputs_changed = client.proxy.receive_inputs_changed().await
        .map_err(|e| DaemonClientError::SignalSubscriptionFailed(e.to_string()))?;

    loop {
        tokio::select! {
            Some(signal) = channel_added.next() => {
                if let Ok(args) = signal.args() {
                    let _ = tx.send(DaemonEvent::ChannelAdded(args.channel));
                }
            }
            Some(signal) = channel_removed.next() => {
                if let Ok(args) = signal.args() {
                    let _ = tx.send(DaemonEvent::ChannelRemoved(args.channel_id.to_string()));
                }
            }
            Some(signal) = channel_updated.next() => {
                if let Ok(args) = signal.args() {
                    let _ = tx.send(DaemonEvent::ChannelUpdated(args.channel));
                }
            }
            Some(signal) = volume_changed.next() => {
                if let Ok(args) = signal.args() {
                    let _ = tx.send(DaemonEvent::VolumeChanged {
                        channel_id: args.channel_id.to_string(),
                        volume_db: args.volume_db,
                    });
                }
            }
            Some(signal) = mute_changed.next() => {
                if let Ok(args) = signal.args() {
                    let _ = tx.send(DaemonEvent::MuteChanged {
                        channel_id: args.channel_id.to_string(),
                        muted: args.muted,
                    });
                }
            }
            Some(signal) = app_discovered.next() => {
                if let Ok(args) = signal.args() {
                    let _ = tx.send(DaemonEvent::AppDiscovered(args.app));
                }
            }
            Some(signal) = app_removed.next() => {
                if let Ok(args) = signal.args() {
                    let _ = tx.send(DaemonEvent::AppRemoved(args.app_id.to_string()));
                }
            }
            Some(signal) = app_routed.next() => {
                if let Ok(args) = signal.args() {
                    let _ = tx.send(DaemonEvent::AppRouted {
                        app_id: args.app_id.to_string(),
                        channel_id: args.channel_id.to_string(),
                    });
                }
            }
            Some(signal) = app_unrouted.next() => {
                if let Ok(args) = signal.args() {
                    let _ = tx.send(DaemonEvent::AppUnrouted {
                        app_id: args.app_id.to_string(),
                        channel_id: args.channel_id.to_string(),
                    });
                }
            }
            Some(signal) = connection_changed.next() => {
                if let Ok(args) = signal.args() {
                    let _ = tx.send(DaemonEvent::PipeWireConnectionChanged(args.connected));
                }
            }
            Some(signal) = error_occurred.next() => {
                if let Ok(args) = signal.args() {
                    let _ = tx.send(DaemonEvent::Error(args.message.to_string()));
                }
            }
            Some(signal) = meter_update.next() => {
                if let Ok(args) = signal.args() {
                    let _ = tx.send(DaemonEvent::MeterUpdate(args.data));
                }
            }
            Some(signal) = master_volume_changed.next() => {
                if let Ok(args) = signal.args() {
                    let _ = tx.send(DaemonEvent::MasterVolumeChanged(args.volume_db));
                }
            }
            Some(signal) = master_mute_changed.next() => {
                if let Ok(args) = signal.args() {
                    let _ = tx.send(DaemonEvent::MasterMuteChanged(args.muted));
                }
            }
            Some(_signal) = outputs_changed.next() => {
                let _ = tx.send(DaemonEvent::OutputsChanged);
            }
            Some(_signal) = inputs_changed.next() => {
                let _ = tx.send(DaemonEvent::InputsChanged);
            }
            else => {
                // All streams ended, connection lost
                break;
            }
        }
    }

    Ok(())
}

/// Handle a command by calling the appropriate daemon method.
async fn handle_daemon_command(
    client: &std::sync::Arc<DaemonClient>,
    cmd: DaemonCommand,
) -> Result<(), DaemonClientError> {
    match cmd {
        DaemonCommand::CreateChannel(name) => {
            client.create_channel(&name).await?;
        }
        DaemonCommand::CreateInputChannel(name) => {
            client.create_input_channel(&name).await?;
        }
        DaemonCommand::DeleteChannel(id) => {
            client.delete_channel(&id).await?;
        }
        DaemonCommand::RenameChannel { id, name } => {
            client.rename_channel(&id, &name).await?;
        }
        DaemonCommand::SetChannelVolume { id, volume_db } => {
            client.set_channel_volume(&id, volume_db).await?;
        }
        DaemonCommand::SetChannelMute { id, muted } => {
            client.set_channel_mute(&id, muted).await?;
        }
        DaemonCommand::SetMasterVolume(volume_db) => {
            client.set_master_volume(volume_db).await?;
        }
        DaemonCommand::SetMasterMute(muted) => {
            client.set_master_mute(muted).await?;
        }
        DaemonCommand::AssignApp { app_id, channel_id } => {
            client.assign_app(&app_id, &channel_id).await?;
        }
        DaemonCommand::UnassignApp { app_id, channel_id } => {
            client.unassign_app(&app_id, &channel_id).await?;
        }
        DaemonCommand::SetChannelOutput { channel_id, device_name } => {
            client.set_channel_output(&channel_id, &device_name).await?;
        }
        DaemonCommand::SetMasterOutput(device_name) => {
            client.set_master_output(&device_name).await?;
        }
        DaemonCommand::SetMasterRecording(enabled) => {
            client.set_master_recording(enabled).await?;
        }
        DaemonCommand::SetChannelNoiseSuppression { channel_id, enabled } => {
            client.set_channel_noise_suppression(&channel_id, enabled).await?;
        }
        DaemonCommand::SetChannelVadThreshold { channel_id, threshold } => {
            client.set_channel_vad_threshold(&channel_id, threshold).await?;
        }
        DaemonCommand::SetChannelInputGain { channel_id, gain_db } => {
            client.set_channel_input_gain(&channel_id, gain_db).await?;
        }
        DaemonCommand::MoveChannel { channel_id, direction } => {
            client.move_channel(&channel_id, direction).await?;
        }
    }
    Ok(())
}

/// Listen to daemon signals (Arc version for concurrent use).
async fn listen_to_signals_arc(
    client: &std::sync::Arc<DaemonClient>,
    tx: tokio::sync::mpsc::UnboundedSender<DaemonEvent>,
) -> Result<(), DaemonClientError> {
    use futures::StreamExt;

    // Get signal streams
    let mut channel_added = client.proxy.receive_channel_added().await
        .map_err(|e| DaemonClientError::SignalSubscriptionFailed(e.to_string()))?;
    let mut channel_removed = client.proxy.receive_channel_removed().await
        .map_err(|e| DaemonClientError::SignalSubscriptionFailed(e.to_string()))?;
    let mut channel_updated = client.proxy.receive_channel_updated().await
        .map_err(|e| DaemonClientError::SignalSubscriptionFailed(e.to_string()))?;
    let mut volume_changed = client.proxy.receive_volume_changed().await
        .map_err(|e| DaemonClientError::SignalSubscriptionFailed(e.to_string()))?;
    let mut mute_changed = client.proxy.receive_mute_changed().await
        .map_err(|e| DaemonClientError::SignalSubscriptionFailed(e.to_string()))?;
    let mut app_discovered = client.proxy.receive_app_discovered().await
        .map_err(|e| DaemonClientError::SignalSubscriptionFailed(e.to_string()))?;
    let mut app_removed = client.proxy.receive_app_removed().await
        .map_err(|e| DaemonClientError::SignalSubscriptionFailed(e.to_string()))?;
    let mut app_routed = client.proxy.receive_app_routed().await
        .map_err(|e| DaemonClientError::SignalSubscriptionFailed(e.to_string()))?;
    let mut app_unrouted = client.proxy.receive_app_unrouted().await
        .map_err(|e| DaemonClientError::SignalSubscriptionFailed(e.to_string()))?;
    let mut connection_changed = client.proxy.receive_connection_changed().await
        .map_err(|e| DaemonClientError::SignalSubscriptionFailed(e.to_string()))?;
    let mut error_occurred = client.proxy.receive_error_occurred().await
        .map_err(|e| DaemonClientError::SignalSubscriptionFailed(e.to_string()))?;
    let mut meter_update = client.proxy.receive_meter_update().await
        .map_err(|e| DaemonClientError::SignalSubscriptionFailed(e.to_string()))?;
    let mut master_volume_changed = client.proxy.receive_master_volume_changed().await
        .map_err(|e| DaemonClientError::SignalSubscriptionFailed(e.to_string()))?;
    let mut master_mute_changed = client.proxy.receive_master_mute_changed().await
        .map_err(|e| DaemonClientError::SignalSubscriptionFailed(e.to_string()))?;
    let mut outputs_changed = client.proxy.receive_outputs_changed().await
        .map_err(|e| DaemonClientError::SignalSubscriptionFailed(e.to_string()))?;
    let mut inputs_changed = client.proxy.receive_inputs_changed().await
        .map_err(|e| DaemonClientError::SignalSubscriptionFailed(e.to_string()))?;

    loop {
        tokio::select! {
            Some(signal) = channel_added.next() => {
                if let Ok(args) = signal.args() {
                    let _ = tx.send(DaemonEvent::ChannelAdded(args.channel));
                }
            }
            Some(signal) = channel_removed.next() => {
                if let Ok(args) = signal.args() {
                    let _ = tx.send(DaemonEvent::ChannelRemoved(args.channel_id.to_string()));
                }
            }
            Some(signal) = channel_updated.next() => {
                if let Ok(args) = signal.args() {
                    let _ = tx.send(DaemonEvent::ChannelUpdated(args.channel));
                }
            }
            Some(signal) = volume_changed.next() => {
                if let Ok(args) = signal.args() {
                    let _ = tx.send(DaemonEvent::VolumeChanged {
                        channel_id: args.channel_id.to_string(),
                        volume_db: args.volume_db,
                    });
                }
            }
            Some(signal) = mute_changed.next() => {
                if let Ok(args) = signal.args() {
                    let _ = tx.send(DaemonEvent::MuteChanged {
                        channel_id: args.channel_id.to_string(),
                        muted: args.muted,
                    });
                }
            }
            Some(signal) = app_discovered.next() => {
                if let Ok(args) = signal.args() {
                    let _ = tx.send(DaemonEvent::AppDiscovered(args.app));
                }
            }
            Some(signal) = app_removed.next() => {
                if let Ok(args) = signal.args() {
                    let _ = tx.send(DaemonEvent::AppRemoved(args.app_id.to_string()));
                }
            }
            Some(signal) = app_routed.next() => {
                if let Ok(args) = signal.args() {
                    let _ = tx.send(DaemonEvent::AppRouted {
                        app_id: args.app_id.to_string(),
                        channel_id: args.channel_id.to_string(),
                    });
                }
            }
            Some(signal) = app_unrouted.next() => {
                if let Ok(args) = signal.args() {
                    let _ = tx.send(DaemonEvent::AppUnrouted {
                        app_id: args.app_id.to_string(),
                        channel_id: args.channel_id.to_string(),
                    });
                }
            }
            Some(signal) = connection_changed.next() => {
                if let Ok(args) = signal.args() {
                    let _ = tx.send(DaemonEvent::PipeWireConnectionChanged(args.connected));
                }
            }
            Some(signal) = error_occurred.next() => {
                if let Ok(args) = signal.args() {
                    let _ = tx.send(DaemonEvent::Error(args.message.to_string()));
                }
            }
            Some(signal) = meter_update.next() => {
                if let Ok(args) = signal.args() {
                    let _ = tx.send(DaemonEvent::MeterUpdate(args.data));
                }
            }
            Some(signal) = master_volume_changed.next() => {
                if let Ok(args) = signal.args() {
                    let _ = tx.send(DaemonEvent::MasterVolumeChanged(args.volume_db));
                }
            }
            Some(signal) = master_mute_changed.next() => {
                if let Ok(args) = signal.args() {
                    let _ = tx.send(DaemonEvent::MasterMuteChanged(args.muted));
                }
            }
            Some(_signal) = outputs_changed.next() => {
                let _ = tx.send(DaemonEvent::OutputsChanged);
            }
            Some(_signal) = inputs_changed.next() => {
                let _ = tx.send(DaemonEvent::InputsChanged);
            }
            else => {
                // All streams ended, connection lost
                break;
            }
        }
    }

    Ok(())
}
