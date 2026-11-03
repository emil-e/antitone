use super::*;

struct Patch {
    footprint: AnalysisFootprint,
    pixels: [[f32; 4]; 64],
    floor: bool,
    nonfinite: bool,
}

/// Original camera-code patches, independent of native leases, areas and measured correction.
pub(crate) struct AnalysisInput {
    area: Rect,
    budget: usize,
    floor: f64,
    patches: Box<[Patch]>,
}

impl AnalysisInput {
    pub(crate) fn capture(
        scan: &Scan,
        budget: usize,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Option<Self>> {
        let grid = if scan.area.width < 8 || scan.area.height < 8 {
            vec![]
        } else {
            footprints(scan.area, budget)?
        };
        let image = scan.source.camera_image()?;
        let mut patches = Vec::with_capacity(grid.len());
        for footprint in grid {
            if cancelled() {
                return Ok(None);
            }
            let mut pixels = [[0.; 4]; 64];
            let mut floor = false;
            let mut nonfinite = false;
            for y in 0..8 {
                for x in 0..8 {
                    let pixel =
                        image.pixel_samples(footprint.region.x + x, footprint.region.y + y)?;
                    // Developed::pixel uses precisely these packed camera codes, widened before
                    // division. Preserve its f64 floor decision rather than an f32 comparison.
                    let raw = DVec3::new(
                        f64::from(pixel[0]),
                        f64::from(pixel[1]),
                        f64::from(pixel[2]),
                    ) / 65535.;
                    nonfinite |= !raw.is_finite();
                    floor |= raw.min_element() <= scan.source.input_floor;
                    pixels[(y * 8 + x) as usize] = pixel;
                }
            }
            patches.push(Patch {
                footprint,
                pixels,
                floor,
                nonfinite,
            });
        }
        Ok((!cancelled()).then(|| Self {
            area: scan.area,
            budget,
            floor: scan.source.input_floor,
            patches: patches.into_boxed_slice(),
        }))
    }

    pub(crate) fn area(&self) -> Rect {
        self.area
    }

    pub(crate) fn effective_floor(&self, field: Option<&FlatField>) -> Result<f64> {
        match field {
            Some(field) => {
                if !self.area.contains_region(Rect::from(field.area())) {
                    return Err(Error::InvalidParameter(
                        "view region is outside the active source area",
                    ));
                }
                Ok(self.floor * field.bounds().1.max_element())
            }
            None => Ok(self.floor),
        }
    }

    /// Explicit owned payload; excludes Arc/allocator bookkeeping and process/driver memory.
    pub(crate) fn retained_bytes(&self) -> u64 {
        (size_of::<Self>() + std::mem::size_of_val(&*self.patches)) as u64
    }

    pub(crate) fn patch_index(&self, origin: [u32; 2]) -> Result<usize> {
        self.patches
            .binary_search_by_key(&(origin[1], origin[0]), |p| {
                (p.footprint.region.y, p.footprint.region.x)
            })
            .map_err(|_| {
                Error::InvalidParameter("camera footprint is not in retained sampling grid")
            })
    }

    pub(crate) fn pixels(&self, index: usize) -> &[[f32; 4]; 64] {
        &self.patches[index].pixels
    }

    pub(crate) fn plan(
        &self,
        area: Rect,
        budget: usize,
        mut inside: impl FnMut(Rect) -> bool,
        cancelled: &mut impl FnMut() -> bool,
    ) -> Result<Option<AnalysisPlan>> {
        if cancelled() {
            return Ok(None);
        }
        if area != self.area || budget != self.budget {
            return Err(Error::InvalidParameter(
                "retained sampling grid does not match request",
            ));
        }
        if area.width < 8 || area.height < 8 {
            return Err(Error::UnusableSample(
                "analysis area is smaller than a capture footprint",
            ));
        }
        let mut plan = AnalysisPlan::new(self.patches.len());
        for patch in &self.patches {
            if cancelled() {
                return Ok(None);
            }
            if !inside(patch.footprint.region) {
                continue;
            }
            plan.samples.attempted += 1;
            if patch.nonfinite {
                return Err(Error::NumericalOverflow);
            }
            if patch.floor {
                plan.samples.rejected_floor += 1;
            } else {
                plan.footprints.push(patch.footprint);
            }
        }
        Ok((!cancelled()).then_some(plan))
    }
}

#[cfg(test)]
mod tests;
