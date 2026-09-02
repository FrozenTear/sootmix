// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! PipeWire type definitions for nodes, ports, and links.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Represents a PipeWire node (app, device, or virtual sink).
#[derive(Debug, Clone)]
pub struct PwNode {
    pub id: u32,
    pub name: String,
    pub description: String,
    pub media_class: MediaClass,
    pub app_name: Option<String>,
    pub binary_name: Option<String>,
    pub media_name: Option<String>,
    #[allow(dead_code)]
    pub ports: Vec<PwPort>,
    pub properties: HashMap<String, String>,
    /// Last known PipeWire node run state. `Unknown` until an info listener
    /// reports it. Used by restore logic so we do not recreate links to
    /// devices that are gone / in Error (device loss should fall through
    /// to fallback, not fight `NodeRemoved`).
    pub run_state: NodeRunState,
}

impl PwNode {
    pub fn new(id: u32) -> Self {
        Self {
            id,
            name: String::new(),
            description: String::new(),
            media_class: MediaClass::Unknown(String::new()),
            app_name: None,
            binary_name: None,
            media_name: None,
            ports: Vec::new(),
            properties: HashMap::new(),
            run_state: NodeRunState::Unknown,
        }
    }

    /// Whether this node is a valid restore target.
    ///
    /// Missing from the graph is handled by the caller. Here we refuse
    /// Error/Creating — those mean the device is dying or not ready.
    /// Idle/Suspended/Running/Unknown are still present devices
    /// (Idle hardware is the normal unused-sink state).
    pub fn is_available_for_restore(&self) -> bool {
        self.run_state.is_available_for_restore()
    }

    pub fn is_playback_stream(&self) -> bool {
        matches!(self.media_class, MediaClass::StreamOutputAudio)
    }

    #[allow(dead_code)]
    pub fn is_sink(&self) -> bool {
        matches!(self.media_class, MediaClass::AudioSink)
    }

    #[allow(dead_code)]
    pub fn is_source(&self) -> bool {
        matches!(self.media_class, MediaClass::AudioSource)
    }

    /// Check if this node can provide audio input (microphone, line-in, or duplex device).
    pub fn is_audio_input(&self) -> bool {
        matches!(
            self.media_class,
            MediaClass::AudioSource | MediaClass::AudioDuplex
        )
    }

    #[allow(dead_code)]
    pub fn display_name(&self) -> &str {
        if !self.description.is_empty() {
            &self.description
        } else if let Some(ref app) = self.app_name {
            app
        } else if !self.name.is_empty() {
            &self.name
        } else {
            "Unknown"
        }
    }
}

/// PipeWire node run state (from the Node info callback).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum NodeRunState {
    #[default]
    Unknown,
    Creating,
    Idle,
    Running,
    Suspended,
    Error,
}

impl NodeRunState {
    pub fn is_available_for_restore(self) -> bool {
        matches!(
            self,
            Self::Unknown | Self::Idle | Self::Running | Self::Suspended
        )
    }

    pub fn from_pw_str(s: &str) -> Self {
        // pipewire-rs Debug is typically "Running"; tolerate "NodeState::Running".
        let s = s.rsplit("::").next().unwrap_or(s);
        match s {
            "creating" | "Creating" => Self::Creating,
            "idle" | "Idle" => Self::Idle,
            "running" | "Running" => Self::Running,
            "suspended" | "Suspended" => Self::Suspended,
            "error" | "Error" => Self::Error,
            _ => Self::Unknown,
        }
    }
}

#[cfg(test)]
mod node_run_state_tests {
    use super::*;

    #[test]
    fn restore_refuses_error_and_creating() {
        let mut node = PwNode::new(1);
        assert!(node.is_available_for_restore());
        node.run_state = NodeRunState::Error;
        assert!(!node.is_available_for_restore());
        node.run_state = NodeRunState::Creating;
        assert!(!node.is_available_for_restore());
        node.run_state = NodeRunState::Idle;
        assert!(node.is_available_for_restore());
        node.run_state = NodeRunState::Running;
        assert!(node.is_available_for_restore());
    }

    #[test]
    fn from_pw_str_debug_and_path() {
        assert_eq!(NodeRunState::from_pw_str("Running"), NodeRunState::Running);
        assert_eq!(
            NodeRunState::from_pw_str("NodeState::Error"),
            NodeRunState::Error
        );
    }
}

/// Media class classification for PipeWire nodes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MediaClass {
    AudioSink,
    AudioSource,
    /// Combined input/output device (e.g., USB headset with mic).
    AudioDuplex,
    StreamOutputAudio,
    StreamInputAudio,
    VideoSource,
    Unknown(String),
}

