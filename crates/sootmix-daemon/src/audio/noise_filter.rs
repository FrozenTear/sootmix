// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Noise suppression filter using PipeWire filter-chain.
//!
//! Creates a filter node that processes audio through noise reduction.
//! Uses PipeWire's built-in speex:denoise filter or LADSPA RNNoise plugin.

use parking_lot::Mutex;
use std::collections::HashMap;
use std::io::Write;
use std::process::{Child, Command};
use tempfile::NamedTempFile;
use thiserror::Error;
use tracing::{debug, info, warn};
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum NoiseFilterError {
    #[error("Failed to spawn filter-chain: {0}")]
    SpawnFailed(#[from] std::io::Error),
    #[error("Failed to create config file: {0}")]
    ConfigFailed(String),
    #[error("Failed to find created node")]
    NodeNotFound,
    #[error("pw-dump failed: {0}")]
    PwDumpFailed(String),
    #[error("Invalid JSON from pw-dump")]
    InvalidJson,
}

/// Info about a running noise filter instance.
struct NoiseFilterInstance {
    child: Child,
    #[allow(dead_code)]
    config_file: NamedTempFile,
    #[allow(dead_code)]
    source_node_id: Option<u32>,
}

/// Track running filter processes for cleanup.
static NOISE_FILTER_PROCESSES: Mutex<Option<HashMap<Uuid, NoiseFilterInstance>>> = Mutex::new(None);

fn get_processes() -> parking_lot::MutexGuard<'static, Option<HashMap<Uuid, NoiseFilterInstance>>> {
    NOISE_FILTER_PROCESSES.lock()
}

fn ensure_processes_map() {
    let mut guard = get_processes();
    if guard.is_none() {
        *guard = Some(HashMap::new());
    }
}

/// Generate a filter-chain config for noise suppression.
///
/// Creates an Audio/Source node that:
/// - Captures from the specified input device (or default if None)
/// - Processes through noise suppression
/// - Outputs as a virtual source for applications to use
fn generate_noise_filter_config(
    channel_name: &str,
    input_device_name: Option<&str>,
    plugin_path: &str,
    plugin_label: &str,
    vad_threshold: f32,
) -> String {
    // Use same node name as regular loopback so apps don't need to reselect
    let source_node_name = format!("sootmix.{}", channel_name);

    // Build the filter chain configuration
    let mut config = String::new();

    config.push_str(&format!(
        r#"context.properties = {{
    core.daemon = false
    core.name = "sootmix-ns-{channel_name}"
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
            node.name = "{source_node_name}"
            node.description = "{channel_name}"
            media.name = "SootMix Input - {channel_name}"
            filter.graph = {{
                nodes = [
                    {{
                        type = ladspa
                        name = rnnoise
                        plugin = "{plugin_path}"
                        label = {plugin_label}
                        control = {{
                            "VAD Threshold" = {vad_threshold}
                        }}
                    }}
                ]
                inputs = [ "rnnoise:Input" ]
                outputs = [ "rnnoise:Output" ]
            }}
            capture.props = {{
                media.class = Stream/Input/Audio
                node.passive = true
                audio.position = [ MONO ]
"#
    ));

    // If a specific input device is specified, target it
    if let Some(device) = input_device_name.filter(|d| *d != "system-default") {
        config.push_str(&format!(
            r#"                target.object = "{device}"
"#
        ));
    }

    config.push_str(&format!(
        r#"            }}
            playback.props = {{
                media.class = Audio/Source
                node.name = "{source_node_name}"
                node.description = "{channel_name}"
                node.virtual = false
                device.class = audio-input
                audio.position = [ MONO ]
                priority.session = 2000
            }}
        }}
    }}
]
"#
    ));

    config
}

