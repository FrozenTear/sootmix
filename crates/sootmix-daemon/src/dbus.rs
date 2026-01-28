// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! D-Bus interface implementation for the daemon.

use crate::service::DaemonService;
use sootmix_ipc::{AppInfo, ChannelInfo, InputInfo, MeterData, OutputInfo, RoutingRuleInfo};
use std::sync::{Arc, Mutex};
use tracing::debug;
use zbus::interface;

/// Input validation helpers for D-Bus method arguments.
mod validate {
    /// Validate a channel name: non-empty, max 128 chars, no control characters.
    pub fn validate_channel_name(name: &str) -> Result<(), zbus::fdo::Error> {
        if name.is_empty() {
            return Err(zbus::fdo::Error::InvalidArgs("Channel name must not be empty".into()));
        }
        if name.len() > 128 {
            return Err(zbus::fdo::Error::InvalidArgs(
                format!("Channel name exceeds 128 character limit (got {})", name.len()),
            ));
        }
        if name.chars().any(|c| c.is_control()) {
            return Err(zbus::fdo::Error::InvalidArgs(
                "Channel name must not contain control characters".into(),
            ));
        }
        Ok(())
    }

    /// Validate a volume in dB: reject NaN/Infinity, clamp to -96.0..=24.0 range.
    pub fn validate_volume_db(volume_db: f64) -> Result<f64, zbus::fdo::Error> {
        if volume_db.is_nan() || volume_db.is_infinite() {
            return Err(zbus::fdo::Error::InvalidArgs(
                "Volume must be a finite number".into(),
            ));
        }
        Ok(volume_db.clamp(-96.0, 24.0))
    }

    /// Validate a device name: non-empty, max 256 chars, no control characters.
    pub fn validate_device_name(name: &str) -> Result<(), zbus::fdo::Error> {
        if name.is_empty() {
            return Err(zbus::fdo::Error::InvalidArgs("Device name must not be empty".into()));
        }
        if name.len() > 256 {
            return Err(zbus::fdo::Error::InvalidArgs(
                format!("Device name exceeds 256 character limit (got {})", name.len()),
            ));
        }
        if name.chars().any(|c| c.is_control()) {
            return Err(zbus::fdo::Error::InvalidArgs(
                "Device name must not contain control characters".into(),
            ));
        }
        Ok(())
    }
}

/// The D-Bus interface implementation.
pub struct DaemonDbusService {
    service: Arc<Mutex<DaemonService>>,
}

impl DaemonDbusService {
    pub fn new(service: Arc<Mutex<DaemonService>>) -> Self {
        Self { service }
    }
}

#[interface(name = "com.sootmix.Daemon")]
impl DaemonDbusService {
    // ==================== Channel Management ====================

    /// Create a new mixer channel.
    async fn create_channel(
        &self,
        #[zbus(signal_context)] ctx: zbus::SignalContext<'_>,
        name: &str,
    ) -> zbus::fdo::Result<String> {
        validate::validate_channel_name(name)?;
        debug!("D-Bus: create_channel({})", name);
        let (channel_id, channel_info) = {
            let mut service = self
                .service
                .lock()
                .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
            service.process_pw_events();
            let id = service
                .create_channel(name)
                .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
            // Get the channel info to emit in the signal
            let info = service
                .state
                .channels
                .iter()
                .find(|c| c.id.to_string() == id)
                .map(|c| c.to_channel_info());
            (id, info)
        };

        // Emit signal after releasing the lock
        if let Some(info) = channel_info {
            let _ = Self::channel_added(&ctx, info).await;
        }

        Ok(channel_id)
    }

    /// Create a new input (microphone) channel.
    async fn create_input_channel(
        &self,
        #[zbus(signal_context)] ctx: zbus::SignalContext<'_>,
        name: &str,
    ) -> zbus::fdo::Result<String> {
        validate::validate_channel_name(name)?;
        debug!("D-Bus: create_input_channel({})", name);
        let (channel_id, channel_info) = {
            let mut service = self
                .service
                .lock()
                .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
            service.process_pw_events();
            let id = service
                .create_input_channel(name)
                .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
            // Get the channel info to emit in the signal
            let info = service
                .state
                .channels
                .iter()
                .find(|c| c.id.to_string() == id)
                .map(|c| c.to_channel_info());
            (id, info)
        };

        // Emit signal after releasing the lock
        if let Some(info) = channel_info {
            let _ = Self::channel_added(&ctx, info).await;
        }

        Ok(channel_id)
    }

