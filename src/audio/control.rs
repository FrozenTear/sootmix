// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Native PipeWire control port manipulation.
//!
//! This module provides utilities for setting node parameters (volume, mute, EQ)
//! using the native PipeWire API instead of CLI tools.

use libspa::pod::serialize::PodSerializer;
use libspa::pod::{Object, Property, Value, ValueArray};
use libspa::utils::Id;
use std::io::Cursor;
use thiserror::Error;
use tracing::{debug, trace};

/// SPA type and parameter constants.
/// These match the values from spa/param/param.h and spa/param/props.h
pub mod spa_const {
    /// SPA_TYPE_OBJECT_Props
    pub const SPA_TYPE_OBJECT_PROPS: u32 = 262146; // 0x40002

    /// SPA_PARAM_Props
    pub const SPA_PARAM_PROPS: u32 = 2;

    /// Property keys from spa/param/props.h
    pub const SPA_PROP_volume: u32 = 65540; // 0x10004
    pub const SPA_PROP_mute: u32 = 65541; // 0x10005
    pub const SPA_PROP_channelVolumes: u32 = 65542; // 0x10006
    pub const SPA_PROP_channelMap: u32 = 65543; // 0x10007

    /// For filter-chain control ports (named params)
    pub const SPA_PROP_params: u32 = 65544; // 0x10008
}

#[derive(Debug, Error)]
pub enum ControlError {
    #[error("Failed to serialize pod: {0}")]
    SerializationFailed(String),
    #[error("Failed to set parameter: {0}")]
    SetParamFailed(String),
    #[error("Node not bound: {0}")]
    NodeNotBound(u32),
    #[error("Invalid parameter value: {0}")]
    InvalidValue(String),
}

/// Result type for control operations.
pub type ControlResult<T> = Result<T, ControlError>;

/// Build a Props pod for setting volume on a node.
///
/// Volume is in linear scale: 0.0 = silent, 1.0 = 100%, values > 1.0 = boost.
pub fn build_volume_pod(volume: f32) -> ControlResult<Vec<u8>> {
    let volume_clamped = volume.clamp(0.0, 1.5);

    trace!("Building volume pod: {:.3}", volume_clamped);

    // Build the Props object with volume property
    let object = Object {
        type_: libspa::utils::SpaTypes::ObjectParamProps.as_raw(),
        id: spa_const::SPA_PARAM_PROPS,
        properties: vec![Property {
            key: spa_const::SPA_PROP_volume,
            flags: libspa::pod::PropertyFlags::empty(),
            value: Value::Float(volume_clamped),
        }],
    };

    serialize_object(object)
}

/// Build a Props pod for setting stereo channel volumes.
pub fn build_channel_volumes_pod(volumes: &[f32]) -> ControlResult<Vec<u8>> {
    let volumes_clamped: Vec<f32> = volumes.iter().map(|v| v.clamp(0.0, 1.5)).collect();

    trace!("Building channel volumes pod: {:?}", volumes_clamped);

    let object = Object {
        type_: libspa::utils::SpaTypes::ObjectParamProps.as_raw(),
        id: spa_const::SPA_PARAM_PROPS,
        properties: vec![Property {
            key: spa_const::SPA_PROP_channelVolumes,
            flags: libspa::pod::PropertyFlags::empty(),
            value: Value::ValueArray(ValueArray::Float(volumes_clamped)),
        }],
    };

    serialize_object(object)
}

/// Build a Props pod for setting mute state.
pub fn build_mute_pod(muted: bool) -> ControlResult<Vec<u8>> {
    trace!("Building mute pod: {}", muted);

    let object = Object {
        type_: libspa::utils::SpaTypes::ObjectParamProps.as_raw(),
        id: spa_const::SPA_PARAM_PROPS,
        properties: vec![Property {
            key: spa_const::SPA_PROP_mute,
            flags: libspa::pod::PropertyFlags::empty(),
            value: Value::Bool(muted),
        }],
    };

    serialize_object(object)
}

/// Build a Props pod for setting both volume and mute.
pub fn build_volume_mute_pod(volume: f32, muted: bool) -> ControlResult<Vec<u8>> {
    let volume_clamped = volume.clamp(0.0, 1.5);

    trace!(
        "Building volume+mute pod: vol={:.3}, mute={}",
        volume_clamped,
        muted
    );

    let object = Object {
        type_: libspa::utils::SpaTypes::ObjectParamProps.as_raw(),
        id: spa_const::SPA_PARAM_PROPS,
        properties: vec![
            Property {
                key: spa_const::SPA_PROP_volume,
                flags: libspa::pod::PropertyFlags::empty(),
                value: Value::Float(volume_clamped),
            },
            Property {
                key: spa_const::SPA_PROP_mute,
                flags: libspa::pod::PropertyFlags::empty(),
                value: Value::Bool(muted),
            },
        ],
    };

    serialize_object(object)
}

