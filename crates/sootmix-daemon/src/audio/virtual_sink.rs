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
    pub sink_node_id: u32,
    pub loopback_output_node_id: Option<u32>,
}

/// Create a virtual sink using pw-loopback.
pub fn create_virtual_sink_full(name: &str, description: &str) -> Result<VirtualSinkResult, VirtualSinkError> {
    ensure_processes_map();

    let safe_name: String = name
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    let sink_node_name = format!("sootmix.{}", safe_name);
    let loopback_node_name = format!("sootmix.{}.output", safe_name);

    let capture_props = format!(
        "media.class=Audio/Sink node.name={} node.description=\"{}\" audio.position=[FL FR] priority.session=2000",
        sink_node_name, description
    );

    let playback_props = "media.class=Stream/Output/Audio node.autoconnect=false audio.position=[FL FR]".to_string();

    info!("Creating virtual sink: {} (description: {})", sink_node_name, description);

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
    let loopback_output_node_id = find_node_by_name_and_class(&full_loopback_name, "Stream/Output/Audio").ok();

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

/// Update the description of an existing node.
pub fn update_node_description(node_id: u32, new_description: &str) -> Result<(), VirtualSinkError> {
    info!("Updating node {} description to '{}'", node_id, new_description);

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

    warn!("No tracked process for node {}, attempting pw-cli destroy", node_id);
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
    let objects: Vec<serde_json::Value> = serde_json::from_str(&json_str)
        .map_err(|_| VirtualSinkError::InvalidJson)?;

    for obj in objects {
        let obj_type = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if obj_type != "PipeWire:Interface:Node" {
            continue;
        }

        let props = obj.get("info").and_then(|i| i.get("props"));
        let node_name = props.and_then(|p| p.get("node.name")).and_then(|n| n.as_str()).unwrap_or("");
        let media_class = props.and_then(|p| p.get("media.class")).and_then(|c| c.as_str()).unwrap_or("");

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
pub fn create_virtual_source(name: &str) -> Result<VirtualSourceResult, VirtualSinkError> {
    ensure_processes_map();

    let safe_name = name
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
        .collect::<String>();

    let source_name = format!("sootmix.recording.{}", safe_name);

    let capture_props = "media.class=Stream/Input/Audio node.passive=true audio.position=[FL FR]".to_string();
    let playback_props = format!(
        "media.class=Audio/Source node.name={} node.description=\"SootMix Recording - {}\" audio.position=[FL FR]",
        source_name, name
    );

    info!("Creating virtual source for recording: {}", name);

    let child = Command::new("pw-loopback")
        .arg("--capture-props")
        .arg(&capture_props)
        .arg("--playback-props")
        .arg(&playback_props)
        .spawn()?;

    let pid = child.id();
    debug!("pw-loopback (source) spawned with PID: {}", pid);

    std::thread::sleep(std::time::Duration::from_millis(200));

    let source_node_id = find_node_by_name_and_class(&source_name, "Audio/Source")?;

    if let Some(ref mut map) = *get_processes() {
        map.insert(source_node_id, child);
    }

    info!("Created virtual source '{}' with source_id={}", name, source_node_id);

    Ok(VirtualSourceResult {
        source_node_id,
        capture_stream_node_id: None,
    })
}

/// Destroy a virtual source.
pub fn destroy_virtual_source(node_id: u32) -> Result<(), VirtualSinkError> {
    destroy_virtual_sink(node_id)
}
