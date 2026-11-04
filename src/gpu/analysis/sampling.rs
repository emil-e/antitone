//! Original-coordinate camera-light samples for the statistical roll analysis.

use super::*;
use crate::{
    gpu::{GpuFlatField, ProcessingSource},
    renderer::GpuRenderer,
    scan::{AnalysisSamples, DevelopedSource, Scan},
};

#[cfg(test)]
mod tests;

const BUDGET: evaluation::Budget = evaluation::Budget {
    tile: [8, 8192],
    scratch_bytes: 32 << 20,
};

pub(crate) struct CameraSampler<'a> {
    input: Input<'a>,
    floor: f64,
    context: &'a evaluation::Context,
    queue: &'a wgpu::Queue,
    submissions: &'a SubmissionGate,
    field: Option<Arc<GpuFlatField>>,
}

enum Input<'a> {
    Native(&'a Scan),
    Retained(Arc<crate::scan::AnalysisInput>),
}

enum Pixels {
    Native(Arc<DevelopedSource>),
    Retained {
        input: Arc<crate::scan::AnalysisInput>,
        indices: Vec<usize>,
    },
}

struct CameraFootprints {
    source: Pixels,
    origins: Arc<[[u32; 2]]>,
    domain: evaluation::Domain,
}

impl evaluation::Source for CameraFootprints {
    fn domain(&self) -> evaluation::Domain {
        self.domain
    }

    fn read(&self, region: evaluation::CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        let region = region.source()?;
        let mut pixels = Vec::with_capacity((region.width * region.height) as usize);
        match &self.source {
            Pixels::Native(source) => {
                let image = source.camera_image()?;
                for y in region.y..region.y + region.height {
                    let origin = self.origins[(y / 8) as usize];
                    for x in region.x..region.x + region.width {
                        pixels.push(image.pixel_samples(origin[0] + x, origin[1] + y % 8)?);
                    }
                }
            }
            Pixels::Retained { input, indices } => {
                for y in region.y..region.y + region.height {
                    let patch = input.pixels(indices[(y / 8) as usize]);
                    for x in region.x..region.x + region.width {
                        pixels.push(patch[((y % 8) * 8 + x) as usize]);
                    }
                }
            }
        }

        ProcessingSource::validate_continuous(&pixels)?;

        Ok(pixels)
    }
}

#[derive(Clone, Copy, Default)]
struct Footprint {
    centre: DVec3,
    floor: bool,
    cells: u8,
}

impl<'a> CameraSampler<'a> {
    pub(crate) fn new(
        device: &'a wgpu::Device,
        renderer: &'a GpuRenderer,
        gpu: (&'a wgpu::Queue, &'a SubmissionGate),
        scan: &'a Scan,
    ) -> Result<Self, Error> {
        let field = scan
            .flat_field()
            .cloned()
            .map(|field| GpuFlatField::new(device, field).map(Arc::new))
            .transpose()?;

        if let Some(field) = &field {
            field.validate_matrix(glam::DMat3::IDENTITY, crate::processing::MAX_SOURCE_GAIN)?;
        }

        Ok(Self {
            input: Input::Native(scan),
            floor: scan.effective_floor(),
            context: renderer.evaluation(),
            queue: gpu.0,
            submissions: gpu.1,
            field,
        })
    }

    pub(crate) fn retained(
        device: &'a wgpu::Device,
        renderer: &'a GpuRenderer,
        gpu: (&'a wgpu::Queue, &'a SubmissionGate),
        input: Arc<crate::scan::AnalysisInput>,
        field: Option<Arc<crate::flat_field::FlatField>>,
    ) -> Result<Self, Error> {
        let floor = input.effective_floor(field.as_deref())?;
        let field = field
            .map(|field| GpuFlatField::new(device, field).map(Arc::new))
            .transpose()?;
        if let Some(field) = &field {
            field.validate_matrix(glam::DMat3::IDENTITY, crate::processing::MAX_SOURCE_GAIN)?;
        }
        Ok(Self {
            input: Input::Retained(input),
            floor,
            context: renderer.evaluation(),
            queue: gpu.0,
            submissions: gpu.1,
            field,
        })
    }

