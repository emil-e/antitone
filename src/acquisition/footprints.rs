//! Bounded original reads and destination work for filter-before-grade acquisition.
#[cfg(test)]
mod tests;

use super::{
    windows::{self, Window},
    *,
};
use std::sync::Arc;

#[derive(Clone, Copy)]
pub struct FootprintBudget {
    pub source_tile: [u32; 2],
    pub destination_tile: [u32; 2],
    pub destination_pixels: u64,
}

impl Default for FootprintBudget {
    fn default() -> Self {
        Self {
            source_tile: [512, 128],
            destination_tile: [512, 128],
            destination_pixels: 512 * 128,
        }
    }
}

impl FootprintBudget {
    fn require(self) -> Result<(), Error> {
        if self.source_tile.contains(&0)
            || self.destination_tile.contains(&0)
            || u64::from(self.destination_tile[0]) * u64::from(self.destination_tile[1])
                > self.destination_pixels
        {
            return Err(Error::InvalidParameter("invalid footprint work budget"));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FilterWindow {
    pub destination: usize,
    pub window: Window,
}

pub struct FootprintUnit {
    pub read: SourceRegion,
    pub submissions: Vec<Vec<FilterWindow>>,
}

pub struct FootprintBatch {
    plans: Vec<Arc<Plan>>,
    bounds: SourceRegion,
    budget: FootprintBudget,
}

impl FootprintBatch {
    pub fn new(plans: Vec<Arc<Plan>>, budget: FootprintBudget) -> Result<Self, Error> {
        budget.require()?;
        let first = plans
            .first()
            .ok_or(Error::InvalidParameter("empty footprint acquisition"))?;
        let source = first.output.composition.area();
        let mut bounds: Option<SourceRegion> = None;
        for plan in &plans {
            if plan.sampling != Sampling::NeutralFootprints
                || plan.output.composition.area() != source
            {
                return Err(Error::InvalidParameter(
                    "footprint batch requires matching original source grids",
                ));
            }
            let SourceRead::Originals(read) = plan.read else {
                unreachable!("neutral footprint read")
            };
            bounds = Some(match bounds {
                None => read,
                Some(previous) => {
                    let x = previous.x.min(read.x);
                    let y = previous.y.min(read.y);
                    [
                        x,
                        y,
                        (previous.x + previous.width).max(read.x + read.width) - x,
                        (previous.y + previous.height).max(read.y + read.height) - y,
                    ]
                    .into()
                }
            });
        }
        Ok(Self {
            plans,
            bounds: bounds.unwrap(),
            budget,
        })
    }

    /// No per-source-sized work list is retained; cancellation can run between every source unit.
    pub fn units(&self) -> impl Iterator<Item = Result<FootprintUnit, Error>> + '_ {
        let mut next = [0; 2];
        std::iter::from_fn(move || {
            if next[1] >= self.bounds.height {
                return None;
            }
            let read = SourceRegion {
                x: self.bounds.x + next[0],
                y: self.bounds.y + next[1],
                width: self.budget.source_tile[0].min(self.bounds.width - next[0]),
                height: self.budget.source_tile[1].min(self.bounds.height - next[1]),
            };
            next[0] = next[0].saturating_add(self.budget.source_tile[0]);
            if next[0] >= self.bounds.width {
                next[0] = 0;
                next[1] = next[1].saturating_add(self.budget.source_tile[1]);
            }
            Some(self.unit(read))
        })
    }

    fn unit(&self, read: SourceRegion) -> Result<FootprintUnit, Error> {
        let mut submissions = Vec::new();
        let mut submission = Vec::new();
        let mut pixels = 0;
        for (destination, plan) in self.plans.iter().enumerate() {
            for window in plan.source_windows(read, self.budget)? {
                let count = u64::from(window.1[0]) * u64::from(window.1[1]);
                if pixels + count > self.budget.destination_pixels {
                    submissions.push(std::mem::take(&mut submission));
                    pixels = 0;
                }
                pixels += count;
                submission.push(FilterWindow {
                    destination,
                    window,
                });
            }
        }
        if !submission.is_empty() {
            submissions.push(submission);
        }
        Ok(FootprintUnit { read, submissions })
    }
}

impl Plan {
    /// Source changes are mapped into the actual acquired raster, including real-edge extension.
    pub fn affected_windows(
        &self,
        regions: impl IntoIterator<Item = SourceRegion>,
        budget: FootprintBudget,
    ) -> Result<Vec<Window>, Error> {
        budget.require()?;
        let mut windows = Vec::new();
        for region in regions {
            windows.extend(self.source_windows(region, budget)?);
        }
        windows.sort_unstable();
        windows.dedup();
        Ok(windows)
    }

    fn source_windows(
        &self,
        region: SourceRegion,
        budget: FootprintBudget,
    ) -> Result<Vec<Window>, Error> {
        region.validate()?;
        let frame = self.processing_frame()?;
        let bounds = frame.processing_bounds()?;
        let lo = [region.x.max(bounds.x), region.y.max(bounds.y)];
        let hi = [
            (region.x + region.width).min(bounds.x + bounds.width),
            (region.y + region.height).min(bounds.y + bounds.height),
        ];
        if (0..2).any(|i| lo[i] >= hi[i]) {
            return Ok(Vec::new());
        }
        Ok(windows::bounded_by(
            frame.processing_window([lo[0], lo[1], hi[0] - lo[0], hi[1] - lo[1]].into())?,
            budget.destination_tile,
        ))
    }

    /// A subwindow of the acquired processing grid, not of the extracted output grid.
    pub fn window_frame(&self, (origin, size): Window) -> Result<FrameRaster, Error> {
        let frame = self.processing_frame()?;
        if size.contains(&0)
            || (0..2).any(|i| origin[i] > frame.output[i] || size[i] > frame.output[i] - origin[i])
        {
            return Err(Error::InvalidParameter("window outside processing raster"));
        }
        let point = |offset: [u32; 2]| {
            crate::geometry::FramePoint(std::array::from_fn(|i| {
                frame.min.0[i]
                    + f64::from(offset[i]) * (frame.max.0[i] - frame.min.0[i])
                        / f64::from(frame.output[i])
            }))
        };
        let region = FrameRaster {
            composition: frame.composition,
            min: point(origin),
            max: point([origin[0] + size[0], origin[1] + size[1]]),
            output: size,
        };
        region.processing_bounds()?;
        Ok(region)
    }
}
