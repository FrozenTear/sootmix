// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! LADSPA plugin wrapping nnnoiseless for noise suppression.
//!
//! This creates a LADSPA-compatible plugin that can be used with
//! PipeWire's filter-chain module.

use nnnoiseless::DenoiseState;
use std::ffi::CStr;
use std::os::raw::{c_char, c_int, c_ulong};
use std::ptr;

// LADSPA constants
const LADSPA_PROPERTY_REALTIME: c_int = 0x1;
const LADSPA_PROPERTY_HARD_RT_CAPABLE: c_int = 0x4;

const LADSPA_PORT_INPUT: c_int = 0x1;
const LADSPA_PORT_OUTPUT: c_int = 0x2;
const LADSPA_PORT_CONTROL: c_int = 0x4;
const LADSPA_PORT_AUDIO: c_int = 0x8;

const LADSPA_HINT_BOUNDED_BELOW: c_int = 0x1;
const LADSPA_HINT_BOUNDED_ABOVE: c_int = 0x2;
const LADSPA_HINT_DEFAULT_MIDDLE: c_int = 0x100;

// Port indices
const PORT_VAD_THRESHOLD: c_ulong = 0;
const PORT_INPUT: c_ulong = 1;
const PORT_OUTPUT: c_ulong = 2;
const PORT_COUNT: c_ulong = 3;

// Plugin unique ID (arbitrary, just needs to be unique)
const PLUGIN_UNIQUE_ID: c_ulong = 0x534D5252; // "SMRR" in hex

// RNNoise frame size
const FRAME_SIZE: usize = DenoiseState::FRAME_SIZE; // 480 samples

// nnnoiseless expects audio in 16-bit PCM range [-32768, 32767], not [-1.0, 1.0]
const SCALE_IN: f32 = 32767.0;
const SCALE_OUT: f32 = 1.0 / 32767.0;

/// LADSPA port range hint
#[repr(C)]
struct LadspaPortRangeHint {
    hint_descriptor: c_int,
    lower_bound: f32,
    upper_bound: f32,
}

/// LADSPA descriptor
#[repr(C)]
struct LadspaDescriptor {
    unique_id: c_ulong,
    label: *const c_char,
    properties: c_int,
    name: *const c_char,
    maker: *const c_char,
    copyright: *const c_char,
    port_count: c_ulong,
    port_descriptors: *const c_int,
    port_names: *const *const c_char,
    port_range_hints: *const LadspaPortRangeHint,
    implementation_data: *mut (),
    instantiate: Option<extern "C" fn(descriptor: *const LadspaDescriptor, sample_rate: c_ulong) -> *mut PluginInstance>,
    connect_port: Option<extern "C" fn(instance: *mut PluginInstance, port: c_ulong, data: *mut f32)>,
    activate: Option<extern "C" fn(instance: *mut PluginInstance)>,
    run: Option<extern "C" fn(instance: *mut PluginInstance, sample_count: c_ulong)>,
    run_adding: Option<extern "C" fn(instance: *mut PluginInstance, sample_count: c_ulong)>,
    set_run_adding_gain: Option<extern "C" fn(instance: *mut PluginInstance, gain: f32)>,
    deactivate: Option<extern "C" fn(instance: *mut PluginInstance)>,
    cleanup: Option<extern "C" fn(instance: *mut PluginInstance)>,
}

// Safety: LadspaDescriptor contains only static data and function pointers
// that are safe to share between threads
unsafe impl Sync for LadspaDescriptor {}

/// Plugin instance data
struct PluginInstance {
    /// RNNoise denoiser state
    denoiser: Box<DenoiseState<'static>>,
    /// Input buffer for accumulating samples until we have a full frame
    input_buffer: Vec<f32>,
    /// Output buffer for samples waiting to be written
    output_buffer: Vec<f32>,
    /// Position in output buffer
    output_pos: usize,
    /// VAD threshold control port
    vad_threshold: *mut f32,
    /// Audio input port
    input: *mut f32,
    /// Audio output port
    output: *mut f32,
}

