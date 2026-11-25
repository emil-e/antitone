//! Source identity, presentation framing and acquisition geometry.

#[cfg(test)]
use super::authoring::Assembly;
use crate::acquisition::{Plan, Request as AcquisitionRequest, Sampling};
#[cfg(test)]
use crate::photographic::stages::interpretation::InputTransform;
use crate::{Error, frame_raster::FrameRaster, gpu::source::SourceGeometry};
#[cfg(test)]
use glam::DMat3;
use std::sync::Arc;

#[derive(Clone)]
pub struct SourceRequest {
    pub identity: crate::gpu::source::identity::Identity,
    pub area: crate::raw::Rect,
    #[cfg(test)]
    fixture: Option<SourceFixture>,
    #[cfg(test)]
    pub field: Option<Arc<crate::flat_field::FlatField>>,
    plan: Arc<Plan>,
    presentation: FrameRaster,
    #[cfg(test)]
    definition: Arc<Assembly>,
}

#[cfg(test)]
#[derive(Clone)]
enum SourceFixture {
    Photograph(crate::gpu::test_inputs::AuthoredPhotograph),
    Inspection(DMat3),
}

impl PartialEq for SourceRequest {
    fn eq(&self, other: &Self) -> bool {
        self.same_acquisition(other)
    }
}

impl SourceRequest {
    #[cfg(test)]
    pub fn for_processing(
        frame: FrameRaster,
        input: &crate::gpu::test_inputs::AuthoredPhotograph,
    ) -> Result<Self, Error> {
        let program = input.source_transform().ok_or(Error::InvalidParameter(
            "working photograph has no captured source interpretation",
        ))?;
        program.neutral_parameters()?;
        let request = Self::for_frame(frame)?;
        #[cfg(test)]
        let request = Self {
            fixture: Some(SourceFixture::Photograph(input.clone())),
            field: input.field.as_ref().map(|field| field.field().clone()),
            ..request
        };
        let mut definition = input.definition();
        definition.grading = input.grade().cloned();
        request.with_definition(definition)
    }

    #[cfg(test)]
    pub fn processing_fixture(&self) -> crate::gpu::test_inputs::AuthoredPhotograph {
        match &self.fixture {
            Some(SourceFixture::Photograph(input)) => input
                .clone()
                .with_definition(self.definition.as_ref().clone()),
            _ => crate::gpu::test_inputs::AuthoredPhotograph::from(
                crate::gpu::parameters::ProcessingParameters::source(),
            ),
        }
    }

    pub fn plan(&self) -> &Plan {
        &self.plan
    }

    pub(crate) fn with_plan(mut self, plan: Plan) -> Self {
        self.plan = Arc::new(plan);
        self
    }

    pub fn frame(&self) -> FrameRaster {
        self.presentation
    }

    pub fn with_identity(mut self, identity: crate::gpu::source::identity::Identity) -> Self {
        self.identity = identity;
        self
    }

    pub fn capture(
        mut self,
        identity: crate::gpu::source::identity::Identity,
    ) -> Result<Self, Error> {
        if self.identity.captured() && self.identity != identity {
            return Err(Error::Gpu(
                "captured source content differs from available samples".into(),
            ));
        }
        self.identity = identity;
        Ok(self)
    }

    pub fn same_source(&self, other: &Self) -> bool {
        self.identity == other.identity && self.area == other.area
    }

    pub fn same_acquisition(&self, other: &Self) -> bool {
        self.same_source(other)
            && self.presentation == other.presentation
            && self.plan.same_acquisition_layout(&other.plan)
    }

    #[cfg(test)]
    pub fn with_frame(mut self, frame: FrameRaster) -> Result<Self, Error> {
        if frame.composition.area() != self.area {
            return Err(Error::InvalidParameter(
                "frame belongs to a different source grid",
            ));
        }
        Self::require_frame(frame)?;
        self.presentation = frame;
        let sampling = self.plan.sampling();
        self.replan(sampling)
    }

