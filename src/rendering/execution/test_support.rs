//! Authored original-cell arguments retained by fixtures for independent controls and comparisons.

use super::*;
use crate::photographic::graph::authoring::PhotographTaps;
use crate::{
    Error,
    adjustment_layers::AdjustmentLayerSnapshot,
    geometry::Composition,
    photographic::stages::flat_field::GpuFlatField,
    photographic::{
        graph::authoring::Assembly,
        stages::{grading::Grading, interpretation::InputTransform},
    },
    processing::{InputSettings, RollSettings},
    rendering::OutputTarget,
    scan::Scan,
};
use std::sync::Arc;

/// Processing reference, independent of file codec, transfer encoding and sample storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutputReference {
    SceneReferred,
    DisplayReferred(OutputTarget),
}

#[cfg(all(target_os = "macos", feature = "ocio-reference"))]
impl OutputReference {
    pub fn display_target(self) -> Option<OutputTarget> {
        match self {
            Self::SceneReferred => None,
            Self::DisplayReferred(target) => Some(target),
        }
    }
}

pub(crate) struct OriginalControls<'a> {
    pub scan: &'a Scan,
    pub field: Option<Arc<GpuFlatField>>,
    pub input: InputSettings,
    pub roll: RollSettings,
    pub grade: AdjustmentLayerSnapshot,
    pub repairs: &'a [crate::dust_repair::Repair],
    pub composition: Composition,
    pub stages: Assembly,
    pub output: OutputReference,
    pub luts: crate::looks::LutChain,
    pub encoding: SampleEncoding,
}

impl OriginalControls<'_> {
    pub fn lower(
        self,
        device: &wgpu::Device,
        renderer: &crate::render_fixture::RenderFixture,
    ) -> Result<Request, Error> {
        if self.scan.area != self.composition.area()
            || self.scan.flat_field() != self.field.as_ref().map(|field| field.field())
        {
            return Err(Error::Gpu("fixture source/photograph mismatch".into()));
        }
        let photograph = PhotographTaps::authored_original(
            &device.limits(),
            crate::acquisition::camera::camera_source(self.scan)?,
            self.field,
            InputTransform::for_scan(self.scan, self.input)?,
            Grading::new(&device.limits(), self.roll, self.grade, self.scan.area)?,
            self.repairs.to_vec(),
            &self.stages,
        )?;
        let definition = match self.output {
            OutputReference::SceneReferred => photograph.processed,
            OutputReference::DisplayReferred(target) => crate::photographic::graph::display_output(
                photograph.processed,
                &self.luts,
                &renderer.looks,
                renderer.drt.get(target),
            )?,
        };
        Ok(Request {
            definition,
            composition: self.composition,
            encoding: self.encoding,
        })
    }
}