    pub(crate) fn effective_floor(&self) -> f64 {
        self.floor
    }

    fn gathered(&self, origins: Vec<[u32; 2]>) -> Result<Arc<evaluation::ImageNode>, Error> {
        let height = u32::try_from(origins.len())
            .ok()
            .and_then(|count| count.checked_mul(8))
            .filter(|height| *height > 0)
            .ok_or(Error::InvalidParameter("invalid camera footprint batch"))?;
        let domain = evaluation::Domain {
            bounds: Some(crate::gpu::evaluation::CellRegion::new([0, 0], [8, height])),
            grid: evaluation::Grid::axis_aligned([0.; 2], [1.; 2]),
            meaning: evaluation::Meaning::CameraCodes,
        };
        let codes = evaluation::source(Arc::new(CameraFootprints {
            source: match &self.input {
                Input::Native(scan) => Pixels::Native(scan.source().clone()),
                Input::Retained(input) => Pixels::Retained {
                    indices: origins
                        .iter()
                        .map(|&origin| input.patch_index(origin))
                        .collect::<Result<_, _>>()?,
                    input: input.clone(),
                },
            },
            origins: origins.clone().into(),
            domain,
        }));

        evaluation::Photograph::camera_footprints(codes, self.field.clone(), &origins)
    }

    pub(crate) fn sample(
        &self,
        area: crate::geometry::SourceRegion,
        budget: usize,
        inside: impl FnMut(crate::geometry::SourceRegion) -> bool,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Option<AnalysisSamples>, Error> {
        let plan = match &self.input {
            Input::Native(scan) => scan.analysis_plan(area, budget, inside, &mut cancelled)?,
            Input::Retained(input) => input.plan(area, budget, inside, &mut cancelled)?,
        };
        let Some(mut plan) = plan else {
            return Ok(None);
        };

        if plan.footprints.is_empty() {
            return Ok(Some(plan.samples));
        }

        let origins = plan
            .footprints
            .iter()
            .map(|footprint| [footprint.region.x, footprint.region.y])
            .collect();
        let image = self.gathered(origins)?;
        let request = self.context.request(
            image.clone(),
            evaluation::Region {
                bounds: image.domain().finite()?,
                grid: image.domain().grid,
            },
            BUDGET,
        )?;
        let floor = self.floor as f32;
        let mut footprints = vec![Footprint::default(); plan.footprints.len()];
        let cancelled = RefCell::new(&mut cancelled);
        let interrupted = Cell::new(false);

        // The index raster contains only selected original cells. Its sideband origins preserve
        // physical correction coordinates without transferring the intervening sensor pixels.
        let result = self.context.read(
            self.queue,
            self.submissions,
            &request,
            || {
                let mut predicate = cancelled.borrow_mut();
                interrupted.get() || (**predicate)()
            },
            |block, pixels| {
                let block = block.source()?;
                for (index, pixel) in pixels.iter().enumerate() {
                    let mut predicate = cancelled.borrow_mut();
                    if (**predicate)() {
                        interrupted.set(true);
                        return Ok(());
                    }
                    drop(predicate);

                    let x = block.x + index as u32 % block.width;
                    let y = block.y + index as u32 / block.width;
                    let state = &mut footprints[(y / 8) as usize];

                    if !pixel[..3].iter().all(|value| value.is_finite()) {
                        return Err(Error::NumericalOverflow);
                    }

                    state.floor |= pixel[..3].iter().any(|value| *value <= floor);
                    state.cells += 1;

                    if [x, y % 8] == [4, 4] {
                        state.centre = DVec3::new(
                            f64::from(pixel[0]),
                            f64::from(pixel[1]),
                            f64::from(pixel[2]),
                        );
                    }
                }
                Ok(())
            },
        )?;

        if result.is_none() {
            return Ok(None);
        }

        for state in footprints {
            if state.cells != 64 {
                return Err(Error::InvalidParameter(
                    "analysis footprint lacks camera cells",
                ));
            }

            if state.floor {
                plan.samples.rejected_floor += 1;
            } else {
                plan.samples.pixels.push(state.centre);
            }
        }

        Ok(Some(plan.samples))
    }
}
