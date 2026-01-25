// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! VST3 plugin adapter implementing the AudioEffect trait.

use super::{Vst3Module, Vst3PluginMeta};
use crate::plugins::PluginLoadError;
use abi_stable::std_types::{ROption, RResult, RSlice, RSliceMut, RString, RVec};
use sootmix_plugin_api::{
    ActivationContext, AudioEffect, ParameterCurve, ParameterInfo, PluginError, PluginInfo,
};
use std::sync::Arc;
use tracing::{debug, warn};
use vst3::ComPtr;
use vst3::Steinberg::Vst::{
    AudioBusBuffers, IAudioProcessor, IAudioProcessorTrait, IComponent, IComponentTrait,
    IEditController, IEditControllerTrait, ParameterInfo as Vst3ParameterInfo, ProcessData,
    ProcessSetup, SymbolicSampleSizes_,
};
use vst3::Steinberg::{kResultOk, IPluginBaseTrait};

/// Adapter that wraps a VST3 plugin to implement AudioEffect.
pub struct Vst3PluginAdapter {
    /// Reference to the VST3 module (must outlive components).
    _module: Arc<Vst3Module>,
    /// Plugin metadata.
    meta: Vst3PluginMeta,
    /// The VST3 component.
    component: ComPtr<IComponent>,
    /// The audio processor interface.
    processor: Option<ComPtr<IAudioProcessor>>,
    /// The edit controller (for parameters).
    controller: Option<ComPtr<IEditController>>,
    /// Whether the plugin is activated.
    activated: bool,
    /// Current sample rate.
    sample_rate: f32,
    /// Maximum block size.
    max_block_size: u32,
    /// Cached parameter count.
    parameter_count: u32,
    /// Parameter IDs (VST3 uses arbitrary IDs, not sequential indices).
    parameter_ids: Vec<u32>,
    /// Audio input buffers.
    audio_in_buffers: Vec<Vec<f32>>,
    /// Audio output buffers.
    audio_out_buffers: Vec<Vec<f32>>,
}

// SAFETY: VST3 components are designed to be thread-safe when properly synchronized.
unsafe impl Send for Vst3PluginAdapter {}
unsafe impl Sync for Vst3PluginAdapter {}

impl Vst3PluginAdapter {
    /// Create a new VST3 plugin adapter.
    pub fn new(module: Arc<Vst3Module>, meta: &Vst3PluginMeta) -> Result<Self, PluginLoadError> {
        // Create the component
        let component = module.create_component(&meta.tuid)?;

        // Initialize the component
        let result = unsafe { component.initialize(std::ptr::null_mut()) };
        if result != kResultOk {
            return Err(PluginLoadError::Vst3Error(
                "Failed to initialize component".to_string(),
            ));
        }

        // Get the audio processor interface
        let processor: Option<ComPtr<IAudioProcessor>> = component.cast();

        // Try to get the edit controller
        // First check if the component implements it directly
        let controller: Option<ComPtr<IEditController>> = component.cast();

        // If not, we'd need to create it separately via controller class ID
        // For simplicity, assume single-component architecture for now

        // Get parameter count and IDs
        let (parameter_count, parameter_ids) = if let Some(ref ctrl) = controller {
            let count = unsafe { ctrl.getParameterCount() };
            let mut ids = Vec::with_capacity(count as usize);

            for i in 0..count {
                let mut info: Vst3ParameterInfo = unsafe { std::mem::zeroed() };
                if unsafe { ctrl.getParameterInfo(i, &mut info) } == kResultOk {
                    ids.push(info.id);
                }
            }

            (count as u32, ids)
        } else {
            (0, Vec::new())
        };

        Ok(Self {
            _module: module,
            meta: meta.clone(),
            component,
            processor,
            controller,
            activated: false,
            sample_rate: 48000.0,
            max_block_size: 512,
            parameter_count,
            parameter_ids,
            audio_in_buffers: Vec::new(),
            audio_out_buffers: Vec::new(),
        })
    }
}

impl AudioEffect for Vst3PluginAdapter {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            id: RString::from(self.meta.class_id.as_str()),
            name: RString::from(self.meta.name.as_str()),
            vendor: RString::from(self.meta.vendor.as_str()),
            version: RString::from(self.meta.version.as_str()),
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
        self.max_block_size = context.max_block_size;

