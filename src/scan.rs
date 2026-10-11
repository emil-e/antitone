use crate::flat_field::FlatField;
use crate::{
    Error,
    color::Balance,
    processing::{Calibration, PhotoSettings, Pipeline, RollSettings},
    raw::{RawFile, Rect, SensorDiagnostics},
    sampling,
};
use glam::{DMat3, DVec3};
use std::{path::Path, sync::Arc};

type Result<T> = std::result::Result<T, Error>;

pub struct Scan {
    raw: RawFile,
    pub camera_to_ap0: DMat3,
    pub area: Rect,
    pub fixed_wb: [f64; 4],
    pub input_floor: f64,
    flat_field: Option<Arc<FlatField>>,
    corrected_floor: f64,
}

impl Scan {
    pub fn open(path: &Path, fixed_wb: Option<[f64; 4]>) -> Result<Self> {
        let mut raw = RawFile::open(path)?;
        let wb = fixed_wb.unwrap_or(raw.metadata.cam_mul);
        let matrix = raw.metadata.camera_to_ap0()?;
        let area = raw.metadata.image_area()?;
        raw.develop_camera(wb)?;
        let input_floor = Self::precision_floor(matrix);
        Ok(Self {
            raw,
            camera_to_ap0: matrix,
            area,
            fixed_wb: wb,
            input_floor,
            flat_field: None,
            corrected_floor: input_floor,
        })
    }

    pub(crate) fn precision_floor(matrix: DMat3) -> f64 {
        (matrix.x_axis.abs() + matrix.y_axis.abs() + matrix.z_axis.abs()).max_element() / 65535.0
    }
    pub fn dimensions(&self) -> Result<(u32, u32)> {
        let image = self.raw.camera_image()?;
        Ok((image.width, image.height))
    }
    pub fn set_flat_field(&mut self, field: Option<Arc<FlatField>>) -> Result<()> {
        let floor = if let Some(field) = &field {
            let [x, y, width, height] = field.area();
            self.validate_area(Rect {
                x,
                y,
                width,
                height,
            })?;
            field.precision_floor(self.camera_to_ap0)?
        } else {
            self.input_floor
        };
        self.flat_field = field;
        self.corrected_floor = floor;
        Ok(())
    }

