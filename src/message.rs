// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Message types for UI actions and PipeWire events.

use crate::audio::types::{PwLink, PwNode, PwPort};
use crate::config::eq_preset::EqPreset;
use uuid::Uuid;

/// All messages in the application.
#[derive(Debug, Clone)]
pub enum Message {
    // ==================== UI Actions ====================
    /// Channel volume changed (channel_id, volume_db).
    ChannelVolumeChanged(Uuid, f32),
    /// Channel volume slider released (for committing changes).
    ChannelVolumeReleased(Uuid),
    /// Channel mute toggled.
    ChannelMuteToggled(Uuid),
    /// Channel EQ enabled/disabled.
    ChannelEqToggled(Uuid),
    /// Channel EQ preset changed (channel_id, preset_name).
    ChannelEqPresetChanged(Uuid, String),
    /// Channel deleted.
    ChannelDeleted(Uuid),
    /// Channel renamed (channel_id, new_name).
    ChannelRenamed(Uuid, String),
    /// App assigned to channel (channel_id, app_identifier).
    AppAssigned(Uuid, String),
    /// App unassigned from channel (channel_id, app_identifier).
    AppUnassigned(Uuid, String),

    /// Master volume changed.
    MasterVolumeChanged(f32),
    /// Master volume slider released.
    MasterVolumeReleased,
    /// Master mute toggled.
    MasterMuteToggled,
    /// Output device changed.
    OutputDeviceChanged(String),

    /// Request to create a new channel.
    NewChannelRequested,
    /// Global preset selected.
    PresetSelected(String),
    /// Save current configuration as preset.
    PresetSaved(String),
    /// Delete a preset.
    PresetDeleted(String),

    /// Open EQ panel for channel.
    OpenEqPanel(Uuid),
    /// Close EQ panel.
    CloseEqPanel,
    /// EQ band gain changed (band_index, gain_db).
    EqBandChanged(usize, f32),
    /// EQ Q factor changed (band_index, q).
    EqQChanged(usize, f32),
    /// Reset EQ to flat.
    EqReset,
    /// Save current EQ as preset.
    EqPresetSaved(String),

    /// Open settings modal.
    OpenSettings,
    /// Close settings modal.
    CloseSettings,

    // ==================== PipeWire Events (from PW thread) ====================
    /// PipeWire connection established.
    PwConnected,
    /// PipeWire connection lost.
    PwDisconnected,
    /// New node appeared in the graph.
    PwNodeAdded(PwNode),
    /// Node removed from the graph.
    PwNodeRemoved(u32),
    /// Node properties changed.
    PwNodeChanged(PwNode),
    /// New port appeared.
    PwPortAdded(PwPort),
    /// Port removed.
    PwPortRemoved(u32),
    /// New link created.
    PwLinkAdded(PwLink),
    /// Link removed.
    PwLinkRemoved(u32),
    /// Virtual sink successfully created (channel_id, pw_node_id).
    PwVirtualSinkCreated(Uuid, u32),
    /// Virtual sink destroyed.
    PwVirtualSinkDestroyed(u32),
    /// Error from PipeWire thread.
    PwError(String),

    // ==================== Commands to PipeWire Thread ====================
    /// Create a virtual sink (channel_id, name).
    CreateVirtualSink(Uuid, String),
    /// Destroy a virtual sink by PW node ID.
    DestroyVirtualSink(u32),
    /// Create a link between ports (output_port_id, input_port_id).
    CreateLink(u32, u32),
    /// Destroy a link by ID.
    DestroyLink(u32),
    /// Set volume on a node (node_id, volume as linear 0.0-1.0+).
    SetNodeVolume(u32, f32),
    /// Set mute state on a node.
    SetNodeMute(u32, bool),
    /// Load EQ filter for channel (channel_id, preset).
    LoadEqFilter(Uuid, EqPreset),
    /// Update EQ filter parameters (node_id, preset).
    UpdateEqFilter(u32, EqPreset),
    /// Unload EQ filter by node ID.
    UnloadEqFilter(u32),

    // ==================== Internal ====================
    /// Tick for periodic updates (e.g., checking PW state).
    Tick,
    /// Application initialization complete.
    Initialized,
    /// Font loaded.
    FontLoaded(Result<(), iced::font::Error>),
}
