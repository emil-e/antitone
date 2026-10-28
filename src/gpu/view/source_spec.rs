//! Presentation ticket wrapped around publication-independent captured source demand.
use crate::{
    Error,
    frame_raster::FrameRaster,
    gpu::execution::{footprints::Request, source::Identity},
    pixel_pipeline::PixelPipeline,
    processing::InputSettings,
};
use glam::DMat3;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct ViewSourceSpec {
    pub authority: u64,
    pub(crate) request: Request,
}
impl std::ops::Deref for ViewSourceSpec {
    type Target = Request;
    fn deref(&self) -> &Request {
        &self.request
    }
}
impl PartialEq for ViewSourceSpec {
    fn eq(&self, other: &Self) -> bool {
        self.authority == other.authority && self.request == other.request
    }
}
impl ViewSourceSpec {
    pub(crate) fn new(
        authority: u64,
        frame: FrameRaster,
        input: impl Into<InputSettings>,
        matrix: DMat3,
        field: Option<Arc<crate::flat_field::FlatField>>,
    ) -> Result<Self, Error> {
        Ok(Self {
            authority,
            request: Request::new(frame, input, matrix, field)?,
        })
    }
    pub(crate) fn inspection(
        authority: u64,
        frame: FrameRaster,
        matrix: DMat3,
        field: Option<Arc<crate::flat_field::FlatField>>,
    ) -> Result<Self, Error> {
        Ok(Self {
            authority,
            request: Request::inspection(frame, matrix, field)?,
        })
    }
    pub(crate) fn with_identity(mut self, identity: Identity) -> Self {
        self.request = self.request.with_identity(identity);
        self
    }
    pub(crate) fn with_frame(mut self, frame: FrameRaster) -> Result<Self, Error> {
        self.request = self.request.with_frame(frame)?;
        Ok(self)
    }
    pub(crate) fn with_pipeline(mut self, pipeline: PixelPipeline) -> Result<Self, Error> {
        self.request = self.request.with_pipeline(pipeline)?;
        Ok(self)
    }
    #[cfg(test)]
    pub(crate) fn with_sampling(
        mut self,
        sampling: crate::acquisition::Sampling,
    ) -> Result<Self, Error> {
        self.request = self.request.with_sampling(sampling)?;
        Ok(self)
    }
    pub(crate) fn with_repairs(mut self, repairs: crate::gpu::repairs::Snapshot) -> Self {
        self.request = self.request.with_repairs(repairs);
        self
    }
    pub(crate) fn underlay(&self) -> Result<Self, Error> {
        Ok(Self {
            authority: self.authority,
            request: self.request.underlay()?,
        })
    }
}
