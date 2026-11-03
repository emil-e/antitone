//! Immutable source-space sampling demand; mathematical inputs belong to the calculation.

#[cfg(test)]
use crate::gpu::{GpuInputProgram, SourceInterpretation};
use crate::{Error, frame_raster::FrameRaster, gpu::SourceGeometry};
use crate::{
    acquisition::{Plan, Request as AcquisitionRequest, Sampling},
    pixel_pipeline::PixelPipeline,
};
#[cfg(test)]
use glam::DMat3;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct SourceRequest {
    pub(crate) identity: crate::gpu::execution::source::Identity,
    pub(crate) repairs: crate::gpu::repairs::Snapshot,
    pub(crate) area: crate::raw::Rect,
    inspection: bool,
    #[cfg(test)]
    fixture: Option<SourceInterpretation>,
    #[cfg(test)]
    pub(crate) field: Option<Arc<crate::flat_field::FlatField>>,
    pub(crate) plan: Arc<Plan>,
    presentation: FrameRaster,
}

impl PartialEq for SourceRequest {
    fn eq(&self, other: &Self) -> bool {
        self.same_acquisition(other) && self.repairs == other.repairs
    }
}

impl SourceRequest {
    pub(crate) fn for_processing(
        frame: FrameRaster,
        input: &crate::gpu::ProcessingInput,
    ) -> Result<Self, Error> {
        let program = input.source_program().ok_or(Error::InvalidParameter(
            "working photograph has no captured source interpretation",
        ))?;
        program.neutral_parameters()?;
        let request = Self::for_frame(frame, false)?;
        #[cfg(test)]
        let request = Self {
            fixture: Some(SourceInterpretation::Photograph(*program)),
            ..request
        };
        request
            .with_pipeline(input.pixel_pipeline())
            .map(|request| request.with_repairs(input.repairs.clone()))
    }

    #[cfg(test)]
    pub(crate) fn processing_fixture(&self) -> crate::gpu::ProcessingInput {
        if let Some(program) = self.program() {
            crate::gpu::ProcessingInput::new(
                &wgpu::Limits::default(),
                program.input,
                Default::default(),
                crate::adjustment_layers::AdjustmentLayerSnapshot::new(Vec::new()).unwrap(),
                program.matrix,
                self.area,
            )
            .unwrap()
            .with_pixel_pipeline(self.plan.pipeline().clone())
        } else {
            crate::gpu::ProcessingInput::from(crate::gpu::ProcessingParameters::source())
        }
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
            && self.inspection == other.inspection
    }

    pub(crate) fn same_acquisition(&self, other: &Self) -> bool {
        self.same_source(other)
            && self.presentation == other.presentation
            && self.plan.same_acquisition_layout(&other.plan)
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
        if self.is_inspection() && !pipeline.is_empty() {
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

    pub(crate) fn input_region(&self) -> Result<crate::gpu::evaluation::CellRegion, Error> {
        let offset = self.plan.output_offset().ok_or(Error::InvalidParameter(
            "physical view requires a frame grid",
        ))?;
        let offset = offset.map(i32::try_from);
        let x = -offset[0]
            .as_ref()
            .map_err(|_| Error::InvalidParameter("view padding exceeds cell coordinates"))?;
        let y = -offset[1]
            .as_ref()
            .map_err(|_| Error::InvalidParameter("view padding exceeds cell coordinates"))?;
        crate::gpu::evaluation::CellRegion::new([x, y], self.plan.input().size()).validate()
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

    #[cfg(test)]
    pub(crate) fn new(
        frame: FrameRaster,
        input: impl Into<crate::processing::InputSettings>,
        matrix: DMat3,
        field: Option<Arc<crate::flat_field::FlatField>>,
    ) -> Result<Self, Error> {
        let program = GpuInputProgram::new(input, matrix)?;
        program.neutral_parameters()?;
        Ok(Self {
            fixture: Some(SourceInterpretation::Photograph(program)),
            field,
            ..Self::for_frame(frame, false)?
        })
    }

    pub(crate) fn inspection(frame: FrameRaster) -> Result<Self, Error> {
        Self::for_frame(frame, true)
    }

    #[cfg(test)]
    pub(crate) fn inspection_fixture(
        frame: FrameRaster,
        matrix: DMat3,
        field: Option<Arc<crate::flat_field::FlatField>>,
    ) -> Result<Self, Error> {
        crate::processing::validate_camera_matrix(matrix)?;
        crate::gpu::CameraMatrix::new(matrix)?;
        Ok(Self {
            fixture: Some(SourceInterpretation::Inspection(matrix)),
            field,
            ..Self::inspection(frame)?
        })
    }

    pub(crate) fn is_inspection(&self) -> bool {
        self.inspection
    }

    #[cfg(test)]
    pub(crate) fn matrix(&self) -> DMat3 {
        self.fixture
            .as_ref()
            .expect("declared scalar fixture")
            .matrix()
    }

    #[cfg(test)]
    pub(crate) fn program(&self) -> Option<&GpuInputProgram> {
        self.fixture
            .as_ref()
            .and_then(SourceInterpretation::program)
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

    fn for_frame(frame: FrameRaster, inspection: bool) -> Result<Self, Error> {
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
            inspection,
            #[cfg(test)]
            fixture: None,
            #[cfg(test)]
            field: None,
            presentation: frame,
            plan: Arc::new(Plan::new(AcquisitionRequest {
                output: frame,
                stages: &PixelPipeline::default(),
                sampling: Sampling::NeutralFootprints,
            })?),
        })
    }
}
