// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Audio routing - creating and managing links between nodes.

use std::process::Command;
use thiserror::Error;
use tracing::{debug, info};

#[derive(Debug, Error)]
pub enum RoutingError {
    #[error("Failed to execute pw-link: {0}")]
    PwLinkFailed(String),
    #[error("Link creation failed: {0}")]
    LinkFailed(String),
}

/// Create a link between two ports using pw-link.
pub fn create_link(output_port: u32, input_port: u32) -> Result<(), RoutingError> {
    info!("Creating link: port {} -> port {}", output_port, input_port);

    let output = Command::new("pw-link")
        .arg("-L")
        .arg(output_port.to_string())
        .arg(input_port.to_string())
        .output()
        .map_err(|e| RoutingError::PwLinkFailed(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.contains("already linked") {
            return Err(RoutingError::LinkFailed(stderr.to_string()));
        }
        debug!("Ports already linked");
    }

    Ok(())
}

/// Create a link between two ports by name.
pub fn create_link_by_name(
    output_port_name: &str,
    input_port_name: &str,
) -> Result<(), RoutingError> {
    info!("Creating link: {} -> {}", output_port_name, input_port_name);

    let output = Command::new("pw-link")
        .arg("-L")
        .arg(output_port_name)
        .arg(input_port_name)
        .output()
        .map_err(|e| RoutingError::PwLinkFailed(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.contains("already linked") {
            return Err(RoutingError::LinkFailed(stderr.to_string()));
        }
    }

    Ok(())
}

/// Destroy a link by its link ID.
pub fn destroy_link(link_id: u32) -> Result<(), RoutingError> {
    info!("Destroying link: {}", link_id);

    let output = Command::new("pw-link")
        .arg("-d")
        .arg(link_id.to_string())
        .output()
        .map_err(|e| RoutingError::PwLinkFailed(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(RoutingError::LinkFailed(stderr.to_string()));
    }

    Ok(())
}

/// Destroy a link by port names.
pub fn destroy_link_by_name(
    output_port_name: &str,
    input_port_name: &str,
) -> Result<(), RoutingError> {
    info!(
        "Destroying link: {} -> {}",
        output_port_name, input_port_name
    );

    let output = Command::new("pw-link")
        .arg("-d")
        .arg(output_port_name)
        .arg(input_port_name)
        .output()
        .map_err(|e| RoutingError::PwLinkFailed(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(RoutingError::LinkFailed(stderr.to_string()));
    }

    Ok(())
}

/// Set the target sink for a stream node using WirePlumber metadata.
/// This tells WirePlumber to route the stream to a specific sink and prevents
/// it from auto-linking to the default sink.
pub fn set_stream_target(stream_node_id: u32, target_sink_id: u32) -> Result<(), RoutingError> {
    info!(
        "Setting stream {} target to sink {}",
        stream_node_id, target_sink_id
    );

    // Use pw-metadata to set the target.node for this stream
    // This is the WirePlumber-compatible way to force routing
    let output = Command::new("pw-metadata")
        .args([
            "-n",
            "default",
            &stream_node_id.to_string(),
            "target.node",
            &target_sink_id.to_string(),
        ])
        .output()
        .map_err(|e| RoutingError::PwLinkFailed(format!("pw-metadata failed: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        debug!("pw-metadata set target.node failed: {}", stderr);
        // Continue anyway - the link creation may still work
    }

    // Give WirePlumber time to process the metadata change before we manipulate links.
    // Without this delay, WirePlumber may race with our link creation and re-route
    // the stream to a different sink.
    std::thread::sleep(std::time::Duration::from_millis(50));

    // Verify the metadata was actually set
    if let Some(current_target) = get_stream_target(stream_node_id) {
        if current_target != target_sink_id {
            debug!(
                "Warning: stream {} target is {} but we set {}, retrying",
                stream_node_id, current_target, target_sink_id
            );
            // Retry once
            let _ = Command::new("pw-metadata")
                .args([
                    "-n",
                    "default",
                    &stream_node_id.to_string(),
                    "target.node",
                    &target_sink_id.to_string(),
                ])
                .output();
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
    }

    Ok(())
}

/// Get the current target sink for a stream node from WirePlumber metadata.
pub fn get_stream_target(stream_node_id: u32) -> Option<u32> {
    let output = Command::new("pw-metadata")
        .args(["-n", "default", &stream_node_id.to_string(), "target.node"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    // Output format: "found 'target.node' '42'"
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if line.contains("target.node") {
            // Extract the value after the last single quote pair
            if let Some(start) = line.rfind('\'') {
                let before_end = &line[..start];
                if let Some(value_start) = before_end.rfind('\'') {
                    let value_str = &before_end[value_start + 1..];
                    return value_str.trim().parse().ok();
                }
            }
        }
    }
    None
}

/// Get the WirePlumber default audio sink node ID.
///
/// Uses `wpctl inspect @DEFAULT_AUDIO_SINK@` to find the system's current
/// default output device. Returns `None` if the command fails or no default
/// is set.
pub fn get_default_sink_id() -> Option<u32> {
    let output = Command::new("wpctl")
        .args(["inspect", "@DEFAULT_AUDIO_SINK@"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    // First line looks like: "id 42, type PipeWire:Interface:Node/3"
    let stdout = String::from_utf8_lossy(&output.stdout);
    let first_line = stdout.lines().next()?;
    let id_str = first_line
        .strip_prefix("id ")?
        .split(',')
        .next()?
        .trim();
    let id = id_str.parse::<u32>().ok()?;
    debug!("WirePlumber default sink: node {}", id);
    Some(id)
}

/// Clear the target sink for a stream node, allowing WirePlumber to manage it again.
pub fn clear_stream_target(stream_node_id: u32) -> Result<(), RoutingError> {
    info!("Clearing stream {} target", stream_node_id);

    let output = Command::new("pw-metadata")
        .args([
            "-n",
            "default",
            "-d",
            &stream_node_id.to_string(),
            "target.node",
        ])
        .output()
        .map_err(|e| RoutingError::PwLinkFailed(format!("pw-metadata failed: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        debug!("pw-metadata delete target.node failed: {}", stderr);
    }

    Ok(())
}
