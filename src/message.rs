// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Message types for UI actions and PipeWire events.

use crate::audio::types::{PwLink, PwNode, PwPort};
use crate::config::eq_preset::EqPreset;
use crate::daemon_client::DaemonEvent;
use crate::state::SnapshotSlot;
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
    /// Start editing a channel name.
    StartEditingChannelName(Uuid),
    /// Update channel name edit text.
    ChannelNameEditChanged(String),
    /// Cancel channel name editing.
    CancelEditingChannelName,
    /// App assigned to channel (channel_id, app_identifier).
    AppAssigned(Uuid, String),
    /// App unassigned from channel (channel_id, app_identifier).
    AppUnassigned(Uuid, String),
    /// Channel output device changed (channel_id, device_name or None for default).
    ChannelOutputDeviceChanged(Uuid, Option<String>),
    /// Start dragging an app for assignment (node_id, app_identifier).
    StartDraggingApp(u32, String),
    /// Cancel the current drag operation.
    CancelDrag,
    /// Drop the dragged app onto a channel.
    DropAppOnChannel(Uuid),

    /// Master volume changed.
    MasterVolumeChanged(f32),
    /// Master volume slider released.
    MasterVolumeReleased,
    /// Master mute toggled.
    MasterMuteToggled,
    /// Output device changed.
    OutputDeviceChanged(String),
    /// Toggle master recording output.
    ToggleMasterRecording,

    /// Request to create a new channel.
    NewChannelRequested,
    /// Global preset selected.
    PresetSelected(String),
    /// Save current configuration as preset.
    PresetSaved(String),
    /// Delete a preset.
    PresetDeleted(String),

    /// Select a channel for the focus panel (None to deselect).
    SelectChannel(Option<Uuid>),
    /// Toggle the left sidebar collapsed state.
    ToggleLeftSidebar,
    /// Toggle the bottom detail panel expanded state.
    ToggleBottomPanel,

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

    // ==================== Routing Rules ====================
    /// Open the routing rules panel.
    OpenRoutingRulesPanel,
    /// Close the routing rules panel.
    CloseRoutingRulesPanel,
    /// Toggle a routing rule's enabled state.
    ToggleRoutingRule(Uuid),
    /// Delete a routing rule.
    DeleteRoutingRule(Uuid),
    /// Move a routing rule up in priority.
    MoveRoutingRuleUp(Uuid),
    /// Move a routing rule down in priority.
    MoveRoutingRuleDown(Uuid),
    /// Start editing a routing rule (None for new rule).
    StartEditingRule(Option<Uuid>),
    /// Cancel editing a routing rule.
    CancelEditingRule,
    /// Update the rule name field.
    RuleNameChanged(String),
    /// Update the rule pattern field.
    RulePatternChanged(String),
    /// Update the rule match type.
    RuleMatchTypeChanged(String),
    /// Update the rule match target.
    RuleMatchTargetChanged(crate::config::MatchTarget),
    /// Update the rule target channel.
    RuleTargetChannelChanged(String),
    /// Update the rule priority.
    RulePriorityChanged(String),
    /// Save the current rule being edited.
    SaveRoutingRule,
    /// Create a quick rule from an app (app_name, binary, target_channel).
    CreateQuickRule(String, Option<String>, String),

    // ==================== Snapshot A/B Comparison ====================
    /// Capture current mixer state to a snapshot slot.
    CaptureSnapshot(SnapshotSlot),
    /// Recall (apply) a snapshot from a slot.
    RecallSnapshot(SnapshotSlot),
    /// Clear a snapshot slot.
    ClearSnapshot(SnapshotSlot),
    /// Save a single channel's current state to the active snapshot.
    SaveChannelToSnapshot(Uuid),

    // ==================== Plugin Chain ====================
    /// Open the plugin browser for a channel.
    OpenPluginBrowser(Uuid),
    /// Close the plugin browser.
    ClosePluginBrowser,
    /// Add a plugin to a channel's chain (channel_id, plugin_id).
    AddPluginToChannel(Uuid, String),
    /// Remove a plugin from a channel's chain (channel_id, instance_id).
    RemovePluginFromChannel(Uuid, Uuid),
    /// Move a plugin in the chain (channel_id, instance_id, direction: -1=up, 1=down).
    MovePluginInChain(Uuid, Uuid, i32),
    /// Toggle plugin bypass (channel_id, instance_id).
    TogglePluginBypass(Uuid, Uuid),
    /// Open the plugin parameter editor (channel_id, instance_id).
    OpenPluginEditor(Uuid, Uuid),
    /// Close the plugin editor.
    ClosePluginEditor,
    /// Plugin parameter changed (instance_id, param_index, value).
    PluginParameterChanged(Uuid, u32, f32),
    /// Plugin chain loaded from persistence (channel_id).
    PluginChainLoaded(Uuid),
    /// Plugin sidechain source changed (channel_id, slot_index, source_channel_id or None).
    PluginSidechainSourceChanged(Uuid, usize, Option<Uuid>),

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
    /// Save current configuration to disk.
    SaveConfig,
    /// Configuration loaded from disk.
    ConfigLoaded(crate::config::MixerConfig),
    /// Startup: wait for PipeWire discovery before restoring channels.
    StartupDiscoveryComplete,

    // ==================== Window & Tray ====================
    /// Window close requested (from window manager).
    WindowCloseRequested(iced::window::Id),
    /// Show window (from tray).
    TrayShowWindow,
    /// Toggle mute all (from tray).
    TrayToggleMuteAll,
    /// Quit application (from tray).
    TrayQuit,

    // ==================== Daemon Events (from D-Bus) ====================
    /// Event received from the SootMix daemon.
    Daemon(DaemonEvent),
}
