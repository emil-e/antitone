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
    presentation: FrameRaster,
}

impl PartialEq for Request {
    fn eq(&self, other: &Self) -> bool {
        self.same_acquisition(other) && self.repairs == other.repairs
    }
}

// Numerical source provenance and representation, independent of requested support and ticket.
#[derive(Clone)]
struct NeutralContent(Request);

impl PartialEq for NeutralContent {
    fn eq(&self, other: &Self) -> bool {
        self.0.same_source(&other.0)
            && self.0.repairs == other.0.repairs
            && self.0.plan.sampling() == other.0.plan.sampling()
    }
}

impl std::fmt::Debug for NeutralContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NeutralContent")
            .field("area", &self.0.area)
            .field("sampling", &self.0.plan.sampling())
            .field("inspection", &self.0.inspection)
            .finish_non_exhaustive()
    }
}

impl Request {
    pub(crate) fn cache_key(&self) -> crate::gpu::evaluation::RetainedKey {
        crate::gpu::evaluation::RetainedKey::new(NeutralContent(self.clone()))
    }

    pub(crate) fn frame(&self) -> FrameRaster {
        self.presentation
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
        self.same_source(other)
            && self.presentation == other.presentation
            && self.plan.same_acquisition_layout(&other.plan)
    }

    /// A source at its retained frame can run current stages only with sufficient neighbours.
    pub(crate) fn compatible_processing(&self, wanted: &Self) -> bool {
        self.same_source(wanted)
            && self.plan.sampling() == wanted.plan.sampling()
            && self.view_grid_sufficient(wanted.plan.pipeline())
            && self
                .plan
                .for_pipeline(wanted.plan.pipeline())
                .is_ok_and(|demand| self.plan.can_supply(&demand))
    }

    /// Exact output demand may be satisfied by a larger acquired input; repairs remain content.
    pub(crate) fn satisfies(&self, wanted: &Self) -> bool {
        self.same_source(wanted)
            && self.repairs == wanted.repairs
            && self.presentation == wanted.presentation
            && self.plan.can_supply(&wanted.plan)
    }

    pub(crate) fn with_frame(mut self, frame: FrameRaster) -> Result<Self, Error> {
        if frame.composition.area() != self.area {
            return Err(Error::InvalidParameter(
                "frame belongs to a different source grid",
            ));
        }
        Self::require_frame(frame)?;
        self.presentation = frame;
        let pipeline = self.plan.pipeline().clone();
        let sampling = self.plan.sampling();
        self.replan(pipeline, sampling)
    }

    pub(crate) fn with_pipeline(self, pipeline: PixelPipeline) -> Result<Self, Error> {
        if self.inspection && !pipeline.is_empty() {
            return Err(Error::Gpu(
                "source inspection bypasses post-grade stages".into(),
            ));
        }
        let sampling = self.plan.sampling();
        self.replan(pipeline, sampling)
    }

    pub(crate) fn with_sampling(self, sampling: Sampling) -> Result<Self, Error> {
        let pipeline = self.plan.pipeline().clone();
        self.replan(pipeline, sampling)
    }

    fn replan(mut self, pipeline: PixelPipeline, sampling: Sampling) -> Result<Self, Error> {
        let layout = crate::pixel_pipeline::sampling::ViewSampling::new(
            self.presentation,
            &pipeline,
            sampling,
        )?;
        log::trace!(
            "captured view sampling: ideal {:?}, working {:?}, limited {}",
            layout.ideal,
            layout.plan.output().output,
            layout.limited()
        );
        self.presentation = layout.presentation;
        self.plan = Arc::new(layout.plan);
        Ok(self)
    }

    fn view_grid_sufficient(&self, pipeline: &PixelPipeline) -> bool {
        crate::pixel_pipeline::sampling::ViewSampling::new(
            self.presentation,
            pipeline,
            self.plan.sampling(),
        )
        .is_ok_and(|wanted| {
            (0..2).all(|i| self.plan.output().output[i] >= wanted.plan.output().output[i])
        })
    }

    pub(crate) fn processing_plan(&self, pipeline: &PixelPipeline) -> Result<Plan, Error> {
        if !self.view_grid_sufficient(pipeline) {
            return Err(Error::InvalidParameter(
                "acquired view grid is too coarse for processing",
            ));
        }
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
            presentation: frame,
            plan: Arc::new(Plan::new(AcquisitionRequest {
                output: frame,
                stages: &PixelPipeline::default(),
                sampling: Sampling::NeutralFootprints,
            })?),
        })
    }
}
