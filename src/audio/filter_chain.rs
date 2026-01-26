// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! EQ filter chain creation and management using pw-filter-chain.

use crate::config::eq_preset::EqPreset;
use std::collections::HashMap;
use std::io::Write;
use std::process::{Child, Command};
use parking_lot::Mutex;
use tempfile::NamedTempFile;
use thiserror::Error;
use tracing::{debug, info, warn};
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum FilterChainError {
    #[error("Failed to spawn pw-filter-chain: {0}")]
    SpawnFailed(#[from] std::io::Error),
    #[error("Failed to create config file: {0}")]
    ConfigFailed(String),
    #[error("Failed to find created node")]
    NodeNotFound,
    #[error("pw-dump failed: {0}")]
    PwDumpFailed(String),
    #[error("Invalid JSON from pw-dump")]
    InvalidJson,
    #[error("Routing failed: {0}")]
    RoutingFailed(String),
}

/// Info about a running filter chain instance.
struct FilterChainInstance {
    child: Child,
    #[allow(dead_code)]
    config_file: NamedTempFile,
    sink_node_id: Option<u32>,
    output_node_id: Option<u32>,
}

/// Track running filter-chain processes for cleanup.
static FILTER_PROCESSES: Mutex<Option<HashMap<Uuid, FilterChainInstance>>> = Mutex::new(None);

fn get_processes() -> parking_lot::MutexGuard<'static, Option<HashMap<Uuid, FilterChainInstance>>> {
    FILTER_PROCESSES.lock()
}

fn ensure_processes_map() {
    let mut guard = get_processes();
    if guard.is_none() {
        *guard = Some(HashMap::new());
    }
}