/// Generate a stereo noise filter config (two mono RNNoise instances).
fn generate_noise_filter_config_stereo(
    channel_name: &str,
    input_device_name: Option<&str>,
    plugin_path: &str,
    plugin_label: &str,
    vad_threshold: f32,
) -> String {
    // Use same node name as regular loopback so apps don't need to reselect
    let source_node_name = format!("sootmix.{}", channel_name);

    let mut config = String::new();

    config.push_str(&format!(
        r#"context.properties = {{
    core.daemon = false
    core.name = "sootmix-ns-{channel_name}"
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
            node.name = "{source_node_name}"
            node.description = "{channel_name}"
            media.name = "SootMix Input - {channel_name}"
            filter.graph = {{
                nodes = [
                    {{
                        type = ladspa
                        name = rnnoise_l
                        plugin = "{plugin_path}"
                        label = {plugin_label}
                        control = {{
                            "VAD Threshold" = {vad_threshold}
                        }}
                    }}
                    {{
                        type = ladspa
                        name = rnnoise_r
                        plugin = "{plugin_path}"
                        label = {plugin_label}
                        control = {{
                            "VAD Threshold" = {vad_threshold}
                        }}
                    }}
                ]
                inputs = [ "rnnoise_l:Input" "rnnoise_r:Input" ]
                outputs = [ "rnnoise_l:Output" "rnnoise_r:Output" ]
            }}
            capture.props = {{
                media.class = Stream/Input/Audio
                node.passive = true
                audio.position = [ FL FR ]
"#
    ));

    if let Some(device) = input_device_name.filter(|d| *d != "system-default") {
        config.push_str(&format!(
            r#"                target.object = "{device}"
"#
        ));
    }

    config.push_str(&format!(
        r#"            }}
            playback.props = {{
                media.class = Audio/Source
                node.name = "{source_node_name}"
                node.description = "{channel_name}"
                node.virtual = false
                device.class = audio-input
                audio.position = [ FL FR ]
                priority.session = 2000
            }}
        }}
    }}
]
"#
    ));

    config
}

/// Get the path to our built-in RNNoise LADSPA plugin.
fn get_builtin_rnnoise_path() -> Option<String> {
    // Check relative to the executable first (for installed version)
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            let lib_path = exe_dir.join("libsootmix_rnnoise_ladspa.so");
            if lib_path.exists() {
                return Some(lib_path.to_string_lossy().to_string());
            }
            // Check ../lib directory (common install layout)
            let lib_path = exe_dir.join("../lib/libsootmix_rnnoise_ladspa.so");
            if lib_path.exists() {
                return Some(lib_path.canonicalize().ok()?.to_string_lossy().to_string());
            }
        }
    }

    // Development: check target/release and target/debug
    let dev_paths = [
        "target/release/libsootmix_rnnoise_ladspa.so",
        "target/debug/libsootmix_rnnoise_ladspa.so",
        // Also try from daemon's working directory
        "../../../target/release/libsootmix_rnnoise_ladspa.so",
        "../../../target/debug/libsootmix_rnnoise_ladspa.so",
    ];

    for path in &dev_paths {
        let p = std::path::Path::new(path);
        if p.exists() {
            return p.canonicalize().ok().map(|p| p.to_string_lossy().to_string());
        }
    }

    None
}

/// Check if the RNNoise LADSPA plugin is available.
#[allow(dead_code)]
pub fn is_rnnoise_available() -> bool {
    // First check our built-in plugin
    if get_builtin_rnnoise_path().is_some() {
        return true;
    }

    // Fallback: check common locations for the system RNNoise LADSPA plugin
    let paths = [
        "/usr/lib/ladspa/librnnoise_ladspa.so",
        "/usr/lib64/ladspa/librnnoise_ladspa.so",
        "/usr/local/lib/ladspa/librnnoise_ladspa.so",
    ];

    for path in &paths {
        if std::path::Path::new(path).exists() {
            return true;
        }
    }

    // Also check LADSPA_PATH environment variable
    if let Ok(ladspa_path) = std::env::var("LADSPA_PATH") {
        for dir in ladspa_path.split(':') {
            let plugin_path = std::path::Path::new(dir).join("librnnoise_ladspa.so");
            if plugin_path.exists() {
                return true;
            }
        }
    }

    false
}

/// Get the plugin path and label for the RNNoise filter.
fn get_rnnoise_plugin_info() -> Option<(String, &'static str)> {
    // Prefer our built-in plugin
    if let Some(path) = get_builtin_rnnoise_path() {
        // Our plugin uses the same label as the original for compatibility
        return Some((path, "noise_suppressor_mono"));
    }

    // Fallback to system plugin
    let paths = [
        "/usr/lib/ladspa/librnnoise_ladspa.so",
        "/usr/lib64/ladspa/librnnoise_ladspa.so",
        "/usr/local/lib/ladspa/librnnoise_ladspa.so",
    ];

    for path in paths {
        if std::path::Path::new(path).exists() {
            return Some((path.to_string(), "noise_suppressor_mono"));
        }
    }

    None
}