    #[cfg(test)]
    pub(crate) fn with_definition(mut self, definition: Assembly) -> Result<Self, Error> {
        self.definition = Arc::new(definition);
        let sampling = self.plan.sampling();
        self.replan(sampling)
    }

    #[cfg(test)]
    pub(crate) fn definition(&self) -> &Assembly {
        &self.definition
    }

    #[cfg(test)]
    pub fn with_sampling(self, sampling: Sampling) -> Result<Self, Error> {
        self.replan(sampling)
    }

    #[cfg(test)]
    fn replan(self, sampling: Sampling) -> Result<Self, Error> {
        let layout = self.definition.view_sampling(self.presentation, sampling)?;
        Ok(self.with_layout(layout))
    }

    pub(super) fn with_layout(mut self, layout: super::sampling::ViewSampling) -> Self {
        log::trace!(
            "captured view sampling: ideal {:?}, working {:?}, limited {}",
            layout.ideal,
            layout.plan.output().output,
            layout.limited()
        );
        self.presentation = layout.presentation;
        self.with_plan(layout.plan)
    }

    #[cfg(test)]
    pub fn input_region(&self) -> Result<crate::gpu::evaluation::CellRegion, Error> {
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

    #[cfg(test)]
    pub fn input_frame(&self) -> Result<FrameRaster, Error> {
        self.plan.processing_frame()
    }

    #[cfg(test)]
    pub fn new(
        frame: FrameRaster,
        input: impl Into<crate::processing::InputSettings>,
        matrix: DMat3,
        field: Option<Arc<crate::flat_field::FlatField>>,
    ) -> Result<Self, Error> {
        let fixture = crate::gpu::test_inputs::AuthoredPhotograph::new(
            &wgpu::Limits::default(),
            input,
            Default::default(),
            crate::adjustment_layers::AdjustmentLayerSnapshot::new(Vec::new())?,
            matrix,
            frame.composition.area(),
        )?;
        fixture.input_transform().neutral_parameters()?;
        Ok(Self {
            fixture: Some(SourceFixture::Photograph(fixture)),
            field,
            ..Self::for_frame(frame)?
        })
    }

    #[cfg(test)]
    pub fn inspection(frame: FrameRaster) -> Result<Self, Error> {
        Self::for_frame(frame)
    }

    #[cfg(test)]
    pub fn inspection_fixture(
        frame: FrameRaster,
        matrix: DMat3,
        field: Option<Arc<crate::flat_field::FlatField>>,
    ) -> Result<Self, Error> {
        crate::processing::validate_camera_matrix(matrix)?;
        crate::gpu::parameters::CameraMatrix::new(matrix)?;
        Ok(Self {
            fixture: Some(SourceFixture::Inspection(matrix)),
            field,
            ..Self::inspection(frame)?
        })
    }

    #[cfg(test)]
    pub fn is_inspection(&self) -> bool {
        !matches!(self.fixture, Some(SourceFixture::Photograph(_)))
    }

    #[cfg(test)]
    pub fn matrix(&self) -> DMat3 {
        match self.fixture.as_ref().expect("declared scalar fixture") {
            SourceFixture::Photograph(input) => input.input_transform().matrix(),
            SourceFixture::Inspection(matrix) => *matrix,
        }
    }

    #[cfg(test)]
    pub fn program(&self) -> Option<&InputTransform> {
        match &self.fixture {
            Some(SourceFixture::Photograph(input)) => input.source_transform(),
            _ => None,
        }
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

    pub fn for_frame(frame: FrameRaster) -> Result<Self, Error> {
        let area = frame.composition.area();
        Self::require_frame(frame)?;
        SourceGeometry::new(
            [area.x, area.y, area.width, area.height],
            [area.width, area.height],
        )?;
        Ok(Self {
            identity: crate::gpu::source::identity::Identity::unbound(),
            area,
            #[cfg(test)]
            fixture: None,
            #[cfg(test)]
            field: None,
            presentation: frame,
            #[cfg(test)]
            definition: Arc::default(),
            plan: Arc::new(Plan::new(AcquisitionRequest {
                output: frame,
                sampling: Sampling::NeutralFootprints,
            })?),
        })
    }
}
