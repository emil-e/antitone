use crate::flat_field::FlatField;
use crate::{
    Error,
    color::Balance,
    coverage::Coverage,
    geometry::SourceRaster,
    processing::{Calibration, InputSettings, RollSettings},
    raw::{Developed, FrozenRaw, Metadata, RawFile, Rect, SensorDiagnostics},
    sampling,
};
use glam::{DMat3, DVec3};
use std::{path::Path, sync::Arc};

type Result<T> = std::result::Result<T, Error>;

mod analysis;
mod identity;
pub use identity::CameraIdentity;
mod image_source;
pub use image_source::{ImageIdentity, ImageSource};

/// Integer inference evidence only, using the same declared camera-code scale for the whole roll.
#[cfg(feature = "editor")]
pub(crate) fn measurement_codes(pixel: [f32; 4], code_scale: f64) -> Result<[u16; 4]> {
    if !code_scale.is_finite() || code_scale < 1. {
        return Err(Error::Raw("invalid region-evidence scale".into()));
    }
    let mut codes = [0; 4];
    for c in 0..4 {
        let value = pixel[c];
        let rounded = (f64::from(value) / code_scale).round();
        if !value.is_finite()
            || !(0. ..=crate::processing::MAX_CAMERA_SOURCE_CODE).contains(&value)
            || !(0. ..=65535.).contains(&rounded)
        {
            return Err(Error::Raw(
                "unsupported camera range for region evidence".into(),
            ));
        }
        codes[c] = rounded as u16;
    }
    Ok(codes)
}

#[derive(Debug, Default)]
pub struct AnalysisSamples {
    pub pixels: Vec<DVec3>,
    pub attempted: usize,
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
#[derive(Clone)]
pub struct DevelopedSource {
    raw: Arc<FrozenRaw>,
    pub(crate) file: crate::source::FileKey,
    camera_to_ap1: DMat3,
    camera_model: String,
    area: Rect,
    fixed_wb: [f64; 4],
    source_scale: f64,
    input_floor: f64,
}

impl DevelopedSource {
    fn from_raw(mut raw: RawFile, fixed_wb: Option<[f64; 4]>) -> Result<Self> {
        let wb = fixed_wb.unwrap_or_else(|| raw.metadata.as_shot_wb());
        raw.prepare_basis(wb)?;
        let matrix = raw.metadata.camera_to_ap1()?;
        let camera_model = raw.metadata.camera_model();
        let area = raw.metadata.image_area()?;
        raw.develop_camera(wb)?;
        let wb = raw.fixed_wb(wb);
        let source_scale = raw.metadata.source_scale(wb)?;
        let input_floor = raw.metadata.source_input_floor(wb)?;
        Ok(Self {
            file: raw.file.clone(),
            raw: Arc::new(raw.freeze()?),
            camera_to_ap1: matrix,
            camera_model,
            area,
            fixed_wb: wb,
            source_scale,
            input_floor,
        })
    }

    /// Explicit native allocations plus this Rust payload/string capacity, charged once.
    /// Excludes SDK-private/stdlib heap, Arc/store bookkeeping and allocator-held VM;
    /// this is not total native heap or process footprint. None disables cache admission.
    pub fn retained_bytes(&self) -> Option<u64> {
        let bytes = self
            .raw
            .retained_bytes()?
            .checked_add(size_of::<Self>() as u64)?
            .checked_add(size_of::<FrozenRaw>() as u64)?
            .checked_add(self.camera_model.capacity() as u64)?;
        let bytes = bytes.checked_add(self.file.path.capacity() as u64)?;
        Some(bytes)
    }

    pub fn metadata(&self) -> Metadata {
        self.raw.metadata()
    }

    /// Recorded colour basis; positive image production uses `positive_matrix`.
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

    /// Conservative developed-source quantum before an optional measured flat field.
    pub fn input_floor(&self) -> f64 {
        self.input_floor
    }

