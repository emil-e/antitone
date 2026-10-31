//! Canonical footprint selection and numerical eligibility before camera-light evaluation.

use super::*;

pub(crate) struct AnalysisFootprint {
    pub centre: [u32; 2],
    pub region: Rect,
}

pub(crate) struct AnalysisPlan {
    pub samples: AnalysisSamples,
    pub footprints: Vec<AnalysisFootprint>,
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
        let image = self.source.camera_image()?;
        let mut plan = AnalysisPlan {
            samples: AnalysisSamples {
                pixels: Vec::with_capacity(columns * rows),
                ..Default::default()
            },
            footprints: Vec::with_capacity(columns * rows),
        };

        for gy in 0..rows {
            if cancelled() {
                return Ok(None);
            }
            for gx in 0..columns {
                let centre = grid.source_pixel([gx as u32, gy as u32])?;
                let footprint = Rect {
                    x: centre[0] - 4,
                    y: centre[1] - 4,
                    width: 8,
                    height: 8,
                };
                if !inside(footprint) {
                    continue;
                }
                plan.samples.attempted += 1;
                let mut floor = false;
                for y in footprint.y..footprint.y + 8 {
                    for x in footprint.x..footprint.x + 8 {
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
                    plan.footprints.push(AnalysisFootprint {
                        centre,
                        region: footprint,
                    });
                }
            }
        }

        Ok((!cancelled()).then_some(plan))
    }
}
