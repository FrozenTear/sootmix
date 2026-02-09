// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! SootMix Plugin API
//!
//! This crate defines the interface for audio effect plugins in SootMix.
//! Plugins can be either native (using abi_stable) or WASM-based.
//!
//! # Example Plugin
//!
//! ```ignore
//! use sootmix_plugin_api::*;
//!
//! pub struct MyGainPlugin {
//!     gain: f32,
//!     sample_rate: f32,
//! }
//!
//! impl AudioEffect for MyGainPlugin {
//!     fn info(&self) -> PluginInfo {
//!         PluginInfo {
//!             id: "com.example.my-gain".into(),
//!             name: "My Gain".into(),
//!             vendor: "Example".into(),
//!             version: "1.0.0".into(),
//!             category: PluginCategory::Utility,
//!             input_channels: 2,
//!             output_channels: 2,
//!         }
//!     }
//!
//!     fn process(&mut self, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
//!         for (input, output) in inputs.iter().zip(outputs.iter_mut()) {
//!             for (i, o) in input.iter().zip(output.iter_mut()) {
//!                 *o = *i * self.gain;
//!             }
//!         }
//!     }
//!     // ... implement other methods
//! }
//! ```

#![warn(missing_docs)]
#![allow(non_local_definitions)]

use abi_stable::{
    sabi_trait,
    std_types::{RBox, ROption, RResult, RSlice, RSliceMut, RString, RVec},
    StableAbi,
};
use serde::{Deserialize, Serialize};

/// API version for compatibility checking.
/// Increment MAJOR for breaking changes, MINOR for additions.
pub const API_VERSION_MAJOR: u32 = 0;
/// Minor API version.
pub const API_VERSION_MINOR: u32 = 1;

// ============================================================================
// Plugin Metadata
// ============================================================================

/// Plugin category for UI organization and filtering.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, StableAbi, Serialize, Deserialize)]
pub enum PluginCategory {
    /// Equalizers, filters, tone shaping.
    Eq,
    /// Compressors, limiters, gates, expanders.
    Dynamics,
    /// Reverb effects.
    Reverb,
    /// Delay, echo effects.
    Delay,
    /// Modulation effects (chorus, flanger, phaser).
    Modulation,
    /// Distortion, saturation, amp simulation.
    Distortion,
    /// Utility (gain, pan, phase, routing).
    Utility,
    /// Analyzers, meters, visualization.
    Analyzer,
    /// Filters, crossovers.
    Filter,
    /// Signal generators, oscillators.
    Generator,
    /// Synthesizers, instruments.
    Synth,
    /// Multi-effects or uncategorized.
    Other,
}

impl Default for PluginCategory {
    fn default() -> Self {
        Self::Other
    }
}

/// Metadata describing a plugin.
#[repr(C)]
#[derive(Debug, Clone, StableAbi)]
pub struct PluginInfo {
    /// Unique identifier (reverse domain notation recommended).
    /// Example: "org.sootmix.eq.parametric"
    pub id: RString,

    /// Human-readable name.
    pub name: RString,

    /// Plugin vendor/author.
    pub vendor: RString,

    /// Version string (semver recommended).
    pub version: RString,

    /// Plugin category.
    pub category: PluginCategory,

    /// Number of input audio channels (usually 2 for stereo).
    pub input_channels: u32,

    /// Number of output audio channels (usually 2 for stereo).
    pub output_channels: u32,
}

impl PluginInfo {
    /// Create a new PluginInfo with required fields.
    pub fn new(id: &str, name: &str) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            vendor: "Unknown".into(),
            version: "0.0.0".into(),
            category: PluginCategory::Other,
            input_channels: 2,
            output_channels: 2,
        }
    }

    /// Builder: set vendor.
    pub fn with_vendor(mut self, vendor: &str) -> Self {
        self.vendor = vendor.into();
        self
    }

    /// Builder: set version.
    pub fn with_version(mut self, version: &str) -> Self {
        self.version = version.into();
        self
    }

    /// Builder: set category.
    pub fn with_category(mut self, category: PluginCategory) -> Self {
        self.category = category;
        self
    }

    /// Builder: set channel counts.
    pub fn with_channels(mut self, input: u32, output: u32) -> Self {
        self.input_channels = input;
        self.output_channels = output;
        self
    }
}

// ============================================================================
// Parameters
// ============================================================================

