//! Spatial dependencies of processing, independent of the stages requesting them.
#[cfg(test)]
mod tests;
use crate::{
    Error,
    frame_raster::FrameRaster,
    geometry::{FramePoint, SourceRegion},
};

/// Symmetric input margins, in pixels of the declared raster or source grid.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Support(pub [u32; 2]);

impl Support {
    pub const NONE: Self = Self([0, 0]);

    pub fn then(self, next: Self) -> Result<Self, Error> {
        Ok(Self([
            self.0[0]
                .checked_add(next.0[0])
                .ok_or(Error::InvalidParameter("processing support overflow"))?,
            self.0[1]
                .checked_add(next.0[1])
                .ok_or(Error::InvalidParameter("processing support overflow"))?,
        ]))
    }

    pub fn expand(self, output: SourceRegion, source: SourceRegion) -> SourceRegion {
        let x = output.x.saturating_sub(self.0[0]).max(source.x);
        let y = output.y.saturating_sub(self.0[1]).max(source.y);
        let right = (output.x + output.width)
            .saturating_add(self.0[0])
            .min(source.x + source.width);
        let bottom = (output.y + output.height)
            .saturating_add(self.0[1])
            .min(source.y + source.height);
        SourceRegion {
            x,
            y,
            width: right - x,
            height: bottom - y,
        }
    }

    /// Extend beyond accepted composition while preserving pitch and coordinate basis.
    /// Acquisition resolves actual source bounds and their constant edge extension.
    pub fn raster(self, output: FrameRaster) -> Result<FrameRaster, Error> {
        if self == Self::NONE {
            return Ok(output);
        }
        let pitch: [f64; 2] = std::array::from_fn(|i| {
            (output.max.0[i] - output.min.0[i]) / f64::from(output.output[i])
        });
        let mut size = [0; 2];
        for (i, value) in size.iter_mut().enumerate() {
            *value = self.0[i]
                .checked_mul(2)
                .and_then(|margin| output.output[i].checked_add(margin))
                .ok_or(Error::InvalidParameter("processing raster overflow"))?;
        }
        let raster = FrameRaster {
            composition: output.composition,
            min: FramePoint(std::array::from_fn(|i| {
                output.min.0[i] - f64::from(self.0[i]) * pitch[i]
            })),
            max: FramePoint(std::array::from_fn(|i| {
                output.max.0[i] + f64::from(self.0[i]) * pitch[i]
            })),
            output: size,
        };
        raster.processing_bounds()?;
        Ok(raster)
    }
}
