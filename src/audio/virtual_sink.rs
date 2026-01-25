// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Virtual sink creation and management using pw-loopback.

use std::collections::HashMap;
use std::process::{Child, Command};
use std::sync::Mutex;
use thiserror::Error;
use tracing::{debug, info, warn};

#[derive(Debug, Error)]
pub enum VirtualSinkError {
    #[error("Failed to spawn pw-loopback: {0}")]
    SpawnFailed(#[from] std::io::Error),
    #[error("Failed to find created node")]
    NodeNotFound,
    #[error("pw-dump failed: {0}")]
    PwDumpFailed(String),
    #[error("Invalid JSON from pw-dump")]
    InvalidJson,
}

/// Track running pw-loopback processes for cleanup.
static LOOPBACK_PROCESSES: Mutex<Option<HashMap<u32, Child>>> = Mutex::new(None);

fn get_processes() -> std::sync::MutexGuard<'static, Option<HashMap<u32, Child>>> {
    LOOPBACK_PROCESSES.lock().unwrap()
}

fn ensure_processes_map() {
    let mut guard = get_processes();
    if guard.is_none() {
        *guard = Some(HashMap::new());
    }
}

/// Result of creating a virtual sink.
#[derive(Debug, Clone, Copy)]
pub struct VirtualSinkResult {
    /// The Audio/Sink node ID (apps connect here).
    pub sink_node_id: u32,
    /// The Stream/Output/Audio node ID (loopback output for routing).
    pub loopback_output_node_id: Option<u32>,
}

/// Create a virtual sink using pw-loopback.
///
/// Returns the node ID of the created sink and optionally the loopback output node ID.
pub fn create_virtual_sink(name: &str, description: &str) -> Result<u32, VirtualSinkError> {
    let result = create_virtual_sink_full(name, description)?;
    Ok(result.sink_node_id)
}

/// Create a virtual sink using pw-loopback.
///
/// Returns both the sink node ID and the loopback output node ID.
pub fn create_virtual_sink_full(name: &str, description: &str) -> Result<VirtualSinkResult, VirtualSinkError> {
    ensure_processes_map();

    // Sanitize name for use in properties
    let safe_name = name
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
        .collect::<String>();

    // Node name for the loopback output (playback side)
    let loopback_name = format!("sootmix.{}.output", safe_name);

    let capture_props = format!(
        "media.class=Audio/Sink node.name=sootmix.{} node.description=\"{}\" audio.position=[FL FR]",
        safe_name, description
    );

    let playback_props = format!(
        "media.class=Stream/Output/Audio node.autoconnect=false audio.position=[FL FR]"
    );

    info!("Creating virtual sink: {}", name);
    debug!("capture_props: {}", capture_props);
    debug!("playback_props: {}", playback_props);
    debug!("loopback_name: {}", loopback_name);

    // Spawn pw-loopback as a background process
    // --name sets the node name for the loopback output (playback side)
    let child = Command::new("pw-loopback")
        .arg("--name")
        .arg(&loopback_name)
        .arg("--capture-props")
        .arg(&capture_props)
        .arg("--playback-props")
        .arg(&playback_props)
        .spawn()?;

    let pid = child.id();
    debug!("pw-loopback spawned with PID: {}", pid);

    // Give it a moment to register with PipeWire
    std::thread::sleep(std::time::Duration::from_millis(200));

    // Find the sink node ID by querying pw-dump
    let sink_node_id = find_node_by_name(&format!("sootmix.{}", safe_name))?;

    // Find the loopback output node ID
    let loopback_output_node_id = find_node_by_name_and_class(&loopback_name, "Stream/Output/Audio").ok();

    // Track the process
    if let Some(ref mut map) = *get_processes() {
        map.insert(sink_node_id, child);
    }

    info!("Created virtual sink '{}' with sink_id={}, loopback_output_id={:?}",
          name, sink_node_id, loopback_output_node_id);

    Ok(VirtualSinkResult {
        sink_node_id,
        loopback_output_node_id,
    })
}

/// Destroy a virtual sink by killing its pw-loopback process.
pub fn destroy_virtual_sink(node_id: u32) -> Result<(), VirtualSinkError> {
    if let Some(ref mut map) = *get_processes() {
        if let Some(mut child) = map.remove(&node_id) {
            info!("Destroying virtual sink with node ID {}", node_id);
            let _ = child.kill();
            let _ = child.wait();
            return Ok(());
        }
    }

    // Fallback: try to destroy via pw-cli
    warn!(
        "No tracked process for node {}, attempting pw-cli destroy",
        node_id
    );
    let _ = Command::new("pw-cli")
        .args(["destroy", &node_id.to_string()])
        .output();

    Ok(())
}

