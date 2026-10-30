//! Shared pointwise photographic response over the captured camera samples used by roll fitting.

use super::{SubmissionGate, evaluation};
use crate::{Error, processing::NegativeInput, roll_analysis::ForwardModel};
use glam::DVec3;
use std::{
    cell::{Cell, RefCell},
    sync::Arc,
};

mod sampling;
pub(crate) use sampling::CameraSampler;

#[cfg(test)]
mod tests;

pub(crate) struct ResponseModel<'a> {
    context: &'a evaluation::Context,
    queue: &'a wgpu::Queue,
    submissions: &'a SubmissionGate,
}

impl<'a> ResponseModel<'a> {
    pub(crate) fn new(
        context: &'a evaluation::Context,
        gpu: (&'a wgpu::Queue, &'a SubmissionGate),
    ) -> Self {
        Self {
            context,
            queue: gpu.0,
            submissions: gpu.1,
        }
    }
}

struct CameraSamples {
    domain: evaluation::Domain,
    points: Arc<[DVec3]>,
}

impl evaluation::Source for CameraSamples {
    fn domain(&self) -> evaluation::Domain {
        self.domain
    }

    fn read(&self, region: crate::geometry::SourceRegion) -> Result<Vec<[f32; 4]>, Error> {
        let mut pixels = Vec::with_capacity((region.width * region.height) as usize);
        for y in region.y..region.y + region.height {
            for x in region.x..region.x + region.width {
                let index =
                    (u64::from(y) * u64::from(self.domain.bounds.width) + u64::from(x)) as usize;
                let point = self.points.get(index).copied().unwrap_or_default();
                let rgb = point.as_vec3();
                // These vectors already include residual correction; the developed-source
                // ceiling no longer describes them. Require their actual f32 representation.
                if !rgb.is_finite() || point.min_element() < 0. {
                    return Err(Error::InvalidParameter(
                        "analysis corrected camera samples exceed the f32 domain",
                    ));
                }
                pixels.push([rgb.x, rgb.y, rgb.z, 0.]);
            }
        }
        Ok(pixels)
    }
}

impl ForwardModel for ResponseModel<'_> {
    fn read(
        &mut self,
        negative: NegativeInput,
        points: &[DVec3],
        cancelled: &mut dyn FnMut() -> bool,
        consume: &mut dyn FnMut(usize, DVec3) -> Result<(), Error>,
    ) -> Result<bool, Error> {
        if cancelled() {
            return Ok(false);
        }
        if points.is_empty() {
            return Ok(true);
        }
        let width = points.len().min(4096) as u32;
        let height = u32::try_from(points.len().div_ceil(width as usize))
            .map_err(|_| Error::InvalidParameter("analysis response batch is too large"))?;
        // This is an index grid over captured camera vectors, not a reconstructed photograph.
        // Only pointwise interpretation runs here; physical source support was selected earlier.
        let domain = evaluation::Domain {
            bounds: [0, 0, width, height].into(),
            grid: evaluation::Grid::axis_aligned([0.; 2], [1.; 2]),
            meaning: evaluation::Meaning::Camera,
        };
        let camera = evaluation::source(Arc::new(CameraSamples {
            domain,
            points: points.to_vec().into(),
        }));
        let readout = evaluation::CameraReadout::normalized(camera, negative)?;
        let request = self.context.request(
            readout.image.clone(),
            evaluation::Region {
                bounds: domain.bounds,
                grid: domain.grid,
            },
            evaluation::Budget {
                tile: [width, 16],
                scratch_bytes: 32 << 20,
            },
        )?;
        let cancelled = RefCell::new(cancelled);
        let interrupted = Cell::new(false);

        let result = self.context.read(
            self.queue,
            self.submissions,
            &request,
            || {
                let mut predicate = cancelled.borrow_mut();
                interrupted.get() || (**predicate)()
            },
            |block, pixels| {
                for (index, pixel) in pixels.iter().enumerate() {
                    let mut predicate = cancelled.borrow_mut();
                    if (**predicate)() {
                        interrupted.set(true);
                        return Ok(());
                    }
                    drop(predicate);
                    let x = block.x + index as u32 % block.width;
                    let y = block.y + index as u32 / block.width;
                    let index = (u64::from(y) * u64::from(width) + u64::from(x)) as usize;
                    if index >= points.len() {
                        continue;
                    }
                    if !pixel.iter().all(|value| value.is_finite()) || pixel[3] < 0. {
                        return Err(Error::NumericalOverflow);
                    }
                    let positive = DVec3::new(
                        f64::from(pixel[0]),
                        f64::from(pixel[1]),
                        f64::from(pixel[2]),
                    ) * readout.decode_gain;
                    consume(index, positive)?;
                }
                Ok(())
            },
        )?;

        Ok(result.is_some())
    }
}
