use super::*;
use crate::geometry::SourceRaster;

/// Bounded original camera sites shared by statistical and spatial analysis.
/// Camera codes remain uncorrected; positions follow the full active-source grid.
pub(crate) struct AnalysisInput {
    grid: SourceRaster,
    floor: f64,
    pixels: Box<[[f32; 3]]>,
}

pub(crate) const ANALYSIS_LONG_EDGE: u32 = 1024;

/// A regular, integer-strided view into the captured source grid; no resampled pixels.
#[derive(Clone, Copy)]
pub(crate) struct AnalysisWindow {
    origin: [u32; 2],
    stride: u32,
    size: [u32; 2],
    source_width: u32,
}

impl AnalysisWindow {
    pub(crate) fn size(self) -> [usize; 2] {
        self.size.map(|v| v as usize)
    }

    pub(crate) fn index(self, local: usize) -> usize {
        let x = self.origin[0] + local as u32 % self.size[0] * self.stride;
        let y = self.origin[1] + local as u32 / self.size[0] * self.stride;
        (y * self.source_width + x) as usize
    }
}

impl AnalysisInput {
    #[cfg(test)]
    pub(crate) fn authored(grid: SourceRaster, pixels: Vec<[f32; 3]>, floor: f64) -> Self {
        assert_eq!(pixels.len(), (grid.size()[0] * grid.size()[1]) as usize);
        Self {
            grid,
            pixels: pixels.into_boxed_slice(),
            floor,
        }
    }

    pub(crate) fn capture(
        scan: &Scan,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Option<Self>> {
        let scale = (f64::from(ANALYSIS_LONG_EDGE)
            / f64::from(scan.area.width.max(scan.area.height)))
        .min(1.);
        let size = [scan.area.width, scan.area.height]
            .map(|v| (f64::from(v) * scale).round().max(1.) as u32);
        let grid = SourceRaster::new(scan.area, size)?;
        let image = scan.source.camera_image()?;
        let mut pixels = Vec::with_capacity((size[0] * size[1]) as usize);
        for y in 0..size[1] {
            if cancelled() {
                return Ok(None);
            }
            for x in 0..size[0] {
                let [sx, sy] = grid.source_pixel([x, y])?;
                let p = image.pixel_samples(sx, sy)?;
                pixels.push([p[0], p[1], p[2]]);
            }
        }
        Ok((!cancelled()).then(|| Self {
            grid,
            floor: scan.source.precision_floor,
            pixels: pixels.into_boxed_slice(),
        }))
    }

    pub(crate) fn area(&self) -> Rect {
        self.grid.region()
    }

    pub(crate) fn size(&self) -> [u32; 2] {
        self.grid.size()
    }

    pub(crate) fn position(&self, index: usize) -> Result<[u32; 2]> {
        let [w, _] = self.size();
        self.grid.source_pixel([index as u32 % w, index as u32 / w])
    }

    pub(crate) fn pixel(&self, index: usize) -> Result<[f32; 4]> {
        let p = self.pixels.get(index).ok_or(Error::InvalidParameter(
            "camera sample is outside captured source grid",
        ))?;
        Ok([p[0], p[1], p[2], 0.])
    }

    pub(crate) fn above_native_floor(&self, index: usize) -> Result<bool> {
        let p = self.pixel(index)?;
        let rgb = DVec3::new(f64::from(p[0]), f64::from(p[1]), f64::from(p[2])) / 65535.;
        if !rgb.is_finite() {
            return Err(Error::NumericalOverflow);
        }
        Ok(rgb.min_element() > self.floor)
    }

    pub(crate) fn effective_floor(&self, field: Option<&FlatField>) -> Result<f64> {
        match field {
            Some(field) => Ok(self.floor * field.bounds().1.max_element()),
            None => Ok(self.floor),
        }
    }

    /// Explicit owned payload; excludes Arc/allocator bookkeeping and process/driver memory.
    pub(crate) fn retained_bytes(&self) -> u64 {
        (size_of::<Self>() + std::mem::size_of_val(&*self.pixels)) as u64
    }

    pub(crate) fn select(
        &self,
        area: Rect,
        budget: usize,
        mut inside: impl FnMut(Rect) -> bool,
        cancelled: &mut impl FnMut() -> bool,
    ) -> Result<Option<Vec<usize>>> {
        if !self.area().contains_region(area)
            || budget == 0
            || budget > crate::roll_analysis::MAX_SAMPLES_PER_FRAME
        {
            return Err(Error::InvalidParameter("invalid camera sampling request"));
        }
        // Statistical populations mask one source-wide grid rather than rephasing
        // its strata whenever the reviewed region moves.
        let window = self.window(self.area(), u32::MAX)?;
        let [width, height] = window.size();
        let columns = ((budget as f64 * width as f64 / height as f64).sqrt() as usize)
            .max(1)
            .min(budget)
            .min(width);
        let rows = (budget / columns).min(height);
        let mut selected = Vec::with_capacity(columns * rows);
        for y in 0..rows {
            if cancelled() {
                return Ok(None);
            }
            let sy = (2 * y + 1) * height / (2 * rows);
            for x in 0..columns {
                let sx = (2 * x + 1) * width / (2 * columns);
                let index = window.index(sy * width + sx);
                let [x, y] = self.position(index)?;
                let cell = Rect {
                    x,
                    y,
                    width: 1,
                    height: 1,
                };
                if area.contains_region(cell) && inside(cell) {
                    selected.push(index);
                }
            }
        }
        Ok((!cancelled()).then_some(selected))
    }

    pub(crate) fn window(&self, area: Rect, limit: u32) -> Result<AnalysisWindow> {
        if !self.area().contains_region(area) || limit == 0 {
            return Err(Error::InvalidParameter("invalid spatial sampling window"));
        }
        let size = self.size();
        let bounds: [[u32; 2]; 2] = std::array::from_fn(|axis| {
            let low = [area.x, area.y][axis];
            let high = low + [area.width, area.height][axis];
            let at = |i| {
                self.grid
                    .source_pixel(if axis == 0 { [i, 0] } else { [0, i] })
                    .expect("captured grid position")[axis]
            };
            [
                (0..size[axis])
                    .find(|&i| at(i) >= low)
                    .unwrap_or(size[axis]),
                (0..size[axis])
                    .find(|&i| at(i) >= high)
                    .unwrap_or(size[axis]),
            ]
        });
        let extent = bounds.map(|[low, high]| high - low);
        if extent.contains(&0) {
            return Err(Error::UnusableSample(
                "picture contains no captured camera sites",
            ));
        }
        let stride = extent[0].max(extent[1]).div_ceil(limit);
        Ok(AnalysisWindow {
            origin: bounds.map(|v| v[0]),
            stride,
            size: extent.map(|v| v.div_ceil(stride)),
            source_width: size[0],
        })
    }
}

#[cfg(test)]
mod tests;
