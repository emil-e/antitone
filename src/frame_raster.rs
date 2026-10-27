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

    pub(crate) fn processing_bounds(self) -> Result<SourceRegion, Error> {
        self.composition
            .coverage(self.min, self.max)?
            .clipped_bounds(self.composition.area())
    }

    /// A source unit also supplies constant extension beyond the actual source edges.
    #[cfg(feature = "editor")]
    pub(crate) fn processing_window(
        self,
        region: SourceRegion,
    ) -> Result<crate::resample::windows::Window, Error> {
        let bounds = self.processing_bounds()?;
        let area = self.composition.area();
        let points = [
            [0., 0.],
            [f64::from(self.output[0]), 0.],
            self.output.map(f64::from),
            [0., f64::from(self.output[1])],
        ]
        .map(|p| self.source_point(p).0);
        let origin = [bounds.x, bounds.y].map(f64::from);
        let mut low = [region.x, region.y].map(f64::from);
        let mut high = [region.x + region.width, region.y + region.height].map(f64::from);
        let source_low = [area.x, area.y].map(f64::from);
        let source_high = [area.x + area.width, area.y + area.height].map(f64::from);
        for axis in 0..2 {
            if low[axis] == source_low[axis] {
                low[axis] =
                    low[axis].min(points.iter().map(|p| p[axis]).fold(f64::INFINITY, f64::min));
            }
            if high[axis] == source_high[axis] {
                high[axis] = high[axis].max(
                    points
                        .iter()
                        .map(|p| p[axis])
                        .fold(f64::NEG_INFINITY, f64::max),
                );
            }
            low[axis] -= origin[axis];
            high[axis] -= origin[axis];
        }
        Ok(crate::resample::windows::affine_output_rect(
            self.affine_in(bounds)?,
            self.output,
            low,
            high,
        ))
    }

    /// Source coordinates relative to the iteration bounds, per destination pixel edge.
    pub fn affine(self) -> Result<[[f32; 2]; 3], Error> {
        self.affine_in(self.bounds()?)
    }

    pub(crate) fn affine_in(self, bounds: SourceRegion) -> Result<[[f32; 2]; 3], Error> {
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