/// Hint about special parameter behavior.
///
/// Used to enable special features like sidechain routing.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, StableAbi)]
pub enum ParameterHint {
    /// No special behavior.
    #[default]
    None,
    /// This parameter receives level from a sidechain source channel.
    /// The host will automatically update this parameter with the RMS level
    /// of the sidechain source channel (if configured).
    SidechainLevel,
}

/// Parameter value range and characteristics.
#[repr(C)]
#[derive(Debug, Clone, StableAbi)]
pub struct ParameterInfo {
    /// Parameter index (0-based).
    pub index: u32,

    /// Internal identifier for automation/presets.
    pub id: RString,

    /// Display name.
    pub name: RString,

    /// Unit label (e.g., "dB", "Hz", "%").
    pub unit: RString,

    /// Minimum value.
    pub min: f32,

    /// Maximum value.
    pub max: f32,

    /// Default value.
    pub default: f32,

    /// Value curve/scaling.
    pub curve: ParameterCurve,

    /// Step size for discrete parameters (0.0 for continuous).
    pub step: f32,

    /// Hint about special parameter behavior.
    pub hint: ParameterHint,
}

impl ParameterInfo {
    /// Create a new continuous parameter.
    pub fn new(index: u32, id: &str, name: &str, min: f32, max: f32, default: f32) -> Self {
        Self {
            index,
            id: id.into(),
            name: name.into(),
            unit: RString::new(),
            min,
            max,
            default,
            curve: ParameterCurve::Linear,
            step: 0.0,
            hint: ParameterHint::None,
        }
    }

    /// Builder: set parameter hint.
    pub fn with_hint(mut self, hint: ParameterHint) -> Self {
        self.hint = hint;
        self
    }

    /// Builder: set unit label.
    pub fn with_unit(mut self, unit: &str) -> Self {
        self.unit = unit.into();
        self
    }

    /// Builder: set curve type.
    pub fn with_curve(mut self, curve: ParameterCurve) -> Self {
        self.curve = curve;
        self
    }

    /// Builder: set step size (makes parameter discrete).
    pub fn with_step(mut self, step: f32) -> Self {
        self.step = step;
        self
    }
}

/// Parameter value scaling/curve type.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, StableAbi, Default)]
pub enum ParameterCurve {
    /// Linear mapping from min to max.
    #[default]
    Linear,
    /// Logarithmic (good for frequency, gain).
    Logarithmic,
    /// Exponential curve.
    Exponential,
    /// Symmetric around center (good for pan, EQ gain).
    Symmetric,
}

// ============================================================================
// Plugin Errors
// ============================================================================

/// Errors that can occur during plugin operations.
#[repr(C)]
#[derive(Debug, Clone, StableAbi)]
pub enum PluginError {
    /// Failed to initialize plugin.
    InitializationFailed(RString),
    /// Invalid parameter index.
    InvalidParameter(u32),
    /// Invalid parameter value.
    InvalidValue {
        /// Parameter index.
        param: u32,
        /// The invalid value.
        value: f32,
    },
    /// Failed to load state.
    StateLoadFailed(RString),
    /// Failed to save state.
    StateSaveFailed(RString),
    /// Generic error with message.
    Other(RString),
}

