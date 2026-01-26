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
pub fn create_link_by_name(output_port_name: &str, input_port_name: &str) -> Result<(), RoutingError> {
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
pub fn destroy_link_by_name(output_port_name: &str, input_port_name: &str) -> Result<(), RoutingError> {
    info!("Destroying link: {} -> {}", output_port_name, input_port_name);

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