    /// Source camera light, with LibRaw's WB-normalization scale already restored.
    pub fn camera_image(&self) -> Result<Developed<'_>> {
        self.raw
            .camera_image()?
            .with_source_scale(self.source_scale)
    }

    pub fn sensor_diagnostics(&self) -> Result<SensorDiagnostics> {
        self.raw.sensor_diagnostics()
    }

    /// Detailed original-sensor statistics; `area` is in unwarped native coordinates.
    pub fn region_sensor_diagnostics(&self, area: Rect) -> Result<SensorDiagnostics> {
        self.raw.region_sensor_diagnostics(area)
    }
}

/// Developed camera-RGB thumbnail data before measured illumination correction or rendering.
/// The production proxy has a 256-pixel maximum edge, without upscaling.
#[derive(Clone)]
pub struct CameraPixels {
    pub(crate) identity: Option<CameraIdentity>,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) pixels: Arc<Vec<[f32; 4]>>,
    // Physical colour basis retained for estimator sampling and source inspection.
    pub(crate) matrix: DMat3,
}

impl CameraPixels {
    pub const MAX_EDGE: u32 = 256;
    #[cfg(feature = "editor")]
    pub(crate) const SAMPLING_VERSION: u32 = 1;

    pub fn from_scan(scan: &Scan) -> Result<Self> {
        Self::sample(scan, Self::MAX_EDGE)
    }

    fn sample(scan: &Scan, maximum: u32) -> Result<Self> {
        let (width, height) = scan.preview_dimensions(maximum)?;
        let image = scan.source.camera_image()?;
        // A bounded stratified grid reduces grain/aliasing without reading every
        // original in each proxy footprint. Measurements use their own source sampling.
        let samples = [
            (scan.area.width / width).clamp(1, 4),
            (scan.area.height / height).clamp(1, 4),
        ];
        let raster = SourceRaster::new(scan.area, [width * samples[0], height * samples[1]])?;
        let count = f64::from(samples[0] * samples[1]);
        let mut pixels = Vec::with_capacity(width as usize * height as usize);
        for y in 0..height {
            for x in 0..width {
                let mut sum = [0f64; 3];
                for sy in 0..samples[1] {
                    for sx in 0..samples[0] {
                        let [px, py] =
                            raster.source_pixel([x * samples[0] + sx, y * samples[1] + sy])?;
                        let sample = image.pixel_samples(px, py)?;
                        for c in 0..3 {
                            sum[c] += f64::from(sample[c]);
                        }
                    }
                }
                pixels.push([
                    (sum[0] / count) as f32,
                    (sum[1] / count) as f32,
                    (sum[2] / count) as f32,
                    0.,
                ]);
            }
        }
        Ok(Self {
            identity: Some(scan.source.identity()),
            width,
            height,
            pixels: Arc::new(pixels),
            matrix: scan.camera_to_ap1,
        })
    }

    pub fn identity(&self) -> Option<&CameraIdentity> {
        self.identity.as_ref()
    }

    pub fn dimensions(&self) -> [u32; 2] {
        [self.width, self.height]
    }

    pub fn retained_bytes(&self) -> u64 {
        (self.pixels.capacity() * size_of::<[f32; 4]>()
            + size_of::<Self>()
            + size_of::<Vec<[f32; 4]>>()) as u64
            + self.identity.as_ref().map_or(0, CameraIdentity::heap_bytes)
    }
}

/// A consumer's fixed interpretation of shared camera-proxy pixels.
pub struct CameraProxy {
    pub data: Arc<CameraPixels>,
    pub reference: Option<crate::exposure_reference::ReferenceExposure>,
}

