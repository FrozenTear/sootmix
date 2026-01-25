// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! LV2 plugin adapter implementing the AudioEffect trait.

use super::{Lv2PluginMeta, Lv2World};
use crate::plugins::PluginLoadError;
use abi_stable::std_types::{ROption, RResult, RSlice, RSliceMut, RString, RVec};
use lilv::instance::ActiveInstance;
use lilv::plugin::Plugin;
use sootmix_plugin_api::{
    ActivationContext, AudioEffect, ParameterCurve, ParameterInfo, PluginError, PluginInfo,
};
use std::sync::Arc;
use tracing::{debug, warn};

/// Adapter that wraps an LV2 plugin instance to implement AudioEffect.
pub struct Lv2PluginAdapter {
    /// Reference to the LV2 world (must outlive the instance).
    _world: Arc<Lv2World>,
    /// Plugin metadata.
    meta: Lv2PluginMeta,
    /// The LV2 plugin reference (needed for instantiation).
    plugin: Plugin,
    /// The active LV2 plugin instance.
    active_instance: Option<ActiveInstance>,
    /// Current sample rate.
    sample_rate: f32,
    /// Whether the plugin is activated.
    activated: bool,
    /// Control port values (indexed by control port number).
    control_values: Vec<f32>,
    /// Mapping from control index to LV2 port index.
    control_port_indices: Vec<usize>,
    /// Audio input buffer storage.
    audio_in_buffers: Vec<Vec<f32>>,
    /// Audio output buffer storage.
    audio_out_buffers: Vec<Vec<f32>>,
    /// Port indices for audio inputs.
    audio_in_port_indices: Vec<usize>,
    /// Port indices for audio outputs.
    audio_out_port_indices: Vec<usize>,
}

// SAFETY: LV2 instances can be sent between threads as long as they're not
// accessed concurrently. We ensure this through proper activation/deactivation.
unsafe impl Send for Lv2PluginAdapter {}
unsafe impl Sync for Lv2PluginAdapter {}

impl Lv2PluginAdapter {
    /// Create a new LV2 plugin adapter.
    pub fn new(world: &Arc<Lv2World>, meta: &Lv2PluginMeta) -> Result<Self, PluginLoadError> {
        let inner = world.inner();

        // Find the plugin by URI
        let uri = inner.new_uri(&meta.uri);
        let plugins = inner.plugins();

        let plugin = plugins.plugin(&uri).ok_or_else(|| {
            PluginLoadError::Lv2Error(format!("Plugin not found: {}", meta.uri))
        })?;

        let control_values: Vec<f32> = meta.control_ports.iter().map(|p| p.default).collect();

        let control_port_indices: Vec<usize> = meta.control_ports.iter().map(|p| p.index).collect();

        // Collect audio port indices
        let audio_port_uri = inner.new_uri("http://lv2plug.in/ns/lv2core#AudioPort");
        let input_port_uri = inner.new_uri("http://lv2plug.in/ns/lv2core#InputPort");
        let output_port_uri = inner.new_uri("http://lv2plug.in/ns/lv2core#OutputPort");

        let mut audio_in_port_indices = Vec::new();
        let mut audio_out_port_indices = Vec::new();

        for port in plugin.iter_ports() {
            if port.is_a(&audio_port_uri) {
                if port.is_a(&input_port_uri) {
                    audio_in_port_indices.push(port.index());
                } else if port.is_a(&output_port_uri) {
                    audio_out_port_indices.push(port.index());
                }
            }
        }

        Ok(Self {
            _world: Arc::clone(world),
            meta: meta.clone(),
            plugin,
            active_instance: None,
            sample_rate: 48000.0,
            activated: false,
            control_values,
            control_port_indices,
            audio_in_buffers: Vec::new(),
            audio_out_buffers: Vec::new(),
            audio_in_port_indices,
            audio_out_port_indices,
        })
    }

    /// Normalize a value from LV2 range to 0-1.
    fn normalize_value(&self, port_idx: usize, value: f32) -> f32 {
        if port_idx >= self.meta.control_ports.len() {
            return 0.0;
        }

        let port = &self.meta.control_ports[port_idx];
        let range = port.max - port.min;

        if range <= 0.0 {
            return 0.0;
        }

        if port.logarithmic && port.min > 0.0 {
            // Logarithmic scaling
            let min_log = port.min.ln();
            let max_log = port.max.ln();
            let val_log = value.clamp(port.min, port.max).ln();
            (val_log - min_log) / (max_log - min_log)
        } else {
            // Linear scaling
            (value - port.min) / range
        }
    }