impl std::fmt::Display for PluginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InitializationFailed(msg) => write!(f, "initialization failed: {}", msg),
            Self::InvalidParameter(idx) => write!(f, "invalid parameter index: {}", idx),
            Self::InvalidValue { param, value } => {
                write!(f, "invalid value {} for parameter {}", value, param)
            }
            Self::StateLoadFailed(msg) => write!(f, "failed to load state: {}", msg),
            Self::StateSaveFailed(msg) => write!(f, "failed to save state: {}", msg),
            Self::Other(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for PluginError {}

// ============================================================================
// Audio Effect Trait
// ============================================================================

/// Context provided to plugins during activation.
#[repr(C)]
#[derive(Debug, Clone, StableAbi)]
pub struct ActivationContext {
    /// Sample rate in Hz.
    pub sample_rate: f32,
    /// Maximum number of samples per process call.
    pub max_block_size: u32,
}

/// The main trait that all audio effect plugins must implement.
///
/// # Safety
///
/// The `process` method is called on a real-time audio thread. Implementations
/// MUST be real-time safe:
/// - No memory allocation
/// - No mutex locks
/// - No file I/O
/// - No unbounded loops
/// - No panics
#[sabi_trait]
pub trait AudioEffect: Send + Sync {
    /// Get plugin metadata.
    fn info(&self) -> PluginInfo;

    /// Called when the plugin is activated (before processing starts).
    ///
    /// Use this to allocate buffers, initialize state, etc.
    fn activate(&mut self, context: ActivationContext);

    /// Called when the plugin is deactivated (after processing stops).
    ///
    /// Use this to free resources.
    fn deactivate(&mut self);

    /// Process audio samples.
    ///
    /// # Arguments
    /// * `inputs` - Input audio buffers (one per input channel)
    /// * `outputs` - Output audio buffers (one per output channel)
    ///
    /// # Real-time Safety
    /// This method MUST be real-time safe. See trait documentation.
    fn process(&mut self, inputs: RSlice<RSlice<f32>>, outputs: RSliceMut<RSliceMut<f32>>);

    /// Get the number of parameters.
    fn parameter_count(&self) -> u32;

    /// Get parameter info by index.
    fn parameter_info(&self, index: u32) -> ROption<ParameterInfo>;

    /// Get current parameter value (normalized 0.0-1.0).
    fn get_parameter(&self, index: u32) -> f32;

    /// Set parameter value (normalized 0.0-1.0).
    ///
    /// Must be thread-safe (may be called from UI thread while processing).
    fn set_parameter(&mut self, index: u32, value: f32);

    /// Save plugin state for preset storage.
    fn save_state(&self) -> RVec<u8>;

    /// Load plugin state from preset.
    fn load_state(&mut self, data: RSlice<u8>) -> RResult<(), PluginError>;

    /// Reset plugin to initial state (clear delay lines, etc.).
    fn reset(&mut self);

    /// Get current latency in samples (for delay compensation).
    fn latency(&self) -> u32 {
        0
    }

    /// Get tail length in samples (reverb/delay tail).
    fn tail_length(&self) -> u32 {
        0
    }
}

/// Type alias for boxed plugin instance.
pub type PluginBox = AudioEffect_TO<'static, RBox<()>>;

// ============================================================================
// Plugin Factory (for native plugins)
// ============================================================================

/// Factory function type for creating plugin instances.
///
/// This is not StableAbi itself, but is used within the entry point mechanism.
pub type PluginFactoryFn = extern "C" fn() -> PluginBox;

/// Plugin entry point structure for native plugins.
///
/// Native plugins must export a function `sootmix_plugin_entry` that returns
/// this structure.
///
/// Note: This struct is #[repr(C)] but not StableAbi because function pointers
/// are handled separately. The ABI stability is ensured by:
/// 1. The struct layout being #[repr(C)]
/// 2. The PluginBox type being StableAbi
/// 3. Version checking before using the factory function
#[repr(C)]
pub struct PluginEntry {
    /// API version (major).
    pub api_version_major: u32,
    /// API version (minor).
    pub api_version_minor: u32,
    /// Factory function to create plugin instances.
    pub create: PluginFactoryFn,
}

/// Macro to declare a native plugin entry point.
///
/// # Example
///
/// ```ignore
/// use sootmix_plugin_api::*;
///
/// struct MyPlugin { /* ... */ }
/// impl AudioEffect for MyPlugin { /* ... */ }
///
/// declare_plugin!(MyPlugin);
/// ```
#[macro_export]
macro_rules! declare_plugin {
    ($plugin_type:ty) => {
        #[no_mangle]
        pub extern "C" fn sootmix_plugin_entry() -> $crate::PluginEntry {
            extern "C" fn create() -> $crate::PluginBox {
                let plugin = <$plugin_type>::default();
                $crate::AudioEffect_TO::from_value(plugin, abi_stable::sabi_trait::TD_Opaque)
            }

            $crate::PluginEntry {
                api_version_major: $crate::API_VERSION_MAJOR,
                api_version_minor: $crate::API_VERSION_MINOR,
                create,
            }
        }
    };

    ($plugin_type:ty, $constructor:expr) => {
        #[no_mangle]
        pub extern "C" fn sootmix_plugin_entry() -> $crate::PluginEntry {
            extern "C" fn create() -> $crate::PluginBox {
                let plugin = $constructor;
                $crate::AudioEffect_TO::from_value(plugin, abi_stable::sabi_trait::TD_Opaque)
            }

            $crate::PluginEntry {
                api_version_major: $crate::API_VERSION_MAJOR,
                api_version_minor: $crate::API_VERSION_MINOR,
                create,
            }
        }
    };
}

// ============================================================================
// WASM Plugin Interface
// ============================================================================

/// Module for WASM plugin interface definitions.
///
/// WASM plugins communicate via a simplified interface using flat memory
/// layouts and exported functions.
pub mod wasm {
    /// WASM plugin must export these functions:
    ///
    /// - `plugin_info() -> *const u8` - Returns pointer to JSON plugin info
    /// - `plugin_activate(sample_rate: f32, max_block_size: u32)`
    /// - `plugin_deactivate()`
    /// - `plugin_process(input_ptr: *const f32, output_ptr: *mut f32, frames: u32)`
    /// - `plugin_parameter_count() -> u32`
    /// - `plugin_get_parameter(index: u32) -> f32`
    /// - `plugin_set_parameter(index: u32, value: f32)`
    /// - `plugin_reset()`
    ///
    /// Memory allocation is handled by the WASM runtime.
    pub const REQUIRED_EXPORTS: &[&str] = &[
        "plugin_info",
        "plugin_activate",
        "plugin_deactivate",
        "plugin_process",
        "plugin_parameter_count",
        "plugin_get_parameter",
        "plugin_set_parameter",
        "plugin_reset",
    ];
}

// ============================================================================
// Utility Functions
// ============================================================================

/// Convert decibels to linear gain.
#[inline]
pub fn db_to_linear(db: f32) -> f32 {
    if db <= -80.0 {
        0.0
    } else {
        10.0_f32.powf(db / 20.0)
    }
}

/// Convert linear gain to decibels.
#[inline]
pub fn linear_to_db(linear: f32) -> f32 {
    if linear <= 0.0 {
        -80.0
    } else {
        20.0 * linear.log10()
    }
}

/// Normalize a value from a parameter range to 0.0-1.0.
#[inline]
pub fn normalize(value: f32, min: f32, max: f32, curve: ParameterCurve) -> f32 {
    let clamped = value.clamp(min, max);
    match curve {
        ParameterCurve::Linear => (clamped - min) / (max - min),
        ParameterCurve::Logarithmic => {
            let min_log = min.max(0.001).ln();
            let max_log = max.ln();
            (clamped.max(0.001).ln() - min_log) / (max_log - min_log)
        }
        ParameterCurve::Exponential => ((clamped - min) / (max - min)).sqrt(),
        ParameterCurve::Symmetric => (clamped - min) / (max - min),
    }
}

/// Denormalize a 0.0-1.0 value to a parameter range.
#[inline]
pub fn denormalize(normalized: f32, min: f32, max: f32, curve: ParameterCurve) -> f32 {
    let n = normalized.clamp(0.0, 1.0);
    match curve {
        ParameterCurve::Linear => min + n * (max - min),
        ParameterCurve::Logarithmic => {
            let min_log = min.max(0.001).ln();
            let max_log = max.ln();
            (min_log + n * (max_log - min_log)).exp()
        }
        ParameterCurve::Exponential => min + n * n * (max - min),
        ParameterCurve::Symmetric => min + n * (max - min),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_db_conversions() {
        assert!((db_to_linear(0.0) - 1.0).abs() < 0.0001);
        assert!((db_to_linear(-6.0) - 0.5012).abs() < 0.001);
        assert!((db_to_linear(-80.0)).abs() < 0.0001);

        assert!((linear_to_db(1.0) - 0.0).abs() < 0.0001);
        assert!((linear_to_db(0.5) - (-6.02)).abs() < 0.1);
    }

    #[test]
    fn test_normalize_denormalize() {
        // Linear
        assert!((normalize(50.0, 0.0, 100.0, ParameterCurve::Linear) - 0.5).abs() < 0.0001);
        assert!((denormalize(0.5, 0.0, 100.0, ParameterCurve::Linear) - 50.0).abs() < 0.0001);

        // Roundtrip
        let original = 42.0;
        let normalized = normalize(original, 0.0, 100.0, ParameterCurve::Linear);
        let denormalized = denormalize(normalized, 0.0, 100.0, ParameterCurve::Linear);
        assert!((denormalized - original).abs() < 0.0001);
    }
}
