//! Canonical footprint selection and numerical eligibility before camera-light evaluation.

use super::*;
#[cfg(feature = "editor")]
mod input;
#[cfg(feature = "editor")]
pub(crate) use input::AnalysisInput;

#[derive(Clone, Copy)]
pub(crate) struct AnalysisFootprint {
    pub centre: [u32; 2],
    pub region: Rect,
}

pub(crate) struct AnalysisPlan {
    pub samples: AnalysisSamples,
    pub footprints: Vec<AnalysisFootprint>,
}

fn footprints(area: Rect, budget: usize) -> Result<Vec<AnalysisFootprint>> {
    if budget == 0 || budget > crate::roll_analysis::MAX_SAMPLES_PER_FRAME {
        return Err(Error::InvalidParameter("invalid analysis sampling budget"));
    }
    if area.width < 8 || area.height < 8 {
        return Err(Error::UnusableSample(
            "analysis area is smaller than a capture footprint",
        ));
    }
    let columns = ((budget as f64 * f64::from(area.width) / f64::from(area.height)).sqrt()
        as usize)
        .max(1)
        .min(budget)
        .min((area.width / 8) as usize);
    let rows = (budget / columns).min((area.height / 8) as usize);
    let grid = crate::geometry::SourceRaster::new(area, [columns as u32, rows as u32])?;
    let mut output = Vec::with_capacity(columns * rows);
    for gy in 0..rows {
        for gx in 0..columns {
            let centre = grid.source_pixel([gx as u32, gy as u32])?;
            output.push(AnalysisFootprint {
                centre,
                region: Rect {
                    x: centre[0] - 4,
                    y: centre[1] - 4,
                    width: 8,
                    height: 8,
                },
            });
        }
    }
    Ok(output)
}

impl AnalysisPlan {
    fn new(capacity: usize) -> Self {
        Self {
            samples: AnalysisSamples {
                pixels: Vec::with_capacity(capacity),
                ..Default::default()
            },
            footprints: Vec::with_capacity(capacity),
        }
    }
}

impl Scan {
    pub(crate) fn analysis_plan(
        &self,
        area: Rect,
        budget: usize,
        mut inside: impl FnMut(Rect) -> bool,
        cancelled: &mut impl FnMut() -> bool,
    ) -> Result<Option<AnalysisPlan>> {
        if cancelled() {
            return Ok(None);
        }
        self.validate_area(area)?;
        let grid = footprints(area, budget)?;
        let image = self.source.camera_image()?;
        let mut plan = AnalysisPlan::new(grid.len());
        for footprint in grid {
            if cancelled() {
                return Ok(None);
            }
            if !inside(footprint.region) {
                continue;
            }
            plan.samples.attempted += 1;
            let mut floor = false;
            for y in footprint.region.y..footprint.region.y + 8 {
                for x in footprint.region.x..footprint.region.x + 8 {
                    let raw = image.pixel(x, y)?;
                    if !raw.is_finite() {
                        return Err(Error::NumericalOverflow);
                    }
                    floor |= raw.min_element() <= self.source.input_floor;
                }
            }
            if floor {
                // Residual correction cannot rescue a sample already at the native floor:
                // its local positive gain is no larger than the gain used by the bound.
                plan.samples.rejected_floor += 1;
            } else {
                plan.footprints.push(footprint);
            }
        }
        Ok((!cancelled()).then_some(plan))
    }
}