    /// Delete a mixer channel.
    async fn delete_channel(
        &self,
        #[zbus(signal_context)] ctx: zbus::SignalContext<'_>,
        channel_id: &str,
    ) -> zbus::fdo::Result<()> {
        debug!("D-Bus: delete_channel({})", channel_id);
        let id_string = channel_id.to_string();
        {
            let mut service = self
                .service
                .lock()
                .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
            service.process_pw_events();
            service
                .delete_channel(channel_id)
                .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        }

        // Emit signal after releasing the lock
        let _ = Self::channel_removed(&ctx, &id_string).await;
        Ok(())
    }

    /// Rename a mixer channel.
    async fn rename_channel(
        &self,
        #[zbus(signal_context)] ctx: zbus::SignalContext<'_>,
        channel_id: &str,
        name: &str,
    ) -> zbus::fdo::Result<()> {
        validate::validate_channel_name(name)?;
        debug!("D-Bus: rename_channel({}, {})", channel_id, name);
        let channel_info = {
            let mut service = self
                .service
                .lock()
                .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
            service.process_pw_events();
            service
                .rename_channel(channel_id, name)
                .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
            // Get updated channel info
            service
                .state
                .channels
                .iter()
                .find(|c| c.id.to_string() == channel_id)
                .map(|c| c.to_channel_info())
        };

        // Emit signal after releasing the lock
        if let Some(info) = channel_info {
            let _ = Self::channel_updated(&ctx, info).await;
        }
        Ok(())
    }

    // ==================== Volume/Mute ====================

    /// Set channel volume in dB.
    async fn set_channel_volume(
        &self,
        #[zbus(signal_context)] ctx: zbus::SignalContext<'_>,
        channel_id: &str,
        volume_db: f64,
    ) -> zbus::fdo::Result<()> {
        let volume_db = validate::validate_volume_db(volume_db)?;
        let id_string = channel_id.to_string();
        {
            let mut service = self
                .service
                .lock()
                .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
            service.process_pw_events();
            service
                .set_channel_volume(channel_id, volume_db)
                .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        }

        // Emit signal after releasing the lock
        let _ = Self::volume_changed(&ctx, &id_string, volume_db).await;
        Ok(())
    }

    /// Set channel mute state.
    async fn set_channel_mute(
        &self,
        #[zbus(signal_context)] ctx: zbus::SignalContext<'_>,
        channel_id: &str,
        muted: bool,
    ) -> zbus::fdo::Result<()> {
        debug!("D-Bus: set_channel_mute({}, {})", channel_id, muted);
        let id_string = channel_id.to_string();
        {
            let mut service = self
                .service
                .lock()
                .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
            service.process_pw_events();
            service
                .set_channel_mute(channel_id, muted)
                .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        }

        // Emit signal after releasing the lock
        let _ = Self::mute_changed(&ctx, &id_string, muted).await;
        Ok(())
    }

    /// Set master volume in dB.
    async fn set_master_volume(
        &self,
        #[zbus(signal_context)] ctx: zbus::SignalContext<'_>,
        volume_db: f64,
    ) -> zbus::fdo::Result<()> {
        let volume_db = validate::validate_volume_db(volume_db)?;
        {
            let mut service = self
                .service
                .lock()
                .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
            service.process_pw_events();
            service
                .set_master_volume(volume_db)
                .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        }

        // Emit signal after releasing the lock
        let _ = Self::master_volume_changed(&ctx, volume_db).await;
        Ok(())
    }

    /// Set master mute state.
    async fn set_master_mute(
        &self,
        #[zbus(signal_context)] ctx: zbus::SignalContext<'_>,
        muted: bool,
    ) -> zbus::fdo::Result<()> {
        debug!("D-Bus: set_master_mute({})", muted);
        {
            let mut service = self
                .service
                .lock()
                .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
            service.process_pw_events();
            service
                .set_master_mute(muted)
                .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        }

        // Emit signal after releasing the lock
        let _ = Self::master_mute_changed(&ctx, muted).await;
        Ok(())
    }

    // ==================== App Routing ====================

