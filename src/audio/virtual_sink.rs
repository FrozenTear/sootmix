// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Virtual sink creation and management using pw-loopback.

#![allow(dead_code, unused_imports)]

use std::collections::HashMap;
use std::process::{Child, Command};
use parking_lot::Mutex;
use thiserror::Error;
use tracing::{debug, info, trace, warn};

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

/// Track running pw-loopback processes for cleanup.
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
    /// The Audio/Sink node ID (apps connect here).
    pub sink_node_id: u32,
    /// The Stream/Output/Audio node ID (loopback output for routing).
    pub loopback_output_node_id: Option<u32>,
}

/// Create a virtual sink using pw-loopback.
///
/// Uses the channel name for node.name (readable in tools like Helvum).
/// The name is sanitized to be safe for PipeWire properties.
///
/// Returns the node ID of the created sink and optionally the loopback output node ID.
pub fn create_virtual_sink(name: &str, description: &str) -> Result<u32, VirtualSinkError> {
    let result = create_virtual_sink_full(name, description)?;
    Ok(result.sink_node_id)
}

/// Create a virtual sink using pw-loopback.
///
/// Uses the channel name for node.name (readable in tools like Helvum).
/// The name is sanitized to be safe for PipeWire properties.
///
/// Returns both the sink node ID and the loopback output node ID.
pub fn create_virtual_sink_full(name: &str, description: &str) -> Result<VirtualSinkResult, VirtualSinkError> {
    ensure_processes_map();

    // Sanitize name for use in node.name (readable in Helvum etc.)
    let safe_name: String = name
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    let sink_node_name = format!("sootmix.{}", safe_name);
    let loopback_node_name = format!("sootmix.{}.output", safe_name);

    // Set high priority.session so WirePlumber prefers our virtual sinks over hardware outputs.
    // This prevents WirePlumber from auto-linking apps directly to hardware, bypassing our mixer.
    // Default hardware sinks typically have priority ~1000-1500.
    let capture_props = format!(
        "media.class=Audio/Sink node.name={} node.description=\"{}\" audio.position=[FL FR] priority.session=2000",
        sink_node_name, description
    );

    // Playback props for the loopback output stream.
    // IMPORTANT: We set stream.capture.sink=false and volume=1.0 to ensure unity gain.
    // The object.linger=true keeps the stream alive and prevents WirePlumber from
    // resetting it. session.suspend-timeout-enabled=false prevents auto-suspend.
    let playback_props = format!(
        "media.class=Stream/Output/Audio node.autoconnect=false audio.position=[FL FR] \
         object.linger=true session.suspend-timeout-enabled=false stream.capture.sink=false"
    );

    info!("Creating virtual sink: {} (description: {})", sink_node_name, description);
    debug!("capture_props: {}", capture_props);
    debug!("playback_props: {}", playback_props);

    // Spawn pw-loopback as a background process
    // --name sets the node name for the loopback output (playback side)
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

    // Poll for node registration instead of blocking sleep.
    // Retry with exponential backoff: 20ms, 40ms, 80ms, 160ms (total max ~300ms)
    let sink_node_id = poll_for_node(&sink_node_name, 4, 20)?;

    // Find the loopback output node ID
    // Note: pw-loopback adds "output." prefix to the --name value
    let full_loopback_name = format!("output.{}", loopback_node_name);
    let loopback_output_node_id = find_node_by_name_and_class(&full_loopback_name, "Stream/Output/Audio").ok();

    // Track the process
    if let Some(ref mut map) = *get_processes() {
        map.insert(sink_node_id, child);
    }

    info!("Created virtual sink '{}' with sink_id={}, loopback_output_id={:?}",
          sink_node_name, sink_node_id, loopback_output_node_id);

    Ok(VirtualSinkResult {
        sink_node_id,
        loopback_output_node_id,
    })
}

/// Result of creating a virtual source (for mic/input channels).
#[derive(Debug, Clone, Copy)]
pub struct VirtualSourceResult {
    /// The Audio/Source node ID (recording apps connect here).
    pub source_node_id: u32,
    /// The Stream/Input/Audio node ID (loopback capture from physical device).
    pub loopback_capture_node_id: Option<u32>,
}