/// Destroy all virtual sinks (cleanup on exit).
pub fn destroy_all_virtual_sinks() {
    if let Some(ref mut map) = *get_processes() {
        for (node_id, mut child) in map.drain() {
            info!("Cleaning up virtual sink {}", node_id);
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Find a node ID by its node.name property using pw-dump.
/// Defaults to finding Audio/Sink class.
fn find_node_by_name(name: &str) -> Result<u32, VirtualSinkError> {
    find_node_by_name_and_class(name, "Audio/Sink")
}

/// Find a node ID by its node.name and media.class properties using pw-dump.
fn find_node_by_name_and_class(name: &str, target_class: &str) -> Result<u32, VirtualSinkError> {
    let output = Command::new("pw-dump")
        .output()
        .map_err(|e| VirtualSinkError::PwDumpFailed(e.to_string()))?;

    if !output.status.success() {
        return Err(VirtualSinkError::PwDumpFailed(
            String::from_utf8_lossy(&output.stderr).to_string(),
        ));
    }

    let json_str = String::from_utf8_lossy(&output.stdout);

    // Parse JSON with serde_json
    let objects: Vec<serde_json::Value> = serde_json::from_str(&json_str)
        .map_err(|_| VirtualSinkError::InvalidJson)?;

    for obj in objects {
        // Check if it's a Node type
        let obj_type = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if obj_type != "PipeWire:Interface:Node" {
            continue;
        }

        // Get props
        let props = obj
            .get("info")
            .and_then(|i| i.get("props"));

        let node_name = props
            .and_then(|p| p.get("node.name"))
            .and_then(|n| n.as_str())
            .unwrap_or("");

        let media_class = props
            .and_then(|p| p.get("media.class"))
            .and_then(|c| c.as_str())
            .unwrap_or("");

        if node_name == name && media_class == target_class {
            if let Some(id) = obj.get("id").and_then(|v| v.as_u64()) {
                debug!("Found node '{}' with ID {} (class={})", name, id, media_class);
                return Ok(id as u32);
            }
        }
    }

    // Fallback: use wpctl status
    debug!("Node not found in pw-dump, trying wpctl status");
    let wpctl_output = Command::new("wpctl")
        .arg("status")
        .output()
        .map_err(|e| VirtualSinkError::PwDumpFailed(e.to_string()))?;

    let status_str = String::from_utf8_lossy(&wpctl_output.stdout);

    // Parse wpctl status output to find our sink
    // Format is like: "  42. sootmix.Game [vol: 1.00]"
    for line in status_str.lines() {
        if line.contains(name) {
            // Extract the ID number at the start
            let trimmed = line.trim();
            if let Some(dot_pos) = trimmed.find('.') {
                if let Ok(id) = trimmed[..dot_pos].trim().parse::<u32>() {
                    debug!("Found node '{}' via wpctl with ID {}", name, id);
                    return Ok(id);
                }
            }
        }
    }

    Err(VirtualSinkError::NodeNotFound)
}

/// Result of creating a virtual source for recording.
#[derive(Debug, Clone, Copy)]
pub struct VirtualSourceResult {
    /// The Audio/Source node ID (recording apps connect here).
    pub source_node_id: u32,
    /// The Stream/Input/Audio node ID (receives audio from master output).
    pub capture_stream_node_id: Option<u32>,
}

/// Create a virtual source for recording using pw-loopback.
///
/// This creates an Audio/Source that recording applications can use as input.
/// The capture side (Stream/Input/Audio) receives audio from the master output.
///
/// Returns both the source node ID and optionally the capture stream node ID.
pub fn create_virtual_source(name: &str) -> Result<VirtualSourceResult, VirtualSinkError> {
    ensure_processes_map();

    // Sanitize name for use in properties
    let safe_name = name
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
        .collect::<String>();

    // The source node name (what recording apps see)
    let source_name = format!("sootmix.recording.{}", safe_name);

    // Capture props: Stream/Input/Audio that receives from master output
    // node.passive=true means it auto-connects to default sink monitor
    let capture_props = format!(
        "media.class=Stream/Input/Audio node.passive=true audio.position=[FL FR]"
    );

    // Playback props: Audio/Source that recording apps connect to
    let playback_props = format!(
        "media.class=Audio/Source node.name={} node.description=\"SootMix Recording - {}\" audio.position=[FL FR]",
        source_name, name
    );

    info!("Creating virtual source for recording: {}", name);
    debug!("capture_props: {}", capture_props);
    debug!("playback_props: {}", playback_props);

    // Spawn pw-loopback as a background process
    let child = Command::new("pw-loopback")
        .arg("--capture-props")
        .arg(&capture_props)
        .arg("--playback-props")
        .arg(&playback_props)
        .spawn()?;

    let pid = child.id();
    debug!("pw-loopback (source) spawned with PID: {}", pid);

    // Give it a moment to register with PipeWire
    std::thread::sleep(std::time::Duration::from_millis(200));

    // Find the source node ID (Audio/Source)
    let source_node_id = find_node_by_name_and_class(&source_name, "Audio/Source")?;

    // Find the capture stream node ID (for routing if needed)
    // Note: The capture stream auto-connects via node.passive=true, so we may not need this
    let capture_stream_node_id = None; // Can be found later if needed

    // Track the process (use source node ID as key)
    if let Some(ref mut map) = *get_processes() {
        map.insert(source_node_id, child);
    }

    info!("Created virtual source '{}' with source_id={}", name, source_node_id);

    Ok(VirtualSourceResult {
        source_node_id,
        capture_stream_node_id,
    })
}

/// Destroy a virtual source by killing its pw-loopback process.
pub fn destroy_virtual_source(node_id: u32) -> Result<(), VirtualSinkError> {
    // Same implementation as destroy_virtual_sink since both use pw-loopback
    destroy_virtual_sink(node_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore] // Requires PipeWire running
    fn test_create_destroy_sink() {
        let result = create_virtual_sink("test_sink", "Test Sink");
        if let Ok(node_id) = result {
            std::thread::sleep(std::time::Duration::from_millis(500));
            destroy_virtual_sink(node_id).unwrap();
        }
    }
}