    /// Assign an app to a channel.
    async fn assign_app(
        &self,
        #[zbus(signal_context)] ctx: zbus::SignalContext<'_>,
        app_id: &str,
        channel_id: &str,
    ) -> zbus::fdo::Result<()> {
        debug!("D-Bus: assign_app({}, {})", app_id, channel_id);
        let (app_id_string, channel_id_string, channel_info) = {
            let mut service = self
                .service
                .lock()
                .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
            service.process_pw_events();
            service
                .assign_app(app_id, channel_id)
                .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
            // Get updated channel info
            let info = service
                .state
                .channels
                .iter()
                .find(|c| c.id.to_string() == channel_id)
                .map(|c| c.to_channel_info());
            (app_id.to_string(), channel_id.to_string(), info)
        };

        // Emit signals after releasing the lock
        let _ = Self::app_routed(&ctx, &app_id_string, &channel_id_string).await;
        if let Some(info) = channel_info {
            let _ = Self::channel_updated(&ctx, info).await;
        }
        Ok(())
    }

    /// Unassign an app from a channel.
    async fn unassign_app(
        &self,
        #[zbus(signal_context)] ctx: zbus::SignalContext<'_>,
        app_id: &str,
        channel_id: &str,
    ) -> zbus::fdo::Result<()> {
        debug!("D-Bus: unassign_app({}, {})", app_id, channel_id);
        let (app_id_string, channel_id_string, channel_info) = {
            let mut service = self
                .service
                .lock()
                .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
            service.process_pw_events();
            service
                .unassign_app(app_id, channel_id)
                .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
            // Get updated channel info
            let info = service
                .state
                .channels
                .iter()
                .find(|c| c.id.to_string() == channel_id)
                .map(|c| c.to_channel_info());
            (app_id.to_string(), channel_id.to_string(), info)
        };

        // Emit signals after releasing the lock
        let _ = Self::app_unrouted(&ctx, &app_id_string, &channel_id_string).await;
        if let Some(info) = channel_info {
            let _ = Self::channel_updated(&ctx, info).await;
        }
        Ok(())
    }

    // ==================== Output Routing ====================

    /// Set the output device for a channel.
    async fn set_channel_output(
        &self,
        #[zbus(signal_context)] ctx: zbus::SignalContext<'_>,
        channel_id: &str,
        device_name: &str,
    ) -> zbus::fdo::Result<()> {
        validate::validate_device_name(device_name)?;
        debug!("D-Bus: set_channel_output({}, {})", channel_id, device_name);
        let channel_info = {
            let mut service = self
                .service
                .lock()
                .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
            service.process_pw_events();
            service
                .set_channel_output(channel_id, device_name)
                .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
            // Get updated channel info
            service
                .state
                .channels
                .iter()
                .find(|c| c.id.to_string() == channel_id)
                .map(|c| c.to_channel_info())
        };

        // Emit signal after releasing the lock
        if let Some(info) = channel_info {
            let _ = Self::channel_updated(&ctx, info).await;
        }
        Ok(())
    }

    /// Set the master output device.
    async fn set_master_output(
        &self,
        #[zbus(signal_context)] ctx: zbus::SignalContext<'_>,
        device_name: &str,
    ) -> zbus::fdo::Result<()> {
        validate::validate_device_name(device_name)?;
        debug!("D-Bus: set_master_output({})", device_name);
        {
            let mut service = self
                .service
                .lock()
                .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
            service.process_pw_events();
            service
                .set_master_output(device_name)
                .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        }

        // Emit signal after releasing the lock
        let _ = Self::outputs_changed(&ctx).await;
        Ok(())
    }

    // ==================== EQ ====================

    /// Toggle EQ on/off for a channel.
    async fn set_channel_eq_enabled(
        &self,
        channel_id: &str,
        enabled: bool,
    ) -> zbus::fdo::Result<()> {
        debug!("D-Bus: set_channel_eq_enabled({}, {})", channel_id, enabled);
        // TODO: Implement EQ
        Ok(())
    }

    /// Set EQ preset for a channel.
    async fn set_channel_eq_preset(
        &self,
        channel_id: &str,
        preset_name: &str,
    ) -> zbus::fdo::Result<()> {
        debug!(
            "D-Bus: set_channel_eq_preset({}, {})",
            channel_id, preset_name
        );
        // TODO: Implement EQ
        Ok(())
    }