/// Create a noise suppression filter for an input channel.
///
/// Returns the source node ID of the created virtual source.
pub fn create_noise_filter(
    channel_id: Uuid,
    channel_name: &str,
    input_device_name: Option<&str>,
    stereo: bool,
    vad_threshold: f32,
) -> Result<u32, NoiseFilterError> {
    ensure_processes_map();

    // Get plugin path and label
    let (plugin_path, plugin_label) = get_rnnoise_plugin_info().ok_or_else(|| {
        warn!("RNNoise LADSPA plugin not found.");
        NoiseFilterError::ConfigFailed("RNNoise LADSPA plugin not found".to_string())
    })?;

    info!("Using RNNoise plugin: {}", plugin_path);

    // Sanitize channel name
    let safe_name = channel_name
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
        .collect::<String>();

    // Generate config
    let config_content = if stereo {
        generate_noise_filter_config_stereo(&safe_name, input_device_name, &plugin_path, plugin_label, vad_threshold)
    } else {
        generate_noise_filter_config(&safe_name, input_device_name, &plugin_path, plugin_label, vad_threshold)
    };
    debug!("Generated noise filter config:\n{}", config_content);

    // Write to temp file
    let mut config_file = tempfile::Builder::new()
        .prefix("sootmix-ns-")
        .suffix(".conf")
        .tempfile()
        .map_err(|e| NoiseFilterError::ConfigFailed(e.to_string()))?;
    config_file
        .write_all(config_content.as_bytes())
        .map_err(|e| NoiseFilterError::ConfigFailed(e.to_string()))?;

    let config_path = config_file.path().to_string_lossy().to_string();
    info!(
        "Creating noise filter for channel '{}' with config: {}",
        channel_name, config_path
    );

    // Spawn pipewire with the filter-chain config
    let child = Command::new("pipewire").arg("-c").arg(&config_path).spawn()?;

    let pid = child.id();
    debug!("Noise filter spawned with PID: {}", pid);

    // Give it time to register with PipeWire
    std::thread::sleep(std::time::Duration::from_millis(300));

    // Find the source node ID (same name as regular loopback)
    let source_node_name = format!("sootmix.{}", safe_name);
    let source_node_id = find_node_by_name(&source_node_name, "Audio/Source")?;

    // Store the instance
    if let Some(ref mut map) = *get_processes() {
        map.insert(
            channel_id,
            NoiseFilterInstance {
                child,
                config_file,
                source_node_id: Some(source_node_id),
            },
        );
    }

    info!(
        "Created noise filter for '{}': source_node={}",
        channel_name, source_node_id
    );
    Ok(source_node_id)
}

/// Destroy a noise suppression filter.
pub fn destroy_noise_filter(channel_id: Uuid) -> Result<(), NoiseFilterError> {
    if let Some(ref mut map) = *get_processes() {
        if let Some(mut instance) = map.remove(&channel_id) {
            info!("Destroying noise filter for channel {}", channel_id);
            let _ = instance.child.kill();
            let _ = instance.child.wait();
            return Ok(());
        }
    }

    warn!("No tracked noise filter for channel {}", channel_id);
    Ok(())
}

/// Destroy all noise filters (cleanup on exit).
#[allow(dead_code)]
pub fn destroy_all_noise_filters() {
    if let Some(ref mut map) = *get_processes() {
        for (channel_id, mut instance) in map.drain() {
            info!("Cleaning up noise filter for channel {}", channel_id);
            let _ = instance.child.kill();
            let _ = instance.child.wait();
        }
    }
}

/// Get the source node ID for a channel's noise filter, if it exists.
#[allow(dead_code)]
pub fn get_noise_filter_node_id(channel_id: Uuid) -> Option<u32> {
    if let Some(ref map) = *get_processes() {
        if let Some(instance) = map.get(&channel_id) {
            return instance.source_node_id;
        }
    }
    None
}

/// Check if a channel has an active noise filter.
#[allow(dead_code)]
pub fn has_noise_filter(channel_id: Uuid) -> bool {
    if let Some(ref map) = *get_processes() {
        return map.contains_key(&channel_id);
    }
    false
}

/// Find a node by name and media class using pw-dump.
fn find_node_by_name(name: &str, media_class: &str) -> Result<u32, NoiseFilterError> {
    let output = Command::new("pw-dump")
        .output()
        .map_err(|e| NoiseFilterError::PwDumpFailed(e.to_string()))?;

    if !output.status.success() {
        return Err(NoiseFilterError::PwDumpFailed(
            String::from_utf8_lossy(&output.stderr).to_string(),
        ));
    }

    let json_str = String::from_utf8_lossy(&output.stdout);
    let objects: Vec<serde_json::Value> =
        serde_json::from_str(&json_str).map_err(|_| NoiseFilterError::InvalidJson)?;

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
                debug!(
                    "Found node '{}' (class={}) with ID {}",
                    name, media_class, id
                );
                return Ok(id as u32);
            }
        }
    }

    Err(NoiseFilterError::NodeNotFound)
}
