//! Deferred, source-grid GI views share the cached camera payload with statistics.
use super::*;
use crate::{geometry::SourceRegion, roll_analysis::SpatialSamples, scan::AnalysisWindow};

#[cfg(test)]
mod tests;

pub(crate) struct CameraWindow {
    input: Arc<AnalysisInput>,
    view: AnalysisWindow,
    support: Vec<bool>,
}

impl CameraWindow {
    pub(crate) fn new(
        input: Arc<AnalysisInput>,
        area: SourceRegion,
        mut inside: impl FnMut(SourceRegion) -> bool,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Option<Self>, Error> {
        if cancelled() {
            return Ok(None);
        }
        let view = input.window(area, crate::roll_analysis::GRAYNESS_LONG_EDGE as u32)?;
        let size = view.size();
        let mut support = Vec::with_capacity(size[0] * size[1]);
        for i in 0..size[0] * size[1] {
            if i % size[0] == 0 && cancelled() {
                return Ok(None);
            }
            let [x, y] = input.position(view.index(i))?;
            support.push(inside(SourceRegion {
                x,
                y,
                width: 1,
                height: 1,
            }));
        }
        Ok((!cancelled()).then_some(Self {
            input,
            view,
            support,
        }))
    }

    pub(crate) fn read(
        &self,
        renderer: &GpuRenderer,
        gpu: (&wgpu::Queue, &SubmissionGate),
        field: Option<Arc<GpuFlatField>>,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Option<SpatialSamples>, Error> {
        let sampler = CameraSampler::captured(renderer, gpu, self.input.clone(), field)?;
        self.read_with(&sampler, &mut cancelled)
    }

    fn read_with(
        &self,
        sampler: &CameraSampler<'_>,
        cancelled: &mut impl FnMut() -> bool,
    ) -> Result<Option<SpatialSamples>, Error> {
        let mut indices = Vec::new();
        let mut locations = Vec::new();
        for (i, &inside) in self.support.iter().enumerate() {
            if i % self.view.size()[0] == 0 && cancelled() {
                return Ok(None);
            }
            let index = self.view.index(i);
            if inside && self.input.above_native_floor(index)? {
                indices.push(index);
                locations.push(i);
            }
        }
        let mut camera = vec![DVec3::ZERO; self.support.len()];
        let mut valid = vec![false; self.support.len()];
        if !indices.is_empty() {
            let Some(pixels) = sampler.read_centres(indices, cancelled)? else {
                return Ok(None);
            };
            for (i, rgb) in locations.into_iter().zip(pixels) {
                if rgb.min_element() > sampler.floor {
                    camera[i] = rgb;
                    valid[i] = true;
                }
            }
        }
        if cancelled() {
            return Ok(None);
        }
        Ok(Some(SpatialSamples::new(self.view.size(), camera, valid)?))
    }

    #[cfg(test)]
    pub(crate) fn size(&self) -> [usize; 2] {
        self.view.size()
    }
}

impl CameraSampler<'_> {
    pub(crate) fn window(
        &self,
        area: SourceRegion,
        inside: impl FnMut(SourceRegion) -> bool,
        cancelled: impl FnMut() -> bool,
    ) -> Result<Option<CameraWindow>, Error> {
        CameraWindow::new(self.input.clone(), area, inside, cancelled)
    }

    #[cfg(test)]
    pub(crate) fn spatial(
        &self,
        area: SourceRegion,
        inside: impl FnMut(SourceRegion) -> bool,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Option<SpatialSamples>, Error> {
        let Some(window) = self.window(area, inside, &mut cancelled)? else {
            return Ok(None);
        };
        window.read_with(self, &mut cancelled)
    }
}
