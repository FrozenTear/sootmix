// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Plugin registry for downloadable plugin packs.
//!
//! Contains metadata about available plugin packs from various sources
//! (LSP, Calf, x42, ZAM) that can be downloaded and installed.

use serde::{Deserialize, Serialize};
use sootmix_plugin_api::PluginCategory;

/// A downloadable plugin pack containing multiple LV2 plugins.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginPack {
    /// Unique identifier for this pack.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Vendor/developer name.
    pub vendor: String,
    /// Version string.
    pub version: String,
    /// Description of what the pack contains.
    pub description: String,
    /// Download URL (GitHub release tarball).
    pub download_url: String,
    /// Approximate download size in bytes.
    pub file_size: u64,
    /// Individual plugins included in this pack.
    pub plugins: Vec<PluginInfo>,
    /// Whether download is supported (some packs use unsupported archive formats).
    #[serde(default = "default_downloadable")]
    pub downloadable: bool,
}

fn default_downloadable() -> bool {
    true
}

/// Information about an individual plugin within a pack.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInfo {
    /// LV2 URI or identifier.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Plugin category.
    pub category: PluginCategory,
}

/// Get all available plugin packs.
pub fn get_available_packs() -> Vec<PluginPack> {
    vec![
        lsp_plugins_pack(),
        calf_plugins_pack(),
        x42_plugins_pack(),
        zam_plugins_pack(),
    ]
}

/// Get a specific pack by ID.
pub fn get_pack_by_id(id: &str) -> Option<PluginPack> {
    get_available_packs().into_iter().find(|p| p.id == id)
}

