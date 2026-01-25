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

/// Create a virtual sink using pw-loopback.
///
/// Returns the node ID of the created sink.
pub fn create_virtual_sink(name: &str, description: &str) -> Result<u32, VirtualSinkError> {
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
        "media.class=Stream/Output/Audio node.passive=true audio.position=[FL FR]"
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

    // Find the node ID by querying pw-dump
    let node_id = find_node_by_name(&format!("sootmix.{}", safe_name))?;

    // Track the process
    if let Some(ref mut map) = *get_processes() {
        map.insert(node_id, child);
    }

    info!("Created virtual sink '{}' with node ID {}", name, node_id);
    Ok(node_id)
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
fn find_node_by_name(name: &str) -> Result<u32, VirtualSinkError> {
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

        // Must be an Audio/Sink, not Stream/Output/Audio
        let media_class = props
            .and_then(|p| p.get("media.class"))
            .and_then(|c| c.as_str())
            .unwrap_or("");

        if node_name == name && media_class == "Audio/Sink" {
            if let Some(id) = obj.get("id").and_then(|v| v.as_u64()) {
                debug!("Found sink node '{}' with ID {} (class={})", name, id, media_class);
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
