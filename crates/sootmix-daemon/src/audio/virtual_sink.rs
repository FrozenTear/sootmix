// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Virtual sink creation and management using pw-loopback.

use std::collections::HashMap;
use std::process::{Child, Command};
use parking_lot::Mutex;
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

/// Check availability of PipeWire CLI tools at startup.
/// Logs warnings for any missing tools with guidance.
pub fn check_pipewire_tools() {
    let tools = [
        ("pw-loopback", "Required for creating virtual sinks"),
        ("pw-cli", "Required for node management and link creation"),
        ("pw-dump", "Required for discovering PipeWire nodes"),
        ("pw-link", "Used for port linking operations"),
        ("wpctl", "Required for volume control and default sink management"),
    ];

    for (tool, purpose) in &tools {
        match Command::new("which").arg(tool).output() {
            Ok(output) if output.status.success() => {
                debug!("{} found", tool);
            }
            _ => {
                warn!(
                    "PipeWire tool '{}' not found in PATH. {}: {}",
                    tool, "This tool is needed", purpose
                );
            }
        }
    }
}

static LOOPBACK_PROCESSES: Mutex<Option<HashMap<u32, Child>>> = Mutex::new(None);

fn get_processes() -> parking_lot::MutexGuard<'static, Option<HashMap<u32, Child>>> {
    LOOPBACK_PROCESSES.lock()
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
    pub sink_node_id: u32,
    pub loopback_output_node_id: Option<u32>,
}

