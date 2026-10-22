//! Physical frame qualification and fixed-size posing from immutable bounded captures.
use super::*;
use crate::{
    project::Development,
    spatial_metadata::{Input, Metadata},
};
use serde::Serialize;

mod scene;
mod shape;

/// One bounded, immutable camera-code capture. Native developments and UI state are not retained.
pub struct FrameInput {
    input: Input,
    capture: Capture,
    evidence: Evidence,
}

impl FrameInput {
    pub fn new(
        input: Input,
        size: [u32; 2],
        rgb: Vec<[u16; 3]>,
        evidence: Evidence,
    ) -> Result<Self, Error> {
        input.development.validate()?;
        let capture = Capture::new(
            input.development.active_area.into(),
            size,
            rgb,
            evidence.clone(),
        )?;
        Ok(Self {
            input,
            capture,
            evidence,
        })
    }
}

/// Pure preparation stage. Callers acquire one source at a time through normal services and
/// own publication/freshness; this stage neither accepts consumer edits nor starts workers.
pub fn prepare_spatial(
    mut frames: Vec<FrameInput>,
    development: Development,
    code_scale: f64,
    checkpoint: impl Fn() -> Result<(), String>,
) -> Result<Metadata, String> {
    development.validate().map_err(|e| e.to_string())?;
    if !code_scale.is_finite() || code_scale < 1. {
        return Err("invalid spatial camera-code scale".into());
    }
    if frames
        .iter()
        .any(|frame| frame.input.code_scale != code_scale)
    {
        return Err("spatial camera-code input basis differs".into());
    }
    frames.sort_by_key(|frame| frame.input.source.0);
    if frames
        .windows(2)
        .any(|pair| pair[0].input.source == pair[1].input.source)
    {
        return Err("duplicate spatial source identity".into());
    }
    checkpoint()?;
    let mut groups: Vec<Vec<FrameInput>> = vec![];
    for frame in frames {
        if let Some(group) = groups.iter_mut().find(|group| {
            group[0].capture.area == frame.capture.area
                && group[0].capture.size == frame.capture.size
                && group[0].input.development.camera_basis()
                    == frame.input.development.camera_basis()
        }) {
            group.push(frame);
        } else {
            groups.push(vec![frame]);
        }
    }
    let mut metadata = Metadata::default();
    for group in groups {
        checkpoint()?;
        let mut inputs = vec![];
        let mut captures = vec![];
        let mut evidence = vec![];
        for frame in group {
            inputs.push(frame.input);
            captures.push(frame.capture);
            evidence.push(frame.evidence);
        }
        metadata.groups.push(scene::prepare(
            &captures,
            &evidence,
            inputs,
            development.clone(),
            code_scale,
            &checkpoint,
        )?);
    }
    metadata.validate().map_err(|e| e.to_string())?;
    checkpoint()?;
    Ok(metadata)
}
