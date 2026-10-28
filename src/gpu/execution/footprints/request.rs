//! Captured upstream content and immutable acquisition demand; no publication authority.
use crate::{
    Error,
    frame_raster::FrameRaster,
    gpu::{GpuInputProgram, ProcessingParameters, SourceGeometry},
    processing::InputSettings,
};
use crate::{
    acquisition::{Plan, Request as AcquisitionRequest, Sampling},
    pixel_pipeline::PixelPipeline,
};
use glam::DMat3;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct Request {
    pub(crate) identity: crate::gpu::execution::source::Identity,
    pub(crate) repairs: crate::gpu::repairs::Snapshot,
    pub(crate) area: crate::raw::Rect,
    pub(crate) matrix: DMat3,
    pub(crate) neutral: ProcessingParameters,
    pub(crate) program: Option<GpuInputProgram>,
    pub(crate) inspection: bool,
    pub(crate) field: Option<Arc<crate::flat_field::FlatField>>,
    pub(crate) plan: Arc<Plan>,
}

impl PartialEq for Request {
    fn eq(&self, other: &Self) -> bool {
        self.same_acquisition(other) && self.repairs == other.repairs
    }
}

impl Request {
    pub(crate) fn frame(&self) -> FrameRaster {
        self.plan.output()
    }

    pub(crate) fn with_identity(
        mut self,
        identity: crate::gpu::execution::source::Identity,
    ) -> Self {
        self.identity = identity;
        self
    }

    pub(crate) fn capture(
        mut self,
        identity: crate::gpu::execution::source::Identity,
    ) -> Result<Self, Error> {
        if self.identity.captured() && self.identity != identity {
            return Err(Error::Gpu(
                "captured source content differs from available samples".into(),
            ));
        }
        self.identity = identity;
        Ok(self)
    }

    pub(crate) fn same_source(&self, other: &Self) -> bool {
        self.identity == other.identity
            && self.area == other.area
            && self.matrix == other.matrix
            && self.program == other.program
            && self.inspection == other.inspection
            && self.field == other.field
            && bytemuck::bytes_of(&self.neutral) == bytemuck::bytes_of(&other.neutral)
    }

    pub(crate) fn same_acquisition(&self, other: &Self) -> bool {
        self.same_source(other) && self.plan.same_acquisition_layout(&other.plan)
    }

    /// A source at its retained frame can run current stages only with sufficient neighbours.
    pub(crate) fn compatible_processing(&self, wanted: &Self) -> bool {
        self.same_source(wanted)
            && self.plan.sampling() == wanted.plan.sampling()
            && self
                .plan
                .for_pipeline(wanted.plan.pipeline())
                .is_ok_and(|demand| self.plan.can_supply(&demand))
    }

    /// Exact output demand may be satisfied by a larger acquired input; repairs remain content.
    pub(crate) fn satisfies(&self, wanted: &Self) -> bool {
        self.same_source(wanted)
            && self.repairs == wanted.repairs
            && self.plan.can_supply(&wanted.plan)
    }

    pub(crate) fn with_frame(mut self, frame: FrameRaster) -> Result<Self, Error> {
        if frame.composition.area() != self.area {
            return Err(Error::InvalidParameter(
                "frame belongs to a different source grid",
            ));
        }
        Self::require_frame(frame)?;
        self.plan = Arc::new(self.plan.for_output(frame)?);
        Ok(self)
    }

    pub(crate) fn with_pipeline(mut self, pipeline: PixelPipeline) -> Result<Self, Error> {
        if self.inspection && !pipeline.is_empty() {
            return Err(Error::Gpu(
                "source inspection bypasses post-grade stages".into(),
            ));
        }
        self.plan = Arc::new(self.plan.for_pipeline(&pipeline)?);
        Ok(self)
    }

    pub(crate) fn with_sampling(mut self, sampling: Sampling) -> Result<Self, Error> {
        self.plan = Arc::new(Plan::new(AcquisitionRequest {
            output: self.frame(),
            stages: self.plan.pipeline(),
            sampling,
        })?);
        Ok(self)
    }

    pub(crate) fn processing_plan(&self, pipeline: &PixelPipeline) -> Result<Plan, Error> {
        self.plan.for_pipeline(pipeline)?.using_input(&self.plan)
    }

    pub(crate) fn input_frame(&self) -> Result<FrameRaster, Error> {
        self.plan.processing_frame()
    }

    pub(crate) fn with_repairs(mut self, repairs: crate::gpu::repairs::Snapshot) -> Self {
        self.repairs = repairs;
        self
    }

    pub(crate) fn underlay(&self) -> Result<Self, Error> {
        let mut frame = FrameRaster::full(self.frame().composition)?;
        // Bound coarse navigation coverage independently of selected detail resolution.
        let scale = (f64::from(frame.output[0].max(frame.output[1])) / 1024.).max(1.);
        frame.output = frame.output.map(|n| (f64::from(n) / scale).ceil() as u32);
        self.clone().with_frame(frame)
    }

    pub(crate) fn new(
        frame: FrameRaster,
        input: impl Into<InputSettings>,
        matrix: DMat3,
        field: Option<Arc<crate::flat_field::FlatField>>,
    ) -> Result<Self, Error> {
        let program = GpuInputProgram::new(input, matrix)?;
        let neutral = program.neutral_parameters()?;
        let mut spec = Self::with_parameters(frame, matrix, neutral, field, false)?;
        spec.program = Some(program);
        Ok(spec)
    }

    pub(crate) fn inspection(
        frame: FrameRaster,
        matrix: DMat3,
        field: Option<Arc<crate::flat_field::FlatField>>,
    ) -> Result<Self, Error> {
        Self::with_parameters(frame, matrix, ProcessingParameters::source(), field, true)
    }

    pub(crate) fn is_inspection(&self) -> bool {
        self.inspection
    }

    fn require_frame(frame: FrameRaster) -> Result<(), Error> {
        frame.bounds()?;
        let affine = frame.affine()?;
        let determinant = affine[1][0] * affine[2][1] - affine[1][1] * affine[2][0];
        if !determinant.is_finite() || determinant == 0. {
            return Err(Error::Gpu(
                "physical-view footprint exceeds GPU precision".into(),
            ));
        }
        Ok(())
    }

    // The correction owner supplies effective calibration and source floor.
    fn with_parameters(
        frame: FrameRaster,
        matrix: DMat3,
        neutral: ProcessingParameters,
        field: Option<Arc<crate::flat_field::FlatField>>,
        inspection: bool,
    ) -> Result<Self, Error> {
        let area = frame.composition.area();
        Self::require_frame(frame)?;
        SourceGeometry::new(
            [area.x, area.y, area.width, area.height],
            [area.width, area.height],
        )?;
        Ok(Self {
            identity: crate::gpu::execution::source::Identity::unbound(),
            repairs: Default::default(),
            area,
            matrix,
            neutral,
            program: None,
            field,
            inspection,
            plan: Arc::new(Plan::new(AcquisitionRequest {
                output: frame,
                stages: &PixelPipeline::default(),
                sampling: Sampling::NeutralFootprints,
            })?),
        })
    }
}
