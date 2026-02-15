// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Application state management.

#![allow(dead_code)]

use crate::audio::types::{
    AudioChannel, InputDevice, MediaClass, OutputDevice, PortDirection, PwLink, PwNode, PwPort,
};
use crate::config::RoutingRulesConfig;
use crate::plugins::PluginSlotConfig;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use tracing::debug;
use uuid::Uuid;

/// Which snapshot slot (A or B) for A/B comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotSlot {
    A,
    B,
}

/// Snapshot of a single channel's audio-relevant state.
#[derive(Debug, Clone)]
pub struct ChannelSnapshot {
    /// Channel ID to match when applying.
    pub id: Uuid,
    /// Volume in dB.
    pub volume_db: f32,
    /// Mute state.
    pub muted: bool,
    /// EQ enabled.
    pub eq_enabled: bool,
    /// EQ preset name.
    pub eq_preset: String,
}

/// Snapshot of the entire mixer state for A/B comparison.
#[derive(Debug, Clone)]
pub struct MixerSnapshot {
    /// Snapshots of each channel.
    pub channels: Vec<ChannelSnapshot>,
    /// Master volume in dB.
    pub master_volume_db: f32,
    /// Master mute state.
    pub master_muted: bool,
}

/// VU meter display state for stereo audio levels.
#[derive(Debug, Clone, Default)]
pub struct MeterDisplayState {
    /// Current left channel level (0.0 to 1.0+, where 1.0 = 0dB).
    pub level_left: f32,
    /// Current right channel level (0.0 to 1.0+).
    pub level_right: f32,
    /// Peak hold level for left channel.
    pub peak_hold_left: f32,
    /// Peak hold level for right channel.
    pub peak_hold_right: f32,
    /// Time since peak hold was set for left channel (seconds).
    pub peak_hold_time_left: f32,
    /// Time since peak hold was set for right channel (seconds).
    pub peak_hold_time_right: f32,
    /// Left channel has clipped (reached 0dBFS).
    pub clipped_left: bool,
    /// Right channel has clipped (reached 0dBFS).
    pub clipped_right: bool,
    /// Time since left channel last clipped (seconds). Used for auto-reset.
    pub clip_time_left: f32,
    /// Time since right channel last clipped (seconds). Used for auto-reset.
    pub clip_time_right: f32,
}

impl MeterDisplayState {
    /// Clip threshold (0dBFS in linear scale).
    const CLIP_THRESHOLD: f32 = 1.0;
    /// Peak hold time in seconds (industry standard: 2-3 seconds).
    const PEAK_HOLD_TIME: f32 = 2.5;
    /// Peak decay rate in linear units per second (IEC 60268-18 standard: ~20dB/s).
    const PEAK_DECAY_RATE: f32 = 0.8;
    /// Clip indicator auto-reset time in seconds.
    const CLIP_RESET_TIME: f32 = 3.0;

    /// Attack time constant in seconds (fast attack to catch transients).
    const ATTACK_TIME: f32 = 0.015;
    /// Decay time constant in seconds (slower decay for smooth falloff).
    const DECAY_TIME: f32 = 0.25;

    /// Calculate time-based exponential smoothing coefficient.
    /// Returns alpha for: new_value = old_value + alpha * (target - old_value)
    #[inline]
    fn smooth_coeff(dt: f32, time_constant: f32) -> f32 {
        1.0 - (-dt / time_constant).exp()
    }

    /// Update meter with new levels. Applies smoothing and peak hold logic.
    /// `dt` is the delta time since last update in seconds.
    pub fn update(&mut self, new_left: f32, new_right: f32, dt: f32) {
        // Time-based exponential smoothing for frame-rate independent behavior
        let attack_coeff = Self::smooth_coeff(dt, Self::ATTACK_TIME);
        let decay_coeff = Self::smooth_coeff(dt, Self::DECAY_TIME);

        // Apply attack/decay smoothing to levels
        if new_left > self.level_left {
            self.level_left += attack_coeff * (new_left - self.level_left);
        } else {
            self.level_left += decay_coeff * (new_left - self.level_left);
        }

        if new_right > self.level_right {
            self.level_right += attack_coeff * (new_right - self.level_right);
        } else {
            self.level_right += decay_coeff * (new_right - self.level_right);
        }

        // Clip detection with auto-reset after CLIP_RESET_TIME seconds
        if new_left >= Self::CLIP_THRESHOLD {
            self.clipped_left = true;
            self.clip_time_left = 0.0;
        } else if self.clipped_left {
            self.clip_time_left += dt;
            if self.clip_time_left >= Self::CLIP_RESET_TIME {
                self.clipped_left = false;
            }
        }
        if new_right >= Self::CLIP_THRESHOLD {
            self.clipped_right = true;
            self.clip_time_right = 0.0;
        } else if self.clipped_right {
            self.clip_time_right += dt;
            if self.clip_time_right >= Self::CLIP_RESET_TIME {
                self.clipped_right = false;
            }
        }

        // Left channel peak hold
        if new_left >= self.peak_hold_left {
            self.peak_hold_left = new_left;
            self.peak_hold_time_left = 0.0;
        } else {
            self.peak_hold_time_left += dt;
            if self.peak_hold_time_left > Self::PEAK_HOLD_TIME {
                self.peak_hold_left -= Self::PEAK_DECAY_RATE * dt;
                if self.peak_hold_left < self.level_left {
                    self.peak_hold_left = self.level_left;
                }
            }
        }

        // Right channel peak hold
        if new_right >= self.peak_hold_right {
            self.peak_hold_right = new_right;
            self.peak_hold_time_right = 0.0;
        } else {
            self.peak_hold_time_right += dt;
            if self.peak_hold_time_right > Self::PEAK_HOLD_TIME {
                self.peak_hold_right -= Self::PEAK_DECAY_RATE * dt;
                if self.peak_hold_right < self.level_right {
                    self.peak_hold_right = self.level_right;
                }
            }
        }
    }