/// Generate a filter-chain config file for a 5-band parametric EQ.
///
/// The filter chain creates:
/// - A sink node (Audio/Sink) for receiving audio from the loopback output
/// - An output stream (Stream/Output/Audio) for sending processed audio to master
fn generate_eq_config(channel_name: &str, preset: &EqPreset, _loopback_output_name: &str) -> String {
    // Generate unique node names
    let sink_node_name = format!("sootmix.eq.{}", channel_name);
    let output_node_name = format!("sootmix.eq.{}.output", channel_name);

    // Build the filter chain configuration
    // Must include context.properties to run as a client connecting to existing PipeWire
    let mut config = String::new();

    config.push_str(&format!(r#"context.properties = {{
    core.daemon = false
    core.name = "sootmix-eq-{channel_name}"
}}

context.spa-libs = {{
    audio.convert.* = audioconvert/libspa-audioconvert
    support.*       = support/libspa-support
}}

context.modules = [
    {{ name = libpipewire-module-rt
        args = {{
            nice.level = -11
        }}
        flags = [ ifexists nofail ]
    }}
    {{ name = libpipewire-module-protocol-native }}
    {{ name = libpipewire-module-client-node }}
    {{ name = libpipewire-module-adapter }}
    {{ name = libpipewire-module-filter-chain
        args = {{
            node.name = "{sink_node_name}"
            node.description = "SootMix EQ - {channel_name}"
            media.name = "SootMix EQ"
            filter.graph = {{
                nodes = [
"#));

    // Add a biquad filter for each EQ band
    for (i, band) in preset.bands.iter().enumerate() {
        config.push_str(&format!(
            r#"                    {{
                        type = builtin
                        name = eq_band_{i}
                        label = bq_peaking
                        control = {{ "Freq" = {freq} "Q" = {q:.3} "Gain" = {gain:.2} }}
                    }}
"#,
            i = i,
            freq = band.freq,
            q = band.q,
            gain = band.gain
        ));
    }

    config.push_str(r#"                ]
                links = [
"#);

    // Link the bands in series: band0 -> band1 -> band2 -> band3 -> band4
    for i in 0..preset.bands.len() - 1 {
        config.push_str(&format!(
            r#"                    {{ output = "eq_band_{i}:Out" input = "eq_band_{next}:In" }}
"#,
            i = i,
            next = i + 1
        ));
    }

    // Configure the graph inputs/outputs
    config.push_str(&format!(r#"                ]
                inputs = [ "eq_band_0:In" ]
                outputs = [ "eq_band_4:Out" ]
            }}
            capture.props = {{
                media.class = Audio/Sink
                node.name = "{sink_node_name}"
                node.description = "SootMix EQ - {channel_name}"
                audio.position = [ FL FR ]
            }}
            playback.props = {{
                media.class = Stream/Output/Audio
                node.name = "{output_node_name}"
                audio.position = [ FL FR ]
            }}
        }}
    }}
]
"#));

    config
}

/// Create an EQ filter chain for a channel.
///
/// Returns (sink_node_id, output_node_id) of the created filter.
/// The caller must then route:
/// - loopback output -> EQ sink
/// - EQ output -> master sink
pub fn create_eq_filter(
    channel_id: Uuid,
    channel_name: &str,
    preset: &EqPreset,
) -> Result<(u32, u32), FilterChainError> {
    ensure_processes_map();

    // Sanitize channel name
    let safe_name = channel_name
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
        .collect::<String>();

    // Generate config (loopback output name used for documentation only now)
    let loopback_output_name = format!("sootmix.{}.output", safe_name);
    let config_content = generate_eq_config(&safe_name, preset, &loopback_output_name);
    debug!("Generated filter-chain config:\n{}", config_content);

    // Write to temp file with .conf extension (required by PipeWire)
    let mut config_file = tempfile::Builder::new()
        .prefix("sootmix-eq-")
        .suffix(".conf")
        .tempfile()
        .map_err(|e| FilterChainError::ConfigFailed(e.to_string()))?;
    config_file
        .write_all(config_content.as_bytes())
        .map_err(|e| FilterChainError::ConfigFailed(e.to_string()))?;

    let config_path = config_file.path().to_string_lossy().to_string();
    info!("Creating EQ filter for channel '{}' with config: {}", channel_name, config_path);

    // Spawn pipewire with the filter-chain config
    let child = Command::new("pipewire")
        .arg("-c")
        .arg(&config_path)
        .spawn()?;

    let pid = child.id();
    debug!("pw-filter-chain spawned with PID: {}", pid);

    // Give it time to register with PipeWire
    std::thread::sleep(std::time::Duration::from_millis(300));

    // Find both the sink and output node IDs
    let sink_node_name = format!("sootmix.eq.{}", safe_name);
    let output_node_name = format!("sootmix.eq.{}.output", safe_name);

    let sink_node_id = find_node_by_name(&sink_node_name, "Audio/Sink")?;
    let output_node_id = find_node_by_name(&output_node_name, "Stream/Output/Audio")?;

    // Store the instance
    if let Some(ref mut map) = *get_processes() {
        map.insert(
            channel_id,
            FilterChainInstance {
                child,
                config_file,
                sink_node_id: Some(sink_node_id),
                output_node_id: Some(output_node_id),
            },
        );
    }

    info!(
        "Created EQ filter for '{}': sink_node={}, output_node={}",
        channel_name, sink_node_id, output_node_id
    );
    Ok((sink_node_id, output_node_id))
}

/// Destroy an EQ filter chain.
pub fn destroy_eq_filter(channel_id: Uuid) -> Result<(), FilterChainError> {
    if let Some(ref mut map) = *get_processes() {
        if let Some(mut instance) = map.remove(&channel_id) {
            info!("Destroying EQ filter for channel {}", channel_id);
            let _ = instance.child.kill();
            let _ = instance.child.wait();
            // config_file is automatically deleted when dropped
            return Ok(());
        }
    }

    warn!("No tracked EQ filter for channel {}", channel_id);
    Ok(())
}

/// Destroy all EQ filters (cleanup on exit).
pub fn destroy_all_eq_filters() {
    if let Some(ref mut map) = *get_processes() {
        for (channel_id, mut instance) in map.drain() {
            info!("Cleaning up EQ filter for channel {}", channel_id);
            let _ = instance.child.kill();
            let _ = instance.child.wait();
        }
    }
}

/// Get the node IDs for a channel's EQ filter, if it exists.
/// Returns (sink_node_id, output_node_id).
pub fn get_eq_node_ids(channel_id: Uuid) -> Option<(u32, u32)> {
    if let Some(ref map) = *get_processes() {
        if let Some(instance) = map.get(&channel_id) {
            if let (Some(sink), Some(output)) = (instance.sink_node_id, instance.output_node_id) {
                return Some((sink, output));
            }
        }
    }
    None
}

/// Try to create a link between two ports, attempting multiple naming conventions.
/// Returns true if link was created successfully.
fn try_link_ports(source_node: &str, dest_node: &str, channel: &str) -> bool {
    // Port naming conventions to try for output ports
    let output_prefixes = ["playback_", "output_", "monitor_"];
    // Port naming conventions to try for input ports
    let input_prefixes = ["playback_", "input_", "capture_"];

    for out_prefix in &output_prefixes {
        for in_prefix in &input_prefixes {
            let output_port = format!("{}:{}{}", source_node, out_prefix, channel);
            let input_port = format!("{}:{}{}", dest_node, in_prefix, channel);

            let result = Command::new("pw-link")
                .arg("-L")
                .arg(&output_port)
                .arg(&input_port)
                .output();

            if let Ok(output) = result {
                if output.status.success() {
                    debug!("Linked {} -> {}", output_port, input_port);
                    return true;
                }
                let stderr = String::from_utf8_lossy(&output.stderr);
                if stderr.contains("already linked") {
                    debug!("Already linked {} -> {}", output_port, input_port);
                    return true;
                }
            }
        }
    }
    false
}

/// Route audio through the EQ filter chain using pw-link.
///
/// Creates links:
/// - loopback_output_node -> eq_sink_node (FL/FR channels)
///
/// The EQ output is automatically connected to the default sink by PipeWire's
/// session manager, so we don't need to manually link it.
pub fn route_through_eq(
    loopback_output_name: &str,
    eq_sink_name: &str,
    _eq_output_name: &str,
    _master_sink_name: &str,
) -> Result<(), FilterChainError> {
    info!(
        "Routing audio through EQ: {} -> {} (output auto-connects to default sink)",
        loopback_output_name, eq_sink_name
    );

    // Link loopback output to EQ sink (FL and FR channels)
    for channel in &["FL", "FR"] {
        if !try_link_ports(loopback_output_name, eq_sink_name, channel) {
            warn!("Could not link {} -> {} for channel {}",
                loopback_output_name, eq_sink_name, channel);
        }
    }

    // EQ output auto-connects to default sink via session manager

    Ok(())
}

/// Try to destroy a link between two ports, attempting multiple naming conventions.
fn try_unlink_ports(source_node: &str, dest_node: &str, channel: &str) {
    let output_prefixes = ["playback_", "output_", "monitor_"];
    let input_prefixes = ["playback_", "input_", "capture_"];

    for out_prefix in &output_prefixes {
        for in_prefix in &input_prefixes {
            let output_port = format!("{}:{}{}", source_node, out_prefix, channel);
            let input_port = format!("{}:{}{}", dest_node, in_prefix, channel);

            let _ = Command::new("pw-link")
                .arg("-d")
                .arg(&output_port)
                .arg(&input_port)
                .output();
        }
    }
}

/// Remove routing through EQ.
///
/// Destroys links from loopback to EQ sink. The loopback output will
/// auto-connect back to the default sink via session manager.
pub fn unroute_eq(
    loopback_output_name: &str,
    eq_sink_name: &str,
    _eq_output_name: &str,
    _master_sink_name: &str,
) -> Result<(), FilterChainError> {
    info!("Removing EQ routing from {}", loopback_output_name);

    // Destroy links from loopback to EQ
    for channel in &["FL", "FR"] {
        try_unlink_ports(loopback_output_name, eq_sink_name, channel);
    }

    // The loopback output should auto-connect back to default sink
    // when the EQ filter is destroyed

    Ok(())
}

/// Find a node by name and media class using pw-dump.
fn find_node_by_name(name: &str, media_class: &str) -> Result<u32, FilterChainError> {
    let output = Command::new("pw-dump")
        .output()
        .map_err(|e| FilterChainError::PwDumpFailed(e.to_string()))?;

    if !output.status.success() {
        return Err(FilterChainError::PwDumpFailed(
            String::from_utf8_lossy(&output.stderr).to_string(),
        ));
    }

    let json_str = String::from_utf8_lossy(&output.stdout);
    let objects: Vec<serde_json::Value> =
        serde_json::from_str(&json_str).map_err(|_| FilterChainError::InvalidJson)?;

    for obj in objects {
        let obj_type = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if obj_type != "PipeWire:Interface:Node" {
            continue;
        }

        let props = obj.get("info").and_then(|i| i.get("props"));
        let node_name = props
            .and_then(|p| p.get("node.name"))
            .and_then(|n| n.as_str())
            .unwrap_or("");
        let node_class = props
            .and_then(|p| p.get("media.class"))
            .and_then(|c| c.as_str())
            .unwrap_or("");

        if node_name == name && node_class == media_class {
            if let Some(id) = obj.get("id").and_then(|v| v.as_u64()) {
                debug!("Found node '{}' (class={}) with ID {}", name, media_class, id);
                return Ok(id as u32);
            }
        }
    }

    Err(FilterChainError::NodeNotFound)
}

/// Find the default audio sink name for routing.
pub fn find_default_sink_name() -> Option<String> {
    let output = Command::new("wpctl")
        .args(["inspect", "@DEFAULT_AUDIO_SINK@"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Parse wpctl inspect output to find node.name
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("node.name") {
            // Format: node.name = "alsa_output.pci-0000_00_1f.3.analog-stereo"
            if let Some(name) = trimmed.split('=').nth(1) {
                let name = name.trim().trim_matches('"');
                return Some(name.to_string());
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::eq_preset::EqPreset;

    #[test]
    fn test_generate_eq_config() {
        let preset = EqPreset::bass_boost();
        let config = generate_eq_config("test", &preset, "sootmix.test.output");
        assert!(config.contains("sootmix.eq.test"));
        assert!(config.contains("bq_peaking"));
        assert!(config.contains("eq_band_0"));
        assert!(config.contains("sootmix.eq.test.output"));
    }

    #[test]
    #[ignore] // Requires PipeWire running
    fn test_create_destroy_eq_filter() {
        let channel_id = Uuid::new_v4();
        let preset = EqPreset::flat();

        let result = create_eq_filter(channel_id, "test_eq", &preset);
        if let Ok((_sink_id, _output_id)) = result {
            std::thread::sleep(std::time::Duration::from_millis(500));
            destroy_eq_filter(channel_id).unwrap();
        }
    }
}
