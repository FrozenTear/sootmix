// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! VST3 plugin scanning and metadata extraction.

use super::factory::{tuid_to_string, Vst3Module};
use crate::plugins::PluginLoadError;
use sootmix_plugin_api::PluginCategory;
use std::ffi::CStr;
use std::path::{Path, PathBuf};
use tracing::debug;
use vst3::Steinberg::TUID;

/// Metadata for a discovered VST3 plugin.
#[derive(Debug, Clone)]
pub struct Vst3PluginMeta {
    /// VST3 class ID (GUID as hex string).
    pub class_id: String,
    /// VST3 class ID as TUID.
    pub tuid: TUID,
    /// Human-readable name.
    pub name: String,
    /// Plugin vendor.
    pub vendor: String,
    /// Plugin version.
    pub version: String,
    /// Sub-categories (comma-separated string from VST3).
    pub sub_categories: String,
    /// Mapped category for SootMix.
    pub category: PluginCategory,
    /// Path to the .vst3 bundle.
    pub bundle_path: PathBuf,
    /// SDK version.
    pub sdk_version: String,
    /// Number of audio input buses.
    pub audio_inputs: u32,
    /// Number of audio output buses.
    pub audio_outputs: u32,
}

/// Scan a VST3 bundle for plugins.
pub fn scan_bundle(bundle_path: &Path) -> Result<Vec<Vst3PluginMeta>, PluginLoadError> {
    let module = Vst3Module::load(bundle_path)?;
    let mut plugins = Vec::new();

    let class_count = module.class_count();
    debug!("VST3 bundle {:?} has {} classes", bundle_path, class_count);

    for i in 0..class_count {
        // Try to get extended class info first
        if let Some(info2) = module.get_class_info2(i) {
            // Check if this is an Audio component
            let category = unsafe {
                CStr::from_ptr(info2.category.as_ptr())
                    .to_str()
                    .unwrap_or("")
            };

            if category != "Audio Module Class" {
                continue;
            }

            let name = unsafe {
                CStr::from_ptr(info2.name.as_ptr())
                    .to_str()
                    .unwrap_or("Unknown")
                    .to_string()
            };

            let vendor = unsafe {
                CStr::from_ptr(info2.vendor.as_ptr())
                    .to_str()
                    .unwrap_or("Unknown")
                    .to_string()
            };

            let version = unsafe {
                CStr::from_ptr(info2.version.as_ptr())
                    .to_str()
                    .unwrap_or("1.0.0")
                    .to_string()
            };

            let sdk_version = unsafe {
                CStr::from_ptr(info2.sdkVersion.as_ptr())
                    .to_str()
                    .unwrap_or("")
                    .to_string()
            };

            let sub_categories = unsafe {
                CStr::from_ptr(info2.subCategories.as_ptr())
                    .to_str()
                    .unwrap_or("")
                    .to_string()
            };

            let class_id = tuid_to_string(&info2.cid);
            let plugin_category = map_vst3_subcategories(&sub_categories);

            // Count audio buses would require instantiating the component
            // For now, assume stereo in/out
            let meta = Vst3PluginMeta {
                class_id,
                tuid: info2.cid,
                name,
                vendor,
                version,
                sub_categories,
                category: plugin_category,
                bundle_path: bundle_path.to_path_buf(),
                sdk_version,
                audio_inputs: 2,
                audio_outputs: 2,
            };

            plugins.push(meta);
        } else if let Some(info) = module.get_class_info(i) {
            // Fall back to basic class info
            let category = unsafe {
                CStr::from_ptr(info.category.as_ptr())
                    .to_str()
                    .unwrap_or("")
            };

            if category != "Audio Module Class" {
                continue;
            }

            let name = unsafe {
                CStr::from_ptr(info.name.as_ptr())
                    .to_str()
                    .unwrap_or("Unknown")
                    .to_string()
            };

            let class_id = tuid_to_string(&info.cid);

            let meta = Vst3PluginMeta {
                class_id,
                tuid: info.cid,
                name,
                vendor: "Unknown".to_string(),
                version: "1.0.0".to_string(),
                sub_categories: String::new(),
                category: PluginCategory::Other,
                bundle_path: bundle_path.to_path_buf(),
                sdk_version: String::new(),
                audio_inputs: 2,
                audio_outputs: 2,
            };

            plugins.push(meta);
        }
    }

    Ok(plugins)
}

/// Map VST3 sub-categories to SootMix category.
fn map_vst3_subcategories(sub_categories: &str) -> PluginCategory {
    let cats = sub_categories.to_lowercase();

    // Check for specific categories (order matters - more specific first)
    if cats.contains("eq") || cats.contains("filter") {
        PluginCategory::Eq
    } else if cats.contains("dynamics")
        || cats.contains("compressor")
        || cats.contains("limiter")
        || cats.contains("gate")
        || cats.contains("expander")
    {
        PluginCategory::Dynamics
    } else if cats.contains("reverb") || cats.contains("delay") || cats.contains("echo") {
        PluginCategory::Reverb
    } else if cats.contains("modulation")
        || cats.contains("chorus")
        || cats.contains("flanger")
        || cats.contains("phaser")
    {
        PluginCategory::Modulation
    } else if cats.contains("distortion")
        || cats.contains("overdrive")
        || cats.contains("saturation")
    {
        PluginCategory::Distortion
    } else if cats.contains("analyzer")
        || cats.contains("meter")
        || cats.contains("tools")
        || cats.contains("utility")
    {
        PluginCategory::Utility
    } else if cats.contains("fx") {
        // Generic effect
        PluginCategory::Other
    } else {
        PluginCategory::Other
    }
}