    /// Reset meter to zero.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Reset clip indicators only (called when user clicks on clip indicator).
    pub fn reset_clip(&mut self) {
        self.clipped_left = false;
        self.clipped_right = false;
    }

    /// Check if either channel has clipped.
    pub fn has_clipped(&self) -> bool {
        self.clipped_left || self.clipped_right
    }
}

// Re-export ChannelKind from the IPC crate for consistency.
pub use sootmix_ipc::ChannelKind;

/// Filter for which channels to display.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChannelFilter {
    #[default]
    All,
    Outputs,
    Inputs,
}

/// A virtual mixer channel created by SootMix.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MixerChannel {
    /// Unique identifier for this channel.
    pub id: Uuid,
    /// Display name.
    pub name: String,
    /// Volume in decibels (-60.0 to +12.0).
    pub volume_db: f32,
    /// Whether the channel is muted.
    pub muted: bool,
    /// Whether EQ is enabled for this channel.
    pub eq_enabled: bool,
    /// Name of the EQ preset applied.
    pub eq_preset: String,
    /// App identifiers assigned to this channel.
    pub assigned_apps: Vec<String>,
    /// Whether this channel owns its sink (managed) or just controls it (adopted).
    /// Managed sinks are created/destroyed by SootMix.
    /// Adopted sinks are user-created sinks that SootMix only controls.
    #[serde(default = "default_is_managed")]
    pub is_managed: bool,
    /// PipeWire sink name for matching on startup (for adopted sinks).
    #[serde(default)]
    pub sink_name: Option<String>,
    /// Runtime PipeWire node ID for the virtual sink (not serialized).
    #[serde(skip)]
    pub pw_sink_id: Option<u32>,
    /// Runtime PipeWire node ID for the EQ filter-chain (not serialized).
    #[serde(skip)]
    pub pw_eq_node_id: Option<u32>,
    /// VU meter display state (not serialized).
    #[serde(skip)]
    pub meter_display: MeterDisplayState,
    /// Plugin chain configuration for persistence.
    #[serde(default)]
    pub plugin_chain: Vec<PluginSlotConfig>,
    /// Runtime plugin instance IDs (not serialized).
    /// These are the Uuids of loaded PluginInstance objects in the PluginManager.
    #[serde(skip)]
    pub plugin_instances: Vec<Uuid>,
    /// Output device node ID for this channel (not serialized).
    /// None means use default output device.
    #[serde(skip)]
    pub output_device_id: Option<u32>,
    /// Output device name for persistence (description of the device).
    /// None means use default output device.
    #[serde(default)]
    pub output_device_name: Option<String>,
    /// Runtime loopback output node ID (not serialized).
    /// This is the Stream/Output/Audio node created by pw-loopback for routing.
    #[serde(skip)]
    pub pw_loopback_output_id: Option<u32>,
    /// Atomic meter levels for real-time audio metering (not serialized).
    /// Shared with the audio processing thread for lock-free level updates.
    #[serde(skip)]
    pub meter_levels: Option<std::sync::Arc<crate::audio::meter_stream::AtomicMeterLevels>>,

    // ==================== Input Channel Fields ====================
    /// Whether this channel is an output (app routing) or input (mic capture) channel.
    #[serde(default)]
    pub kind: ChannelKind,
    /// Input device name for persistence (description of the mic/input device).
    /// Only used when kind == Input.
    #[serde(default)]
    pub input_device_name: Option<String>,
    /// Runtime input device node ID (not serialized).
    #[serde(skip)]
    pub input_device_id: Option<u32>,
    /// Runtime virtual source node ID (Audio/Source created by pw-loopback).
    /// Apps (Discord, OBS) connect to this to get processed mic audio.
    #[serde(skip)]
    pub pw_source_id: Option<u32>,
    /// Runtime loopback capture node ID for input channels.
    /// This is the Stream/Input/Audio node that captures from the physical device.
    #[serde(skip)]
    pub pw_loopback_capture_id: Option<u32>,
    /// Whether sidetone (input monitoring) is enabled.
    /// Routes processed mic audio to the headphone output.
    #[serde(default)]
    pub sidetone_enabled: bool,
    /// Sidetone volume in decibels (-60.0 to 0.0).
    #[serde(default = "default_sidetone_db")]
    pub sidetone_volume_db: f32,
    /// Whether RNNoise noise suppression is enabled for this input channel.
    #[serde(default)]
    pub noise_suppression_enabled: bool,
    /// VAD threshold for noise suppression (0-100%). Higher = more aggressive noise gating.
    #[serde(default = "default_vad_threshold")]
    pub vad_threshold: f32,
    /// Hardware microphone gain in dB (-12.0 to +12.0). Controls the physical input device level.
    #[serde(default)]
    pub input_gain_db: f32,
}