impl MediaClass {
    pub fn from_str(s: &str) -> Self {
        match s {
            "Audio/Sink" => Self::AudioSink,
            "Audio/Source" => Self::AudioSource,
            "Audio/Duplex" => Self::AudioDuplex,
            "Stream/Output/Audio" => Self::StreamOutputAudio,
            "Stream/Input/Audio" => Self::StreamInputAudio,
            "Video/Source" => Self::VideoSource,
            other => Self::Unknown(other.to_string()),
        }
    }

    #[allow(dead_code)]
    pub fn as_str(&self) -> &str {
        match self {
            Self::AudioSink => "Audio/Sink",
            Self::AudioSource => "Audio/Source",
            Self::AudioDuplex => "Audio/Duplex",
            Self::StreamOutputAudio => "Stream/Output/Audio",
            Self::StreamInputAudio => "Stream/Input/Audio",
            Self::VideoSource => "Video/Source",
            Self::Unknown(s) => s,
        }
    }
}

/// Represents a port on a PipeWire node.
#[derive(Debug, Clone)]
pub struct PwPort {
    pub id: u32,
    pub node_id: u32,
    pub name: String,
    pub direction: PortDirection,
    pub channel: AudioChannel,
}

impl PwPort {
    pub fn new(id: u32, node_id: u32) -> Self {
        Self {
            id,
            node_id,
            name: String::new(),
            direction: PortDirection::Unknown,
            channel: AudioChannel::Unknown,
        }
    }
}

/// Direction of a port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PortDirection {
    Input,
    Output,
    Unknown,
}

impl PortDirection {
    pub fn from_str(s: &str) -> Self {
        match s {
            "in" => Self::Input,
            "out" => Self::Output,
            _ => Self::Unknown,
        }
    }
}

/// Audio channel position.
/// Ordered by standard channel layout for consistent pairing.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AudioChannel {
    FrontLeft,
    FrontRight,
    FrontCenter,
    LowFrequency,
    RearLeft,
    RearRight,
    Mono,
    Unknown,
}

impl AudioChannel {
    pub fn from_str(s: &str) -> Self {
        let s_lower = s.to_lowercase();
        if s_lower.contains("fl")
            || s_lower.contains("front_left")
            || s_lower.contains("playback_fl")
        {
            Self::FrontLeft
        } else if s_lower.contains("fr")
            || s_lower.contains("front_right")
            || s_lower.contains("playback_fr")
        {
            Self::FrontRight
        } else if s_lower.contains("fc") || s_lower.contains("front_center") {
            Self::FrontCenter
        } else if s_lower.contains("mono") {
            Self::Mono
        } else if s_lower.contains("rl") || s_lower.contains("rear_left") {
            Self::RearLeft
        } else if s_lower.contains("rr") || s_lower.contains("rear_right") {
            Self::RearRight
        } else if s_lower.contains("lfe") || s_lower.contains("subwoofer") {
            Self::LowFrequency
        } else {
            Self::Unknown
        }
    }

    /// Check if two channels are compatible for linking.
    /// Allows matching same channels or mono to stereo mappings.
    pub fn is_compatible(&self, other: &Self) -> bool {
        if self == other {
            return true;
        }
        matches!(
            (self, other),
            // Mono can connect to either stereo channel (for stereo↔mono downmix/upmix)
            (Self::Mono, Self::FrontLeft)
                | (Self::FrontLeft, Self::Mono)
                | (Self::Mono, Self::FrontRight)
                | (Self::FrontRight, Self::Mono)
                // Unknown channels (e.g. Bluetooth numeric ports like capture_0)
                // are compatible with any named channel — positional pairing handles order
                | (Self::Unknown, _)
                | (_, Self::Unknown)
        )
    }
}

/// A link between two ports in the PipeWire graph.
#[derive(Debug, Clone)]
pub struct PwLink {
    pub id: u32,
    pub output_node: u32,
    pub output_port: u32,
    pub input_node: u32,
    pub input_port: u32,
    #[allow(dead_code)]
    pub active: bool,
}

impl PwLink {
    pub fn new(id: u32) -> Self {
        Self {
            id,
            output_node: 0,
            output_port: 0,
            input_node: 0,
            input_port: 0,
            active: false,
        }
    }
}

/// Information about an output device.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct OutputDevice {
    pub node_id: u32,
    pub name: String,
    pub description: String,
}

impl OutputDevice {
    #[allow(dead_code)]
    #[allow(dead_code)]
    pub fn display_name(&self) -> &str {
        if !self.description.is_empty() {
            &self.description
        } else {
            &self.name
        }
    }
}