/// LSP Plugins pack - comprehensive suite of professional audio plugins.
fn lsp_plugins_pack() -> PluginPack {
    PluginPack {
        id: "lsp-plugins".to_string(),
        name: "LSP Plugins".to_string(),
        vendor: "Linux Studio Plugins".to_string(),
        version: "1.2.26".to_string(),
        description: "Professional suite with EQ, compressors, limiters, gates, analyzers, and more".to_string(),
        // LSP releases use .7z format (contains all plugin formats including LV2)
        download_url: "https://github.com/lsp-plugins/lsp-plugins/releases/download/1.2.26/lsp-plugins-1.2.26-Linux-x86_64.7z".to_string(),
        file_size: 45_000_000, // ~45 MB
        downloadable: true,
        plugins: vec![
            PluginInfo { id: "http://lsp-plug.in/plugins/lv2/para_equalizer_x32_stereo".to_string(), name: "Parametric Equalizer x32 Stereo".to_string(), category: PluginCategory::Eq },
            PluginInfo { id: "http://lsp-plug.in/plugins/lv2/para_equalizer_x16_stereo".to_string(), name: "Parametric Equalizer x16 Stereo".to_string(), category: PluginCategory::Eq },
            PluginInfo { id: "http://lsp-plug.in/plugins/lv2/graph_equalizer_x32_stereo".to_string(), name: "Graphic Equalizer x32 Stereo".to_string(), category: PluginCategory::Eq },
            PluginInfo { id: "http://lsp-plug.in/plugins/lv2/compressor_stereo".to_string(), name: "Compressor Stereo".to_string(), category: PluginCategory::Dynamics },
            PluginInfo { id: "http://lsp-plug.in/plugins/lv2/sc_compressor_stereo".to_string(), name: "Sidechain Compressor Stereo".to_string(), category: PluginCategory::Dynamics },
            PluginInfo { id: "http://lsp-plug.in/plugins/lv2/mb_compressor_stereo".to_string(), name: "Multiband Compressor Stereo".to_string(), category: PluginCategory::Dynamics },
            PluginInfo { id: "http://lsp-plug.in/plugins/lv2/dyna_processor_stereo".to_string(), name: "Dynamic Processor Stereo".to_string(), category: PluginCategory::Dynamics },
            PluginInfo { id: "http://lsp-plug.in/plugins/lv2/limiter_stereo".to_string(), name: "Limiter Stereo".to_string(), category: PluginCategory::Dynamics },
            PluginInfo { id: "http://lsp-plug.in/plugins/lv2/mb_limiter_stereo".to_string(), name: "Multiband Limiter Stereo".to_string(), category: PluginCategory::Dynamics },
            PluginInfo { id: "http://lsp-plug.in/plugins/lv2/gate_stereo".to_string(), name: "Gate Stereo".to_string(), category: PluginCategory::Dynamics },
            PluginInfo { id: "http://lsp-plug.in/plugins/lv2/expander_stereo".to_string(), name: "Expander Stereo".to_string(), category: PluginCategory::Dynamics },
            PluginInfo { id: "http://lsp-plug.in/plugins/lv2/loud_comp_stereo".to_string(), name: "Loudness Compensator Stereo".to_string(), category: PluginCategory::Dynamics },
            PluginInfo { id: "http://lsp-plug.in/plugins/lv2/spectrum_analyzer_x12".to_string(), name: "Spectrum Analyzer x12".to_string(), category: PluginCategory::Analyzer },
            PluginInfo { id: "http://lsp-plug.in/plugins/lv2/phase_detector".to_string(), name: "Phase Detector".to_string(), category: PluginCategory::Analyzer },
            PluginInfo { id: "http://lsp-plug.in/plugins/lv2/oscilloscope_x4".to_string(), name: "Oscilloscope x4".to_string(), category: PluginCategory::Analyzer },
            PluginInfo { id: "http://lsp-plug.in/plugins/lv2/crossover_stereo".to_string(), name: "Crossover Stereo".to_string(), category: PluginCategory::Filter },
            PluginInfo { id: "http://lsp-plug.in/plugins/lv2/filter_stereo".to_string(), name: "Filter Stereo".to_string(), category: PluginCategory::Filter },
            PluginInfo { id: "http://lsp-plug.in/plugins/lv2/impulse_reverb_stereo".to_string(), name: "Impulse Reverb Stereo".to_string(), category: PluginCategory::Reverb },
            PluginInfo { id: "http://lsp-plug.in/plugins/lv2/room_builder_stereo".to_string(), name: "Room Builder Stereo".to_string(), category: PluginCategory::Reverb },
            PluginInfo { id: "http://lsp-plug.in/plugins/lv2/comp_delay_stereo".to_string(), name: "Compensation Delay Stereo".to_string(), category: PluginCategory::Delay },
            PluginInfo { id: "http://lsp-plug.in/plugins/lv2/slap_delay_stereo".to_string(), name: "Slap Delay Stereo".to_string(), category: PluginCategory::Delay },
            PluginInfo { id: "http://lsp-plug.in/plugins/lv2/chorus_stereo".to_string(), name: "Chorus Stereo".to_string(), category: PluginCategory::Modulation },
            PluginInfo { id: "http://lsp-plug.in/plugins/lv2/flanger_stereo".to_string(), name: "Flanger Stereo".to_string(), category: PluginCategory::Modulation },
            PluginInfo { id: "http://lsp-plug.in/plugins/lv2/phaser_stereo".to_string(), name: "Phaser Stereo".to_string(), category: PluginCategory::Modulation },
            PluginInfo { id: "http://lsp-plug.in/plugins/lv2/trigger_stereo".to_string(), name: "Trigger Stereo".to_string(), category: PluginCategory::Other },
            PluginInfo { id: "http://lsp-plug.in/plugins/lv2/clipper_stereo".to_string(), name: "Clipper Stereo".to_string(), category: PluginCategory::Distortion },
            PluginInfo { id: "http://lsp-plug.in/plugins/lv2/surge_filter_stereo".to_string(), name: "Surge Filter Stereo".to_string(), category: PluginCategory::Filter },
            PluginInfo { id: "http://lsp-plug.in/plugins/lv2/noise_generator_stereo".to_string(), name: "Noise Generator Stereo".to_string(), category: PluginCategory::Generator },
            PluginInfo { id: "http://lsp-plug.in/plugins/lv2/oscillator_stereo".to_string(), name: "Oscillator Stereo".to_string(), category: PluginCategory::Generator },
            PluginInfo { id: "http://lsp-plug.in/plugins/lv2/mixer_x8_stereo".to_string(), name: "Mixer x8 Stereo".to_string(), category: PluginCategory::Utility },
            PluginInfo { id: "http://lsp-plug.in/plugins/lv2/latency_meter".to_string(), name: "Latency Meter".to_string(), category: PluginCategory::Analyzer },
            PluginInfo { id: "http://lsp-plug.in/plugins/lv2/profiler_stereo".to_string(), name: "Profiler Stereo".to_string(), category: PluginCategory::Analyzer },
        ],
    }
}

