// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! LV2 plugin scanning and metadata extraction.

#![allow(dead_code)]

use super::Lv2World;
use sootmix_plugin_api::PluginCategory;
use std::sync::Arc;
use tracing::debug;

/// Metadata for a discovered LV2 plugin.
#[derive(Debug, Clone)]
pub struct Lv2PluginMeta {
    /// LV2 URI (unique identifier).
    pub uri: String,
    /// Human-readable name.
    pub name: String,
    /// Plugin author/vendor.
    pub author: Option<String>,
    /// LV2 class label.
    pub class: Option<String>,
    /// Mapped category for SootMix.
    pub category: PluginCategory,
    /// Number of audio input ports.
    pub audio_inputs: u32,
    /// Number of audio output ports.
    pub audio_outputs: u32,
    /// Control port information.
    pub control_ports: Vec<Lv2PortInfo>,
    /// Bundle URI.
    pub bundle_uri: Option<String>,
}

/// Information about an LV2 control port.
#[derive(Debug, Clone)]
pub struct Lv2PortInfo {
    /// Port index within the plugin.
    pub index: usize,
    /// Port symbol (identifier).
    pub symbol: String,
    /// Port name.
    pub name: String,
    /// Port type.
    pub port_type: Lv2PortType,
    /// Minimum value.
    pub min: f32,
    /// Maximum value.
    pub max: f32,
    /// Default value.
    pub default: f32,
    /// Whether the port uses logarithmic scale.
    pub logarithmic: bool,
    /// Whether this is an input (true) or output (false) port.
    pub is_input: bool,
}

/// Type of LV2 port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lv2PortType {
    /// Audio port (samples).
    Audio,
    /// Control port (single value per block).
    Control,
    /// CV (control voltage) port.
    Cv,
    /// Atom port (events, MIDI, etc.).
    Atom,
}

/// Scan all available LV2 plugins.
pub fn scan_plugins(world: &Arc<Lv2World>) -> Vec<Lv2PluginMeta> {
    let inner = world.inner();
    let plugins = inner.plugins();

    // Create URI nodes for port type checking
    let audio_port_uri = inner.new_uri("http://lv2plug.in/ns/lv2core#AudioPort");
    let control_port_uri = inner.new_uri("http://lv2plug.in/ns/lv2core#ControlPort");
    let input_port_uri = inner.new_uri("http://lv2plug.in/ns/lv2core#InputPort");
    let output_port_uri = inner.new_uri("http://lv2plug.in/ns/lv2core#OutputPort");
    let log_property_uri = inner.new_uri("http://lv2plug.in/ns/ext/port-props#logarithmic");

    let mut result = Vec::new();

    for plugin in plugins.iter() {
        let uri = match plugin.uri().as_uri() {
            Some(u) => u.to_string(),
            None => continue,
        };

        let name = plugin
            .name()
            .as_str()
            .map(|s| s.to_string())
            .unwrap_or_else(|| uri.clone());

        // Get author if available
        let author = plugin.author_name().and_then(|n| n.as_str().map(|s| s.to_string()));

        // Get plugin class label
        let class = plugin.class().label().as_str().map(|s| s.to_string());

        // Map LV2 class to SootMix category
        let category = map_lv2_class_to_category(class.as_deref());

        // Count audio ports and collect control port info
        let mut audio_inputs = 0u32;
        let mut audio_outputs = 0u32;
        let mut control_ports = Vec::new();

        // Get port ranges for all ports at once
        let port_ranges = plugin.port_ranges_float();

        for port in plugin.iter_ports() {
            let port_index = port.index();
            let is_audio = port.is_a(&audio_port_uri);
            let is_control = port.is_a(&control_port_uri);
            let is_input = port.is_a(&input_port_uri);
            let is_output = port.is_a(&output_port_uri);

            if is_audio {
                if is_input {
                    audio_inputs += 1;
                } else if is_output {
                    audio_outputs += 1;
                }
            } else if is_control && is_input {
                let symbol = port
                    .symbol()
                    .and_then(|s| s.as_str().map(|s| s.to_string()))
                    .unwrap_or_else(|| format!("port_{}", port_index));

                let port_name = port
                    .name()
                    .and_then(|n| n.as_str().map(|s| s.to_string()))
                    .unwrap_or_else(|| symbol.clone());

                // Get port range from pre-fetched values
                let (mut min, mut max, mut default) = if let Some(range) = port_ranges.get(port_index) {
                    (range.min, range.max, range.default)
                } else {
                    (0.0, 1.0, 0.5)
                };

                // Handle NaN or infinite values
                if !min.is_finite() {
                    min = 0.0;
                }
                if !max.is_finite() {
                    max = 1.0;
                }
                if !default.is_finite() {
                    default = (min + max) / 2.0;
                }

                // Ensure proper ordering
                if min > max {
                    std::mem::swap(&mut min, &mut max);
                }
                default = default.clamp(min, max);

                // Check for logarithmic property
                let logarithmic = port.has_property(&log_property_uri);

                control_ports.push(Lv2PortInfo {
                    index: port_index,
                    symbol,
                    name: port_name,
                    port_type: Lv2PortType::Control,
                    min,
                    max,
                    default,
                    logarithmic,
                    is_input: true,
                });
            }
        }

        // Skip plugins with no audio I/O (not audio effects)
        if audio_inputs == 0 && audio_outputs == 0 {
            debug!("Skipping non-audio plugin: {}", name);
            continue;
        }

        // Get bundle URI
        let bundle_uri = plugin.bundle_uri().as_uri().map(|s| s.to_string());

        result.push(Lv2PluginMeta {
            uri,
            name,
            author,
            class,
            category,
            audio_inputs,
            audio_outputs,
            control_ports,
            bundle_uri,
        });
    }

    result
}

/// Map LV2 plugin class to SootMix category.
fn map_lv2_class_to_category(class: Option<&str>) -> PluginCategory {
    let class = match class {
        Some(c) => c.to_lowercase(),
        None => return PluginCategory::Other,
    };

    if class.contains("eq") || class.contains("filter") || class.contains("parametric") {
        PluginCategory::Eq
    } else if class.contains("compressor")
        || class.contains("limiter")
        || class.contains("gate")
        || class.contains("expander")
        || class.contains("dynamics")
    {
        PluginCategory::Dynamics
    } else if class.contains("reverb") || class.contains("delay") || class.contains("echo") {
        PluginCategory::Reverb
    } else if class.contains("chorus")
        || class.contains("flanger")
        || class.contains("phaser")
        || class.contains("modulation")
    {
        PluginCategory::Modulation
    } else if class.contains("distortion")
        || class.contains("overdrive")
        || class.contains("saturation")
        || class.contains("amp")
        || class.contains("waveshaper")
    {
        PluginCategory::Distortion
    } else if class.contains("utility")
        || class.contains("gain")
        || class.contains("meter")
        || class.contains("analyser")
        || class.contains("analyzer")
    {
        PluginCategory::Utility
    } else {
        PluginCategory::Other
    }
}
