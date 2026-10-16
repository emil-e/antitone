use crate::flat_field::FlatField;
use crate::{
    Error,
    color::Balance,
    coverage::Coverage,
    geometry::SourceRaster,
    processing::{Calibration, PhotoSettings, Pipeline, RollSettings},
    raw::{Developed, FrozenRaw, Metadata, RawFile, Rect, SensorDiagnostics},
    sampling,
};
use glam::{DMat3, DVec3};
use std::{path::Path, sync::Arc};

type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Default)]
pub struct AnalysisSamples {
    pub pixels: Vec<DVec3>,
    pub attempted: usize,
    pub rejected_capture: usize,
    pub rejected_development: usize,
    pub rejected_floor: usize,
}

/// Immutable native development shared by independent Scan consumers.
///
/// Neither the native owner nor mutation APIs are exposed:
/// ```compile_fail
/// use antitone::scan::DevelopedSource;
/// fn redevelop(source: &mut DevelopedSource) {
///     source.raw.develop_camera([1.0; 4]);
/// }
/// ```
/// Image borrows cannot outlive their source lease:
/// ```compile_fail
/// use std::sync::Arc;
/// use antitone::scan::DevelopedSource;
/// fn release_while_reading(source: Arc<DevelopedSource>) {
///     let image = source.camera_image().unwrap();
///     drop(source);
///     image.pixel_codes(0, 0).unwrap();
/// }
/// ```
pub struct DevelopedSource {
    raw: FrozenRaw,
    camera_to_ap1: DMat3,
    camera_model: String,
    area: Rect,
    fixed_wb: [f64; 4],
    input_floor: f64,
}

impl DevelopedSource {
    fn from_raw(mut raw: RawFile, fixed_wb: Option<[f64; 4]>) -> Result<Self> {
        let wb = fixed_wb.unwrap_or_else(|| raw.metadata.as_shot_wb());
        let matrix = raw.metadata.camera_to_ap1()?;
        let camera_model = raw.metadata.camera_model();
        let area = raw.metadata.image_area()?;
        raw.develop_camera(wb)?;
        Ok(Self {
            raw: raw.freeze()?,
            camera_to_ap1: matrix,
            camera_model,
            area,
            fixed_wb: wb,
            input_floor: crate::processing::CAMERA_INPUT_FLOOR,
        })
    }

    /// Explicit native allocations plus this Rust payload/string capacity, charged once.
    /// Excludes SDK-private/stdlib heap, Arc/store bookkeeping and allocator-held VM;
    /// this is not total native heap or process footprint. None disables cache admission.
    pub fn retained_bytes(&self) -> Option<u64> {
        self.raw
            .retained_bytes()?
            .checked_add(size_of::<Self>() as u64)?
            .checked_add(self.camera_model.capacity() as u64)
    }

    pub fn metadata(&self) -> Metadata {
        self.raw.metadata()
    }

    pub fn camera_to_ap1(&self) -> DMat3 {
        self.camera_to_ap1
    }

    pub fn camera_model(&self) -> &str {
        &self.camera_model
    }

    pub fn area(&self) -> Rect {
        self.area
    }

    pub fn fixed_wb(&self) -> [f64; 4] {
        self.fixed_wb
    }

    pub fn input_floor(&self) -> f64 {
        self.input_floor
    }

    pub fn camera_image(&self) -> Result<Developed<'_>> {
        self.raw.camera_image()
    }

    pub fn sensor_diagnostics(&self) -> Result<SensorDiagnostics> {
        self.raw.sensor_diagnostics()
    }

    pub fn region_sensor_diagnostics(&self, area: Rect) -> Result<SensorDiagnostics> {
        self.raw.region_sensor_diagnostics(area)
    }
}

/// Uncorrected camera-RGB thumbnail data; independent of native retention and rendering.
/// The production proxy has a 256-pixel maximum edge, without upscaling.
pub struct CameraProxy {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) pixels: Vec<[u16; 4]>,
    pub(crate) matrix: DMat3,
}

impl CameraProxy {
    pub const MAX_EDGE: u32 = 256;

    pub fn from_scan(scan: &Scan) -> Result<Self> {
        let (width, height, pixels) = scan.preview_source(Self::MAX_EDGE)?;
        Ok(Self {
            width,
            height,
            pixels,
            matrix: scan.camera_to_ap1,
        })
    }

    pub fn dimensions(&self) -> [u32; 2] {
        [self.width, self.height]
    }