fn default_vad_threshold() -> f32 {
    95.0
}

fn default_is_managed() -> bool {
    true
}

fn default_sidetone_db() -> f32 {
    -20.0
}

impl MixerChannel {
    /// Create a new output channel (routes app audio to output device).
    pub fn new(name: impl Into<String>) -> Self {
        use crate::audio::meter_stream::AtomicMeterLevels;
        use std::sync::Arc;

        let name = name.into();
        Self {
            id: Uuid::new_v4(),
            sink_name: Some(format!("sootmix.{}", name)),
            name,
            volume_db: 0.0,
            muted: false,
            eq_enabled: false,
            eq_preset: "Flat".to_string(),
            assigned_apps: Vec::new(),
            is_managed: true,
            pw_sink_id: None,
            pw_eq_node_id: None,
            meter_display: MeterDisplayState::default(),
            plugin_chain: Vec::new(),
            plugin_instances: Vec::new(),
            output_device_id: None,
            output_device_name: None,
            pw_loopback_output_id: None,
            meter_levels: Some(Arc::new(AtomicMeterLevels::new())),
            kind: ChannelKind::Output,
            input_device_name: None,
            input_device_id: None,
            pw_source_id: None,
            pw_loopback_capture_id: None,
            sidetone_enabled: false,
            sidetone_volume_db: -20.0,
            noise_suppression_enabled: false,
            vad_threshold: 95.0,
            input_gain_db: 0.0,
        }
    }

    /// Create a new input channel (captures from mic/input device).
    pub fn new_input(name: impl Into<String>) -> Self {
        use crate::audio::meter_stream::AtomicMeterLevels;
        use std::sync::Arc;

        let name = name.into();
        Self {
            id: Uuid::new_v4(),
            sink_name: Some(format!("sootmix.{}", name)),
            kind: ChannelKind::Input,
            name,
            volume_db: 0.0,
            muted: false,
            eq_enabled: false,
            eq_preset: "Flat".to_string(),
            assigned_apps: Vec::new(),
            is_managed: true,
            pw_sink_id: None,
            pw_eq_node_id: None,
            meter_display: MeterDisplayState::default(),
            plugin_chain: Vec::new(),
            plugin_instances: Vec::new(),
            output_device_id: None,
            output_device_name: None,
            pw_loopback_output_id: None,
            meter_levels: Some(Arc::new(AtomicMeterLevels::new())),
            input_device_name: Some("system-default".to_string()),
            input_device_id: None,
            pw_source_id: None,
            pw_loopback_capture_id: None,
            sidetone_enabled: false,
            sidetone_volume_db: -20.0,
            noise_suppression_enabled: false,
            vad_threshold: 95.0,
            input_gain_db: 0.0,
        }
    }

    /// Whether this is an input (mic) channel.
    pub fn is_input(&self) -> bool {
        self.kind == ChannelKind::Input
    }

    /// Convert volume in dB to linear scale (0.0 to ~4.0 for +12dB).
    pub fn volume_linear(&self) -> f32 {
        if self.muted {
            0.0
        } else {
            db_to_linear(self.volume_db)
        }
    }
}

/// Convert decibels to linear volume.
pub fn db_to_linear(db: f32) -> f32 {
    if db <= -60.0 {
        0.0
    } else {
        10.0_f32.powf(db / 20.0)
    }
}

/// Convert linear volume to decibels.
pub fn linear_to_db(linear: f32) -> f32 {
    if linear <= 0.0 {
        -60.0
    } else {
        20.0 * linear.log10()
    }
}

/// Well-known domain-to-friendly-name mapping for Chromium PWAs.
fn friendly_name_from_url(url: &str) -> Option<String> {
    let domain = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url)
        .split('/')
        .next()?;

    let name = match domain {
        "music.youtube.com" => "YouTube Music",
        "youtube.com" | "www.youtube.com" => "YouTube",
        "discord.com" | "www.discord.com" => "Discord",
        "open.spotify.com" => "Spotify Web",
        "web.whatsapp.com" => "WhatsApp",
        "web.telegram.org" => "Telegram",
        "meet.google.com" => "Google Meet",
        "teams.microsoft.com" => "Teams",
        "netflix.com" | "www.netflix.com" => "Netflix",
        "twitch.tv" | "www.twitch.tv" => "Twitch",
        "soundcloud.com" | "www.soundcloud.com" => "SoundCloud",
        "tidal.com" | "listen.tidal.com" => "Tidal",
        _ => {
            let parts: Vec<&str> = domain.split('.').collect();
            let main = if parts.len() >= 2 {
                parts[parts.len() - 2]
            } else {
                parts[0]
            };
            return Some(prettify_package_name(main));
        }
    };
    Some(name.to_string())
}

