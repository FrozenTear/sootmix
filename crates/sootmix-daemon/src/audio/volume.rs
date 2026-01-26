// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Volume and mute control using wpctl.

use std::process::Command;
use thiserror::Error;
use tracing::{debug, error, info};

#[derive(Debug, Error)]
pub enum VolumeError {
    #[error("Failed to execute wpctl: {0}")]
    WpctlFailed(String),
    #[error("Volume operation failed: {0}")]
    OperationFailed(String),
}

/// Set volume on a node (linear scale: 0.0 = silent, 1.0 = 100%).
pub fn set_volume(node_id: u32, volume: f32) -> Result<(), VolumeError> {
    let volume_clamped = volume.max(0.0).min(1.5);

    info!("wpctl set-volume {} {:.2}", node_id, volume_clamped);

    let output = Command::new("wpctl")
        .args(["set-volume", &node_id.to_string(), &format!("{:.2}", volume_clamped)])
        .output()
        .map_err(|e| {
            error!("Failed to execute wpctl: {}", e);
            VolumeError::WpctlFailed(e.to_string())
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        error!("wpctl set-volume {} failed: {}", node_id, stderr.trim());
        return Err(VolumeError::OperationFailed(format!(
            "wpctl set-volume {} {:.2} failed: {}",
            node_id, volume_clamped, stderr
        )));
    }

    Ok(())
}

/// Set mute state on a node.
pub fn set_mute(node_id: u32, muted: bool) -> Result<(), VolumeError> {
    let mute_value = if muted { "1" } else { "0" };

    info!("wpctl set-mute {} {}", node_id, mute_value);

    let output = Command::new("wpctl")
        .args(["set-mute", &node_id.to_string(), mute_value])
        .output()
        .map_err(|e| {
            error!("Failed to execute wpctl set-mute: {}", e);
            VolumeError::WpctlFailed(e.to_string())
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        error!("wpctl set-mute {} failed: {}", node_id, stderr.trim());
        return Err(VolumeError::OperationFailed(stderr.to_string()));
    }

    Ok(())
}

/// Get current volume of a node. Returns (volume, is_muted).
pub fn get_volume(node_id: u32) -> Result<(f32, bool), VolumeError> {
    let output = Command::new("wpctl")
        .args(["get-volume", &node_id.to_string()])
        .output()
        .map_err(|e| VolumeError::WpctlFailed(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(VolumeError::OperationFailed(stderr.to_string()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut volume = 1.0;
    let mut muted = false;

    for part in stdout.split_whitespace() {
        if let Ok(v) = part.parse::<f32>() {
            volume = v;
        }
        if part.contains("MUTED") {
            muted = true;
        }
    }

    Ok((volume, muted))
}

/// Set volume on the default sink.
pub fn set_default_sink_volume(volume: f32) -> Result<(), VolumeError> {
    let volume_clamped = volume.max(0.0).min(1.5);

    debug!("Setting default sink volume to {:.2}", volume_clamped);

    let output = Command::new("wpctl")
        .args(["set-volume", "@DEFAULT_AUDIO_SINK@", &format!("{:.2}", volume_clamped)])
        .output()
        .map_err(|e| VolumeError::WpctlFailed(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(VolumeError::OperationFailed(stderr.to_string()));
    }

    Ok(())
}

/// Set mute on the default sink.
pub fn set_default_sink_mute(muted: bool) -> Result<(), VolumeError> {
    let mute_value = if muted { "1" } else { "0" };

    debug!("Setting default sink mute to {}", muted);

    let output = Command::new("wpctl")
        .args(["set-mute", "@DEFAULT_AUDIO_SINK@", mute_value])
        .output()
        .map_err(|e| VolumeError::WpctlFailed(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(VolumeError::OperationFailed(stderr.to_string()));
    }

    Ok(())
}
