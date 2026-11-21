//! Original-coordinate camera samples from the shared, source-owned analysis raster.
use super::*;
use crate::{
    gpu::{flat::GpuFlatField, source::ProcessingSource},
    scan::{AnalysisInput, AnalysisSamples},
};

#[cfg(test)]
use crate::scan::Scan;

#[cfg(test)]
mod spatial;
#[cfg(test)]
mod tests;

const BUDGET: evaluation::Tiling = evaluation::Tiling { tile: [1, 8192] };

pub(crate) struct PreviewSamples {
    pub size: [usize; 2],
    pub lo: [u32; 2],
    pub hi: [u32; 2],
    pub camera: Vec<DVec3>,
}

pub(crate) struct CameraSampler<'a> {
    input: Arc<AnalysisInput>,
    floor: f64,
    context: &'a evaluation::Context,
    queue: &'a crate::gpu::submission::Queue,
    field: Option<Arc<GpuFlatField>>,
}

struct CameraSites {
    input: Arc<AnalysisInput>,
    indices: Arc<[usize]>,
    domain: evaluation::Domain,
}

impl evaluation::Source for CameraSites {
    fn domain(&self) -> evaluation::Domain {
        self.domain
    }

    fn read(&self, region: evaluation::CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        let region = region.source()?;
        let pixels = (region.y..region.y + region.height)
            .map(|y| self.input.pixel(self.indices[y as usize]))
            .collect::<Result<Vec<_>, _>>()?;
        ProcessingSource::validate_continuous(&pixels)?;
        Ok(pixels)
    }
}

impl<'a> CameraSampler<'a> {
    #[cfg(test)]
    pub(crate) fn new(
        context: &'a evaluation::Context,
        gpu: &'a crate::gpu::submission::Queue,
        scan: &Scan,
    ) -> Result<Self, Error> {
        let input = AnalysisInput::capture(scan, || false)?
            .ok_or(Error::UnusableSample("camera capture cancelled"))?;
        Self::retained(context, gpu, Arc::new(input), scan.flat_field().cloned())
    }

    #[cfg(test)]
    pub(crate) fn retained(
        context: &'a evaluation::Context,
        gpu: &'a crate::gpu::submission::Queue,
        input: Arc<AnalysisInput>,
        field: Option<Arc<crate::flat_field::FlatField>>,
    ) -> Result<Self, Error> {
        let field = field
            .map(|f| GpuFlatField::new(context.device(), f).map(Arc::new))
            .transpose()?;
        Self::captured(context, gpu, input, field)
    }

    pub(crate) fn captured(
        context: &'a evaluation::Context,
        gpu: &'a crate::gpu::submission::Queue,
        input: Arc<AnalysisInput>,
        field: Option<Arc<GpuFlatField>>,
    ) -> Result<Self, Error> {
        let floor = input.effective_floor(field.as_ref().map(|f| f.field().as_ref()))?;
        if let Some(field) = &field {
            field.validate_matrix(glam::DMat3::IDENTITY, crate::processing::MAX_SOURCE_GAIN)?;
        }
        Ok(Self {
            input,
            floor,
            context,
            queue: gpu,
            field,
        })
    }

    pub(crate) fn effective_floor(&self) -> f64 {
        self.floor
    }

    fn gathered(&self, indices: Vec<usize>) -> Result<evaluation::Value, Error> {
        let count = u32::try_from(indices.len())
            .ok()
            .filter(|v| *v > 0)
            .ok_or(Error::InvalidParameter("invalid camera sample batch"))?;
        let positions = indices
            .iter()
            .map(|&i| self.input.position(i))
            .collect::<Result<Vec<_>, _>>()?;
        let domain = evaluation::Domain {
            bounds: Some(evaluation::CellRegion::new([0, 0], [1, count])),
            grid: evaluation::Grid::axis_aligned([0.; 2], [1.; 2]),
            meaning: evaluation::Meaning::CameraCodes,
        };
        let codes = evaluation::source(Arc::new(CameraSites {
            input: self.input.clone(),
            indices: indices.into(),
            domain,
        }));
        crate::gpu::photograph::PhotographGraph::camera_samples(
            codes,
            self.field.clone(),
            &positions,
        )
    }

