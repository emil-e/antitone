use crate::{
    Error,
    color::Balance,
    processing::{Calibration, PhotoSettings, Pipeline, RollSettings},
    raw::{RawFile, Rect, SensorDiagnostics},
    sampling,
};
use glam::{DMat3, DVec3};
use std::path::Path;

type Result<T> = std::result::Result<T, Error>;

pub struct Scan {
    raw: RawFile,
    pub camera_to_ap0: DMat3,
    pub area: Rect,
    pub fixed_wb: [f64; 4],
    pub input_floor: f64,
}

impl Scan {
    pub fn open(path: &Path, fixed_wb: Option<[f64; 4]>) -> Result<Self> {
        let mut raw = RawFile::open(path)?;
        let wb = fixed_wb.unwrap_or(raw.metadata.cam_mul);
        let matrix = raw.metadata.camera_to_ap0()?;
        let area = raw.metadata.image_area()?;
        raw.develop_camera(wb)?;
        let input_floor = (matrix.x_axis.abs() + matrix.y_axis.abs() + matrix.z_axis.abs())
            .max_element()
            / 65535.0;
        Ok(Self {
            raw,
            camera_to_ap0: matrix,
            area,
            fixed_wb: wb,
            input_floor,
        })
    }

    pub fn dimensions(&self) -> Result<(u32, u32)> {
        let image = self.raw.camera_image()?;
        Ok((image.width, image.height))
    }
    pub fn pixel(&self, x: u32, y: u32) -> Result<DVec3> {
        Ok(self.camera_to_ap0 * self.raw.camera_image()?.pixel(x, y)?)
    }

    fn checked_region(&mut self, region: Rect) -> Result<SensorDiagnostics> {
        let diagnostics = self.raw.region_sensor_diagnostics(region)?;
        if diagnostics.at_white.iter().any(|v| *v > 0) {
            return Err(Error::UnusableSample(
                "patch contains sensor values at/above white",
            ));
        }
        if diagnostics.below_black.iter().any(|v| *v > 0) {
            return Err(Error::UnusableSample(
                "patch contains sensor values at/below black",
            ));
        }
        Ok(diagnostics)
    }

    pub fn reference(&mut self, region: Rect) -> Result<DVec3> {
        self.checked_region(region)?;
        let mean = self
            .raw
            .camera_image()?
            .sample_mean(region, self.camera_to_ap0)?;
        if !mean.is_finite() || mean.min_element() <= self.input_floor {
            return Err(Error::UnusableSample(
                "reference mean is at/below the scan precision floor",
            ));
        }
        Ok(mean)
    }

    fn patch(&mut self, region: Rect) -> Result<Vec<DVec3>> {
        self.checked_region(region)?;
        let mut pixels = Vec::with_capacity(region.width as usize * region.height as usize);
        for y in region.y..region.y + region.height {
            for x in region.x..region.x + region.width {
                let pixel = self.pixel(x, y)?;
                if !pixel.is_finite() || pixel.min_element() < self.input_floor {
                    return Err(Error::UnusableSample(
                        "patch contains nonfinite or floored scan inputs",
                    ));
                }
                pixels.push(pixel);
            }
        }
        Ok(pixels)
    }

    pub fn roll_neutral(
        &mut self,
        region: Rect,
        calibration: Calibration,
        roll: RollSettings,
    ) -> Result<Balance> {
        sampling::roll_neutral(calibration, roll, &self.patch(region)?)
    }

    pub fn photo_neutral(
        &mut self,
        region: Rect,
        calibration: Calibration,
        roll: RollSettings,
        photo: PhotoSettings,
    ) -> Result<Balance> {
        let pipeline = Pipeline::new(
            calibration,
            roll,
            PhotoSettings {
                wb: Balance::default(),
                ..photo
            },
        )?;
        let mut pixels = self.patch(region)?;
        for pixel in &mut pixels {
            let result = pipeline.apply(*pixel)?;
            if result.floored.into_iter().any(|v| v) {
                return Err(Error::UnusableSample(
                    "WB patch contains inputs below the calibration floor",
                ));
            }
            *pixel = result.rgb;
        }
        sampling::photo_neutral(&pixels)
    }

    pub fn tile_region(
        &self,
        area: Rect,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    ) -> Result<Vec<[u16; 4]>> {
        self.validate_area(area)?;
        if width == 0 || height == 0 || x >= area.width || y >= area.height {
            return Err(Error::InvalidParameter("invalid source tile"));
        }
        let image = self.raw.camera_image()?;
        let mut pixels = vec![[0; 4]; width as usize * height as usize];
        for dy in 0..height.min(area.height - y) {
            for dx in 0..width.min(area.width - x) {
                pixels[(dy * width + dx) as usize] =
                    image.pixel_codes(area.x + x + dx, area.y + y + dy)?;
            }
        }
        Ok(pixels)
    }

    pub fn validate_area(&self, area: Rect) -> Result<()> {
        if area.width == 0
            || area.height == 0
            || area.x < self.area.x
            || area.y < self.area.y
            || u64::from(area.x) + u64::from(area.width)
                > u64::from(self.area.x) + u64::from(self.area.width)
            || u64::from(area.y) + u64::from(area.height)
                > u64::from(self.area.y) + u64::from(self.area.height)
        {
            return Err(Error::InvalidParameter(
                "view region is outside the active source area",
            ));
        }
        Ok(())
    }

    pub fn preview_dimensions(&self, maximum: u32) -> Result<(u32, u32)> {
        if maximum == 0 {
            return Err(Error::InvalidParameter("preview size must be positive"));
        }
        let scale =
            (f64::from(maximum) / f64::from(self.area.width.max(self.area.height))).min(1.0);
        let width = (f64::from(self.area.width) * scale).round().max(1.0) as u32;
        let height = (f64::from(self.area.height) * scale).round().max(1.0) as u32;
        Ok((width, height))
    }

    /// Unconverted camera RGB16 samples at display-grid centers for the provisional preview.
    pub fn preview_source(&self, maximum: u32) -> Result<(u32, u32, Vec<[u16; 4]>)> {
        let (width, height) = self.preview_dimensions(maximum)?;
        let image = self.raw.camera_image()?;
        let mut pixels = Vec::with_capacity(width as usize * height as usize);
        for y in 0..height {
            for x in 0..width {
                let sx = self.area.x
                    + ((2 * u64::from(x) + 1) * u64::from(self.area.width) / (2 * u64::from(width)))
                        as u32;
                let sy = self.area.y
                    + ((2 * u64::from(y) + 1) * u64::from(self.area.height)
                        / (2 * u64::from(height))) as u32;
                pixels.push(image.pixel_codes(sx, sy)?);
            }
        }
        Ok((width, height, pixels))
    }
}