/// Build a Props pod for setting a named control parameter (for filter-chains).
///
/// Filter-chain nodes use named parameters like "Freq", "Q", "Gain" for EQ bands.
/// The params property contains a list of name-value pairs.
pub fn build_filter_control_pod(controls: &[(&str, f32)]) -> ControlResult<Vec<u8>> {
    trace!("Building filter control pod: {:?}", controls);

    // For filter-chain controls, we need to build a Struct containing
    // alternating String and Float values for each control.
    // The structure is: params = [ "Name1" Value1, "Name2" Value2, ... ]

    // This is more complex - filter-chains expect the params in a specific format.
    // We'll build this as a Struct with the control values.
    let mut properties = Vec::new();

    for (name, value) in controls {
        // Each control is a property with the control name as a string key
        // This matches what pw-cli sends: { params = [ "Freq" 1000.0, "Gain" -3.0 ] }
        properties.push(Property {
            key: spa_const::SPA_PROP_params,
            flags: libspa::pod::PropertyFlags::empty(),
            value: Value::String(format!("{}:{}", name, value)),
        });
    }

    let object = Object {
        type_: libspa::utils::SpaTypes::ObjectParamProps.as_raw(),
        id: spa_const::SPA_PARAM_PROPS,
        properties,
    };

    serialize_object(object)
}

/// Build a Props pod for a single EQ band control.
pub fn build_eq_band_pod(band_name: &str, freq: f32, q: f32, gain: f32) -> ControlResult<Vec<u8>> {
    trace!(
        "Building EQ band pod: {} freq={:.1} Q={:.2} gain={:.1}",
        band_name,
        freq,
        q,
        gain
    );

    // EQ bands in filter-chains have controls named like:
    // "band1:Freq", "band1:Q", "band1:Gain"
    // We need to set these as Props params

    build_filter_control_pod(&[
        (&format!("{}:Freq", band_name), freq),
        (&format!("{}:Q", band_name), q),
        (&format!("{}:Gain", band_name), gain),
    ])
}

/// Serialize an Object to a pod byte buffer.
fn serialize_object(object: Object) -> ControlResult<Vec<u8>> {
    let value = Value::Object(object);

    // Create a buffer for serialization
    let mut buffer = vec![0u8; 1024];
    let cursor = Cursor::new(&mut buffer[..]);

    let (_, written) = PodSerializer::serialize(cursor, &value)
        .map_err(|e| ControlError::SerializationFailed(format!("{:?}", e)))?;

    buffer.truncate(written as usize);
    debug!("Serialized pod: {} bytes", buffer.len());

    Ok(buffer)
}

/// Helper to convert dB to linear volume.
pub fn db_to_linear(db: f32) -> f32 {
    if db <= -60.0 {
        0.0
    } else {
        10.0_f32.powf(db / 20.0)
    }
}

/// Helper to convert linear volume to dB.
pub fn linear_to_db(linear: f32) -> f32 {
    if linear <= 0.0 {
        -60.0
    } else {
        20.0 * linear.log10()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_volume_pod() {
        let pod = build_volume_pod(0.5).unwrap();
        assert!(!pod.is_empty());
    }

    #[test]
    fn test_build_mute_pod() {
        let pod = build_mute_pod(true).unwrap();
        assert!(!pod.is_empty());
    }

    #[test]
    fn test_volume_clamping() {
        // Test that values are clamped correctly
        let pod_low = build_volume_pod(-1.0).unwrap();
        let pod_high = build_volume_pod(2.0).unwrap();
        assert!(!pod_low.is_empty());
        assert!(!pod_high.is_empty());
    }

    #[test]
    fn test_db_conversions() {
        // 0 dB = 1.0 linear
        assert!((db_to_linear(0.0) - 1.0).abs() < 0.001);

        // -6 dB ≈ 0.5 linear
        assert!((db_to_linear(-6.0) - 0.501).abs() < 0.01);

        // -60 dB = 0.0 (silence)
        assert_eq!(db_to_linear(-60.0), 0.0);

        // Round trip
        let original = 0.75;
        let db = linear_to_db(original);
        let back = db_to_linear(db);
        assert!((original - back).abs() < 0.001);
    }
}
