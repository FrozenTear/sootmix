// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Volume and mute control using wpctl.

use std::process::Command;
use thiserror::Error;
use tracing::{debug, info};

#[derive(Debug, Error)]
pub enum VolumeError {
    #[error("Failed to execute wpctl: {0}")]
    WpctlFailed(String),
    #[error("Volume operation failed: {0}")]
    OperationFailed(String),
}

/// Set volume on a node.
///
/// Volume is in linear scale (0.0 = silent, 1.0 = 100%, >1.0 = boost).
pub fn set_volume(node_id: u32, volume: f32) -> Result<(), VolumeError> {
    let volume_clamped = volume.max(0.0).min(1.5); // Allow up to 150%

    debug!("Setting volume on node {} to {:.2}", node_id, volume_clamped);

    let output = Command::new("wpctl")
        .args(["set-volume", &node_id.to_string(), &format!("{:.2}", volume_clamped)])
        .output()
        .map_err(|e| VolumeError::WpctlFailed(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(VolumeError::OperationFailed(stderr.to_string()));
    }

    Ok(())
}

/// Set volume by percentage change.
pub fn adjust_volume(node_id: u32, delta_percent: i32) -> Result<(), VolumeError> {
    let delta_str = if delta_percent >= 0 {
        format!("{}%+", delta_percent)
    } else {
        format!("{}%-", delta_percent.abs())
    };

    debug!("Adjusting volume on node {} by {}", node_id, delta_str);

    let output = Command::new("wpctl")
        .args(["set-volume", &node_id.to_string(), &delta_str])
        .output()
        .map_err(|e| VolumeError::WpctlFailed(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(VolumeError::OperationFailed(stderr.to_string()));
    }

    Ok(())
}

/// Set mute state on a node.
pub fn set_mute(node_id: u32, muted: bool) -> Result<(), VolumeError> {
    let mute_value = if muted { "1" } else { "0" };

    debug!("Setting mute on node {} to {}", node_id, muted);

    let output = Command::new("wpctl")
        .args(["set-mute", &node_id.to_string(), mute_value])
        .output()
        .map_err(|e| VolumeError::WpctlFailed(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(VolumeError::OperationFailed(stderr.to_string()));
    }

    Ok(())
}

/// Toggle mute state on a node.
pub fn toggle_mute(node_id: u32) -> Result<(), VolumeError> {
    debug!("Toggling mute on node {}", node_id);

    let output = Command::new("wpctl")
        .args(["set-mute", &node_id.to_string(), "toggle"])
        .output()
        .map_err(|e| VolumeError::WpctlFailed(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(VolumeError::OperationFailed(stderr.to_string()));
    }

    Ok(())
}

/// Get current volume of a node.
///
/// Returns (volume, is_muted).
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
    // Output format: "Volume: 1.00" or "Volume: 0.50 [MUTED]"

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
        .args([
            "set-volume",
            "@DEFAULT_AUDIO_SINK@",
            &format!("{:.2}", volume_clamped),
        ])
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