/// Calf Studio Gear - versatile plugins with great UIs.
fn calf_plugins_pack() -> PluginPack {
    PluginPack {
        id: "calf-plugins".to_string(),
        name: "Calf Studio Gear".to_string(),
        vendor: "Calf Studio Gear".to_string(),
        version: "0.90.3".to_string(),
        description: "Full-featured studio plugins with synthesizers, effects, and analyzers".to_string(),
        // Calf doesn't provide binary releases - must be built from source
        download_url: "https://github.com/calf-studio-gear/calf/releases".to_string(),
        file_size: 25_000_000, // ~25 MB
        downloadable: false, // No binary releases available
        plugins: vec![
            PluginInfo { id: "http://calf.sourceforge.net/plugins/Equalizer5Band".to_string(), name: "Equalizer 5 Band".to_string(), category: PluginCategory::Eq },
            PluginInfo { id: "http://calf.sourceforge.net/plugins/Equalizer8Band".to_string(), name: "Equalizer 8 Band".to_string(), category: PluginCategory::Eq },
            PluginInfo { id: "http://calf.sourceforge.net/plugins/Equalizer12Band".to_string(), name: "Equalizer 12 Band".to_string(), category: PluginCategory::Eq },
            PluginInfo { id: "http://calf.sourceforge.net/plugins/Compressor".to_string(), name: "Compressor".to_string(), category: PluginCategory::Dynamics },
            PluginInfo { id: "http://calf.sourceforge.net/plugins/Sidechain Compressor".to_string(), name: "Sidechain Compressor".to_string(), category: PluginCategory::Dynamics },
            PluginInfo { id: "http://calf.sourceforge.net/plugins/MultibandCompressor".to_string(), name: "Multiband Compressor".to_string(), category: PluginCategory::Dynamics },
            PluginInfo { id: "http://calf.sourceforge.net/plugins/Limiter".to_string(), name: "Limiter".to_string(), category: PluginCategory::Dynamics },
            PluginInfo { id: "http://calf.sourceforge.net/plugins/MultibandLimiter".to_string(), name: "Multiband Limiter".to_string(), category: PluginCategory::Dynamics },
            PluginInfo { id: "http://calf.sourceforge.net/plugins/Gate".to_string(), name: "Gate".to_string(), category: PluginCategory::Dynamics },
            PluginInfo { id: "http://calf.sourceforge.net/plugins/Transient Designer".to_string(), name: "Transient Designer".to_string(), category: PluginCategory::Dynamics },
            PluginInfo { id: "http://calf.sourceforge.net/plugins/Deesser".to_string(), name: "De-esser".to_string(), category: PluginCategory::Dynamics },
            PluginInfo { id: "http://calf.sourceforge.net/plugins/Reverb".to_string(), name: "Reverb".to_string(), category: PluginCategory::Reverb },
            PluginInfo { id: "http://calf.sourceforge.net/plugins/VintageDelay".to_string(), name: "Vintage Delay".to_string(), category: PluginCategory::Delay },
            PluginInfo { id: "http://calf.sourceforge.net/plugins/MultiChorus".to_string(), name: "Multi Chorus".to_string(), category: PluginCategory::Modulation },
            PluginInfo { id: "http://calf.sourceforge.net/plugins/Flanger".to_string(), name: "Flanger".to_string(), category: PluginCategory::Modulation },
            PluginInfo { id: "http://calf.sourceforge.net/plugins/Phaser".to_string(), name: "Phaser".to_string(), category: PluginCategory::Modulation },
            PluginInfo { id: "http://calf.sourceforge.net/plugins/Rotary Speaker".to_string(), name: "Rotary Speaker".to_string(), category: PluginCategory::Modulation },
            PluginInfo { id: "http://calf.sourceforge.net/plugins/Pulsator".to_string(), name: "Pulsator".to_string(), category: PluginCategory::Modulation },
            PluginInfo { id: "http://calf.sourceforge.net/plugins/Filter".to_string(), name: "Filter".to_string(), category: PluginCategory::Filter },
            PluginInfo { id: "http://calf.sourceforge.net/plugins/Filterclavier".to_string(), name: "Filterclavier".to_string(), category: PluginCategory::Filter },
            PluginInfo { id: "http://calf.sourceforge.net/plugins/Saturator".to_string(), name: "Saturator".to_string(), category: PluginCategory::Distortion },
            PluginInfo { id: "http://calf.sourceforge.net/plugins/TapeSimulator".to_string(), name: "Tape Simulator".to_string(), category: PluginCategory::Distortion },
            PluginInfo { id: "http://calf.sourceforge.net/plugins/Crusher".to_string(), name: "Crusher".to_string(), category: PluginCategory::Distortion },
            PluginInfo { id: "http://calf.sourceforge.net/plugins/Analyzer".to_string(), name: "Analyzer".to_string(), category: PluginCategory::Analyzer },
            PluginInfo { id: "http://calf.sourceforge.net/plugins/StereoTools".to_string(), name: "Stereo Tools".to_string(), category: PluginCategory::Utility },
            PluginInfo { id: "http://calf.sourceforge.net/plugins/Haas Stereo Enhancer".to_string(), name: "Haas Stereo Enhancer".to_string(), category: PluginCategory::Utility },
            PluginInfo { id: "http://calf.sourceforge.net/plugins/MonoInput".to_string(), name: "Mono Input".to_string(), category: PluginCategory::Utility },
            PluginInfo { id: "http://calf.sourceforge.net/plugins/BassEnhancer".to_string(), name: "Bass Enhancer".to_string(), category: PluginCategory::Other },
            PluginInfo { id: "http://calf.sourceforge.net/plugins/Exciter".to_string(), name: "Exciter".to_string(), category: PluginCategory::Other },
            PluginInfo { id: "http://calf.sourceforge.net/plugins/Monosynth".to_string(), name: "Monosynth".to_string(), category: PluginCategory::Synth },
            PluginInfo { id: "http://calf.sourceforge.net/plugins/Organ".to_string(), name: "Organ".to_string(), category: PluginCategory::Synth },
            PluginInfo { id: "http://calf.sourceforge.net/plugins/Wavetable".to_string(), name: "Wavetable".to_string(), category: PluginCategory::Synth },
        ],
    }
}