    pub fn retained_bytes(&self) -> u64 {
        (self.pixels.capacity() * size_of::<[u16; 4]>() + size_of::<Self>()) as u64
    }
}

pub struct Scan {
    source: Arc<DevelopedSource>,
    pub camera_to_ap1: DMat3,
    pub camera_model: String,
    pub area: Rect,
    pub fixed_wb: [f64; 4],
    pub input_floor: f64,
    flat_field: Option<Arc<FlatField>>,
    corrected_floor: f64,
}

impl Scan {
    pub fn open(path: &Path, fixed_wb: Option<[f64; 4]>) -> Result<Self> {
        Self::open_with_priority(path, fixed_wb, crate::raw::DevelopmentPriority::Foreground)
    }

    pub fn open_with_priority(
        path: &Path,
        fixed_wb: Option<[f64; 4]>,
        priority: crate::raw::DevelopmentPriority,
    ) -> Result<Self> {
        let mut raw = RawFile::open(path)?;
        raw.set_development_priority(priority);
        Self::from_raw(raw, fixed_wb)
    }

    pub(crate) fn from_raw(raw: RawFile, fixed_wb: Option<[f64; 4]>) -> Result<Self> {
        Ok(Self::from_source(Arc::new(DevelopedSource::from_raw(
            raw, fixed_wb,
        )?)))
    }

    /// Starts an independent, uncorrected consumer without developing or copying RGB.
    pub fn from_source(source: Arc<DevelopedSource>) -> Self {
        Self {
            camera_to_ap1: source.camera_to_ap1,
            camera_model: source.camera_model.clone(),
            area: source.area,
            fixed_wb: source.fixed_wb,
            input_floor: source.input_floor,
            corrected_floor: source.input_floor,
            flat_field: None,
            source,
        }
    }

    pub fn source(&self) -> &Arc<DevelopedSource> {
        &self.source
    }