        // Setup the processor
        if let Some(ref processor) = self.processor {
            let mut setup = ProcessSetup {
                processMode: 0, // Realtime
                symbolicSampleSize: SymbolicSampleSizes_::kSample32 as i32,
                maxSamplesPerBlock: context.max_block_size as i32,
                sampleRate: context.sample_rate as f64,
            };

            let result = unsafe { processor.setupProcessing(&mut setup) };
            if result != kResultOk {
                warn!("VST3 setupProcessing failed for {}", self.meta.name);
            }

            // Activate the processor
            let result = unsafe { processor.setProcessing(1) };
            if result != kResultOk {
                warn!("VST3 setProcessing failed for {}", self.meta.name);
            }
        }

        // Activate the component
        let result = unsafe { self.component.setActive(1) };
        if result != kResultOk {
            warn!("VST3 setActive failed for {}", self.meta.name);
        }

        // Initialize audio buffers
        let block_size = context.max_block_size as usize;
        self.audio_in_buffers = vec![vec![0.0f32; block_size]; self.meta.audio_inputs as usize];
        self.audio_out_buffers = vec![vec![0.0f32; block_size]; self.meta.audio_outputs as usize];

        self.activated = true;
        debug!(
            "VST3 plugin activated: {} (sr={}, block={})",
            self.meta.name, self.sample_rate, context.max_block_size
        );
    }

    fn deactivate(&mut self) {
        if !self.activated {
            return;
        }

        // Deactivate processor
        if let Some(ref processor) = self.processor {
            unsafe {
                processor.setProcessing(0);
            }
        }

        // Deactivate component
        unsafe {
            self.component.setActive(0);
        }

        self.activated = false;
        self.audio_in_buffers.clear();
        self.audio_out_buffers.clear();

        debug!("VST3 plugin deactivated: {}", self.meta.name);
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

        let processor = match &self.processor {
            Some(p) => p,
            None => {
                // No processor, pass-through
                for i in 0..inputs.len().min(outputs.len()) {
                    let input = &inputs[i];
                    let output = &mut outputs[i];
                    let len = input.len().min(output.len());
                    output[..len].copy_from_slice(&input[..len]);
                }
                return;
            }
        };

        let frames = inputs.first().map(|i| i.len()).unwrap_or(0);
        if frames == 0 {
            return;
        }

        // Copy input data to internal buffers
        for (i, input) in inputs.iter().enumerate() {
            if i < self.audio_in_buffers.len() {
                let buf = &mut self.audio_in_buffers[i];
                if buf.len() < frames {
                    buf.resize(frames, 0.0);
                }
                buf[..frames].copy_from_slice(&input[..frames]);
            }
        }

        // Ensure output buffers are sized
        for buf in &mut self.audio_out_buffers {
            if buf.len() < frames {
                buf.resize(frames, 0.0);
            }
        }

        // Build input/output pointers
        let mut in_ptrs: Vec<*mut f32> = self
            .audio_in_buffers
            .iter_mut()
            .map(|b| b.as_mut_ptr())
            .collect();

        let mut out_ptrs: Vec<*mut f32> = self
            .audio_out_buffers
            .iter_mut()
            .map(|b| b.as_mut_ptr())
            .collect();

        // Setup audio buses using zeroed structs and direct field access
        let mut input_bus: AudioBusBuffers = unsafe { std::mem::zeroed() };
        input_bus.numChannels = self.meta.audio_inputs as i32;
        input_bus.silenceFlags = 0;
        unsafe {
            input_bus.__field0.channelBuffers32 = in_ptrs.as_mut_ptr();
        }

        let mut output_bus: AudioBusBuffers = unsafe { std::mem::zeroed() };
        output_bus.numChannels = self.meta.audio_outputs as i32;
        output_bus.silenceFlags = 0;
        unsafe {
            output_bus.__field0.channelBuffers32 = out_ptrs.as_mut_ptr();
        }

        // Setup process data
        let mut process_data = ProcessData {
            processMode: 0, // Realtime
            symbolicSampleSize: SymbolicSampleSizes_::kSample32 as i32,
            numSamples: frames as i32,
            numInputs: 1,
            numOutputs: 1,
            inputs: &mut input_bus,
            outputs: &mut output_bus,
            inputParameterChanges: std::ptr::null_mut(),
            outputParameterChanges: std::ptr::null_mut(),
            inputEvents: std::ptr::null_mut(),
            outputEvents: std::ptr::null_mut(),
            processContext: std::ptr::null_mut(),
        };

        // Process
        let result = unsafe { processor.process(&mut process_data) };
        if result != kResultOk {
            warn!("VST3 process failed for {}", self.meta.name);
        }

        // Copy output data to output slices
        for i in 0..outputs.len() {
            if i < self.audio_out_buffers.len() {
                let output = &mut outputs[i];
                let len = output.len().min(frames);
                output[..len].copy_from_slice(&self.audio_out_buffers[i][..len]);
            }
        }
    }

    fn parameter_count(&self) -> u32 {
        self.parameter_count
    }

    fn parameter_info(&self, index: u32) -> ROption<ParameterInfo> {
        let controller = match self.controller.as_ref() {
            Some(c) => c,
            None => return ROption::RNone,
        };

        let mut info: Vst3ParameterInfo = unsafe { std::mem::zeroed() };
        let result = unsafe { controller.getParameterInfo(index as i32, &mut info) };

        if result != kResultOk {
            return ROption::RNone;
        }

        // Convert UTF-16 name to string
        let name = utf16_to_string(&info.title);
        let id = utf16_to_string(&info.shortTitle);
        let unit = utf16_to_string(&info.units);

        ROption::RSome(ParameterInfo {
            index,
            id: RString::from(if id.is_empty() { name.as_str() } else { id.as_str() }),
            name: RString::from(name.as_str()),
            unit: RString::from(unit.as_str()),
            min: 0.0, // VST3 uses normalized 0-1
            max: 1.0,
            default: info.defaultNormalizedValue as f32,
            curve: ParameterCurve::Linear, // VST3 handles curves internally
            step: if info.stepCount > 0 {
                1.0 / info.stepCount as f32
            } else {
                0.0
            },
            hint: sootmix_plugin_api::ParameterHint::None,
        })
    }

    fn get_parameter(&self, index: u32) -> f32 {
        if let (Some(controller), Some(&param_id)) =
            (&self.controller, self.parameter_ids.get(index as usize))
        {
            unsafe { controller.getParamNormalized(param_id) as f32 }
        } else {
            0.0
        }
    }

    fn set_parameter(&mut self, index: u32, value: f32) {
        if let (Some(controller), Some(&param_id)) =
            (&self.controller, self.parameter_ids.get(index as usize))
        {
            unsafe {
                controller.setParamNormalized(param_id, value as f64);
            }
        }
    }

    fn save_state(&self) -> RVec<u8> {
        // Save all parameter values
        let state: Vec<(u32, f32)> = self
            .parameter_ids
            .iter()
            .map(|&id| {
                let value = if let Some(ref ctrl) = self.controller {
                    unsafe { ctrl.getParamNormalized(id) as f32 }
                } else {
                    0.0
                };
                (id, value)
            })
            .collect();

        match serde_json::to_vec(&state) {
            Ok(data) => RVec::from(data),
            Err(_) => RVec::new(),
        }
    }

    fn load_state(&mut self, data: RSlice<u8>) -> RResult<(), PluginError> {
        let state: Vec<(u32, f32)> = match serde_json::from_slice(data.as_slice()) {
            Ok(s) => s,
            Err(e) => {
                return RResult::RErr(PluginError::StateLoadFailed(RString::from(format!(
                    "JSON parse error: {}",
                    e
                ))));
            }
        };

        if let Some(ref controller) = self.controller {
            for (param_id, value) in state {
                unsafe {
                    controller.setParamNormalized(param_id, value as f64);
                }
            }
        }

        RResult::ROk(())
    }

    fn reset(&mut self) {
        // Reset parameters to defaults
        if let Some(ref controller) = self.controller {
            for i in 0..self.parameter_count {
                let mut info: Vst3ParameterInfo = unsafe { std::mem::zeroed() };
                if unsafe { controller.getParameterInfo(i as i32, &mut info) } == kResultOk {
                    unsafe {
                        controller.setParamNormalized(info.id, info.defaultNormalizedValue);
                    }
                }
            }
        }
    }

    fn latency(&self) -> u32 {
        if let Some(ref processor) = self.processor {
            unsafe { processor.getLatencySamples() as u32 }
        } else {
            0
        }
    }

    fn tail_length(&self) -> u32 {
        if let Some(ref processor) = self.processor {
            unsafe { processor.getTailSamples() as u32 }
        } else {
            0
        }
    }
}

impl Drop for Vst3PluginAdapter {
    fn drop(&mut self) {
        self.deactivate();

        // Terminate the component
        unsafe {
            self.component.terminate();
        }
    }
}

/// Convert UTF-16 null-terminated string to Rust String.
fn utf16_to_string(utf16: &[u16]) -> String {
    let end = utf16.iter().position(|&c| c == 0).unwrap_or(utf16.len());
    String::from_utf16_lossy(&utf16[..end])
}