/// x42 Plugins - high-quality professional meters and effects.
fn x42_plugins_pack() -> PluginPack {
    PluginPack {
        id: "x42-plugins".to_string(),
        name: "x42 Plugins".to_string(),
        vendor: "Robin Gareus".to_string(),
        version: "0.6.6".to_string(),
        description: "Professional meters, EQ, and stereo tools with minimal latency".to_string(),
        // x42 releases use .tar.xz format from gareus.org
        download_url: "http://gareus.org/misc/x42-plugins/x42-plugins-20260125.tar.xz".to_string(),
        file_size: 15_000_000, // ~15 MB
        downloadable: true, // .tar.xz supported
        plugins: vec![
            PluginInfo { id: "http://gareus.org/oss/lv2/fil4#stereo".to_string(), name: "4-Band Parametric EQ (Stereo)".to_string(), category: PluginCategory::Eq },
            PluginInfo { id: "http://gareus.org/oss/lv2/fil4#mono".to_string(), name: "4-Band Parametric EQ (Mono)".to_string(), category: PluginCategory::Eq },
            PluginInfo { id: "http://gareus.org/oss/lv2/darc#stereo".to_string(), name: "Dynamic Audio Range Compressor".to_string(), category: PluginCategory::Dynamics },
            PluginInfo { id: "http://gareus.org/oss/lv2/dpl#stereo".to_string(), name: "Digital Peak Limiter".to_string(), category: PluginCategory::Dynamics },
            PluginInfo { id: "http://gareus.org/oss/lv2/meters#goniometer".to_string(), name: "Goniometer".to_string(), category: PluginCategory::Analyzer },
            PluginInfo { id: "http://gareus.org/oss/lv2/meters#spectr30stereo".to_string(), name: "Spectrum Analyzer 30-Band".to_string(), category: PluginCategory::Analyzer },
            PluginInfo { id: "http://gareus.org/oss/lv2/meters#dr14stereo".to_string(), name: "DR-14 Loudness Meter".to_string(), category: PluginCategory::Analyzer },
            PluginInfo { id: "http://gareus.org/oss/lv2/meters#K20stereo".to_string(), name: "K-Meter (K-20)".to_string(), category: PluginCategory::Analyzer },
            PluginInfo { id: "http://gareus.org/oss/lv2/meters#VUstereo".to_string(), name: "VU Meter".to_string(), category: PluginCategory::Analyzer },
            PluginInfo { id: "http://gareus.org/oss/lv2/meters#TPnRMSstereo".to_string(), name: "True Peak + RMS Meter".to_string(), category: PluginCategory::Analyzer },
            PluginInfo { id: "http://gareus.org/oss/lv2/meters#EBUr128".to_string(), name: "EBU R128 Loudness Meter".to_string(), category: PluginCategory::Analyzer },
            PluginInfo { id: "http://gareus.org/oss/lv2/meters#bitmeter".to_string(), name: "Bit Meter".to_string(), category: PluginCategory::Analyzer },
            PluginInfo { id: "http://gareus.org/oss/lv2/stereoroute#route".to_string(), name: "Stereo Routing".to_string(), category: PluginCategory::Utility },
            PluginInfo { id: "http://gareus.org/oss/lv2/balance#stereo".to_string(), name: "Stereo Balance Control".to_string(), category: PluginCategory::Utility },
            PluginInfo { id: "http://gareus.org/oss/lv2/mixtri#lv2".to_string(), name: "Mixer/Trigger".to_string(), category: PluginCategory::Utility },
            PluginInfo { id: "http://gareus.org/oss/lv2/nodelay#stereo".to_string(), name: "No-Latency Convolution".to_string(), category: PluginCategory::Reverb },
            PluginInfo { id: "http://gareus.org/oss/lv2/tuna#one".to_string(), name: "Instrument Tuner".to_string(), category: PluginCategory::Analyzer },
            PluginInfo { id: "http://gareus.org/oss/lv2/matrixmixer#i4o4".to_string(), name: "Matrix Mixer 4x4".to_string(), category: PluginCategory::Utility },
            PluginInfo { id: "http://gareus.org/oss/lv2/midifilter#keysplit".to_string(), name: "MIDI Key Split".to_string(), category: PluginCategory::Utility },
            PluginInfo { id: "http://gareus.org/oss/lv2/testsignal#lv2".to_string(), name: "Test Signal Generator".to_string(), category: PluginCategory::Generator },
        ],
    }
}