// Static strings for LADSPA
static LABEL: &CStr = unsafe { CStr::from_bytes_with_nul_unchecked(b"noise_suppressor_mono\0") };
static NAME: &CStr = unsafe { CStr::from_bytes_with_nul_unchecked(b"SootMix RNNoise Mono\0") };
static MAKER: &CStr = unsafe { CStr::from_bytes_with_nul_unchecked(b"SootMix (nnnoiseless)\0") };
static COPYRIGHT: &CStr = unsafe { CStr::from_bytes_with_nul_unchecked(b"MPL-2.0\0") };

static PORT_NAME_VAD: &CStr = unsafe { CStr::from_bytes_with_nul_unchecked(b"VAD Threshold\0") };
static PORT_NAME_INPUT: &CStr = unsafe { CStr::from_bytes_with_nul_unchecked(b"Input\0") };
static PORT_NAME_OUTPUT: &CStr = unsafe { CStr::from_bytes_with_nul_unchecked(b"Output\0") };

// Port descriptors
static PORT_DESCRIPTORS: [c_int; PORT_COUNT as usize] = [
    LADSPA_PORT_INPUT | LADSPA_PORT_CONTROL,  // VAD threshold
    LADSPA_PORT_INPUT | LADSPA_PORT_AUDIO,    // Audio input
    LADSPA_PORT_OUTPUT | LADSPA_PORT_AUDIO,   // Audio output
];

// Wrapper to make raw pointer arrays Sync
struct SyncPortNames([*const c_char; PORT_COUNT as usize]);
unsafe impl Sync for SyncPortNames {}

// Port names
static PORT_NAMES: SyncPortNames = SyncPortNames([
    PORT_NAME_VAD.as_ptr(),
    PORT_NAME_INPUT.as_ptr(),
    PORT_NAME_OUTPUT.as_ptr(),
]);

// Port range hints
static PORT_RANGE_HINTS: [LadspaPortRangeHint; PORT_COUNT as usize] = [
    LadspaPortRangeHint {
        hint_descriptor: LADSPA_HINT_BOUNDED_BELOW | LADSPA_HINT_BOUNDED_ABOVE | LADSPA_HINT_DEFAULT_MIDDLE,
        lower_bound: 0.0,
        upper_bound: 100.0,
    },
    LadspaPortRangeHint {
        hint_descriptor: 0,
        lower_bound: 0.0,
        upper_bound: 0.0,
    },
    LadspaPortRangeHint {
        hint_descriptor: 0,
        lower_bound: 0.0,
        upper_bound: 0.0,
    },
];

// LADSPA callbacks
extern "C" fn instantiate(_descriptor: *const LadspaDescriptor, _sample_rate: c_ulong) -> *mut PluginInstance {
    let instance = Box::new(PluginInstance {
        denoiser: DenoiseState::new(),
        input_buffer: Vec::with_capacity(FRAME_SIZE),
        output_buffer: Vec::with_capacity(FRAME_SIZE),
        output_pos: 0,
        vad_threshold: ptr::null_mut(),
        input: ptr::null_mut(),
        output: ptr::null_mut(),
    });
    Box::into_raw(instance)
}

extern "C" fn connect_port(instance: *mut PluginInstance, port: c_ulong, data: *mut f32) {
    if instance.is_null() {
        return;
    }
    let instance = unsafe { &mut *instance };
    match port {
        PORT_VAD_THRESHOLD => instance.vad_threshold = data,
        PORT_INPUT => instance.input = data,
        PORT_OUTPUT => instance.output = data,
        _ => {}
    }
}

extern "C" fn activate(instance: *mut PluginInstance) {
    if instance.is_null() {
        return;
    }
    let instance = unsafe { &mut *instance };
    instance.input_buffer.clear();
    instance.output_buffer.clear();
    instance.output_pos = 0;
}

