//! Destination raster footprints; resolution is separate from accepted composition.
use crate::{
    Error,
    coverage::Coverage,
    geometry::{Composition, FramePoint, SourcePoint, SourceRegion},
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameRaster {
    pub composition: Composition,
    pub min: FramePoint,
    pub max: FramePoint,
    pub output: [u32; 2],
}

impl FrameRaster {
    pub fn new(
        composition: Composition,
        min: FramePoint,
        max: FramePoint,
        output: [u32; 2],
    ) -> Result<Self, Error> {
        let size = composition.size();
        if output.contains(&0)
            || (0..2).any(|i| {
                !min.0[i].is_finite()
                    || !max.0[i].is_finite()
                    || min.0[i] < 0.
                    || max.0[i] > f64::from(size[i])
                    || min.0[i] >= max.0[i]
            })
        {
            return Err(Error::InvalidParameter("invalid frame raster"));
        }
        let raster = Self {
            composition,
            min,
            max,
            output,
        };
        raster.bounds()?;
        Ok(raster)
    }

    pub fn full(composition: Composition) -> Result<Self, Error> {
        Self::new(
            composition,
            FramePoint([0., 0.]),
            FramePoint(composition.size().map(f64::from)),
            composition.size(),
        )
    }

    pub fn source_point(self, point: [f64; 2]) -> SourcePoint {
        self.composition
            .frame_to_source(FramePoint(std::array::from_fn(|i| {
                self.min.0[i]
                    + point[i] * (self.max.0[i] - self.min.0[i]) / f64::from(self.output[i])
            })))
    }

    pub fn footprint(self, pixel: [u32; 2]) -> Result<Coverage, Error> {
        if (0..2).any(|i| pixel[i] >= self.output[i]) {
            return Err(Error::InvalidParameter("pixel outside frame raster"));
        }
        let [x, y] = pixel.map(f64::from);
        Coverage::new(
            [[x, y], [x + 1., y], [x + 1., y + 1.], [x, y + 1.]].map(|p| self.source_point(p)),
        )
    }

    pub fn bounds(self) -> Result<SourceRegion, Error> {
        self.composition
            .coverage(self.min, self.max)?
            .bounds(self.composition.area())
    }

    /// Source coordinates relative to the iteration bounds, per destination pixel edge.
    pub fn affine(self) -> Result<[[f32; 2]; 3], Error> {
        let bounds = self.bounds()?;
        let origin = self.source_point([0., 0.]).0;
        let x = self.source_point([1., 0.]).0;
        let y = self.source_point([0., 1.]).0;
        Ok([
            [
                origin[0] - f64::from(bounds.x),
                origin[1] - f64::from(bounds.y),
            ]
            .map(|v| v as f32),
            std::array::from_fn(|i| (x[i] - origin[i]) as f32),
            std::array::from_fn(|i| (y[i] - origin[i]) as f32),
        ])
    }
}