impl std::ops::Deref for CameraProxy {
    type Target = CameraPixels;
    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl CameraProxy {
    pub const MAX_EDGE: u32 = CameraPixels::MAX_EDGE;
    pub fn new(
        data: Arc<CameraPixels>,
        reference: Option<crate::exposure_reference::ReferenceExposure>,
    ) -> Self {
        Self { data, reference }
    }
    pub fn from_scan(scan: &Scan) -> Result<Self> {
        Ok(Self::new(
            Arc::new(CameraPixels::from_scan(scan)?),
            scan.reference_exposure.clone(),
        ))
    }
    #[cfg(test)]
    fn sample(scan: &Scan, maximum: u32) -> Result<Self> {
        Ok(Self::new(
            Arc::new(CameraPixels::sample(scan, maximum)?),
            scan.reference_exposure.clone(),
        ))
    }
    pub fn positive_matrix(&self, input: &crate::processing::InputSettings) -> Result<DMat3> {
        positive_matrix(input, self.matrix, self.reference.as_ref())
    }
}

#[cfg(test)]
mod proxy_tests;
#[cfg(all(test, feature = "project"))]
mod source_scale_tests;

pub(crate) fn positive_matrix(
    input: &crate::processing::InputSettings,
    matrix: DMat3,
    reference: Option<&crate::exposure_reference::ReferenceExposure>,
) -> Result<DMat3> {
    let gain = if input.is_negative() {
        1.
    } else {
        crate::processing::exposure_multiplier(reference.map_or(0., |value| value.ev()))?
    };
    Ok(matrix * gain)
}

pub struct Scan {
    source: Arc<DevelopedSource>,
    pub reference_exposure: Option<crate::exposure_reference::ReferenceExposure>,
    pub camera_to_ap1: DMat3,
    pub camera_model: String,
    pub area: Rect,
    pub fixed_wb: [f64; 4],
    /// Stored calibration's native base; `effective_floor` includes source and residual gains.
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
        let mut raw = RawFile::open_with_wb(path, fixed_wb)?;
        raw.set_development_priority(priority);
        Self::from_raw(raw, fixed_wb)
    }

    pub(crate) fn from_raw(raw: RawFile, fixed_wb: Option<[f64; 4]>) -> Result<Self> {
        Ok(Self::from_source(Arc::new(DevelopedSource::from_raw(
            raw, fixed_wb,
        )?)))
    }

    /// Starts an independent consumer without a measured flat field or another RGB development.
    pub fn from_source(source: Arc<DevelopedSource>) -> Self {
        Self::with_reference(source, None)
    }