extern "C" fn run(instance: *mut PluginInstance, sample_count: c_ulong) {
    if instance.is_null() {
        return;
    }
    let instance = unsafe { &mut *instance };

    if instance.input.is_null() || instance.output.is_null() {
        return;
    }

    let sample_count = sample_count as usize;
    let input = unsafe { std::slice::from_raw_parts(instance.input, sample_count) };
    let output = unsafe { std::slice::from_raw_parts_mut(instance.output, sample_count) };

    // Get VAD threshold (0-100%) and convert to probability (0.0-1.0)
    let vad_threshold = if !instance.vad_threshold.is_null() {
        (unsafe { *instance.vad_threshold } / 100.0).clamp(0.0, 1.0)
    } else {
        0.5 // Default 50%
    };

    let mut out_idx = 0;

    // First, output any buffered samples from previous processing
    while out_idx < sample_count && instance.output_pos < instance.output_buffer.len() {
        output[out_idx] = instance.output_buffer[instance.output_pos];
        instance.output_pos += 1;
        out_idx += 1;
    }

    // Clear output buffer if fully consumed
    if instance.output_pos >= instance.output_buffer.len() {
        instance.output_buffer.clear();
        instance.output_pos = 0;
    }

    // Process input samples (scale from [-1.0, 1.0] to [-32768, 32767] for nnnoiseless)
    for &sample in input {
        instance.input_buffer.push(sample * SCALE_IN);

        // When we have a full frame, process it
        if instance.input_buffer.len() >= FRAME_SIZE {
            let mut frame_out = [0.0f32; FRAME_SIZE];
            // process_frame returns VAD probability (0.0 = noise, 1.0 = voice)
            let vad_prob = instance.denoiser.process_frame(&mut frame_out, &instance.input_buffer[..FRAME_SIZE]);

            // Apply VAD gating: if voice probability is below threshold, output silence
            let output_frame = vad_prob >= vad_threshold;

            // Output processed samples (scale back to [-1.0, 1.0] for LADSPA)
            for &s in &frame_out {
                let scaled = if output_frame { s * SCALE_OUT } else { 0.0 };
                if out_idx < sample_count {
                    output[out_idx] = scaled;
                    out_idx += 1;
                } else {
                    // Buffer overflow samples for next run
                    instance.output_buffer.push(scaled);
                }
            }

            instance.input_buffer.drain(..FRAME_SIZE);
        }
    }

    // Zero-fill any remaining output (shouldn't happen in steady state)
    while out_idx < sample_count {
        output[out_idx] = 0.0;
        out_idx += 1;
    }
}

extern "C" fn cleanup(instance: *mut PluginInstance) {
    if !instance.is_null() {
        unsafe {
            drop(Box::from_raw(instance));
        }
    }
}

// Static descriptor
static DESCRIPTOR: LadspaDescriptor = LadspaDescriptor {
    unique_id: PLUGIN_UNIQUE_ID,
    label: LABEL.as_ptr(),
    properties: LADSPA_PROPERTY_REALTIME | LADSPA_PROPERTY_HARD_RT_CAPABLE,
    name: NAME.as_ptr(),
    maker: MAKER.as_ptr(),
    copyright: COPYRIGHT.as_ptr(),
    port_count: PORT_COUNT,
    port_descriptors: PORT_DESCRIPTORS.as_ptr(),
    port_names: PORT_NAMES.0.as_ptr(),
    port_range_hints: PORT_RANGE_HINTS.as_ptr(),
    implementation_data: ptr::null_mut(),
    instantiate: Some(instantiate),
    connect_port: Some(connect_port),
    activate: Some(activate),
    run: Some(run),
    run_adding: None,
    set_run_adding_gain: None,
    deactivate: None,
    cleanup: Some(cleanup),
};

/// LADSPA entry point - returns descriptor for the given index
#[no_mangle]
#[allow(private_interfaces)]
pub extern "C" fn ladspa_descriptor(index: c_ulong) -> *const LadspaDescriptor {
    match index {
        0 => &DESCRIPTOR,
        _ => ptr::null(),
    }
}
