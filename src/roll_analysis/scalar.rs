//! Original-pixel camera statistics for the scalar diagnostics and comparisons.

use super::{AnalysisSamples, MAX_SAMPLES_PER_FRAME};
use crate::{Error, flat_field, geometry::SourceRaster, raw::Rect, scan::DevelopedSource};

#[derive(Clone, Copy)]
struct Footprint {
    centre: [u32; 2],
    region: Rect,
}

fn footprints(area: Rect, budget: usize) -> Result<Vec<Footprint>, Error> {
    if budget == 0 || budget > MAX_SAMPLES_PER_FRAME {
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
    let grid = SourceRaster::new(area, [columns as u32, rows as u32])?;
    let mut output = Vec::with_capacity(columns * rows);
    for gy in 0..rows {
        for gx in 0..columns {
            let centre = grid.source_pixel([gx as u32, gy as u32])?;
            output.push(Footprint {
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

/// Sample original sites whose complete 8×8 footprints are admitted by `inside`.
/// The lattice depends on `area` and `budget`, not on the eligible support.
pub fn samples(
    source: &DevelopedSource,
    correction: Option<&flat_field::FlatField>,
    area: Rect,
    budget: usize,
    mut inside: impl FnMut(Rect) -> bool,
    mut cancelled: impl FnMut() -> bool,
) -> Result<Option<AnalysisSamples>, Error> {
    if cancelled() {
        return Ok(None);
    }
    if !source.area().contains_region(area) {
        return Err(Error::InvalidParameter(
            "view region is outside the active source area",
        ));
    }
    let grid = footprints(area, budget)?;
    let mut output = AnalysisSamples {
        pixels: Vec::with_capacity(grid.len()),
        ..Default::default()
    };
    let mut selected = Vec::with_capacity(grid.len());
    for footprint in grid {
        if cancelled() {
            return Ok(None);
        }
        if inside(footprint.region) {
            selected.push(footprint.centre);
        }
    }
    if cancelled() {
        return Ok(None);
    }
    output.attempted = selected.len();
    for point in selected {
        if cancelled() {
            return Ok(None);
        }
        let pixel = flat_field::sample_camera_light(source, correction, point)?;
        if !pixel.is_finite() {
            return Err(Error::NumericalOverflow);
        }
        output.pixels.push(pixel);
    }
    Ok((!cancelled()).then_some(output))
}