    pub fn dimensions(&self) -> Result<(u32, u32)> {
        let image = self.source.camera_image()?;
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
            field.precision_floor()?
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
        let image = self.source.camera_image()?;
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
                (crate::color::AP1_TO_XYZ
                    * (self.camera_to_ap1
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
        let image = self.source.camera_image()?;
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
            self.camera_to_ap1,
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
        if calibration.input_floor != self.input_floor {
            return Err(Error::InvalidParameter(
                "scan calibration requires the native-code base floor",
            ));
        }
        calibration.input_floor = self.corrected_floor;
        calibration.validate()
    }

    pub fn pixel(&self, x: u32, y: u32) -> Result<DVec3> {
        let mut camera = self.source.camera_image()?.pixel(x, y)?;
        if let Some(field) = &self.flat_field {
            camera *= field.gain([x as f64 + 0.5, y as f64 + 0.5])?;
        }
        Ok(camera)
    }

    fn checked_region(&mut self, region: Rect) -> Result<SensorDiagnostics> {
        let diagnostics = self.source.region_sensor_diagnostics(region)?;
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

    /// Bounded original-pixel centres, checked in local 8×8 capture/development
    /// footprints. The rectangle selects eligibility, not an output crop or film mask.
    pub fn analysis_samples(
        &mut self,
        area: Rect,
        budget: usize,
        cancelled: impl FnMut() -> bool,
    ) -> Result<Option<AnalysisSamples>> {
        self.analysis_samples_in(area, budget, |_| true, cancelled)
    }

    /// Select complete spatial footprints before the unchanged numerical gates.
    /// The grid remains canonical to `area`; support never resizes/rephases it.
    pub fn analysis_samples_in(
        &mut self,
        area: Rect,
        budget: usize,
        mut inside: impl FnMut(Rect) -> bool,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Option<AnalysisSamples>> {
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
        let mut result = AnalysisSamples {
            pixels: Vec::with_capacity(columns * rows),
            ..AnalysisSamples::default()
        };
        for gy in 0..rows {
            if cancelled() {
                return Ok(None);
            }
            for gx in 0..columns {
                let [x, y] = grid.source_pixel([gx as u32, gy as u32])?;
                let footprint = Rect {
                    x: x - 4,
                    y: y - 4,
                    width: 8,
                    height: 8,
                };
                if !inside(footprint) {
                    continue;
                }
                result.attempted += 1;
                let diagnostic = self.source.region_sensor_diagnostics(footprint)?;
                if diagnostic.at_white.iter().any(|n| *n > 0)
                    || diagnostic.below_black.iter().any(|n| *n > 0)
                {
                    result.rejected_capture += 1;
                    continue;
                }
                let mut white = false;
                let mut floor = false;
                for yy in footprint.y..footprint.y + 8 {
                    for xx in footprint.x..footprint.x + 8 {
                        white |= self.source.camera_image()?.pixel_codes(xx, yy)?[..3]
                            .contains(&u16::MAX);
                        let p = self.pixel(xx, yy)?;
                        if !p.is_finite() {
                            return Err(Error::NumericalOverflow);
                        }
                        floor |= p.min_element() <= self.corrected_floor;
                    }
                }
                if white {
                    result.rejected_development += 1;
                } else if floor {
                    result.rejected_floor += 1;
                } else {
                    result.pixels.push(self.pixel(x, y)?);
                }
            }
        }
        if cancelled() {
            return Ok(None);
        }
        Ok(Some(result))
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
            self.source
                .camera_image()?
                .sample_mean(region, DMat3::IDENTITY)?
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
        let matrix = self.camera_to_ap1;
        sampling::roll_neutral(calibration, roll, &self.patch(region)?, matrix)
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
            photo.before_white_balance(),
            self.camera_to_ap1,
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

    fn checked_coverage(&mut self, coverage: Coverage) -> Result<()> {
        for span in coverage.row_spans(self.area)? {
            self.checked_region(span)?;
        }
        Ok(())
    }

    pub fn reference_coverage(&mut self, coverage: Coverage) -> Result<DVec3> {
        if let Some(region) = coverage.exact_rectangle(self.area) {
            return self.reference(region);
        }
        self.checked_coverage(coverage)?;
        let mean = sampling::weighted_mean_iter(
            coverage
                .pixels(self.area)?
                .map(|([x, y], weight)| self.pixel(x, y).map(|p| (p, weight))),
        )?;
        if mean.min_element() <= self.corrected_floor {
            return Err(Error::UnusableSample(
                "reference mean is at/below the scan precision floor",
            ));
        }
        Ok(mean)
    }

    fn patch_coverage(&mut self, coverage: Coverage) -> Result<Vec<(DVec3, f64)>> {
        self.checked_coverage(coverage)?;
        coverage
            .pixels(self.area)?
            .map(|([x, y], weight)| {
                let pixel = self.pixel(x, y)?;
                if !pixel.is_finite() || pixel.min_element() < self.corrected_floor {
                    return Err(Error::UnusableSample(
                        "patch contains nonfinite or floored scan inputs",
                    ));
                }
                Ok((pixel, weight))
            })
            .collect()
    }

    pub fn roll_neutral_coverage(
        &mut self,
        coverage: Coverage,
        calibration: Calibration,
        roll: RollSettings,
    ) -> Result<Balance> {
        if let Some(region) = coverage.exact_rectangle(self.area) {
            return self.roll_neutral(region, calibration, roll);
        }
        let calibration = self.processing_calibration(calibration)?;
        let matrix = self.camera_to_ap1;
        sampling::roll_neutral_weighted(calibration, roll, &self.patch_coverage(coverage)?, matrix)
    }

    pub fn photo_neutral_coverage(
        &mut self,
        coverage: Coverage,
        calibration: Calibration,
        roll: RollSettings,
        photo: PhotoSettings,
    ) -> Result<Balance> {
        if let Some(region) = coverage.exact_rectangle(self.area) {
            return self.photo_neutral(region, calibration, roll, photo);
        }
        let pipeline = Pipeline::new(
            self.processing_calibration(calibration)?,
            roll,
            photo.before_white_balance(),
            self.camera_to_ap1,
        )?;
        let mut pixels = self.patch_coverage(coverage)?;
        for (pixel, _) in &mut pixels {
            let result = pipeline.apply(*pixel)?;
            if result.floored.into_iter().any(|v| v) {
                return Err(Error::UnusableSample(
                    "WB patch contains inputs below the calibration floor",
                ));
            }
            *pixel = result.rgb;
        }
        sampling::photo_neutral_weighted(&pixels)
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
        let image = self.source.camera_image()?;
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
        if !self.area.contains_region(area) {
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
        let image = self.source.camera_image()?;
        let raster = SourceRaster::new(self.area, [width, height])?;
        let mut pixels = Vec::with_capacity(width as usize * height as usize);
        for y in 0..height {
            for x in 0..width {
                let [sx, sy] = raster.source_pixel([x, y])?;
                pixels.push(image.pixel_codes(sx, sy)?);
            }
        }
        Ok((width, height, pixels))
    }
}