    pub(crate) fn sample(
        &self,
        area: crate::geometry::SourceRegion,
        budget: usize,
        inside: impl FnMut(crate::geometry::SourceRegion) -> bool,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Option<AnalysisSamples>, Error> {
        let Some(selected) = self.input.select(area, budget, inside, &mut cancelled)? else {
            return Ok(None);
        };
        let mut result = AnalysisSamples {
            attempted: selected.len(),
            ..Default::default()
        };
        let mut indices = Vec::with_capacity(selected.len());
        for index in selected {
            if cancelled() {
                return Ok(None);
            }
            if self.input.above_native_floor(index)? {
                indices.push(index);
            } else {
                result.rejected_floor += 1;
            }
        }
        if indices.is_empty() {
            return Ok(Some(result));
        }
        let source_positions = indices
            .iter()
            .map(|&i| self.input.position(i))
            .collect::<Result<Vec<_>, _>>()?;
        let Some(pixels) = self.read_centres(indices, &mut cancelled)? else {
            return Ok(None);
        };
        for (position, pixel) in source_positions.into_iter().zip(pixels) {
            if pixel.min_element() > self.floor {
                result.positions.push(position);
                result.pixels.push(pixel);
            } else {
                result.rejected_floor += 1;
            }
        }
        Ok(Some(result))
    }

    pub(crate) fn preview_centres(
        &self,
        area: crate::geometry::SourceRegion,
        cancelled: &mut impl FnMut() -> bool,
    ) -> Result<Option<PreviewSamples>, Error> {
        let view = self.input.window(area, 384)?;
        let size = view.size();
        let lo = self.input.position(view.index(0))?;
        let hi = self.input.position(view.index(size[0] * size[1] - 1))?;
        let indices = (0..size[0] * size[1]).map(|i| view.index(i)).collect();
        Ok(self
            .read_centres(indices, cancelled)?
            .map(|camera| PreviewSamples {
                size,
                lo,
                hi,
                camera,
            }))
    }

    fn read_centres(
        &self,
        indices: Vec<usize>,
        cancelled: &mut impl FnMut() -> bool,
    ) -> Result<Option<Vec<DVec3>>, Error> {
        let count = indices.len();
        let image = self.gathered(indices)?;
        let request = self.context.request(
            image.clone(),
            evaluation::Region {
                bounds: image.domain().finite()?,
                grid: image.domain().grid,
            },
            BUDGET,
        )?;
        let cancelled = RefCell::new(cancelled);
        let interrupted = Cell::new(false);
        let mut output = Vec::with_capacity(count);
        let result = self.context.read(
            self.queue,
            &request,
            || interrupted.get() || (**cancelled.borrow_mut())(),
            |block, pixels| {
                let block = block.source()?;
                if block.x != 0
                    || block.width != 1
                    || block.y as usize != output.len()
                    || pixels.len() != block.height as usize
                {
                    return Err(Error::InvalidParameter(
                        "camera sample response changed order",
                    ));
                }
                for pixel in pixels {
                    if (**cancelled.borrow_mut())() {
                        interrupted.set(true);
                        return Ok(());
                    }
                    let rgb = DVec3::new(
                        f64::from(pixel[0]),
                        f64::from(pixel[1]),
                        f64::from(pixel[2]),
                    );
                    if !rgb.is_finite() {
                        return Err(Error::NumericalOverflow);
                    }
                    output.push(rgb);
                }
                Ok(())
            },
        )?;
        if result.is_none() || interrupted.get() {
            return Ok(None);
        }
        if output.len() != count {
            return Err(Error::InvalidParameter("camera response omitted samples"));
        }
        Ok(Some(output))
    }
}