/// ZAM Plugins - dynamics and saturation specialists.
fn zam_plugins_pack() -> PluginPack {
    PluginPack {
        id: "zam-plugins".to_string(),
        name: "ZAM Plugins".to_string(),
        vendor: "Damien Zammit".to_string(),
        version: "4.4".to_string(),
        description: "High-quality dynamics processors and saturation effects".to_string(),
        download_url: "https://github.com/zamaudio/zam-plugins/releases/download/4.4/zam-plugins-4.4-linux-x86_64.tar.xz".to_string(),
        file_size: 8_000_000, // ~8 MB
        downloadable: true, // .tar.xz supported
        plugins: vec![
            PluginInfo { id: "urn:zamaudio:ZamComp".to_string(), name: "ZamComp".to_string(), category: PluginCategory::Dynamics },
            PluginInfo { id: "urn:zamaudio:ZamCompX2".to_string(), name: "ZamCompX2 (Stereo)".to_string(), category: PluginCategory::Dynamics },
            PluginInfo { id: "urn:zamaudio:ZamAutoSat".to_string(), name: "ZamAutoSat".to_string(), category: PluginCategory::Distortion },
            PluginInfo { id: "urn:zamaudio:ZamTube".to_string(), name: "ZamTube".to_string(), category: PluginCategory::Distortion },
            PluginInfo { id: "urn:zamaudio:ZamValve".to_string(), name: "ZamValve".to_string(), category: PluginCategory::Distortion },
            PluginInfo { id: "urn:zamaudio:ZamGate".to_string(), name: "ZamGate".to_string(), category: PluginCategory::Dynamics },
            PluginInfo { id: "urn:zamaudio:ZamGateX2".to_string(), name: "ZamGateX2 (Stereo)".to_string(), category: PluginCategory::Dynamics },
            PluginInfo { id: "urn:zamaudio:ZamGEQ31".to_string(), name: "ZamGEQ31".to_string(), category: PluginCategory::Eq },
            PluginInfo { id: "urn:zamaudio:ZamEQ2".to_string(), name: "ZamEQ2".to_string(), category: PluginCategory::Eq },
            PluginInfo { id: "urn:zamaudio:ZamDelay".to_string(), name: "ZamDelay".to_string(), category: PluginCategory::Delay },
            PluginInfo { id: "urn:zamaudio:ZamDynamicEQ".to_string(), name: "ZamDynamicEQ".to_string(), category: PluginCategory::Eq },
            PluginInfo { id: "urn:zamaudio:ZamHeadX2".to_string(), name: "ZamHeadX2".to_string(), category: PluginCategory::Utility },
            PluginInfo { id: "urn:zamaudio:ZamChild670".to_string(), name: "ZamChild670".to_string(), category: PluginCategory::Dynamics },
            PluginInfo { id: "urn:zamaudio:ZaMultiComp".to_string(), name: "ZaMultiComp".to_string(), category: PluginCategory::Dynamics },
            PluginInfo { id: "urn:zamaudio:ZaMultiCompX2".to_string(), name: "ZaMultiCompX2 (Stereo)".to_string(), category: PluginCategory::Dynamics },
        ],
    }
}

