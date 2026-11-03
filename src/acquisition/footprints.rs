//! Bounded repair windows on the acquired processing grid.

#[cfg(test)]
mod tests;

use super::{
    windows::{self, Window},
    *,
};
#[derive(Clone, Copy)]
pub struct WindowBudget {
    pub destination_tile: [u32; 2],
}
impl Default for WindowBudget {
    fn default() -> Self {
        Self {
            destination_tile: [512, 128],
        }
    }
}
impl WindowBudget {
    fn require(self) -> Result<(), Error> {
        if self.destination_tile.contains(&0) {
            return Err(Error::InvalidParameter(
                "invalid reconstruction window budget",
            ));
        }
        Ok(())
    }
}

impl Plan {
    /// Source changes are mapped into the actual acquired raster, including real-edge extension.
    pub fn affected_windows(
        &self,
        regions: impl IntoIterator<Item = SourceRegion>,
        budget: WindowBudget,
    ) -> Result<Vec<Window>, Error> {
        self.processing_frame()?.affected_windows(regions, budget)
    }

    /// A subwindow of the acquired processing grid, not of the extracted output grid.
    pub fn window_frame(&self, (origin, size): Window) -> Result<FrameRaster, Error> {
        let frame = self.processing_frame()?;
        if size.contains(&0)
            || (0..2).any(|i| origin[i] > frame.output[i] || size[i] > frame.output[i] - origin[i])
        {
            return Err(Error::InvalidParameter("window outside processing raster"));
        }
        frame.processing_region(origin.map(f64::from), size)
    }
}

impl FrameRaster {
    pub(crate) fn affected_windows(
        self,
        regions: impl IntoIterator<Item = SourceRegion>,
        budget: WindowBudget,
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
        self,
        region: SourceRegion,
        budget: WindowBudget,
    ) -> Result<Vec<Window>, Error> {
        region.validate()?;
        let bounds = self.processing_bounds()?;
        let lo = [region.x.max(bounds.x), region.y.max(bounds.y)];
        let hi = [
            (region.x + region.width).min(bounds.x + bounds.width),
            (region.y + region.height).min(bounds.y + bounds.height),
        ];
        if (0..2).any(|i| lo[i] >= hi[i]) {
            return Ok(Vec::new());
        }
        Ok(windows::bounded_by(
            self.processing_window([lo[0], lo[1], hi[0] - lo[0], hi[1] - lo[1]].into())?,
            budget.destination_tile,
        ))
    }
}