    // ==================== Routing Rules ====================

    /// Get all routing rules.
    async fn get_routing_rules(&self) -> zbus::fdo::Result<Vec<RoutingRuleInfo>> {
        let service = self
            .service
            .lock()
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(service.state.get_routing_rules())
    }

    /// Add or update a routing rule.
    async fn set_routing_rule(&self, rule: RoutingRuleInfo) -> zbus::fdo::Result<()> {
        debug!("D-Bus: set_routing_rule({:?})", rule);
        // TODO: Implement routing rules management
        Ok(())
    }

    /// Delete a routing rule.
    async fn delete_routing_rule(&self, rule_id: &str) -> zbus::fdo::Result<()> {
        debug!("D-Bus: delete_routing_rule({})", rule_id);
        // TODO: Implement routing rules management
        Ok(())
    }

    /// Toggle a routing rule's enabled state.
    async fn toggle_routing_rule(&self, rule_id: &str) -> zbus::fdo::Result<()> {
        debug!("D-Bus: toggle_routing_rule({})", rule_id);
        // TODO: Implement routing rules management
        Ok(())
    }

    // ==================== Recording ====================

    /// Enable or disable master recording output.
    async fn set_master_recording(&self, enabled: bool) -> zbus::fdo::Result<()> {
        debug!("D-Bus: set_master_recording({})", enabled);
        let mut service = self
            .service
            .lock()
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        service.process_pw_events();
        service
            .set_master_recording(enabled)
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
    }

    // ==================== State Getters ====================

    /// Get all channels.
    async fn get_channels(&self) -> zbus::fdo::Result<Vec<ChannelInfo>> {
        let mut service = self
            .service
            .lock()
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        service.process_pw_events();
        Ok(service.state.get_channels())
    }

    /// Get all discovered apps.
    async fn get_apps(&self) -> zbus::fdo::Result<Vec<AppInfo>> {
        let mut service = self
            .service
            .lock()
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        service.process_pw_events();
        Ok(service.state.get_apps())
    }

    /// Get all output devices.
    async fn get_outputs(&self) -> zbus::fdo::Result<Vec<OutputInfo>> {
        let mut service = self
            .service
            .lock()
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        service.process_pw_events();
        Ok(service.state.get_outputs())
    }

    /// Get all input devices (microphones, line-in, etc).
    async fn get_inputs(&self) -> zbus::fdo::Result<Vec<InputInfo>> {
        let mut service = self
            .service
            .lock()
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        service.process_pw_events();
        Ok(service.state.get_inputs())
    }

    /// Get master volume in dB.
    async fn get_master_volume(&self) -> zbus::fdo::Result<f64> {
        let service = self
            .service
            .lock()
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(service.state.master_volume_db as f64)
    }

    /// Get master mute state.
    async fn get_master_muted(&self) -> zbus::fdo::Result<bool> {
        let service = self
            .service
            .lock()
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(service.state.master_muted)
    }

    /// Get selected master output device name.
    async fn get_master_output(&self) -> zbus::fdo::Result<String> {
        let service = self
            .service
            .lock()
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(service.state.master_output.clone().unwrap_or_default())
    }

    /// Get whether connected to PipeWire.
    async fn get_connected(&self) -> zbus::fdo::Result<bool> {
        let service = self
            .service
            .lock()
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(service.state.pw_connected)
    }

    /// Get whether master recording is enabled.
    async fn get_master_recording_enabled(&self) -> zbus::fdo::Result<bool> {
        let service = self
            .service
            .lock()
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(service.state.master_recording_enabled)
    }

    // ==================== Signals ====================