/// Extract app name from an Electron .asar path or binary path.
fn app_name_from_path(path: &str) -> Option<String> {
    let p = std::path::Path::new(path);
    if path.contains("app.asar") {
        let parent = p.parent()?;
        let dir_name = parent.file_name()?.to_str()?;
        Some(prettify_package_name(dir_name))
    } else {
        let stem = p.file_stem()?.to_str()?;
        if matches!(stem, "electron" | "chromium" | "chrome" | "google-chrome") {
            None
        } else {
            Some(prettify_package_name(stem))
        }
    }
}

/// Convert a package-style name to a friendly display name.
fn prettify_package_name(name: &str) -> String {
    name.split(|c: char| c == '-' || c == '_')
        .filter(|s| !s.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(c) => {
                    let upper: String = c.to_uppercase().collect();
                    format!("{}{}", upper, chars.as_str())
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Check cmdline args for Chromium PWA, Electron, or user-data-dir identifiers.
fn identify_from_cmdline_args(args: &[String]) -> Option<String> {
    // Check for --app=<URL> (Chromium PWA)
    for arg in args {
        if let Some(url) = arg.strip_prefix("--app=") {
            if let Some(name) = friendly_name_from_url(url) {
                return Some(name);
            }
        }
    }
    // Check for Electron app (.asar in args)
    for arg in args {
        if arg.contains(".asar") {
            if let Some(name) = app_name_from_path(arg) {
                return Some(name);
            }
        }
    }
    // Check --user-data-dir (Electron/Chromium audio subprocesses)
    // e.g., --user-data-dir=/home/soot/.config/YouTube Music → "YouTube Music"
    for arg in args {
        if let Some(dir) = arg.strip_prefix("--user-data-dir=") {
            if let Some(name) = std::path::Path::new(dir).file_name().and_then(|n| n.to_str()) {
                if !name.is_empty() && !matches!(name, "chromium" | "chrome" | "google-chrome" | "BraveSoftware" | "microsoft-edge") {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
}

/// Resolve a friendly app name by reading /proc/<pid>/cmdline.
fn resolve_app_name_from_pid(pid_str: &str) -> Option<String> {
    let cmdline = std::fs::read(format!("/proc/{}/cmdline", pid_str)).ok()?;
    let args: Vec<String> = cmdline
        .split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect();

    if args.is_empty() {
        return None;
    }

    if let Some(name) = identify_from_cmdline_args(&args) {
        return Some(name);
    }

    // Walk up the process tree to find identifying info
    if let Some(name) = resolve_name_from_parent_pid(pid_str) {
        return Some(name);
    }

    // Check binary path for distinctive name
    app_name_from_path(&args[0])
}

fn resolve_name_from_parent_pid(pid_str: &str) -> Option<String> {
    let mut current_pid = pid_str.to_string();
    for _ in 0..10 {
        let status = std::fs::read_to_string(format!("/proc/{}/status", current_pid)).ok()?;
        let ppid = status
            .lines()
            .find(|l| l.starts_with("PPid:"))?
            .split_whitespace()
            .nth(1)?
            .to_string();

        if ppid == "0" || ppid == "1" {
            return None;
        }

        let cmdline = std::fs::read(format!("/proc/{}/cmdline", &ppid)).ok()?;
        let args: Vec<String> = cmdline
            .split(|&b| b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf8_lossy(s).into_owned())
            .collect();

        if let Some(name) = identify_from_cmdline_args(&args) {
            return Some(name);
        }
        if let Some(first) = args.first() {
            if let Some(name) = app_name_from_path(first) {
                return Some(name);
            }
        }

        current_pid = ppid;
    }
    None
}

/// Information about an audio application.
#[derive(Debug, Clone)]
pub struct AppInfo {
    /// PipeWire node ID.
    pub node_id: u32,
    /// Application name (from PipeWire properties).
    pub name: String,
    /// Binary name for pattern matching.
    pub binary: Option<String>,
    /// Icon name hint (if available).
    pub icon: Option<String>,
    /// Media name from PipeWire (e.g., page title for Chromium PWAs).
    pub media_name: Option<String>,
}

/// Check if a media name is generic/unhelpful for identification.
fn is_generic_media_name(name: &str) -> bool {
    matches!(
        name,
        "Playback" | "Audio Stream" | "audio-volume-change" | "AudioStream"
    )
}

/// Check if an app's name/binary is generic (Chromium/Electron), meaning we
/// should prefer media_name or PID-resolved name for identification.
pub fn is_generic_app_identity(name: &str, binary: &str) -> bool {
    let generic_names = [
        "Chromium",
        "Chrome",
        "Google Chrome",
        "Microsoft Edge",
        "Brave Browser",
    ];
    let generic_binaries = [
        "electron",
        "chromium",
        "chrome",
        "google-chrome",
        "msedge",
        "brave",
    ];

    generic_names.iter().any(|g| name == *g)
        || generic_binaries.iter().any(|g| binary == *g)
}

impl AppInfo {
    /// Get identifier used for matching and assignment.
    /// Only prefers media_name for generic Chromium/Electron apps where the app name
    /// is unhelpful. For distinctive apps (Firefox, Zen, etc.), uses app name/binary.
    pub fn identifier(&self) -> &str {
        let binary = self.binary.as_deref().unwrap_or("");
        if is_generic_app_identity(&self.name, binary) {
            if let Some(ref media) = self.media_name {
                if !media.is_empty() && !is_generic_media_name(media) {
                    return media;
                }
            }
        }
        if !binary.is_empty() {
            binary
        } else {
            &self.name
        }
    }
}

/// Current PipeWire graph state.
#[derive(Debug, Default)]
pub struct PwGraphState {
    /// All known nodes by ID.
    pub nodes: HashMap<u32, PwNode>,
    /// All known ports by ID.
    pub ports: HashMap<u32, PwPort>,
    /// All known links by ID.
    pub links: HashMap<u32, PwLink>,
}

/// State for editing a routing rule in the UI.
#[derive(Debug, Clone)]
pub struct EditingRule {
    /// Rule ID (None for new rule).
    pub id: Option<Uuid>,
    /// Rule name.
    pub name: String,
    /// Match target.
    pub match_target: crate::config::MatchTarget,
    /// Match type name ("contains", "exact", "regex", "glob").
    pub match_type_name: String,
    /// Pattern string.
    pub pattern: String,
    /// Target channel name.
    pub target_channel: String,
    /// Priority value.
    pub priority: u32,
}

impl Default for EditingRule {
    fn default() -> Self {
        Self {
            id: None,
            name: String::new(),
            match_target: crate::config::MatchTarget::Either,
            match_type_name: "contains".to_string(),
            pattern: String::new(),
            target_channel: String::new(),
            priority: 100,
        }
    }
}

impl EditingRule {
    /// Create from an existing rule for editing.
    pub fn from_rule(rule: &crate::config::RoutingRule) -> Self {
        Self {
            id: Some(rule.id),
            name: rule.name.clone(),
            match_target: rule.match_target,
            match_type_name: rule.match_type.type_name().to_string(),
            pattern: rule.match_type.pattern().to_string(),
            target_channel: rule.target_channel.clone(),
            priority: rule.priority,
        }
    }

    /// Convert to a RoutingRule.
    pub fn to_rule(&self) -> crate::config::RoutingRule {
        use crate::config::MatchType;

        let match_type = match self.match_type_name.as_str() {
            "exact" => MatchType::Exact(self.pattern.clone()),
            "regex" => MatchType::Regex(self.pattern.clone()),
            "glob" => MatchType::Glob(self.pattern.clone()),
            _ => MatchType::Contains(self.pattern.clone()),
        };

        crate::config::RoutingRule {
            id: self.id.unwrap_or_else(Uuid::new_v4),
            name: self.name.clone(),
            match_target: self.match_target,
            match_type,
            target_channel: self.target_channel.clone(),
            enabled: true,
            priority: self.priority,
        }
    }
}

impl PwGraphState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get all audio playback streams (apps playing audio).
    pub fn playback_streams(&self) -> Vec<&PwNode> {
        self.nodes
            .values()
            .filter(|n| n.is_playback_stream())
            .collect()
    }

    /// Get all audio sinks (output devices and virtual sinks).
    pub fn audio_sinks(&self) -> Vec<&PwNode> {
        self.nodes.values().filter(|n| n.is_sink()).collect()
    }

    /// Get all output devices (hardware sinks, excluding our virtual ones).
    pub fn output_devices(&self, exclude_names: &[&str]) -> Vec<OutputDevice> {
        self.nodes
            .values()
            .filter(|n| {
                n.media_class == MediaClass::AudioSink
                    && !exclude_names.iter().any(|ex| n.name.contains(ex))
            })
            .map(|n| OutputDevice {
                node_id: n.id,
                name: n.name.clone(),
                description: n.description.clone(),
            })
            .collect()
    }

    /// Get ports for a specific node.
    pub fn ports_for_node(&self, node_id: u32) -> Vec<&PwPort> {
        self.ports
            .values()
            .filter(|p| p.node_id == node_id)
            .collect()
    }

    /// Find a link between two nodes.
    pub fn find_link(&self, output_node: u32, input_node: u32) -> Option<&PwLink> {
        self.links
            .values()
            .find(|l| l.output_node == output_node && l.input_node == input_node)
    }

    /// Find all links FROM a node (outgoing links).
    /// Used to disconnect an app from its current sinks before re-routing.
    pub fn links_from_node(&self, node_id: u32) -> Vec<&PwLink> {
        self.links
            .values()
            .filter(|l| l.output_node == node_id)
            .collect()
    }

    /// Find all links TO a node (incoming links).
    /// Used to disconnect existing input sources before routing a new one.
    pub fn links_to_node(&self, node_id: u32) -> Vec<&PwLink> {
        self.links
            .values()
            .filter(|l| l.input_node == node_id)
            .collect()
    }

    /// Get output ports for a node (ports that send audio out).
    pub fn output_ports_for_node(&self, node_id: u32) -> Vec<&PwPort> {
        self.ports
            .values()
            .filter(|p| p.node_id == node_id && p.direction == PortDirection::Output)
            .collect()
    }

    /// Get input ports for a node (ports that receive audio).
    pub fn input_ports_for_node(&self, node_id: u32) -> Vec<&PwPort> {
        self.ports
            .values()
            .filter(|p| p.node_id == node_id && p.direction == PortDirection::Input)
            .collect()
    }

    /// Find matching port pairs for linking two nodes (matches by audio channel).
    /// Returns pairs of (output_port_id, input_port_id).
    ///
    /// Handles mono→stereo expansion: a single mono output port connects to both
    /// FL and FR input ports so audio plays on both channels.
    pub fn find_port_pairs(&self, output_node: u32, input_node: u32) -> Vec<(u32, u32)> {
        let output_ports = self.output_ports_for_node(output_node);
        let input_ports = self.input_ports_for_node(input_node);

        let mut pairs = Vec::new();

        // Check for mono→stereo case: single output port, multiple input ports
        let is_mono_source = output_ports.len() == 1;
        let is_stereo_dest = input_ports.len() >= 2;

        if is_mono_source && is_stereo_dest {
            // Mono→stereo: connect the single output port to ALL input ports
            // This ensures mono mic audio plays on both L and R channels
            let out_port = &output_ports[0];
            for in_port in &input_ports {
                pairs.push((out_port.id, in_port.id));
            }
            return pairs;
        }

        for out_port in &output_ports {
            // Try to find matching input port by channel
            for in_port in &input_ports {
                let out_channel = &out_port.channel;
                let in_channel = &in_port.channel;

                // Match by channel, or if both are mono/unknown, just pair them
                let is_match = match (out_channel, in_channel) {
                    (AudioChannel::FrontLeft, AudioChannel::FrontLeft) => true,
                    (AudioChannel::FrontRight, AudioChannel::FrontRight) => true,
                    (AudioChannel::FrontCenter, AudioChannel::FrontCenter) => true,
                    (AudioChannel::Mono, AudioChannel::Mono) => true,
                    (AudioChannel::RearLeft, AudioChannel::RearLeft) => true,
                    (AudioChannel::RearRight, AudioChannel::RearRight) => true,
                    (AudioChannel::LowFrequency, AudioChannel::LowFrequency) => true,
                    // For unknown channels, match by port name patterns
                    _ => {
                        // Try to match FL/FR patterns in port names
                        let out_name = out_port.name.to_lowercase();
                        let in_name = in_port.name.to_lowercase();
                        (out_name.contains("fl") && in_name.contains("fl"))
                            || (out_name.contains("fr") && in_name.contains("fr"))
                            || (out_name.contains("_0") && in_name.contains("_0"))
                            || (out_name.contains("_1") && in_name.contains("_1"))
                    }
                };

                if is_match {
                    pairs.push((out_port.id, in_port.id));
                }
            }
        }

        // If no matches found but both have ports, just pair them in order
        if pairs.is_empty() && !output_ports.is_empty() && !input_ports.is_empty() {
            for (out_port, in_port) in output_ports.iter().zip(input_ports.iter()) {
                pairs.push((out_port.id, in_port.id));
            }
        }

        pairs
    }
}

/// Main application state.
#[derive(Debug)]
pub struct AppState {
    /// User-created mixer channels.
    pub channels: Vec<MixerChannel>,
    /// Master volume in dB.
    pub master_volume_db: f32,
    /// Master mute state.
    pub master_muted: bool,
    /// Selected output device name.
    pub output_device: Option<String>,
    /// Current preset name.
    pub current_preset: String,
    /// Available apps (populated from PipeWire).
    pub available_apps: Vec<AppInfo>,
    /// Available output devices (populated from PipeWire).
    pub available_outputs: Vec<OutputDevice>,
    /// Available input devices (populated from PipeWire).
    pub available_inputs: Vec<InputDevice>,
    /// Current PipeWire graph state.
    pub pw_graph: PwGraphState,
    /// Whether connected to PipeWire.
    pub pw_connected: bool,
    /// Currently open EQ panel (channel ID).
    pub eq_panel_channel: Option<Uuid>,
    /// Settings modal open.
    pub settings_open: bool,
    /// Last error message.
    pub last_error: Option<String>,
    /// App being dragged for assignment (node_id, app_name).
    pub dragging_app: Option<(u32, String)>,
    /// Channel display filter (All, Outputs, Inputs).
    pub channel_filter: ChannelFilter,
    /// Channel being renamed (channel_id, current_edit_value).
    pub editing_channel: Option<(Uuid, String)>,
    /// Apps waiting to be re-routed after a sink rename (channel_id, app_node_ids).
    pub pending_reroute: Option<(Uuid, Vec<u32>)>,
    /// Startup discovery completed (waited for PipeWire to discover existing sinks).
    pub startup_complete: bool,
    /// Master VU meter display state.
    pub master_meter_display: MeterDisplayState,
    /// Auto-routing rules configuration.
    pub routing_rules: RoutingRulesConfig,
    /// Node IDs that have been auto-routed in this session (to avoid re-routing).
    /// Tracks by node ID so each audio stream is only routed once.
    pub auto_routed_apps: HashSet<u32>,
    /// Whether the routing rules panel is open.
    pub routing_rules_panel_open: bool,
    /// Rule being edited (rule_id, field values for edit form).
    pub editing_rule: Option<EditingRule>,
    /// Snapshot slot A for A/B comparison.
    pub snapshot_a: Option<MixerSnapshot>,
    /// Snapshot slot B for A/B comparison.
    pub snapshot_b: Option<MixerSnapshot>,
    /// Which snapshot is currently active (applied).
    pub active_snapshot: Option<SnapshotSlot>,
    /// Plugin browser open for channel (channel_id).
    pub plugin_browser_channel: Option<Uuid>,
    /// Plugin editor open (channel_id, instance_id).
    pub plugin_editor_open: Option<(Uuid, Uuid)>,
    /// Whether master recording output is enabled.
    pub master_recording_enabled: bool,
    /// Node ID of the virtual recording source (Audio/Source).
    pub master_recording_source_id: Option<u32>,
    /// Currently selected channel for focus panel.
    pub selected_channel: Option<Uuid>,
    /// Whether the left sidebar is collapsed.
    pub left_sidebar_collapsed: bool,
    /// Whether the bottom detail panel is expanded.
    pub bottom_panel_expanded: bool,
    /// Height of the bottom detail panel in pixels.
    pub bottom_panel_height: f32,

    // ==================== Plugin Downloader ====================
    /// Whether the plugin downloader panel is open.
    pub downloader_open: bool,
    /// Current search filter in the downloader.
    pub downloader_search: String,
    /// Currently downloading packs (pack_id -> progress 0.0-1.0).
    pub downloading: std::collections::HashMap<String, f32>,
    /// Set of installed pack IDs.
    pub installed_packs: std::collections::HashSet<String>,
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

impl AppState {
    pub fn new() -> Self {
        Self {
            channels: Vec::new(),
            master_volume_db: 0.0,
            master_muted: false,
            output_device: None,
            current_preset: "Default".to_string(),
            available_apps: Vec::new(),
            available_outputs: Vec::new(),
            available_inputs: Vec::new(),
            pw_graph: PwGraphState::new(),
            pw_connected: false,
            eq_panel_channel: None,
            settings_open: false,
            last_error: None,
            dragging_app: None,
            channel_filter: ChannelFilter::default(),
            editing_channel: None,
            pending_reroute: None,
            startup_complete: false,
            master_meter_display: MeterDisplayState::default(),
            routing_rules: RoutingRulesConfig::default(),
            auto_routed_apps: HashSet::new(),
            routing_rules_panel_open: false,
            editing_rule: None,
            snapshot_a: None,
            snapshot_b: None,
            active_snapshot: None,
            plugin_browser_channel: None,
            plugin_editor_open: None,
            master_recording_enabled: false,
            master_recording_source_id: None,
            selected_channel: None,
            left_sidebar_collapsed: false,
            bottom_panel_expanded: false,
            bottom_panel_height: 200.0,
            downloader_open: false,
            downloader_search: String::new(),
            downloading: std::collections::HashMap::new(),
            installed_packs: std::collections::HashSet::new(),
        }
    }

    /// Capture current mixer state as a snapshot.
    pub fn capture_snapshot(&self) -> MixerSnapshot {
        MixerSnapshot {
            channels: self
                .channels
                .iter()
                .map(|c| ChannelSnapshot {
                    id: c.id,
                    volume_db: c.volume_db,
                    muted: c.muted,
                    eq_enabled: c.eq_enabled,
                    eq_preset: c.eq_preset.clone(),
                })
                .collect(),
            master_volume_db: self.master_volume_db,
            master_muted: self.master_muted,
        }
    }

    /// Apply a snapshot to the current state. Returns channel IDs that were modified.
    pub fn apply_snapshot(&mut self, snapshot: &MixerSnapshot) -> Vec<Uuid> {
        let mut modified = Vec::new();

        // Apply master settings
        self.master_volume_db = snapshot.master_volume_db;
        self.master_muted = snapshot.master_muted;

        // Apply channel settings
        for snap_channel in &snapshot.channels {
            if let Some(channel) = self.channel_mut(snap_channel.id) {
                channel.volume_db = snap_channel.volume_db;
                channel.muted = snap_channel.muted;
                channel.eq_enabled = snap_channel.eq_enabled;
                channel.eq_preset = snap_channel.eq_preset.clone();
                modified.push(channel.id);
            }
        }

        modified
    }

    /// Find a channel by name.
    pub fn channel_by_name(&self, name: &str) -> Option<&MixerChannel> {
        self.channels
            .iter()
            .find(|c| c.name.eq_ignore_ascii_case(name))
    }

    /// Find a channel by name (mutable).
    pub fn channel_by_name_mut(&mut self, name: &str) -> Option<&mut MixerChannel> {
        self.channels
            .iter_mut()
            .find(|c| c.name.eq_ignore_ascii_case(name))
    }

    /// Find a channel by ID.
    pub fn channel(&self, id: Uuid) -> Option<&MixerChannel> {
        self.channels.iter().find(|c| c.id == id)
    }

    /// Find a channel by ID (mutable).
    pub fn channel_mut(&mut self, id: Uuid) -> Option<&mut MixerChannel> {
        self.channels.iter_mut().find(|c| c.id == id)
    }

    /// Get channel that has an app assigned.
    pub fn channel_for_app(&self, app_identifier: &str) -> Option<&MixerChannel> {
        self.channels
            .iter()
            .find(|c| c.assigned_apps.iter().any(|a| a == app_identifier))
    }

    /// Update available apps from PipeWire graph.
    pub fn update_available_apps(&mut self) {
        self.available_apps = self
            .pw_graph
            .playback_streams()
            .iter()
            .filter(|node| {
                // Filter out internal nodes
                let name = &node.name;
                !name.contains("sootmix.")
                    && !name.starts_with("LB-")
                    && !name.contains("loopback")
                    && !name.starts_with("filter-chain")
            })
            .map(|node| {
                let mut media_name = node.properties.get("media.name").cloned();

                // If media_name is missing/generic, or the app itself is generic
                // (Chromium/Electron), try PID-based resolution
                let media_is_generic = media_name
                    .as_deref()
                    .map_or(true, |m| m.is_empty() || is_generic_media_name(m));
                let app_is_generic = is_generic_app_identity(
                    node.app_name.as_deref().unwrap_or(""),
                    node.binary_name.as_deref().unwrap_or(""),
                );
                if media_is_generic || app_is_generic {
                    if let Some(pid) = node.properties.get("application.process.id") {
                        if let Some(resolved) = resolve_app_name_from_pid(pid) {
                            media_name = Some(resolved);
                        }
                    }
                }

                AppInfo {
                    node_id: node.id,
                    name: node
                        .app_name
                        .clone()
                        .or_else(|| {
                            if !node.description.is_empty() {
                                Some(node.description.clone())
                            } else {
                                None
                            }
                        })
                        .unwrap_or_else(|| node.name.clone()),
                    binary: node.binary_name.clone(),
                    icon: None,
                    media_name,
                }
            })
            .collect();
    }

    /// Update available input devices from PipeWire graph.
    pub fn update_available_inputs(&mut self) {
        // Debug: log all nodes and their media classes to diagnose discovery issues
        let total_nodes = self.pw_graph.nodes.len();
        let audio_inputs: Vec<_> = self
            .pw_graph
            .nodes
            .values()
            .filter(|n| n.media_class.is_audio_input())
            .collect();
        let potential_inputs: Vec<_> = self
            .pw_graph
            .nodes
            .values()
            .filter(|n| n.name.contains("alsa_input") || n.name.contains("input"))
            .collect();

        if !potential_inputs.is_empty() || !audio_inputs.is_empty() {
            debug!(
                "update_available_inputs: {} total nodes, {} audio input nodes, {} potential inputs",
                total_nodes, audio_inputs.len(), potential_inputs.len()
            );
            for n in &audio_inputs {
                debug!(
                    "  Audio input: id={} name='{}' desc='{}' class={:?}",
                    n.id, n.name, n.description, n.media_class
                );
            }
            for n in &potential_inputs {
                if !n.media_class.is_audio_input() {
                    debug!(
                        "  Potential input (wrong class): id={} name='{}' class={:?}",
                        n.id, n.name, n.media_class
                    );
                }
            }
        }

        let hw_inputs: Vec<InputDevice> = self
            .pw_graph
            .nodes
            .values()
            .filter(|n| {
                n.media_class.is_audio_input()
                    && !n.name.contains("sootmix.")
                    && !n.name.starts_with("LB-")
                    && !n.name.contains("loopback")
            })
            .map(|n| InputDevice {
                node_id: n.id,
                name: n.name.clone(),
                description: n.description.clone(),
            })
            .collect();

        // Prepend "Default" synthetic entry (uses system default input)
        let mut inputs = vec![InputDevice {
            node_id: 0,
            name: "system-default".to_string(),
            description: "Default".to_string(),
        }];
        inputs.extend(hw_inputs);
        self.available_inputs = inputs;

        debug!(
            "update_available_inputs: result = {} devices",
            self.available_inputs.len()
        );
    }

    /// Update available outputs from PipeWire graph.
    pub fn update_available_outputs(&mut self) {
        // Exclude our virtual sinks from output device list
        let virtual_sink_names: Vec<&str> = self
            .channels
            .iter()
            .filter_map(|c| c.pw_sink_id.map(|_| c.name.as_str()))
            .collect();

        let hw_outputs = self.pw_graph.output_devices(&virtual_sink_names);
        // Prepend "System Default" synthetic entry
        let mut outputs = vec![crate::audio::types::OutputDevice {
            node_id: 0,
            name: "system-default".to_string(),
            description: "System Default".to_string(),
        }];
        outputs.extend(hw_outputs);
        self.available_outputs = outputs;
    }
}