    /// Denormalize a 0-1 value to LV2 range.
    fn denormalize_value(&self, port_idx: usize, normalized: f32) -> f32 {
        if port_idx >= self.meta.control_ports.len() {
            return 0.0;
        }

        let port = &self.meta.control_ports[port_idx];
        let n = normalized.clamp(0.0, 1.0);

        if port.logarithmic && port.min > 0.0 {
            // Logarithmic scaling
            let min_log = port.min.ln();
            let max_log = port.max.ln();
            (min_log + n * (max_log - min_log)).exp()
        } else {
            // Linear scaling
            port.min + n * (port.max - port.min)
        }
    }
}

impl AudioEffect for Lv2PluginAdapter {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            id: RString::from(self.meta.uri.as_str()),
            name: RString::from(self.meta.name.as_str()),
            vendor: RString::from(self.meta.author.as_deref().unwrap_or("Unknown")),
            version: RString::from("1.0.0"),
            category: self.meta.category,
            input_channels: self.meta.audio_inputs,
            output_channels: self.meta.audio_outputs,
        }
    }

    fn activate(&mut self, context: ActivationContext) {
        if self.activated {
            self.deactivate();
        }

        self.sample_rate = context.sample_rate;
        let block_size = context.max_block_size as usize;

        // Initialize audio buffers
        self.audio_in_buffers = vec![vec![0.0f32; block_size]; self.meta.audio_inputs as usize];
        self.audio_out_buffers = vec![vec![0.0f32; block_size]; self.meta.audio_outputs as usize];

        // Instantiate the plugin
        let instance = unsafe { self.plugin.instantiate(self.sample_rate as f64, []) };

        let mut instance = match instance {
            Some(i) => i,
            None => {
                warn!("Failed to instantiate LV2 plugin: {}", self.meta.uri);
                return;
            }
        };

        // Connect audio input ports
        for (i, &port_idx) in self.audio_in_port_indices.iter().enumerate() {
            if i < self.audio_in_buffers.len() {
                unsafe {
                    instance.connect_port_mut(port_idx, self.audio_in_buffers[i].as_mut_ptr());
                }
            }
        }

        // Connect audio output ports
        for (i, &port_idx) in self.audio_out_port_indices.iter().enumerate() {
            if i < self.audio_out_buffers.len() {
                unsafe {
                    instance.connect_port_mut(port_idx, self.audio_out_buffers[i].as_mut_ptr());
                }
            }
        }

        // Connect control ports
        for (ctrl_idx, &lv2_port_idx) in self.control_port_indices.iter().enumerate() {
            if ctrl_idx < self.control_values.len() {
                unsafe {
                    instance.connect_port_mut(
                        lv2_port_idx,
                        &mut self.control_values[ctrl_idx] as *mut f32,
                    );
                }
            }
        }

        // Activate the instance
        let active = unsafe { instance.activate() };
        self.active_instance = Some(active);
        self.activated = true;

        debug!(
            "LV2 plugin activated: {} (sr={}, block={})",
            self.meta.name, self.sample_rate, block_size
        );
    }

    fn deactivate(&mut self) {
        if !self.activated {
            return;
        }

        // Deactivate and drop the instance
        if let Some(active) = self.active_instance.take() {
            unsafe {
                let _ = active.deactivate();
            }
        }

        self.activated = false;
        self.audio_in_buffers.clear();
        self.audio_out_buffers.clear();

        debug!("LV2 plugin deactivated: {}", self.meta.name);
    }

    fn process(&mut self, inputs: RSlice<RSlice<f32>>, mut outputs: RSliceMut<RSliceMut<f32>>) {
        if !self.activated {
            // Pass-through
            for i in 0..inputs.len().min(outputs.len()) {
                let input = &inputs[i];
                let output = &mut outputs[i];
                let len = input.len().min(output.len());
                output[..len].copy_from_slice(&input[..len]);
            }
            return;
        }

        let active = match &mut self.active_instance {
            Some(a) => a,
            None => return,
        };

        let frames = inputs.first().map(|i| i.len()).unwrap_or(0);
        if frames == 0 {
            return;
        }

        // Copy input data to internal buffers
        for (i, input) in inputs.iter().enumerate() {
            if i < self.audio_in_buffers.len() {
                let len = input.len().min(self.audio_in_buffers[i].len());
                self.audio_in_buffers[i][..len].copy_from_slice(&input[..len]);
            }
        }

        // Re-connect ports if buffer pointers might have changed
        // (This is typically only needed if buffers were resized)
        let instance = active.instance_mut();
        for (i, &port_idx) in self.audio_in_port_indices.iter().enumerate() {
            if i < self.audio_in_buffers.len() {
                unsafe {
                    instance.connect_port_mut(port_idx, self.audio_in_buffers[i].as_mut_ptr());
                }
            }
        }
        for (i, &port_idx) in self.audio_out_port_indices.iter().enumerate() {
            if i < self.audio_out_buffers.len() {
                unsafe {
                    instance.connect_port_mut(port_idx, self.audio_out_buffers[i].as_mut_ptr());
                }
            }
        }

        // Run the plugin
        unsafe {
            active.run(frames);
        }

        // Copy output data from internal buffers
        for i in 0..outputs.len() {
            if i < self.audio_out_buffers.len() {
                let output = &mut outputs[i];
                let len = output.len().min(self.audio_out_buffers[i].len());
                output[..len].copy_from_slice(&self.audio_out_buffers[i][..len]);
            }
        }
    }

    fn parameter_count(&self) -> u32 {
        self.meta.control_ports.len() as u32
    }

    fn parameter_info(&self, index: u32) -> ROption<ParameterInfo> {
        let port = match self.meta.control_ports.get(index as usize) {
            Some(p) => p,
            None => return ROption::RNone,
        };

        let curve = if port.logarithmic {
            ParameterCurve::Logarithmic
        } else {
            ParameterCurve::Linear
        };

        ROption::RSome(ParameterInfo {
            index,
            id: RString::from(port.symbol.as_str()),
            name: RString::from(port.name.as_str()),
            unit: RString::new(),
            min: 0.0,  // Normalized
            max: 1.0,  // Normalized
            default: self.normalize_value(index as usize, port.default),
            curve,
            step: 0.0,
        })
    }

    fn get_parameter(&self, index: u32) -> f32 {
        let idx = index as usize;
        if idx < self.control_values.len() {
            self.normalize_value(idx, self.control_values[idx])
        } else {
            0.0
        }
    }

    fn set_parameter(&mut self, index: u32, value: f32) {
        let idx = index as usize;
        if idx < self.control_values.len() {
            self.control_values[idx] = self.denormalize_value(idx, value);
        }
    }

    fn save_state(&self) -> RVec<u8> {
        // Serialize parameter values as JSON
        let state: Vec<(String, f32)> = self
            .meta
            .control_ports
            .iter()
            .enumerate()
            .map(|(i, port)| {
                (
                    port.symbol.clone(),
                    self.control_values.get(i).copied().unwrap_or(port.default),
                )
            })
            .collect();

        match serde_json::to_vec(&state) {
            Ok(data) => RVec::from(data),
            Err(_) => RVec::new(),
        }
    }

    fn load_state(&mut self, data: RSlice<u8>) -> RResult<(), PluginError> {
        let state: Vec<(String, f32)> = match serde_json::from_slice(data.as_slice()) {
            Ok(s) => s,
            Err(e) => {
                return RResult::RErr(PluginError::StateLoadFailed(RString::from(format!(
                    "JSON parse error: {}",
                    e
                ))));
            }
        };

        // Apply state to control values
        for (symbol, value) in state {
            for (i, port) in self.meta.control_ports.iter().enumerate() {
                if port.symbol == symbol && i < self.control_values.len() {
                    self.control_values[i] = value.clamp(port.min, port.max);
                    break;
                }
            }
        }

        RResult::ROk(())
    }

    fn reset(&mut self) {
        // Reset all parameters to defaults
        for (i, port) in self.meta.control_ports.iter().enumerate() {
            if i < self.control_values.len() {
                self.control_values[i] = port.default;
            }
        }

        // Deactivate and reactivate to clear internal state
        if self.activated {
            if let Some(active) = self.active_instance.take() {
                // Deactivate
                let instance = unsafe { active.deactivate() };

                // Reactivate if we got the instance back
                if let Some(instance) = instance {
                    let active = unsafe { instance.activate() };
                    self.active_instance = Some(active);
                }
            }
        }
    }

    fn latency(&self) -> u32 {
        // LV2 plugins may report latency via a control port with lv2:reportsLatency
        // For now, return 0
        0
    }

    fn tail_length(&self) -> u32 {
        0
    }
}

impl Drop for Lv2PluginAdapter {
    fn drop(&mut self) {
        self.deactivate();
    }
}