/// Create a virtual source using pw-loopback (reverse direction).
///
/// This creates a pw-loopback that:
/// - Captures audio from a physical input device (mic)
/// - Exposes an Audio/Source node that recording apps (Discord, OBS) can use
///
/// The capture side becomes the virtual source, and the playback side
/// is a Stream/Input/Audio that reads from the physical device.
pub fn create_virtual_source(name: &str, description: &str) -> Result<VirtualSourceResult, VirtualSinkError> {
    ensure_processes_map();

    let safe_name: String = name
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    let source_node_name = format!("sootmix.{}", safe_name);
    let loopback_node_name = format!("sootmix.{}.capture", safe_name);

    // The "playback" side of pw-loopback becomes our Audio/Source
    // (apps record from this). We swap the media.class roles.
    // IMPORTANT: node.virtual=false prevents WirePlumber from hiding this node.
    // device.class=audio-input classifies it as a user-facing input device.
    // Without these, the node won't appear in Helvum or other patchbays.
    let playback_props = format!(
        "media.class=Audio/Source node.name={} node.description=\"{}\" \
         node.virtual=false device.class=audio-input \
         audio.position=[MONO] priority.session=2000",
        source_node_name, description
    );

    // The "capture" side reads from the system — it's a Stream/Input/Audio
    // that captures from the physical input device.
    // IMPORTANT: We use node.autoconnect=false so we can explicitly link to
    // the user's selected input device rather than the system default.
    let capture_props = format!(
        "media.class=Stream/Input/Audio node.autoconnect=false audio.position=[MONO] \
         object.linger=true session.suspend-timeout-enabled=false"
    );

    info!("Creating virtual source: {} (description: {})", source_node_name, description);
    debug!("playback_props (source): {}", playback_props);
    debug!("capture_props (stream): {}", capture_props);

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

    // Poll for the Audio/Source node (NOT Audio/Sink!)
    let source_node_id = poll_for_node_with_class(&source_node_name, "Audio/Source", 4, 20)?;

    // Find the loopback capture node
    let full_loopback_name = format!("capture.{}", loopback_node_name);
    let loopback_capture_node_id = find_node_by_name_and_class(&full_loopback_name, "Stream/Input/Audio").ok();

    // Track the process
    if let Some(ref mut map) = *get_processes() {
        map.insert(source_node_id, child);
    }

    info!("Created virtual source '{}' with source_id={}, loopback_capture_id={:?}",
          source_node_name, source_node_id, loopback_capture_node_id);

    Ok(VirtualSourceResult {
        source_node_id,
        loopback_capture_node_id,
    })
}

/// Update the description of an existing node using pw-cli.
///
/// This allows renaming a channel without recreating the virtual sink,
/// avoiding any audio interruption.
pub fn update_node_description(node_id: u32, new_description: &str) -> Result<(), VirtualSinkError> {
    info!("Updating node {} description to '{}'", node_id, new_description);

    // Use pw-cli to set the node.description property
    // Format: pw-cli set-param <node_id> Props '{ params = [ "node.description" "<value>" ] }'
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
        warn!("pw-cli set-param failed: {}", stderr);
        // Don't fail - the description update is cosmetic
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

    // Fallback: try to destroy via pw-cli
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
/// Kills tracked pw-loopback processes and cleans up any orphaned nodes
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
                debug!("Process kill returned error (may be already dead): {}", e);
            }
            let _ = child.wait();
        }
    }

    // Also kill any pw-loopback processes that may have respawned
    // with a new PID that we don't have tracked
    if let Err(e) = Command::new("pkill")
        .args(["-f", "pw-loopback.*--name.*sootmix\\."])
        .output()
    {
        warn!("pkill fallback failed: {}", e);
    }

    // Give processes time to die and nodes to be removed
    std::thread::sleep(std::time::Duration::from_millis(100));
}

/// Clean up orphaned sootmix nodes from previous runs.
/// This should be called on startup before creating new channels.
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
            warn!("Failed to destroy orphaned node {}: {}", id, e);
        }
    }

    // Give PipeWire time to process the destructions
    std::thread::sleep(std::time::Duration::from_millis(200));
    info!("Orphan cleanup complete");
}

/// Poll for a node to appear with exponential backoff.
///
/// This is more efficient than a single long sleep, as the node often
/// registers quickly and we can return early.
///
/// # Arguments
/// * `name` - The node name to search for
/// * `max_retries` - Maximum number of retries
/// * `initial_delay_ms` - Initial delay in milliseconds (doubles each retry)
fn poll_for_node(name: &str, max_retries: u32, initial_delay_ms: u64) -> Result<u32, VirtualSinkError> {
    poll_for_node_with_class(name, "Audio/Sink", max_retries, initial_delay_ms)
}

/// Poll for a node with specific media class to appear with exponential backoff.
fn poll_for_node_with_class(
    name: &str,
    media_class: &str,
    max_retries: u32,
    initial_delay_ms: u64,
) -> Result<u32, VirtualSinkError> {
    use std::time::Duration;

    let mut delay_ms = initial_delay_ms;
    for attempt in 0..max_retries {
        // Small initial delay to let PipeWire register the node
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(delay_ms));
            delay_ms *= 2; // Exponential backoff
        } else {
            // First attempt: minimal delay
            std::thread::sleep(Duration::from_millis(10));
        }

        match find_node_by_name_and_class(name, media_class) {
            Ok(node_id) => {
                debug!("Found node '{}' (class={}) on attempt {}", name, media_class, attempt + 1);
                return Ok(node_id);
            }
            Err(_) if attempt < max_retries - 1 => {
                trace!("Node '{}' not found yet, retrying in {}ms...", name, delay_ms);
                continue;
            }
            Err(e) => return Err(e),
        }
    }

    Err(VirtualSinkError::NodeNotFound)
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

// Note: VirtualSourceResult and create_virtual_source are defined above (near create_virtual_sink_full).

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