    pub fn propose_flat_field_area(&self) -> Result<Option<Rect>> {
        let a = self.area;
        let side = a.width.max(a.height).div_ceil(256);
        let size = [
            a.width.div_ceil(side) as usize,
            a.height.div_ceil(side) as usize,
        ];
        let image = self.raw.camera_image()?;
        let mut sums = vec![[0u64; 3]; size[0] * size[1]];
        let mut counts = vec![0u64; sums.len()];
        for y in 0..a.height {
            for x in 0..a.width {
                let i = (y / side) as usize * size[0] + (x / side) as usize;
                let p = image.pixel_codes(a.x + x, a.y + y)?;
                for c in 0..3 {
                    sums[i][c] += p[c] as u64;
                }
                counts[i] += 1;
            }
        }
        let values: Vec<_> = sums
            .iter()
            .zip(counts)
            .map(|(p, n)| {
                (crate::color::AP0_TO_XYZ
                    * (self.camera_to_ap0
                        * (DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64)
                            / (n as f64 * 65535.))))
                    .y
            })
            .collect();
        let center = values[(size[1] / 2) * size[0] + size[0] / 2];
        Ok(
            crate::flat_field::propose_support(&values, size, center).map(
                |[left, top, right, bottom]| Rect {
                    x: a.x + left as u32 * side,
                    y: a.y + top as u32 * side,
                    width: (right - left) as u32 * side,
                    height: (bottom - top) as u32 * side,
                },
            ),
        )
    }

    /// Estimate only inside accepted support, using area-weighted full-pixel camera statistics.
    /// Existing correction is deliberately bypassed when analysing a reference.
    pub fn estimate_flat_field(&mut self, region: Rect) -> Result<FlatField> {
        self.validate_area(region)?;
        let center = Rect {
            width: self.area.width.min(256),
            height: self.area.height.min(256),
            x: self.area.x + (self.area.width - self.area.width.min(256)) / 2,
            y: self.area.y + (self.area.height - self.area.height.min(256)) / 2,
        };
        if center.x < region.x
            || center.y < region.y
            || u64::from(center.x) + u64::from(center.width)
                > u64::from(region.x) + u64::from(region.width)
            || u64::from(center.y) + u64::from(center.height)
                > u64::from(region.y) + u64::from(region.height)
        {
            return Err(Error::UnusableSample(
                "flat-field support must contain the image-centre patch",
            ));
        }
        self.checked_region(region)?;
        let maximum = region.width.max(region.height) as f64;
        let size = [
            (region.width as f64 / maximum * 256.).round() as usize,
            (region.height as f64 / maximum * 256.).round() as usize,
        ];
        if size.iter().any(|n| *n < 7) {
            return Err(Error::UnusableSample("flat-field support is too narrow"));
        }
        let image = self.raw.camera_image()?;
        let center_camera = image.sample_mean(center, DMat3::IDENTITY)?;
        let mut means = Vec::with_capacity(size[0] * size[1]);
        for gy in 0..size[1] {
            for gx in 0..size[0] {
                let left = region.x as f64 + gx as f64 * region.width as f64 / size[0] as f64;
                let right =
                    region.x as f64 + (gx + 1) as f64 * region.width as f64 / size[0] as f64;
                let top = region.y as f64 + gy as f64 * region.height as f64 / size[1] as f64;
                let bottom =
                    region.y as f64 + (gy + 1) as f64 * region.height as f64 / size[1] as f64;
                let mut sum = DVec3::ZERO;
                for y in top.floor() as u32..bottom.ceil() as u32 {
                    for x in left.floor() as u32..right.ceil() as u32 {
                        let weight = (right.min(x as f64 + 1.) - left.max(x as f64))
                            * (bottom.min(y as f64 + 1.) - top.max(y as f64));
                        let p = image.pixel_codes(x, y)?;
                        sum += DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64) * weight;
                    }
                }
                means.push(sum / ((right - left) * (bottom - top) * 65535.));
            }
        }
        FlatField::estimate(
            [region.x, region.y, region.width, region.height],
            size,
            &means,
            self.camera_to_ap0,
            center_camera,
            [
                self.area.x as f64 + self.area.width as f64 / 2.,
                self.area.y as f64 + self.area.height as f64 / 2.,
            ],
        )
    }

    pub fn flat_field(&self) -> Option<&Arc<FlatField>> {
        self.flat_field.as_ref()
    }
    pub fn effective_floor(&self) -> f64 {
        self.corrected_floor
    }

    /// Stored calibration remains RGB-only; correction transforms its explicit precision bound.
    pub fn processing_calibration(&self, mut calibration: Calibration) -> Result<Calibration> {
        if self.flat_field.is_some() {
            calibration.input_floor *= self.corrected_floor / self.input_floor;
        }
        calibration.validate()
    }

    pub fn pixel(&self, x: u32, y: u32) -> Result<DVec3> {
        let mut camera = self.raw.camera_image()?.pixel(x, y)?;
        if let Some(field) = &self.flat_field {
            camera *= field.gain([x as f64 + 0.5, y as f64 + 0.5])?;
        }
        Ok(self.camera_to_ap0 * camera)
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
        let mean = if self.flat_field.is_some() {
            let mut sum = DVec3::ZERO;
            let mut compensation = DVec3::ZERO;
            for y in region.y..region.y + region.height {
                for x in region.x..region.x + region.width {
                    let corrected = self.pixel(x, y)? - compensation;
                    let next = sum + corrected;
                    compensation = (next - sum) - corrected;
                    sum = next;
                }
            }
            sum / (region.width as f64 * region.height as f64)
        } else {
            self.raw
                .camera_image()?
                .sample_mean(region, self.camera_to_ap0)?
        };
        if !mean.is_finite() || mean.min_element() <= self.corrected_floor {
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
                if !pixel.is_finite() || pixel.min_element() < self.corrected_floor {
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
        let calibration = self.processing_calibration(calibration)?;
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
            self.processing_calibration(calibration)?,
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