    /// Emitted when a channel is added.
    #[zbus(signal)]
    async fn channel_added(ctx: &zbus::SignalContext<'_>, channel: ChannelInfo)
        -> zbus::Result<()>;

    /// Emitted when a channel is removed.
    #[zbus(signal)]
    async fn channel_removed(ctx: &zbus::SignalContext<'_>, channel_id: &str) -> zbus::Result<()>;

    /// Emitted when channel volume changes.
    #[zbus(signal)]
    async fn volume_changed(
        ctx: &zbus::SignalContext<'_>,
        channel_id: &str,
        volume_db: f64,
    ) -> zbus::Result<()>;

    /// Emitted when channel mute state changes.
    #[zbus(signal)]
    async fn mute_changed(
        ctx: &zbus::SignalContext<'_>,
        channel_id: &str,
        muted: bool,
    ) -> zbus::Result<()>;

    /// Emitted when a new app is discovered.
    #[zbus(signal)]
    async fn app_discovered(ctx: &zbus::SignalContext<'_>, app: AppInfo) -> zbus::Result<()>;

    /// Emitted when an app is removed.
    #[zbus(signal)]
    async fn app_removed(ctx: &zbus::SignalContext<'_>, app_id: &str) -> zbus::Result<()>;

    /// Emitted when an app is routed to a channel.
    #[zbus(signal)]
    async fn app_routed(
        ctx: &zbus::SignalContext<'_>,
        app_id: &str,
        channel_id: &str,
    ) -> zbus::Result<()>;

    /// Emitted when an app is unrouted from a channel.
    #[zbus(signal)]
    async fn app_unrouted(
        ctx: &zbus::SignalContext<'_>,
        app_id: &str,
        channel_id: &str,
    ) -> zbus::Result<()>;

    /// Emitted when PipeWire connection state changes.
    #[zbus(signal)]
    async fn connection_changed(ctx: &zbus::SignalContext<'_>, connected: bool)
        -> zbus::Result<()>;

    /// Emitted when an error occurs.
    #[zbus(signal)]
    async fn error_occurred(ctx: &zbus::SignalContext<'_>, message: &str) -> zbus::Result<()>;

    /// Emitted with meter data updates.
    #[zbus(signal)]
    async fn meter_update(ctx: &zbus::SignalContext<'_>, data: Vec<MeterData>) -> zbus::Result<()>;

    /// Emitted when master volume changes.
    #[zbus(signal)]
    async fn master_volume_changed(
        ctx: &zbus::SignalContext<'_>,
        volume_db: f64,
    ) -> zbus::Result<()>;

    /// Emitted when master mute state changes.
    #[zbus(signal)]
    async fn master_mute_changed(ctx: &zbus::SignalContext<'_>, muted: bool) -> zbus::Result<()>;

    /// Emitted when output device list changes.
    #[zbus(signal)]
    async fn outputs_changed(ctx: &zbus::SignalContext<'_>) -> zbus::Result<()>;

    /// Emitted when input device list changes.
    #[zbus(signal)]
    async fn inputs_changed(ctx: &zbus::SignalContext<'_>) -> zbus::Result<()>;

    /// Emitted when a channel's properties change.
    #[zbus(signal)]
    async fn channel_updated(
        ctx: &zbus::SignalContext<'_>,
        channel: ChannelInfo,
    ) -> zbus::Result<()>;
}

// ==================== Public Signal Emission Helpers ====================
// These functions allow emitting D-Bus signals from outside the interface methods.

const INTERFACE_NAME: &str = "com.sootmix.Daemon";

/// Emit AppDiscovered signal.
pub async fn emit_app_discovered(
    ctx: &zbus::SignalContext<'_>,
    app: AppInfo,
) -> zbus::Result<()> {
    ctx.connection()
        .emit_signal(
            ctx.destination(),
            ctx.path(),
            INTERFACE_NAME,
            "AppDiscovered",
            &(app,),
        )
        .await
}

/// Emit AppRemoved signal.
pub async fn emit_app_removed(
    ctx: &zbus::SignalContext<'_>,
    app_id: &str,
) -> zbus::Result<()> {
    ctx.connection()
        .emit_signal(
            ctx.destination(),
            ctx.path(),
            INTERFACE_NAME,
            "AppRemoved",
            &(app_id,),
        )
        .await
}

/// Emit OutputsChanged signal.
pub async fn emit_outputs_changed(ctx: &zbus::SignalContext<'_>) -> zbus::Result<()> {
    ctx.connection()
        .emit_signal(
            ctx.destination(),
            ctx.path(),
            INTERFACE_NAME,
            "OutputsChanged",
            &(),
        )
        .await
}

/// Emit InputsChanged signal.
pub async fn emit_inputs_changed(ctx: &zbus::SignalContext<'_>) -> zbus::Result<()> {
    ctx.connection()
        .emit_signal(
            ctx.destination(),
            ctx.path(),
            INTERFACE_NAME,
            "InputsChanged",
            &(),
        )
        .await
}
