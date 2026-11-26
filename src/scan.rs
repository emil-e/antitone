use crate::flat_field::FlatField;
use crate::{
    Error,
    geometry::SourceRaster,
    processing::InputSettings,
    raw::{Developed, FrozenRaw, Metadata, RawFile, Rect},
};
use glam::{DMat3, DVec3};
use std::{path::Path, sync::Arc};

type Result<T> = std::result::Result<T, Error>;

#[cfg(feature = "editor")]
mod analysis;
#[cfg(feature = "editor")]
pub(crate) use analysis::AnalysisInput;
#[cfg(all(feature = "editor", test))]
pub(crate) use analysis::AnalysisWindow;
mod identity;
pub use identity::CameraIdentity;
mod image_source;
pub use image_source::{ImageIdentity, ImageSource};

/// Immutable native pixels and development facts shared by camera-source consumers.
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
    file: crate::source::FileKey,
    camera_to_ap1: DMat3,
    camera_model: String,
    area: Rect,
    white_balance: [f64; 4],
    source_scale: f64,
}

impl DevelopedSource {
    pub fn open(path: &Path, white_balance: Option<[f64; 4]>) -> Result<Arc<Self>> {
        Self::open_with_priority(
            path,
            white_balance,
            crate::raw::DevelopmentPriority::Foreground,
        )
    }

    pub fn open_with_priority(
        path: &Path,
        white_balance: Option<[f64; 4]>,
        priority: crate::raw::DevelopmentPriority,
    ) -> Result<Arc<Self>> {
        let mut raw = RawFile::open_with_wb(path, white_balance)?;
        raw.set_development_priority(priority);
        Self::from_raw(raw, white_balance).map(Arc::new)
    }

    /// Develop and freeze a caller-configured, uniquely owned RAW handle.
    pub fn from_raw(mut raw: RawFile, white_balance: Option<[f64; 4]>) -> Result<Self> {
        let wb = white_balance.unwrap_or_else(|| raw.metadata.as_shot_wb());
        raw.prepare_basis(wb)?;
        let matrix = raw.metadata.camera_to_ap1()?;
        let camera_model = raw.metadata.camera_model();
        let area = raw.metadata.image_area()?;
        raw.develop_camera(wb)?;
        let wb = raw.white_balance(wb);
        let source_scale = raw.metadata.source_scale(wb)?;
        Ok(Self {
            file: raw.file.clone(),
            raw: Arc::new(raw.freeze()?),
            camera_to_ap1: matrix,
            camera_model,
            area,
            white_balance: wb,
            source_scale,
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

    #[cfg(any(feature = "project", all(test, feature = "gpu")))]
    pub(crate) fn path(&self) -> &std::path::Path {
        &self.file.path
    }

    pub fn metadata(&self) -> Metadata {
        self.raw.metadata()
    }

    pub fn demosaic_backend(&self) -> crate::raw::DemosaicBackend {
        self.raw.demosaic_backend()
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

    pub fn white_balance(&self) -> [f64; 4] {
        self.white_balance
    }

    pub(crate) fn require_input(&self, input: &crate::processing::InputSettings) -> Result<()> {
        if input.is_negative() && self.white_balance != [1.; 4] {
            return Err(crate::Error::Raw(
                "source white balance differs from input interpretation".into(),
            ));
        }
        Ok(())
    }

    pub fn validate_area(&self, area: Rect) -> Result<()> {
        if !self.area.contains_region(area) {
            return Err(Error::InvalidParameter(
                "view region is outside the active source area",
            ));
        }
        Ok(())
    }

    /// Source camera light, with LibRaw's WB-normalization scale already restored.
    pub fn camera_image(&self) -> Result<Developed<'_>> {
        self.raw
            .camera_image()?
            .with_source_scale(self.source_scale)
    }

    pub fn dimensions(&self) -> Result<(u32, u32)> {
        let image = self.camera_image()?;
        Ok((image.width, image.height))
    }

    pub fn pixel(&self, x: u32, y: u32) -> Result<DVec3> {
        self.camera_image()?.pixel(x, y)
    }

    #[cfg(all(test, feature = "gpu"))]
    pub(crate) fn tile_region(
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
        let mut pixels = vec![[0.; 4]; width as usize * height as usize];
        self.camera_image()?.copy_samples(
            Rect {
                x: area.x + x,
                y: area.y + y,
                width: width.min(area.width - x),
                height: height.min(area.height - y),
            },
            &mut pixels,
            width as usize,
        )?;
        Ok(pixels)
    }

    pub fn preview_dimensions(&self, maximum: u32) -> Result<(u32, u32)> {
        preview_dimensions(self.area, maximum)
    }

    /// Unconverted camera samples in code units at display-grid centres; no requantization.
    pub fn preview_source(&self, maximum: u32) -> Result<(u32, u32, Vec<[f32; 4]>)> {
        self.preview_source_in(self.area(), maximum)
    }

    fn preview_source_in(&self, area: Rect, maximum: u32) -> Result<(u32, u32, Vec<[f32; 4]>)> {
        let (width, height) = preview_dimensions(area, maximum)?;
        let image = self.camera_image()?;
        let raster = SourceRaster::new(area, [width, height])?;
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

    pub fn from_source(source: &DevelopedSource) -> Result<Self> {
        Self::sample(source, Self::MAX_EDGE)
    }

    fn sample(source: &DevelopedSource, maximum: u32) -> Result<Self> {
        let area = source.area();
        let (width, height) = preview_dimensions(area, maximum)?;
        let image = source.camera_image()?;
        // A bounded stratified grid reduces grain/aliasing without reading every
        // original in each proxy footprint. Measurements use their own source sampling.
        let samples = [
            (area.width / width).clamp(1, 4),
            (area.height / height).clamp(1, 4),
        ];
        let raster = SourceRaster::new(area, [width * samples[0], height * samples[1]])?;
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
            identity: Some(source.identity()),
            width,
            height,
            pixels: Arc::new(pixels),
            matrix: source.camera_to_ap1(),
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
    #[cfg(all(test, feature = "gpu"))]
    pub(crate) fn from_source(
        source: &DevelopedSource,
        reference: Option<crate::exposure_reference::ReferenceExposure>,
    ) -> Result<Self> {
        Ok(Self::new(
            Arc::new(CameraPixels::from_source(source)?),
            reference,
        ))
    }

    pub fn positive_matrix(&self, input: &crate::processing::InputSettings) -> Result<DMat3> {
        input.positive_matrix(
            self.matrix,
            self.reference.as_ref().map(|reference| reference.ev()),
        )
    }
}

#[cfg(test)]
mod proxy_tests;
#[cfg(all(test, feature = "project"))]
mod source_scale_tests;

fn preview_dimensions(area: Rect, maximum: u32) -> Result<(u32, u32)> {
    if maximum == 0 {
        return Err(Error::InvalidParameter("preview size must be positive"));
    }
    let scale = (f64::from(maximum) / f64::from(area.width.max(area.height))).min(1.0);
    let width = (f64::from(area.width) * scale).round().max(1.0) as u32;
    let height = (f64::from(area.height) * scale).round().max(1.0) as u32;
    Ok((width, height))
}