    /// Captures a consumer's fixed baseline without changing shared native pixels.
    pub fn with_reference(
        source: Arc<DevelopedSource>,
        reference_exposure: Option<crate::exposure_reference::ReferenceExposure>,
    ) -> Self {
        Self {
            reference_exposure,
            camera_to_ap1: source.camera_to_ap1,
            camera_model: source.camera_model.clone(),
            area: source.area,
            fixed_wb: source.fixed_wb,
            input_floor: crate::processing::CAMERA_INPUT_FLOOR,
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
            self.source.input_floor * field.bounds().1.max_element()
        } else {
            self.source.input_floor
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
        let mut sums = vec![[0f64; 3]; size[0] * size[1]];
        let mut counts = vec![0u64; sums.len()];
        for y in 0..a.height {
            for x in 0..a.width {
                let i = (y / side) as usize * size[0] + (x / side) as usize;
                let p = image.pixel_samples(a.x + x, a.y + y)?;
                for c in 0..3 {
                    sums[i][c] += f64::from(p[c]);
                }
                counts[i] += 1;
            }
        }
        let values: Vec<_> = sums
            .iter()
            .zip(counts)
            .map(|(p, n)| {
                (crate::color::AP1_TO_XYZ
                    * (self.camera_to_ap1 * (DVec3::new(p[0], p[1], p[2]) / (n as f64 * 65535.))))
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
                        let p = image.pixel_samples(x, y)?;
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

    pub fn positive_matrix(&self, input: &crate::processing::InputSettings) -> Result<DMat3> {
        positive_matrix(
            input,
            self.source.camera_to_ap1,
            self.reference_exposure.as_ref(),
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
        if calibration.input_floor == self.corrected_floor {
            return calibration.validate();
        }
        if calibration.input_floor != self.input_floor {
            return Err(Error::InvalidParameter(
                "scan calibration requires the native-code base floor",
            ));
        }
        calibration.input_floor = self.corrected_floor;
        calibration.validate()
    }

    pub(crate) fn refined_input(&self, input: &InputSettings) -> Result<InputSettings> {
        match input.calibration() {
            Some(base) => Ok(input.with_calibration(self.processing_calibration(base)?)),
            None => Ok(*input),
        }
    }

    pub fn pixel(&self, x: u32, y: u32) -> Result<DVec3> {
        let mut camera = self.source.camera_image()?.pixel(x, y)?;
        if let Some(field) = &self.flat_field {
            camera *= field.gain([x as f64 + 0.5, y as f64 + 0.5])?;
        }
        Ok(camera)
    }

    /// Bounded original-pixel centres, checked in local 8×8 numerical footprints.
    /// The rectangle selects sampling support, not an output crop or film mask.
    pub fn analysis_samples(
        &mut self,
        area: Rect,
        budget: usize,
        cancelled: impl FnMut() -> bool,
    ) -> Result<Option<AnalysisSamples>> {
        self.analysis_samples_in(area, budget, |_| true, cancelled)
    }

    /// Select complete spatial footprints before checking numerical eligibility.
    /// The grid remains canonical to `area`; support never resizes/rephases it.
    pub fn analysis_samples_in(
        &mut self,
        area: Rect,
        budget: usize,
        inside: impl FnMut(Rect) -> bool,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Option<AnalysisSamples>> {
        let Some(mut plan) = self.analysis_plan(area, budget, inside, &mut cancelled)? else {
            return Ok(None);
        };
        for footprint in plan.footprints {
            if cancelled() {
                return Ok(None);
            }
            let mut floor = false;
            for y in footprint.region.y..footprint.region.y + footprint.region.height {
                for x in footprint.region.x..footprint.region.x + footprint.region.width {
                    let pixel = self.pixel(x, y)?;
                    if !pixel.is_finite() {
                        return Err(Error::NumericalOverflow);
                    }
                    floor |= pixel.min_element() <= self.corrected_floor;
                }
            }
            if floor {
                plan.samples.rejected_floor += 1;
            } else {
                plan.samples
                    .pixels
                    .push(self.pixel(footprint.centre[0], footprint.centre[1])?);
            }
        }
        Ok((!cancelled()).then_some(plan.samples))
    }

    pub fn reference(&mut self, region: Rect) -> Result<DVec3> {
        self.validate_area(region)?;
        let mean = if self.flat_field().is_some() {
            let mut sum = DVec3::ZERO;
            for y in region.y..region.y + region.height {
                for x in region.x..region.x + region.width {
                    sum += self.pixel(x, y)?;
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
        self.validate_area(region)?;
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

    pub fn roll_neutral(&mut self, region: Rect, input: &InputSettings) -> Result<Balance> {
        let negative =
            self.refined_input(input)?
                .negative_input()
                .ok_or(Error::InvalidParameter(
                    "white-balance fitting requires a negative input",
                ))?;
        let matrix = self.camera_to_ap1;
        sampling::roll_neutral(negative, &self.patch(region)?, matrix)
    }

    pub fn reference_coverage(&mut self, coverage: Coverage) -> Result<DVec3> {
        if let Some(region) = coverage.exact_rectangle(self.area) {
            return self.reference(region);
        }
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
        input: &InputSettings,
    ) -> Result<Balance> {
        if let Some(region) = coverage.exact_rectangle(self.area) {
            return self.roll_neutral(region, input);
        }
        let negative =
            self.refined_input(input)?
                .negative_input()
                .ok_or(Error::InvalidParameter(
                    "white-balance fitting requires a negative input",
                ))?;
        let matrix = self.camera_to_ap1;
        sampling::roll_neutral_weighted(negative, &self.patch_coverage(coverage)?, matrix)
    }

    /// Fit Roll WB or a selected layer's replacement from its captured upstream image.
    pub fn white_balance_coverage(
        &mut self,
        coverage: Coverage,
        input: &InputSettings,
        roll: RollSettings,
        upstream: crate::adjustment_layers::AdjustmentLayerSnapshot,
        layer: Option<crate::adjustment_layers::AdjustmentLayer>,
    ) -> Result<Balance> {
        let pipeline = crate::adjustment_layers::AdjustmentLayerPipeline::new(
            self.refined_input(input)?,
            roll,
            upstream,
            self.positive_matrix(input)?,
        )?;
        let pixels = coverage
            .pixels(self.area)?
            .map(|([x, y], area)| {
                let pixel = self.pixel(x, y)?;
                if !pixel.is_finite() || pixel.min_element() < self.corrected_floor {
                    return Err(Error::UnusableSample(
                        "patch contains nonfinite or floored scan inputs",
                    ));
                }
                let result = pipeline.apply(pixel, [x, y])?;
                if result.floored.into_iter().any(|v| v) {
                    return Err(Error::UnusableSample(
                        "WB patch contains inputs below the calibration floor",
                    ));
                }
                let weight = match layer {
                    Some(layer) => layer.weight(crate::geometry::SourcePoint([
                        f64::from(x) + 0.5,
                        f64::from(y) + 0.5,
                    ]))?,
                    None => 1.,
                };
                let exposure = layer.map_or(0., |layer| layer.photo.exposure) * weight;
                Ok((
                    result.rgb * crate::processing::exposure_multiplier(exposure)?,
                    area,
                    weight,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        match layer {
            Some(_) => sampling::adjustment_layer_white_balance(&pixels),
            None => sampling::white_balance_weighted(
                &pixels
                    .into_iter()
                    .map(|(rgb, area, _)| (rgb, area))
                    .collect::<Vec<_>>(),
            ),
        }
    }

    pub fn tile_region(
        &self,
        area: Rect,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    ) -> Result<Vec<[f32; 4]>> {
        self.validate_area(area)?;
        if width == 0 || height == 0 || x >= area.width || y >= area.height {
            return Err(Error::InvalidParameter("invalid source tile"));
        }
        let image = self.source.camera_image()?;
        let mut pixels = vec![[0.; 4]; width as usize * height as usize];
        for dy in 0..height.min(area.height - y) {
            for dx in 0..width.min(area.width - x) {
                pixels[(dy * width + dx) as usize] =
                    image.pixel_samples(area.x + x + dx, area.y + y + dy)?;
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

    /// Rounded inference evidence, normalized by one declared headroom bound for the whole roll.
    /// Photographic samples stay in their original continuous code units.
    #[cfg(feature = "editor")]
    pub(crate) fn measurement_preview(
        &self,
        maximum: u32,
        code_scale: f64,
    ) -> Result<(u32, u32, Vec<[u16; 4]>)> {
        if !code_scale.is_finite()
            || code_scale < self.source.input_floor / crate::processing::CAMERA_INPUT_FLOOR
        {
            return Err(Error::Raw(
                "region-evidence scale does not cover source headroom".into(),
            ));
        }
        let (width, height, samples) = self.preview_source(maximum)?;
        let pixels = samples
            .into_iter()
            .map(|p| measurement_codes(p, code_scale))
            .collect::<Result<_>>()?;
        Ok((width, height, pixels))
    }

    /// Unconverted camera samples in code units at display-grid centres; no requantization.
    pub fn preview_source(&self, maximum: u32) -> Result<(u32, u32, Vec<[f32; 4]>)> {
        let (width, height) = self.preview_dimensions(maximum)?;
        let image = self.source.camera_image()?;
        let raster = SourceRaster::new(self.area, [width, height])?;
        let mut pixels = Vec::with_capacity(width as usize * height as usize);
        for y in 0..height {
            for x in 0..width {
                let [sx, sy] = raster.source_pixel([x, y])?;
                pixels.push(image.pixel_samples(sx, sy)?);
            }
        }
        Ok((width, height, pixels))
    }
}