/// Create a virtual sink using pw-loopback.
pub fn create_virtual_sink_full(
    name: &str,
    description: &str,
) -> Result<VirtualSinkResult, VirtualSinkError> {
    ensure_processes_map();

    let safe_name: String = name
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    let sink_node_name = format!("sootmix.{}", safe_name);
    let loopback_node_name = format!("sootmix.{}.output", safe_name);

    // Check if a node with this name already exists and destroy it first
    if let Ok(existing_id) = find_node_by_name(&sink_node_name) {
        warn!(
            "Found existing orphaned node '{}' (id={}), destroying it first",
            sink_node_name, existing_id
        );

        // Remove from process tracking map before destroying
        if let Some(ref mut map) = *get_processes() {
            if let Some(mut child) = map.remove(&existing_id) {
                info!(
                    "Killing orphaned pw-loopback process for node {}",
                    existing_id
                );
                let _ = child.kill();
                let _ = child.wait();
            }
        }

        if let Err(e) = Command::new("pw-cli")
            .args(["destroy", &existing_id.to_string()])
            .output()
        {
            warn!("pw-cli destroy failed for node {}: {}", existing_id, e);
        }
        // Also destroy the output node if it exists
        let full_loopback_name = format!("output.{}", loopback_node_name);
        if let Ok(output_id) =
            find_node_by_name_and_class(&full_loopback_name, "Stream/Output/Audio")
        {
            // Remove output node from process tracking as well (in case it was tracked separately)
            if let Some(ref mut map) = *get_processes() {
                if let Some(mut child) = map.remove(&output_id) {
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
            if let Err(e) = Command::new("pw-cli")
                .args(["destroy", &output_id.to_string()])
                .output()
            {
                warn!("pw-cli destroy failed for output node {}: {}", output_id, e);
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    let capture_props = format!(
        "media.class=Audio/Sink node.name={} node.description=\"{}\" audio.position=[FL FR] priority.session=2000",
        sink_node_name, description
    );

    let playback_props =
        "media.class=Stream/Output/Audio node.autoconnect=false audio.position=[FL FR]".to_string();

    info!(
        "Creating virtual sink: {} (description: {})",
        sink_node_name, description
    );

    let child = Command::new("pw-loopback")
        .arg("--name")
        .arg(&loopback_node_name)
        .arg("--capture-props")
        .arg(&capture_props)
        .arg("--playback-props")
        .arg(&playback_props)
        .spawn()?;

    let pid = child.id();
    debug!("pw-loopback spawned with PID: {}", pid);

    std::thread::sleep(std::time::Duration::from_millis(200));

    let sink_node_id = find_node_by_name(&sink_node_name)?;
    let full_loopback_name = format!("output.{}", loopback_node_name);
    let loopback_output_node_id =
        find_node_by_name_and_class(&full_loopback_name, "Stream/Output/Audio").ok();

    if let Some(ref mut map) = *get_processes() {
        map.insert(sink_node_id, child);
    }

    info!(
        "Created virtual sink '{}' with sink_id={}, loopback_output_id={:?}",
        sink_node_name, sink_node_id, loopback_output_node_id
    );

    Ok(VirtualSinkResult {
        sink_node_id,
        loopback_output_node_id,
    })
}

/// Update the description of an existing node.
pub fn update_node_description(
    node_id: u32,
    new_description: &str,
) -> Result<(), VirtualSinkError> {
    info!(
        "Updating node {} description to '{}'",
        node_id, new_description
    );

    let props_json = format!(
        "{{ params = [ \"node.description\" \"{}\" ] }}",
        new_description.replace('"', "\\\"")
    );

    let output = Command::new("pw-cli")
        .args(["set-param", &node_id.to_string(), "Props", &props_json])
        .output()
        .map_err(|e| VirtualSinkError::PwDumpFailed(format!("Failed to run pw-cli: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(VirtualSinkError::PwDumpFailed(format!(
            "pw-cli set-param failed: {}",
            stderr.trim()
        )));
    }

    Ok(())
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

    warn!(
        "No tracked process for node {}, attempting pw-cli destroy",
        node_id
    );
    match Command::new("pw-cli")
        .args(["destroy", &node_id.to_string()])
        .output()
    {
        Ok(output) if !output.status.success() => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("pw-cli destroy failed for node {}: {}", node_id, stderr);
        }
        Err(e) => {
            warn!("pw-cli destroy failed for node {}: {}", node_id, e);
        }
        _ => {}
    }

    Ok(())
}

/// Destroy all virtual sinks (cleanup on exit).
/// This kills tracked pw-loopback processes and cleans up any orphaned nodes
/// that may have been created by processes that respawned after PipeWire restart.
pub fn destroy_all_virtual_sinks() {
    info!("Destroying all virtual sinks");

    // First, kill all tracked pw-loopback processes
    if let Some(ref mut map) = *get_processes() {
        for (node_id, mut child) in map.drain() {
            debug!(
                "Killing pw-loopback for sink node {} (pid: {:?})",
                node_id,
                child.id()
            );
            if let Err(e) = child.kill() {
                // Process may already be dead if PipeWire restarted
                debug!("Process kill returned error (may be already dead): {}", e);
            }
            let _ = child.wait();
        }
    }

    // Also kill any pw-loopback processes that may have respawned
    // This handles the case where PipeWire restarted and pw-loopback auto-restarted
    // with a new PID that we don't have tracked
    let _ = std::process::Command::new("pkill")
        .args(["-f", "pw-loopback.*--name.*sootmix\\."])
        .output();

    // Give processes time to die and nodes to be removed
    std::thread::sleep(std::time::Duration::from_millis(100));
}

/// Clean up orphaned sootmix nodes from previous runs.
/// This should be called on daemon startup before creating new channels.
pub fn cleanup_orphaned_nodes() {
    info!("Scanning for orphaned sootmix nodes...");

    let output = match Command::new("pw-dump").output() {
        Ok(o) => o,
        Err(e) => {
            warn!("Failed to run pw-dump for orphan cleanup: {}", e);
            return;
        }
    };

    if !output.status.success() {
        warn!("pw-dump failed during orphan cleanup");
        return;
    }

    let json_str = String::from_utf8_lossy(&output.stdout);
    let objects: Vec<serde_json::Value> = match serde_json::from_str(&json_str) {
        Ok(o) => o,
        Err(_) => {
            warn!("Failed to parse pw-dump JSON for orphan cleanup");
            return;
        }
    };

    let mut orphaned_ids: Vec<u32> = Vec::new();

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

        // Check if this is a sootmix node (either sink or output)
        if node_name.starts_with("sootmix.") || node_name.starts_with("output.sootmix.") {
            if let Some(id) = obj.get("id").and_then(|v| v.as_u64()) {
                orphaned_ids.push(id as u32);
            }
        }
    }

    if orphaned_ids.is_empty() {
        info!("No orphaned sootmix nodes found");
        return;
    }

    info!(
        "Found {} orphaned sootmix nodes, cleaning up...",
        orphaned_ids.len()
    );
    for id in orphaned_ids {
        debug!("Destroying orphaned node {}", id);
        if let Err(e) = Command::new("pw-cli")
            .args(["destroy", &id.to_string()])
            .output()
        {
            warn!("pw-cli destroy failed for orphaned node {}: {}", id, e);
        }
    }

    // Give PipeWire time to process the destructions
    std::thread::sleep(std::time::Duration::from_millis(200));
    info!("Orphan cleanup complete");
}

fn find_node_by_name(name: &str) -> Result<u32, VirtualSinkError> {
    find_node_by_name_and_class(name, "Audio/Sink")
}

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
    let objects: Vec<serde_json::Value> =
        serde_json::from_str(&json_str).map_err(|_| VirtualSinkError::InvalidJson)?;

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
        let media_class = props
            .and_then(|p| p.get("media.class"))
            .and_then(|c| c.as_str())
            .unwrap_or("");

        if node_name == name && media_class == target_class {
            if let Some(id) = obj.get("id").and_then(|v| v.as_u64()) {
                debug!(
                    "Found node '{}' with ID {} (class={})",
                    name, id, media_class
                );
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

    for line in status_str.lines() {
        if line.contains(name) {
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

/// Result of creating a virtual source.
#[derive(Debug, Clone, Copy)]
pub struct VirtualSourceResult {
    pub source_node_id: u32,
    pub capture_stream_node_id: Option<u32>,
}

/// Create a virtual source for recording.
/// If `target_device` is Some, the capture stream will target that device (mic).
pub fn create_virtual_source(name: &str, target_device: Option<&str>) -> Result<VirtualSourceResult, VirtualSinkError> {
    ensure_processes_map();

    let safe_name = name
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
        .collect::<String>();

    let source_name = format!("sootmix.{}", safe_name);
    let loopback_node_name = format!("sootmix.{}.input", safe_name);

    // Build capture props - ALWAYS disable autoconnect to prevent WirePlumber from
    // linking the capture stream to all available sources. We'll manage links ourselves.
    let capture_props = if let Some(device) = target_device {
        format!(
            "media.class=Stream/Input/Audio node.passive=true node.autoconnect=false audio.position=[FL FR] target.object=\"{}\"",
            device
        )
    } else {
        "media.class=Stream/Input/Audio node.passive=true node.autoconnect=false audio.position=[FL FR]".to_string()
    };

    // IMPORTANT: node.virtual=false prevents WirePlumber from hiding this node.
    // device.class=audio-input classifies it as a user-facing input device.
    // Without these, the node won't appear in Helvum or other patchbays.
    let playback_props = format!(
        "media.class=Audio/Source node.name={} node.description=\"{}\" \
         node.virtual=false device.class=audio-input \
         audio.position=[FL FR] priority.session=2000",
        source_name, name
    );

    info!("Creating virtual source: {} (description: {}, target: {:?})", source_name, name, target_device);

    let child = Command::new("pw-loopback")
        .arg("--name")
        .arg(&loopback_node_name)
        .arg("--capture-props")
        .arg(&capture_props)
        .arg("--playback-props")
        .arg(&playback_props)
        .spawn()?;

    let pid = child.id();
    debug!("pw-loopback (source) spawned with PID: {}", pid);

    std::thread::sleep(std::time::Duration::from_millis(200));

    let source_node_id = find_node_by_name_and_class(&source_name, "Audio/Source")?;

    // Find the capture stream node (the input side that captures from the mic)
    let input_node_name = format!("input.{}", loopback_node_name);
    let capture_stream_node_id = find_node_by_name_and_class(&input_node_name, "Stream/Input/Audio").ok();

    if let Some(ref mut map) = *get_processes() {
        map.insert(source_node_id, child);
    }

    info!(
        "Created virtual source '{}' with source_id={}, capture_stream_id={:?}",
        name, source_node_id, capture_stream_node_id
    );

    Ok(VirtualSourceResult {
        source_node_id,
        capture_stream_node_id,
    })
}

/// Destroy a virtual source.
pub fn destroy_virtual_source(node_id: u32) -> Result<(), VirtualSinkError> {
    destroy_virtual_sink(node_id)
}
