//! Regular original-centre evidence for spatial GI, with source-coordinate correction.
use super::*;
use crate::{geometry::SourceRaster, roll_analysis::SpatialSamples};

#[cfg(test)]
mod tests;

struct CameraSites {
    source: Arc<DevelopedSource>,
    positions: Arc<[[u32; 2]]>,
    domain: evaluation::Domain,
}

impl evaluation::Source for CameraSites {
    fn domain(&self) -> evaluation::Domain {
        self.domain
    }
    fn read(&self, region: evaluation::CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        let region = region.source()?;
        let image = self.source.camera_image()?;
        let mut pixels = Vec::with_capacity(region.height as usize);
        for index in region.y..region.y + region.height {
            let [x, y] = self.positions[index as usize];
            pixels.push(image.pixel_samples(x, y)?);
        }
        ProcessingSource::validate_continuous(&pixels)?;
        Ok(pixels)
    }
}

impl CameraSampler<'_> {
    pub(crate) fn spatial(
        &self,
        area: crate::geometry::SourceRegion,
        mut inside: impl FnMut(crate::geometry::SourceRegion) -> bool,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Option<SpatialSamples>, Error> {
        let Input::Native(scan) = &self.input else {
            return Err(Error::InvalidParameter(
                "GI spatial capture requires original camera samples",
            ));
        };
        scan.validate_area(area)?;
        if cancelled() {
            return Ok(None);
        }
        let scale = (crate::roll_analysis::GRAYNESS_LONG_EDGE as f64
            / f64::from(area.width.max(area.height)))
        .min(1.);
        let size = [area.width, area.height].map(|v| (f64::from(v) * scale).round().max(1.) as u32);
        let grid = SourceRaster::new(area, size)?;
        let image = scan.source().camera_image()?;
        let count = (size[0] * size[1]) as usize;
        let mut positions = Vec::new();
        let mut indices = Vec::new();
        for y in 0..size[1] {
            if cancelled() {
                return Ok(None);
            }
            for x in 0..size[0] {
                let [sx, sy] = grid.source_pixel([x, y])?;
                if sx < 4 || sy < 4 {
                    continue;
                }
                let footprint = crate::geometry::SourceRegion {
                    x: sx - 4,
                    y: sy - 4,
                    width: 8,
                    height: 8,
                };
                if !scan.area.contains_region(footprint) || !inside(footprint) {
                    continue;
                }
                let mut at_floor = false;
                for yy in sy - 4..sy + 4 {
                    for xx in sx - 4..sx + 4 {
                        let p = image.pixel(xx, yy)?;
                        if !p.is_finite() {
                            return Err(Error::NumericalOverflow);
                        }
                        at_floor |= p.min_element() <= scan.source().input_floor();
                    }
                }
                if !at_floor {
                    indices.push((y * size[0] + x) as usize);
                    positions.push([sx, sy]);
                }
            }
        }
        let mut camera = vec![DVec3::ZERO; count];
        let mut valid = vec![false; count];
        if positions.is_empty() {
            return Ok(Some(SpatialSamples::new(
                size.map(|v| v as usize),
                camera,
                valid,
            )?));
        }
        if self.field.is_some() {
            // A corrected centre alone cannot establish precision eligibility: a
            // neighbouring source cell can fall below the common correction floor.
            let origins = positions.iter().map(|p| [p[0] - 4, p[1] - 4]).collect();
            let Some(footprints) = self.read_footprints(origins, &mut cancelled)? else {
                return Ok(None);
            };
            for (index, footprint) in indices.into_iter().zip(footprints) {
                if !footprint.floor {
                    camera[index] = footprint.centre;
                    valid[index] = true;
                }
            }
            return Ok(Some(SpatialSamples::new(
                size.map(|v| v as usize),
                camera,
                valid,
            )?));
        }
        let domain = evaluation::Domain {
            bounds: Some(evaluation::CellRegion::new(
                [0, 0],
                [1, positions.len() as u32],
            )),
            grid: evaluation::Grid::axis_aligned([0.; 2], [1.; 2]),
            meaning: evaluation::Meaning::CameraCodes,
        };
        let positions: Arc<[[u32; 2]]> = positions.into();
        let codes = evaluation::source(Arc::new(CameraSites {
            source: scan.source().clone(),
            positions: positions.clone(),
            domain,
        }));
        let corrected = crate::gpu::photograph::PhotographGraph::camera_samples(
            codes,
            self.field.clone(),
            &positions,
        )?;
        let request = self.context.request(
            corrected.clone(),
            evaluation::Region {
                bounds: corrected.domain().finite()?,
                grid: corrected.domain().grid,
            },
            evaluation::Budget {
                tile: [1, 8192],
                scratch_bytes: 32 << 20,
            },
        )?;
        let interrupted = Cell::new(false);
        let cancelled = RefCell::new(&mut cancelled);
        let read = self.context.read(
            self.queue,
            self.submissions,
            &request,
            || interrupted.get() || (**cancelled.borrow_mut())(),
            |block, pixels| {
                let block = block.source()?;
                if block.x != 0 || block.width != 1 {
                    return Err(Error::InvalidParameter(
                        "GI camera response changed raster layout",
                    ));
                }
                for (offset, p) in pixels.iter().enumerate() {
                    if (**cancelled.borrow_mut())() {
                        interrupted.set(true);
                        return Ok(());
                    }
                    let index = indices[block.y as usize + offset];
                    let p = DVec3::new(f64::from(p[0]), f64::from(p[1]), f64::from(p[2]));
                    if !p.is_finite() {
                        return Err(Error::NumericalOverflow);
                    }
                    camera[index] = p;
                    valid[index] = p.min_element() > self.floor;
                }
                Ok(())
            },
        )?;
        if read.is_none() || interrupted.get() {
            return Ok(None);
        }
        Ok(Some(SpatialSamples::new(
            size.map(|v| v as usize),
            camera,
            valid,
        )?))
    }
}
