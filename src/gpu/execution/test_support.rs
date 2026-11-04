//! Authored original-cell arguments retained by fixtures for independent controls and comparisons.

use super::*;
use crate::{
    Error,
    adjustment_layers::AdjustmentLayerSnapshot,
    geometry::Composition,
    gpu::{
        evaluation, flat::GpuFlatField, grading::Grading, interpretation::InputTransform,
        photograph::PhotographGraph,
    },
    pixel_pipeline::PixelPipeline,
    processing::{InputSettings, RollSettings},
    rendering::OutputReference,
    scan::Scan,
};
use std::sync::Arc;

pub(crate) struct OriginalControls<'a> {
    pub scan: &'a Scan,
    pub field: Option<Arc<GpuFlatField>>,
    pub input: InputSettings,
    pub roll: RollSettings,
    pub grade: AdjustmentLayerSnapshot,
    pub repairs: &'a [Arc<crate::dust_repair::EvaluatedRepair>],
    pub composition: Composition,
    pub stages: PixelPipeline,
    pub output: OutputReference,
    pub luts: crate::looks::LutChain,
    pub encoding: SampleEncoding,
}

impl OriginalControls<'_> {
    pub fn lower(
        self,
        device: &wgpu::Device,
        renderer: &crate::renderer::GpuRenderer,
    ) -> Result<Request, Error> {
        if self.scan.area != self.composition.area()
            || self.scan.flat_field() != self.field.as_ref().map(|field| field.field())
        {
            return Err(Error::Gpu("fixture source/photograph mismatch".into()));
        }
        let photograph = PhotographGraph::original(
            &device.limits(),
            evaluation::camera_source(self.scan)?,
            self.field,
            InputTransform::for_scan(self.scan, self.input)?,
            Grading::new(&device.limits(), self.roll, self.grade, self.scan.area)?,
            crate::gpu::repairs::Snapshot::logical(self.repairs.to_vec())?,
            &self.stages,
        )?;
        let definition = evaluation::Reference::capture(device, renderer, self.output, &self.luts)?
            .compose(photograph.processed, renderer)?;
        Ok(Request {
            definition,
            composition: self.composition,
            encoding: self.encoding,
        })
    }
}