/// Count total plugins across all packs.
pub fn total_plugin_count() -> usize {
    get_available_packs().iter().map(|p| p.plugins.len()).sum()
}

/// Format file size for display.
pub fn format_file_size(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1_000_000_000.0)
    } else if bytes >= 1_000_000 {
        format!("{:.0} MB", bytes as f64 / 1_000_000.0)
    } else if bytes >= 1_000 {
        format!("{:.0} KB", bytes as f64 / 1_000.0)
    } else {
        format!("{} bytes", bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_available_packs() {
        let packs = get_available_packs();
        assert_eq!(packs.len(), 4);
        assert!(packs.iter().any(|p| p.id == "lsp-plugins"));
        assert!(packs.iter().any(|p| p.id == "calf-plugins"));
        assert!(packs.iter().any(|p| p.id == "x42-plugins"));
        assert!(packs.iter().any(|p| p.id == "zam-plugins"));
    }

    #[test]
    fn test_get_pack_by_id() {
        let pack = get_pack_by_id("lsp-plugins").unwrap();
        assert_eq!(pack.name, "LSP Plugins");
        assert!(!pack.plugins.is_empty());
    }

    #[test]
    fn test_format_file_size() {
        assert_eq!(format_file_size(500), "500 bytes");
        assert_eq!(format_file_size(1500), "2 KB");
        assert_eq!(format_file_size(1_500_000), "2 MB");
        assert_eq!(format_file_size(1_500_000_000), "1.5 GB");
    }
}
