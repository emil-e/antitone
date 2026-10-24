use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::marker::PhantomData;
use std::path::Path;
use std::ptr::NonNull;

use glam::{DMat3, DVec3};

use crate::Error;
use crate::color::{AP1_TO_XYZ, BRADFORD};
use crate::sampling::mean;

#[path = "raw_sony.rs"]
mod sony;
pub use sony::{SonyLensData, SonyLensDataStatus};
#[path = "raw_eligibility.rs"]
mod eligibility;
#[path = "raw_lens.rs"]
mod lens_source;

#[cfg(all(test, feature = "project"))]
#[path = "../tests/support/raw_fixture.rs"]
pub(crate) mod fixture;
#[cfg(all(test, feature = "project"))]
#[path = "../tests/support/sony_fixture.rs"]
pub(crate) mod sony_fixture;

const SRGB_TO_XYZ: DMat3 = DMat3::from_cols(
    DVec3::new(0.4124564, 0.2126729, 0.0193339),
    DVec3::new(0.3575761, 0.7151522, 0.1191920),
    DVec3::new(0.1804375, 0.0721750, 0.9503041),
);

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NativeMetadata {
    make: [c_char; 64],
    model: [c_char; 64],
    decoder: [c_char; 128],
    pub width: u32,
    pub height: u32,
    pub raw_width: u32,
    pub raw_height: u32,
    pub left_margin: u32,
    pub top_margin: u32,
    pub inset_left: u32,
    pub inset_top: u32,
    pub inset_width: u32,
    pub inset_height: u32,
    pub flip: i32,
    pub colors: u32,
    pub filters: u32,
    pub raw_bps: u32,
    pub black: u32,
    pub maximum: u32,
    pub cblack: [u32; 4],
    pub linear_max: [u32; 4],
    pub cam_mul: [f64; 4],
    pub daylight_mul: [f64; 4],
    rgb_cam: [f64; 9],
    pub vendor: VendorMetadata,
    lens_disabled: u32,
}

/// Vendor makernote payload. Sony is the vendor with parsed payloads; other makes arrive
/// zeroed and report absent embedded lens data.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VendorMetadata {
    pub sony_file_type: u32,
    pub sony_lens: SonyLensData,
}

/// Shared source descriptor. LibRaw supplies the header/container fields; SDK DNG
/// development supplies its own geometry, camera basis, precision and interpretation.
#[derive(Debug, Clone, Copy)]
pub struct Metadata {
    native: NativeMetadata,
    #[cfg(feature = "dng-sdk")]
    sdk: Option<crate::dng::Header>,
}

impl std::ops::Deref for Metadata {
    type Target = NativeMetadata;
    fn deref(&self) -> &Self::Target {
        &self.native
    }
}
impl std::ops::DerefMut for Metadata {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.native
    }
}

impl Metadata {
    fn from_native(native: NativeMetadata) -> Self {
        Self {
            native,
            #[cfg(feature = "dng-sdk")]
            sdk: None,
        }
    }

    /// The direct-SDK route, whose transactions the bridge serializes process-wide.
    pub(crate) fn uses_sdk(&self) -> bool {
        #[cfg(feature = "dng-sdk")]
        {
            self.sdk.is_some()
        }
        #[cfg(not(feature = "dng-sdk"))]
        {
            false
        }
    }

    pub fn dng_identity(&self) -> Option<String> {
        #[cfg(feature = "dng-sdk")]
        if let Some(header) = self.sdk {
            return Some(
                blake3::Hash::from_bytes(header.interpretation_id)
                    .to_hex()
                    .to_string(),
            );
        }
        None
    }

    #[cfg(feature = "project")]
    pub fn development(&self, wb: [f64; 4]) -> crate::project::Development {
        #[cfg(feature = "dng-sdk")]
        if self.sdk.is_some() {
            return crate::project::Development::dng(wb);
        }
        crate::project::Development::new(version(), wb)
    }

    pub fn make(&self) -> String {
        ffi_string(&self.make)
    }

    pub fn model(&self) -> String {
        ffi_string(&self.model)
    }

    pub fn decoder(&self) -> String {
        #[cfg(feature = "dng-sdk")]
        if self.sdk.is_some() {
            return crate::dng::INTERPRETATION.into();
        }
        ffi_string(&self.decoder)
    }

    pub fn camera_to_srgb(&self) -> DMat3 {
        #[cfg(feature = "dng-sdk")]
        if let Some(header) = self.sdk {
            let source = DVec3::new(0.3457 / 0.3585, 1., (1. - 0.3457 - 0.3585) / 0.3585);
            let target = DVec3::new(0.95047, 1., 1.08883);
            let adaptation = BRADFORD.inverse()
                * DMat3::from_diagonal((BRADFORD * target) / (BRADFORD * source))
                * BRADFORD;
            return SRGB_TO_XYZ.inverse() * adaptation * header.camera_to_xyz_d50;
        }
        DMat3::from_cols_array(&self.rgb_cam)
    }

    pub fn camera_model(&self) -> String {
        format!("{} {}", self.make(), self.model())
    }

    /// Selects as-shot gains, sharing green with an unspecified second green on three-color sources.
    /// Raw metadata and explicitly supplied development gains remain unchanged.
    pub fn as_shot_wb(&self) -> [f64; 4] {
        #[cfg(feature = "dng-sdk")]
        if let Some(header) = self.sdk {
            let [r, g, b] = header.as_shot_wb;
            return [r, g, b, g];
        }
        let mut wb = self.cam_mul;
        if self.colors == 3 && wb[3] == 0. {
            wb[3] = wb[1];
        }
        wb
    }

    pub fn camera_to_ap1(&self) -> Result<DMat3, Error> {
        #[cfg(feature = "dng-sdk")]
        if let Some(header) = self.sdk {
            // The DNG SDK PCS uses this exact D50 chromaticity.
            let white = DVec3::new(0.3457 / 0.3585, 1., (1. - 0.3457 - 0.3585) / 0.3585);
            let target = AP1_TO_XYZ * DVec3::ONE;
            let adaptation = BRADFORD.inverse()
                * DMat3::from_diagonal((BRADFORD * target) / (BRADFORD * white))
                * BRADFORD;
            let matrix = AP1_TO_XYZ.inverse() * adaptation * header.camera_to_xyz_d50;
            crate::processing::validate_camera_matrix(matrix)?;
            return Ok(matrix);
        }
        let camera_to_srgb = self.camera_to_srgb();
        if !camera_to_srgb.is_finite() || camera_to_srgb.determinant().abs() < 1e-8 {
            return Err(Error::Raw("invalid camera-to-sRGB matrix".into()));
        }
        let source_white = DVec3::new(0.95047, 1.0, 1.08883);
        let target_white = AP1_TO_XYZ * DVec3::ONE;
        let adaptation = BRADFORD.inverse()
            * DMat3::from_diagonal((BRADFORD * target_white) / (BRADFORD * source_white))
            * BRADFORD;
        Ok(AP1_TO_XYZ.inverse() * adaptation * SRGB_TO_XYZ * camera_to_srgb)
    }

    pub fn lens_geometry(&self) -> Result<Option<crate::lens::EmbeddedLensGeometry>, Error> {
        if self.dng_identity().is_some() || self.lens_disabled != 0 {
            return Ok(None);
        }
        let Some(profile) = self.vendor.sony_lens.profile()? else {
            return Ok(None);
        };
        if !profile.has_distortion() && !profile.has_ca() && !profile.has_vignetting() {
            return Ok(None);
        }
        crate::lens::EmbeddedLensGeometry::new(profile, self.native_image_area()?).map(Some)
    }

    /// Original-code precision amplified conservatively by development; never a clipping limit.
    pub fn input_floor(&self) -> Result<f64, Error> {
        #[cfg(feature = "dng-sdk")]
        if let Some(header) = self.sdk {
            return Ok(header.input_floor);
        }
        Ok(crate::processing::CAMERA_INPUT_FLOOR
            * self.lens_geometry()?.map_or(1., |g| g.maximum_gain()))
    }

    pub fn source_dimensions(&self) -> Result<[u32; 2], Error> {
        Ok(self
            .lens_geometry()?
            .map_or([self.width, self.height], |g| g.size()))
    }

    pub fn lens_identity(&self) -> Result<Option<String>, Error> {
        Ok(self.lens_geometry()?.map(|g| g.identity()))
    }

    /// Orientation the camera recorded for this source, as the initial per-photo
    /// composition setting. Every TIFF orientation is representable as turns plus
    /// flips, including the mirrored and transposed cases.
    pub fn photo_orientation(&self) -> Result<crate::geometry::PhotoOrientation, Error> {
        #[cfg(feature = "dng-sdk")]
        if let Some(header) = self.sdk {
            return photo_orientation_from_tiff(header.orientation);
        }
        photo_orientation_from_libraw_flip(self.flip)
    }

    pub fn image_area(&self) -> Result<Rect, Error> {
        if let Some(geometry) = self.lens_geometry()? {
            let [width, height] = geometry.size();
            return Ok(Rect {
                x: 0,
                y: 0,
                width,
                height,
            });
        }
        self.native_image_area()
    }

    pub fn native_image_area(&self) -> Result<Rect, Error> {
        if self.inset_width == 0 && self.inset_height == 0 {
            return Ok(Rect {
                x: 0,
                y: 0,
                width: self.width,
                height: self.height,
            });
        }
        let x = self.inset_left.checked_sub(self.left_margin);
        let y = self.inset_top.checked_sub(self.top_margin);
        match (x, y) {
            (Some(x), Some(y))
                if self.inset_width > 0
                    && self.inset_height > 0
                    && x.checked_add(self.inset_width)
                        .is_some_and(|v| v <= self.width)
                    && y.checked_add(self.inset_height)
                        .is_some_and(|v| v <= self.height) =>
            {
                Ok(Rect {
                    x,
                    y,
                    width: self.inset_width,
                    height: self.inset_height,
                })
            }
            _ => Err(Error::Raw("invalid metadata-defined image inset".into())),
        }
    }
}

/// LibRaw stores TIFF orientation as dcraw's `"50132467"` flip table. The composition
/// model's turns and flips represent all eight orientations, including mirrored and
/// transposed sources, so no recorded orientation is rejected.
fn photo_orientation_from_libraw_flip(
    flip: i32,
) -> Result<crate::geometry::PhotoOrientation, Error> {
    use crate::geometry::{Flips, PhotoOrientation, QuarterTurns};
    const MIRROR: Flips = Flips {
        horizontal: true,
        vertical: false,
    };
    let (turns, flips) = match flip {
        0 => (QuarterTurns::Zero, Flips::default()), // TIFF 1
        1 => (QuarterTurns::Zero, MIRROR),           // TIFF 2: mirror
        2 => (
            QuarterTurns::Zero,
            Flips {
                horizontal: false,
                vertical: true,
            },
        ), // TIFF 4
        3 => (QuarterTurns::Two, Flips::default()),  // TIFF 3: 180 degrees
        4 => (QuarterTurns::One, MIRROR),            // TIFF 5: transpose
        5 => (QuarterTurns::Three, Flips::default()), // TIFF 8: 270 degrees
        6 => (QuarterTurns::One, Flips::default()),  // TIFF 6: 90 degrees
        7 => (QuarterTurns::Three, MIRROR),          // TIFF 7: transverse
        _ => return Err(Error::Raw("unknown RAW orientation code".into())),
    };
    Ok(PhotoOrientation { turns, flips })
}

fn photo_orientation_from_tiff(
    orientation: u32,
) -> Result<crate::geometry::PhotoOrientation, Error> {
    if !(1..=8).contains(&orientation) {
        return Err(Error::Raw("invalid DNG orientation tag".into()));
    }
    let flip = (b"50132467"[(orientation & 7) as usize] - b'0') as i32;
    photo_orientation_from_libraw_flip(flip)
}

pub use crate::geometry::SourceRegion as Rect;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SensorDiagnostics {
    pub sites: [u64; 4],
    pub below_black: [u64; 4],
    pub at_white: [u64; 4],
    pub peak: [u32; 4],
}

/// Original sensor-limit warnings affecting a developed region, not corrected RGB saturation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CaptureFlags {
    /// Original sensor white, or contributing native RGB clipping in a corrected source.
    /// Shading headroom above native white does not set this flag.
    pub at_white: bool,
    pub below_black: bool,
    pub repeated_border: bool,
    pub processing_limit: bool,
}

impl CaptureFlags {
    pub fn eligible(self) -> bool {
        !self.at_white && !self.below_black && !self.repeated_border && !self.processing_limit
    }
}

impl From<SensorDiagnostics> for CaptureFlags {
    fn from(value: SensorDiagnostics) -> Self {
        Self {
            at_white: value.at_white.iter().any(|&n| n != 0),
            below_black: value.below_black.iter().any(|&n| n != 0),
            ..Self::default()
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NativeView {
    pixels: *const u16,
    width: u32,
    height: u32,
    maximum: u32,
    data_maximum: u32,
    warnings: u32,
    pre_mul: [f64; 4],
}

#[cfg(feature = "editor")]
#[repr(C)]
struct NativeThumbnail {
    jpeg: *const u8,
    bytes: u32,
    width: u32,
    height: u32,
    flip: i32,
    color_space: i32,
}

#[cfg(feature = "editor")]
pub(crate) struct EmbeddedThumbnail {
    pub jpeg: Vec<u8>,
    pub flip: i32,
    pub color_space: i32,
}

unsafe extern "C" {
    fn antitone_raw_version() -> *const c_char;
    fn antitone_raw_open(
        path: *const c_char,
        metadata: *mut NativeMetadata,
        error: *mut c_char,
        size: usize,
    ) -> *mut c_void;
    #[cfg(feature = "editor")]
    fn antitone_raw_thumbnail(
        handle: *mut c_void,
        thumbnail: *mut NativeThumbnail,
        error: *mut c_char,
        size: usize,
    ) -> c_int;
    #[cfg(feature = "dng-sdk")]
    fn antitone_raw_is_dng(handle: *const c_void) -> c_int;
    fn antitone_raw_unpack(
        handle: *mut c_void,
        threads: c_int,
        error: *mut c_char,
        size: usize,
    ) -> c_int;
    fn antitone_raw_sensor_diagnostics(
        handle: *const c_void,
        diagnostics: *mut SensorDiagnostics,
        error: *mut c_char,
        size: usize,
    ) -> c_int;
    fn antitone_raw_develop(
        handle: *mut c_void,
        wb: *const f64,
        output_color: c_int,
        threads: c_int,
        view: *mut NativeView,
        error: *mut c_char,
        size: usize,
    ) -> c_int;
    fn antitone_raw_region_diagnostics(
        handle: *const c_void,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        diagnostics: *mut SensorDiagnostics,
        error: *mut c_char,
        size: usize,
    ) -> c_int;
    fn antitone_raw_capture_flags(
        handle: *const c_void,
        words: *mut u64,
        count: usize,
        error: *mut c_char,
        size: usize,
    ) -> c_int;
    fn antitone_raw_retained_bytes(handle: *mut c_void) -> u64;
    fn antitone_raw_free_image(handle: *mut c_void);
    fn antitone_raw_close(handle: *mut c_void);
}

pub fn version() -> String {
    // LibRaw owns a static, NUL-terminated version string.
    unsafe { CStr::from_ptr(antitone_raw_version()) }
        .to_string_lossy()
        .into_owned()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DevelopmentPriority {
    #[default]
    Foreground,
    Background,
}

impl DevelopmentPriority {
    fn threads(self, cpus: usize, pressure: crate::resources::MemoryPressure) -> c_int {
        use crate::resources::MemoryPressure;
        let cap = match (self, pressure) {
            (_, MemoryPressure::Critical) => 1,
            (Self::Background, MemoryPressure::Warning) => 1,
            (Self::Background, MemoryPressure::Normal) => 2,
            (_, MemoryPressure::Warning) => 2,
            (Self::Foreground, MemoryPressure::Normal) => 8,
        };
        cpus.saturating_sub(2).max(1).min(cap) as c_int
    }

    /// Concurrent native owners split the reported budget; never below one thread.
    fn shared_threads(
        self,
        cpus: usize,
        pressure: crate::resources::MemoryPressure,
        share: usize,
    ) -> c_int {
        (self.threads(cpus, pressure) / share.max(1) as c_int).max(1)
    }
}

/// Mutable SDK ownership stays worker-local, unlike a consumed frozen source.
/// Concurrent use would break the bridge's lend contract; forbidding even
/// sequential thread migration is a deliberate conservative boundary, not a
/// LibRaw mandate (the pinned source uses no thread-local state). Before
/// relaxing it, audit migration safety, the lend-until-next-mutable-call
/// contract across calls, and DNG SDK/XMP transaction serialization.
/// ```compile_fail
/// fn send<T: Send>() {}
/// send::<antitone::raw::RawFile>();
/// ```
/// ```compile_fail
/// fn sync<T: Sync>() {}
/// sync::<antitone::raw::RawFile>();
/// ```
pub struct RawFile {
    handle: NonNull<c_void>,
    pub metadata: Metadata,
    unpacked: bool,
    priority: DevelopmentPriority,
    share: usize,
    diagnostics: Option<SensorDiagnostics>,
    camera_view: Option<NativeView>,
    correction: Option<crate::lens::EmbeddedLensGeometry>,
    corrected: Option<lens_source::CorrectedImage>,
    #[cfg(feature = "dng-sdk")]
    path: std::path::PathBuf,
    #[cfg(feature = "dng-sdk")]
    dng_source: Option<crate::dng::Source>,
}

impl RawFile {
    pub fn open(path: &Path) -> Result<Self, Error> {
        Self::open_mode(path, true, None)
    }

    /// Diagnostic baseline only; ordinary source acquisition always uses automatic correction.
    pub fn open_uncorrected(path: &Path) -> Result<Self, Error> {
        Self::open_mode(path, false, None)
    }

    pub(crate) fn open_with_wb(path: &Path, wb: Option<[f64; 4]>) -> Result<Self, Error> {
        Self::open_mode(path, true, wb)
    }

    fn open_mode(
        source_path: &Path,
        lens_correction: bool,
        fixed_wb: Option<[f64; 4]>,
    ) -> Result<Self, Error> {
        let path = CString::new(source_path.as_os_str().as_encoded_bytes())
            .map_err(|_| Error::Raw("source path contains NUL".into()))?;
        let mut metadata = std::mem::MaybeUninit::<NativeMetadata>::zeroed();
        let mut error = [0; 512];
        // Metadata contains only numeric fields and fixed char arrays. The bridge fills it on success.
        let handle = unsafe {
            antitone_raw_open(
                path.as_ptr(),
                metadata.as_mut_ptr(),
                error.as_mut_ptr(),
                error.len(),
            )
        };
        let handle = NonNull::new(handle).ok_or_else(|| native_error("open", &error))?;
        let mut raw = Self {
            handle,
            metadata: Metadata::from_native(unsafe { metadata.assume_init() }),
            unpacked: false,
            priority: DevelopmentPriority::default(),
            share: 1,
            diagnostics: None,
            camera_view: None,
            correction: None,
            corrected: None,
            #[cfg(feature = "dng-sdk")]
            path: source_path.to_path_buf(),
            #[cfg(feature = "dng-sdk")]
            dng_source: None,
        };
        #[cfg(feature = "dng-sdk")]
        if lens_correction && unsafe { antitone_raw_is_dng(raw.handle.as_ptr()) } != 0 {
            let header = crate::dng::Header::read(
                source_path,
                sdk_options(fixed_wb)?,
                &crate::dng::Cancellation::default(),
            )
            .map_err(|e| Error::Raw(e.to_string()))?;
            raw.set_dng_header(header);
        }
        #[cfg(not(feature = "dng-sdk"))]
        let _ = fixed_wb;
        raw.metadata.lens_disabled = u32::from(!lens_correction);
        raw.correction = raw.metadata.lens_geometry()?;
        Ok(raw)
    }

    #[cfg(feature = "editor")]
    pub(crate) fn embedded_thumbnail(&mut self) -> Result<Option<EmbeddedThumbnail>, Error> {
        let mut thumbnail = std::mem::MaybeUninit::<NativeThumbnail>::zeroed();
        let mut error = [0; 512];
        // The bridge extracts only bounded JPEG data; it does not unpack/develop the sensor.
        let code = unsafe {
            antitone_raw_thumbnail(
                self.handle.as_ptr(),
                thumbnail.as_mut_ptr(),
                error.as_mut_ptr(),
                error.len(),
            )
        };
        if code != 0 {
            return Err(native_error("embedded thumbnail", &error));
        }
        let thumbnail = unsafe { thumbnail.assume_init() };
        if thumbnail.jpeg.is_null() {
            return Ok(None);
        }
        // Copy the handle-owned bytes before another extraction or closing the unique handle.
        let jpeg = unsafe { std::slice::from_raw_parts(thumbnail.jpeg, thumbnail.bytes as usize) }
            .to_vec();
        Ok(Some(EmbeddedThumbnail {
            jpeg,
            flip: thumbnail.flip,
            color_space: thumbnail.color_space,
        }))
    }

    #[cfg(feature = "dng-sdk")]
    fn set_dng_header(&mut self, header: crate::dng::Header) {
        let [width, height] = header.dimensions;
        let [left, top] = header.active_origin;
        let [x, y, cw, ch] = header.default_crop;
        self.metadata.width = width;
        self.metadata.height = height;
        self.metadata.raw_width = header.original_dimensions[0];
        self.metadata.raw_height = header.original_dimensions[1];
        self.metadata.left_margin = left;
        self.metadata.top_margin = top;
        self.metadata.inset_left = left + x;
        self.metadata.inset_top = top + y;
        self.metadata.inset_width = cw;
        self.metadata.inset_height = ch;
        self.metadata.sdk = Some(header);
    }

    #[cfg(feature = "dng-sdk")]
    fn ensure_dng_capture(&mut self) -> Result<(), Error> {
        if let Some(header) = self.metadata.sdk
            && self.dng_source.is_none()
        {
            let [r, g, b] = header.fixed_wb;
            self.develop_camera([r, g, b, g])?;
        }
        Ok(())
    }

    #[cfg(feature = "dng-sdk")]
    fn validate_dng_region(&self, area: Rect) -> Result<(), Error> {
        if area.width == 0
            || area.height == 0
            || u64::from(area.x) + u64::from(area.width) > u64::from(self.metadata.width)
            || u64::from(area.y) + u64::from(area.height) > u64::from(self.metadata.height)
        {
            return Err(Error::Raw("Invalid SDK capture rectangle".into()));
        }
        Ok(())
    }

    pub(crate) fn prepare_basis(&mut self, requested: [f64; 4]) -> Result<(), Error> {
        #[cfg(feature = "dng-sdk")]
        if self.metadata.sdk.is_some() {
            let header = crate::dng::Header::read(
                &self.path,
                sdk_options(Some(requested))?,
                &crate::dng::Cancellation::default(),
            )
            .map_err(|e| Error::Raw(e.to_string()))?;
            self.set_dng_header(header);
        }
        #[cfg(not(feature = "dng-sdk"))]
        let _ = requested;
        Ok(())
    }

    pub(crate) fn fixed_wb(&self, requested: [f64; 4]) -> [f64; 4] {
        #[cfg(feature = "dng-sdk")]
        if let Some(header) = self.metadata.sdk {
            let [r, g, b] = header.fixed_wb;
            return [r, g, b, g];
        }
        requested
    }

    /// Changes resource scheduling only, never development settings or cached pixels.
    pub fn set_development_priority(&mut self, priority: DevelopmentPriority) {
        self.priority = priority;
    }

    /// Divides the execution budget across concurrent native owners. Resource
    /// scheduling only, exactly like [`Self::set_development_priority`].
    pub fn set_development_share(&mut self, share: usize) {
        self.share = share.max(1);
    }

    fn call_threads(&self) -> c_int {
        self.priority.shared_threads(
            std::thread::available_parallelism().map_or(1, usize::from),
            crate::resources::memory_pressure(),
            self.share,
        )
    }

    pub(crate) fn ensure_unpacked(&mut self) -> Result<(), Error> {
        #[cfg(feature = "dng-sdk")]
        if self.metadata.sdk.is_some() {
            // SDK decode/develop is one transaction in the development phase. Do not
            // unpack through LibRaw first, or admit a fallback after an SDK failure.
            return Ok(());
        }
        if self.unpacked {
            return Ok(());
        }
        let mut error = [0; 512];
        // This uniquely owned handle decodes once, retaining the original sensor snapshot.
        let code = unsafe {
            antitone_raw_unpack(
                self.handle.as_ptr(),
                self.call_threads(),
                error.as_mut_ptr(),
                error.len(),
            )
        };
        if code != 0 {
            return Err(native_error("unpack", &error));
        }
        self.unpacked = true;
        Ok(())
    }

    /// Explicit whole-sensor statistics, cached independently of decoding/development.
    pub fn unpack(&mut self) -> Result<SensorDiagnostics, Error> {
        if let Some(diagnostics) = self.diagnostics {
            return Ok(diagnostics);
        }
        self.ensure_unpacked()?;
        #[cfg(feature = "dng-sdk")]
        self.ensure_dng_capture()?;
        let diagnostics = self.sensor_diagnostics()?;
        self.diagnostics = Some(diagnostics);
        Ok(diagnostics)
    }

    fn sensor_diagnostics(&self) -> Result<SensorDiagnostics, Error> {
        #[cfg(feature = "dng-sdk")]
        if self.metadata.sdk.is_some() {
            return self.native_region_sensor_diagnostics(Rect::from([
                0,
                0,
                self.metadata.width,
                self.metadata.height,
            ]));
        }
        let mut diagnostics = SensorDiagnostics::default();
        let mut error = [0; 512];
        // No SDK calls or mutation: the bridge reads capture data into caller-owned buffers.
        let code = unsafe {
            antitone_raw_sensor_diagnostics(
                self.handle.as_ptr(),
                &mut diagnostics,
                error.as_mut_ptr(),
                error.len(),
            )
        };
        if code != 0 {
            return Err(native_error("unpack", &error));
        }
        Ok(diagnostics)
    }

    /// Detailed diagnostics in original, unwarped native coordinates.
    pub fn region_sensor_diagnostics(&mut self, area: Rect) -> Result<SensorDiagnostics, Error> {
        self.ensure_unpacked()?;
        #[cfg(feature = "dng-sdk")]
        self.ensure_dng_capture()?;
        self.native_region_sensor_diagnostics(area)
    }

    /// Capture-limit eligibility in the developed source's coordinates.
    pub fn capture_eligibility(&mut self, area: Rect) -> Result<CaptureFlags, Error> {
        self.ensure_unpacked()?;
        #[cfg(feature = "dng-sdk")]
        self.ensure_dng_capture()?;
        self.unpacked_capture_eligibility(area)
    }

    fn unpacked_capture_eligibility(&self, area: Rect) -> Result<CaptureFlags, Error> {
        #[cfg(feature = "dng-sdk")]
        if let Some(source) = &self.dng_source {
            self.validate_dng_region(area)?;
            let mut flags = CaptureFlags::default();
            for y in area.y..area.y + area.height {
                for x in area.x..area.x + area.width {
                    let state = source.capture_state(x, y).unwrap();
                    flags.at_white |= state.original_white();
                    flags.below_black |= state.original_black();
                    flags.repeated_border |= state.repeated_border();
                    flags.processing_limit |= state.processing_limit();
                }
            }
            return Ok(flags);
        }
        if let Some(image) = &self.corrected {
            return image.eligibility.flags(area);
        }
        if self.correction.is_some() {
            return Err(Error::Raw(
                "corrected capture eligibility needs camera development".into(),
            ));
        }
        self.native_region_sensor_diagnostics(area).map(Into::into)
    }

    fn capture_mask(&self) -> Result<eligibility::EligibilityMask, Error> {
        let mut mask =
            eligibility::EligibilityMask::new([self.metadata.width, self.metadata.height])?;
        let mut error = [0; 512];
        let words = mask.words_mut();
        // Const capture reads only; the caller owns all storage and the original levels remain intact.
        let code = unsafe {
            antitone_raw_capture_flags(
                self.handle.as_ptr(),
                words.as_mut_ptr(),
                words.len(),
                error.as_mut_ptr(),
                error.len(),
            )
        };
        if code != 0 {
            return Err(native_error("capture mask", &error));
        }
        Ok(mask)
    }

    fn native_region_sensor_diagnostics(&self, area: Rect) -> Result<SensorDiagnostics, Error> {
        #[cfg(feature = "dng-sdk")]
        if let Some(source) = &self.dng_source {
            self.validate_dng_region(area)?;
            let header = source.header();
            let [left, top] = header.active_origin;
            let [raw_width, _] = header.original_dimensions;
            let [black, white] = header.capture_limits;
            let mut diagnostics = SensorDiagnostics::default();
            for y in area.y..area.y + area.height {
                for x in area.x..area.x + area.width {
                    let (sx, sy) = (x + left, y + top);
                    let color = (header.cfa_pattern >> (8 * ((sy % 2) * 2 + sx % 2))) & 255;
                    let c = if color == 1 && sy % 2 == 1 {
                        3
                    } else {
                        color as usize
                    };
                    let value =
                        source.original_cfa()[(sy as usize * raw_width as usize) + sx as usize];
                    diagnostics.sites[c] += 1;
                    diagnostics.below_black[c] += u64::from(f64::from(value) <= black);
                    diagnostics.at_white[c] += u64::from(f64::from(value) >= white);
                    diagnostics.peak[c] = diagnostics.peak[c].max(u32::from(value));
                }
            }
            return Ok(diagnostics);
        }
        let mut diagnostics = SensorDiagnostics::default();
        let mut error = [0; 512];
        // The bridge reads the immutable RAW snapshot, including its original CFA and black/white levels.
        let code = unsafe {
            antitone_raw_region_diagnostics(
                self.handle.as_ptr(),
                area.x,
                area.y,
                area.width,
                area.height,
                &mut diagnostics,
                error.as_mut_ptr(),
                error.len(),
            )
        };
        if code != 0 {
            return Err(native_error("sensor region", &error));
        }
        Ok(diagnostics)
    }

    pub fn develop_camera(&mut self, fixed_wb: [f64; 4]) -> Result<Developed<'_>, Error> {
        self.develop(fixed_wb, 0)
    }

    pub fn develop_srgb_reference(&mut self, fixed_wb: [f64; 4]) -> Result<Developed<'_>, Error> {
        self.develop(fixed_wb, 1)
    }

    fn develop(&mut self, fixed_wb: [f64; 4], output_color: c_int) -> Result<Developed<'_>, Error> {
        #[cfg(feature = "dng-sdk")]
        if self.metadata.sdk.is_some() {
            if output_color != 0 {
                return Err(Error::Raw(
                    "SDK camera development is not a LibRaw sRGB reference".into(),
                ));
            }
            let source = crate::dng::Source::develop_with_threads(
                &self.path,
                sdk_options(Some(fixed_wb))?,
                &crate::dng::Cancellation::default(),
                self.call_threads() as u32,
            )
            .map_err(|e| Error::Raw(e.to_string()))?;
            log::debug!(
                size:? = source.dimensions(),
                priority:? = self.priority,
                timings:? = source.timings();
                "dng_development"
            );

            self.set_dng_header(source.header());
            self.dng_source = Some(source);
            self.unpacked = true;
            return self.camera_image();
        }
        if fixed_wb
            .iter()
            .any(|v| !v.is_finite() || *v <= 0.0 || *v > f32::MAX as f64)
        {
            return Err(Error::Raw(
                "roll WB gains must be positive, finite f32 values".into(),
            ));
        }
        let started = std::time::Instant::now();
        self.ensure_unpacked()?;
        let threads = self.call_threads();
        let unpack_seconds = started.elapsed().as_secs_f64();
        let tick = std::time::Instant::now();
        self.camera_view = None;
        self.corrected = None;
        let mut view = std::mem::MaybeUninit::<NativeView>::zeroed();
        let mut error = [0; 512];
        // A successful bridge call lends its image until the next mutable call or handle destruction.
        let code = unsafe {
            antitone_raw_develop(
                self.handle.as_ptr(),
                fixed_wb.as_ptr(),
                output_color,
                threads,
                view.as_mut_ptr(),
                error.as_mut_ptr(),
                error.len(),
            )
        };
        if code != 0 {
            return Err(native_error("develop", &error));
        }
        let view = unsafe { view.assume_init() };
        log::debug!(
            width = view.width,
            height = view.height,
            priority:? = self.priority,
            threads,
            unpack_seconds,
            process_seconds = tick.elapsed().as_secs_f64();
            "raw_development"
        );

        if output_color == 0 {
            if let Some(geometry) = &self.correction {
                let started = std::time::Instant::now();
                let support = self.capture_mask()?.ahd_support()?;
                let original = self.borrow_view(view)?;
                let corrected = lens_source::reconstruct(
                    original.pixels,
                    [view.width, view.height],
                    geometry,
                    &support,
                    threads as usize,
                )?;
                // The immutable f32 raster replaces RGB16; keep original CFA, not a redundant RGB copy.
                unsafe { antitone_raw_free_image(self.handle.as_ptr()) };
                self.camera_view = Some(NativeView {
                    pixels: std::ptr::null(),
                    ..view
                });
                log::debug!(
                    width = corrected.size[0],
                    height = corrected.size[1],
                    reconstruction_seconds = started.elapsed().as_secs_f64(),
                    corrected_bytes = corrected.pixels.capacity() * size_of::<[f32; 3]>(),
                    eligibility_bytes = corrected.eligibility.retained_bytes(),
                    native_image_retained = false;
                    "embedded_lens_development"
                );

                self.corrected = Some(corrected);
            } else {
                self.camera_view = Some(view);
            }
            return self.camera_image();
        }
        self.borrow_view(view)
    }

    pub(crate) fn freeze(self) -> Result<FrozenRaw, Error> {
        if !self.unpacked {
            return Err(Error::Raw("cannot freeze a source before unpacking".into()));
        }
        let image = self.camera_image()?;
        if [image.width, image.height] != self.metadata.source_dimensions()? {
            return Err(Error::Raw(
                "camera development dimensions disagree with metadata".into(),
            ));
        }
        // Unique ownership still holds here; snapshot before immutable publication.
        let bytes = unsafe { antitone_raw_retained_bytes(self.handle.as_ptr()) };
        let extra = (self
            .corrected
            .as_ref()
            .map_or(0, |image| image.retained_bytes())
            + self
                .correction
                .as_ref()
                .map_or(0, |geometry| geometry.retained_bytes())) as u64;
        #[cfg(feature = "dng-sdk")]
        let extra = extra
            .checked_add(
                self.dng_source
                    .as_ref()
                    .map_or(0, crate::dng::Source::retained_bytes),
            )
            .ok_or(Error::NumericalOverflow)?;
        Ok(FrozenRaw {
            raw: self,
            retained_bytes: (bytes != 0).then(|| bytes.checked_add(extra)).flatten(),
        })
    }

    /// Borrows the cached camera development without rerunning LibRaw.
    pub fn camera_image(&self) -> Result<Developed<'_>, Error> {
        #[cfg(feature = "dng-sdk")]
        if let Some(source) = &self.dng_source {
            let [width, height] = source.dimensions();
            let [r, g, b] = source.fixed_wb();
            return Ok(Developed {
                pixels: &[],
                continuous: Some(source.camera_rgb()),
                continuous_scale: 65535.,
                width,
                height,
                maximum: 65535,
                data_maximum: 65535,
                warnings: 0,
                pre_mul: [r, g, b, g],
                _owner: PhantomData,
            });
        }
        if let Some(image) = &self.corrected {
            let view = self
                .camera_view
                .ok_or_else(|| Error::Raw("camera development is not available".into()))?;
            return Ok(Developed {
                pixels: &[],
                continuous: Some(&image.pixels),
                continuous_scale: 1.,
                width: image.size[0],
                height: image.size[1],
                maximum: view.maximum,
                data_maximum: view.data_maximum,
                warnings: view.warnings,
                pre_mul: view.pre_mul,
                _owner: PhantomData,
            });
        }
        self.borrow_view(
            self.camera_view
                .ok_or_else(|| Error::Raw("camera development is not available".into()))?,
        )
    }

    fn borrow_view(&self, view: NativeView) -> Result<Developed<'_>, Error> {
        let count = (view.width as usize)
            .checked_mul(view.height as usize)
            .and_then(|v| v.checked_mul(4))
            .filter(|v| *v > 0 && *v <= isize::MAX as usize / size_of::<u16>())
            .ok_or_else(|| Error::Raw("invalid developed-image dimensions".into()))?;
        if view.pixels.is_null() {
            return Err(Error::Raw("missing developed pixels".into()));
        }
        Ok(Developed {
            pixels: unsafe { std::slice::from_raw_parts(view.pixels, count) },
            continuous: None,
            continuous_scale: 1.,
            width: view.width,
            height: view.height,
            maximum: view.maximum,
            data_maximum: view.data_maximum,
            warnings: view.warnings,
            pre_mul: view.pre_mul,
            _owner: PhantomData,
        })
    }
}

impl Drop for RawFile {
    fn drop(&mut self) {
        // This is the handle's sole owner; all image borrows end before drop.
        unsafe { antitone_raw_close(self.handle.as_ptr()) };
    }
}

/// Private, consumed native ownership with no path back to mutable SDK access.
pub(crate) struct FrozenRaw {
    raw: RawFile,
    retained_bytes: Option<u64>,
}

// SAFETY: construction consumes the unique RawFile after successful unpack/camera
// development and view validation. No method exposes RawFile, its handle or mutable
// SDK access. RGB borrows keep this owner alive; diagnostics only read retained CFA
// through const bridge loops, with per-call output/error buffers and no LibRaw calls.
// Safe Rust publication provides synchronization. The pinned libraw_r instance owns
// its unpacker/TLS data; destruction frees instance-owned resources without creator-
// thread affinity, and can occur only after the last owner/borrow has retired.
unsafe impl Send for FrozenRaw {}

unsafe impl Sync for FrozenRaw {}

impl FrozenRaw {
    pub(crate) fn retained_bytes(&self) -> Option<u64> {
        self.retained_bytes
    }

    pub(crate) fn metadata(&self) -> Metadata {
        self.raw.metadata
    }

    pub(crate) fn camera_image(&self) -> Result<Developed<'_>, Error> {
        self.raw.camera_image()
    }

    pub(crate) fn sensor_diagnostics(&self) -> Result<SensorDiagnostics, Error> {
        self.raw
            .diagnostics
            .map_or_else(|| self.raw.sensor_diagnostics(), Ok)
    }

    pub(crate) fn region_sensor_diagnostics(&self, area: Rect) -> Result<SensorDiagnostics, Error> {
        self.raw.native_region_sensor_diagnostics(area)
    }

    pub(crate) fn capture_eligibility(&self, area: Rect) -> Result<CaptureFlags, Error> {
        self.raw.unpacked_capture_eligibility(area)
    }
}

pub struct Developed<'a> {
    pixels: &'a [u16],
    continuous: Option<&'a [[f32; 3]]>,
    continuous_scale: f32,
    pub width: u32,
    pub height: u32,
    pub maximum: u32,
    pub data_maximum: u32,
    pub warnings: u32,
    pub pre_mul: [f64; 4],
    _owner: PhantomData<&'a RawFile>,
}

impl Developed<'_> {
    pub fn is_continuous(&self) -> bool {
        self.continuous.is_some()
    }

    pub fn sample_mean(&self, area: Rect, matrix: DMat3) -> Result<DVec3, Error> {
        if area.width == 0
            || area.height == 0
            || area
                .x
                .checked_add(area.width)
                .is_none_or(|v| v > self.width)
            || area
                .y
                .checked_add(area.height)
                .is_none_or(|v| v > self.height)
        {
            return Err(Error::Raw(
                "sampling rectangle outside developed image".into(),
            ));
        }
        let mut rows = Vec::with_capacity(area.height as usize);
        for y in area.y..area.y + area.height {
            let mut row = Vec::with_capacity(area.width as usize);
            for x in area.x..area.x + area.width {
                row.push(matrix * self.pixel(x, y)?);
            }
            rows.push(mean(&row)?);
        }
        mean(&rows)
    }

    pub fn pixel_codes(&self, x: u32, y: u32) -> Result<[u16; 4], Error> {
        if self.continuous.is_some() {
            return Err(Error::Raw(
                "corrected camera samples are continuous, not RGB16 codes".into(),
            ));
        }
        if x >= self.width || y >= self.height {
            return Err(Error::Raw("pixel outside developed image".into()));
        }
        let offset = (y as usize * self.width as usize + x as usize) * 4;
        Ok(self.pixels[offset..offset + 4].try_into().unwrap())
    }

    /// Camera values on the shared 65535-unit scale; integer samples convert exactly to f32.
    pub fn pixel_samples(&self, x: u32, y: u32) -> Result<[f32; 4], Error> {
        if x >= self.width || y >= self.height {
            return Err(Error::Raw("pixel outside developed image".into()));
        }
        if let Some(pixels) = self.continuous {
            let [r, g, b] = pixels[y as usize * self.width as usize + x as usize];
            return Ok([
                r * self.continuous_scale,
                g * self.continuous_scale,
                b * self.continuous_scale,
                0.,
            ]);
        }
        Ok(self.pixel_codes(x, y)?.map(f32::from))
    }

    pub fn pixel(&self, x: u32, y: u32) -> Result<DVec3, Error> {
        let [r, g, b, _] = self.pixel_samples(x, y)?;
        Ok(DVec3::new(r as f64, g as f64, b as f64) / 65535.0)
    }
}

#[cfg(feature = "dng-sdk")]
fn sdk_options(wb: Option<[f64; 4]>) -> Result<crate::dng::Options, Error> {
    if let Some(wb) = wb
        && (wb.iter().any(|v| !v.is_finite() || *v <= 0.) || wb[1] != wb[3])
    {
        return Err(Error::Raw(
            "SDK development needs one positive gain per RGB channel; green gains must match"
                .into(),
        ));
    }
    Ok(crate::dng::Options {
        max_pixels: u32::MAX,
        fixed_wb: wb.map(|[r, g, b, _]| [r, g, b]),
    })
}

fn ffi_string(bytes: &[c_char]) -> String {
    // The bridge always NUL-terminates fixed-size text fields and error buffers.
    unsafe { CStr::from_ptr(bytes.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

fn native_error(operation: &str, error: &[c_char]) -> Error {
    let message = ffi_string(error);
    log::warn!(
        operation,
        error = message.as_str();
        "raw_failed"
    );
    Error::Raw(format!("{operation}: {message}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorded_orientations_seed_every_tiff_case_as_turns_and_flips() {
        use crate::geometry::{Flips, PhotoOrientation, QuarterTurns};
        const MIRROR: Flips = Flips {
            horizontal: true,
            vertical: false,
        };
        let identity = PhotoOrientation {
            turns: QuarterTurns::Zero,
            flips: Flips::default(),
        };
        let turned = |turns| PhotoOrientation {
            turns,
            flips: Flips::default(),
        };
        let mirrored = |turns| PhotoOrientation {
            turns,
            flips: MIRROR,
        };
        for (tiff, flip, expected) in [
            (1u32, 0i32, identity),
            (2, 1, mirrored(QuarterTurns::Zero)),
            (3, 3, turned(QuarterTurns::Two)),
            (
                4,
                2,
                PhotoOrientation {
                    turns: QuarterTurns::Zero,
                    flips: Flips {
                        horizontal: false,
                        vertical: true,
                    },
                },
            ),
            (5, 4, mirrored(QuarterTurns::One)),
            (6, 6, turned(QuarterTurns::One)),
            (7, 7, mirrored(QuarterTurns::Three)),
            (8, 5, turned(QuarterTurns::Three)),
        ] {
            assert_eq!(
                photo_orientation_from_tiff(tiff).unwrap(),
                expected,
                "tiff {tiff}"
            );
            assert_eq!(
                photo_orientation_from_libraw_flip(flip).unwrap(),
                expected,
                "flip {flip}"
            );
        }
        assert!(photo_orientation_from_tiff(0).is_err());
        assert!(photo_orientation_from_tiff(9).is_err());
        assert!(photo_orientation_from_libraw_flip(-1).is_err());
        assert!(photo_orientation_from_libraw_flip(8).is_err());
    }

    /// Seeded orientation must display exactly what LibRaw's own writer emits with
    /// `user_flip` honoured (`LibRaw::flip_index` in file_write.cpp): the oriented
    /// pixel (x, y) reads the native sensor pixel flip_index returns for (row=y, col=x).
    #[test]
    fn seeded_orientation_displays_libraw_flip_index_pixels() {
        use crate::geometry::{Orientation, OrientedPoint, RollOrientation, SourceRegion};
        let area = SourceRegion::from([3, 5, 52, 34]);
        for flip in 0..=7 {
            let seed = photo_orientation_from_libraw_flip(flip).unwrap();
            let orientation = Orientation::new(area, RollOrientation::default(), seed).unwrap();
            let [width, height] = [area.width, area.height];
            let [displayed_width, displayed_height] = orientation.size();
            for (col, row) in [
                (0, 0),
                (0, 3),
                (17, 0),
                (displayed_width - 1, displayed_height - 1),
                (displayed_width / 2, displayed_height / 2),
            ] {
                let mut source_row = row;
                let mut source_col = col;
                if flip & 4 != 0 {
                    std::mem::swap(&mut source_row, &mut source_col);
                }
                if flip & 2 != 0 {
                    source_row = height - 1 - source_row;
                }
                if flip & 1 != 0 {
                    source_col = width - 1 - source_col;
                }
                let expected = source_row * width + source_col;
                // Oriented size swaps axes exactly when the seed turns a quarter.
                let oriented = orientation.oriented_to_source(OrientedPoint([
                    f64::from(col) + 0.5,
                    f64::from(row) + 0.5,
                ]));
                let local_x = oriented.0[0] - f64::from(area.x);
                let local_y = oriented.0[1] - f64::from(area.y);
                assert_eq!(
                    (local_y.floor() * f64::from(width) + local_x.floor()) as i64,
                    i64::from(expected),
                    "flip {flip} at ({col}, {row})"
                );
            }
        }
    }

    fn metadata_fixture() -> Metadata {
        // All fields are numeric or char arrays, so zero is a valid fixture representation.
        let mut m = Metadata::from_native(unsafe { std::mem::zeroed() });
        m.width = 8704;
        m.height = 6144;
        m.inset_left = 12;
        m.inset_top = 12;
        m.inset_width = 8640;
        m.inset_height = 5760;
        m.rgb_cam = DMat3::IDENTITY.to_cols_array();
        m
    }

    #[test]
    fn as_shot_wb_aliases_only_an_unspecified_second_green_on_three_color_sources() {
        let mut m = metadata_fixture();
        m.colors = 3;
        m.cam_mul = [2., 1.25, 1.5, 0.];
        assert_eq!(m.as_shot_wb(), [2., 1.25, 1.5, 1.25]);
        assert_eq!(m.cam_mul, [2., 1.25, 1.5, 0.]);
        for gain in [0.75, -1., f64::NAN, f64::INFINITY] {
            m.cam_mul[3] = gain;
            assert_eq!(
                m.as_shot_wb().map(f64::to_bits),
                m.cam_mul.map(f64::to_bits)
            );
        }
        m.colors = 4;
        m.cam_mul[3] = 0.;
        assert_eq!(m.as_shot_wb(), m.cam_mul);
        m.colors = 3;
        for channel in 0..3 {
            for gain in [0., -1., f64::NAN, f64::INFINITY] {
                m.cam_mul = [2., 1.25, 1.5, 0.];
                m.cam_mul[channel] = gain;
                let wb = m.as_shot_wb();
                assert_eq!(wb[channel].to_bits(), gain.to_bits());
                assert_eq!(wb[3].to_bits(), m.cam_mul[1].to_bits());
            }
        }
    }

    #[cfg(feature = "editor")]
    #[test]
    fn authored_dng_thumbnail_extraction_keeps_sensor_and_development_untouched() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw");
        let path = root.join("preview.dng");
        let original = std::fs::read(&path).unwrap();
        let mut raw = RawFile::open(&path).unwrap();
        let thumbnail = raw.embedded_thumbnail().unwrap().unwrap();
        assert_eq!((thumbnail.flip, thumbnail.color_space), (6, 1));
        assert!(!raw.unpacked);
        assert!(raw.diagnostics.is_none());
        assert!(raw.camera_image().is_err());
        assert_eq!(
            raw.embedded_thumbnail().unwrap().unwrap().jpeg,
            thumbnail.jpeg
        );
        let before = raw.unpack().unwrap();
        let mut plain = RawFile::open(&root.join("patches.dng")).unwrap();
        let expected = plain.develop_camera([1.; 4]).unwrap();
        let actual = raw.develop_camera([1.; 4]).unwrap();
        for y in 0..192 {
            for x in 0..256 {
                assert_eq!(
                    actual.pixel_samples(x, y).unwrap(),
                    expected.pixel_samples(x, y).unwrap()
                );
            }
        }
        assert_eq!(raw.unpack().unwrap(), before);
        assert_eq!(
            raw.embedded_thumbnail().unwrap().unwrap().jpeg,
            thumbnail.jpeg
        );
        drop(raw);
        assert_eq!(
            image::load_from_memory(&thumbnail.jpeg)
                .unwrap()
                .to_rgb8()
                .dimensions(),
            (16, 8)
        );
        assert_eq!(std::fs::read(path).unwrap(), original);
    }

    #[cfg(feature = "editor")]
    #[test]
    fn authored_jpeg_selection_prefers_the_smallest_sufficient_preview_or_largest_tiny_fallback() {
        fn u16_at(bytes: &[u8], offset: usize) -> u16 {
            u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
        }
        fn u32_at(bytes: &[u8], offset: usize) -> u32 {
            u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
        }
        for (dimensions, expected) in [
            (vec![[512, 344], [160, 120], [320, 216]], [320, 216]),
            (vec![[320, 216], [512, 344]], [320, 216]),
            (vec![[2049, 64], [160, 120]], [160, 120]),
            (vec![], [16, 8]),
        ] {
            let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/preview.dng");
            let mut bytes = std::fs::read(path).unwrap();
            // Chain additional authored JPEG IFDs after the fixture's tiny preview;
            // all original sensor data, metadata and absolute TIFF payload offsets stay intact.
            let root = u32_at(&bytes, 4) as usize;
            let root_link = root + 2 + 12 * usize::from(u16_at(&bytes, root));
            let preview = u32_at(&bytes, root_link) as usize;
            let fields = usize::from(u16_at(&bytes, preview));
            let ifd_bytes = 2 + 12 * fields + 4;
            let template = bytes[preview..preview + ifd_bytes].to_vec();
            let mut link = preview + 2 + 12 * fields;
            for [width, height] in dimensions {
                let mut jpeg = Vec::new();
                image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
                    .encode(
                        &vec![127; width as usize * height as usize * 3],
                        width,
                        height,
                        image::ExtendedColorType::Rgb8,
                    )
                    .unwrap();
                if !bytes.len().is_multiple_of(2) {
                    bytes.push(0);
                }
                let start = bytes.len();
                bytes[link..link + 4].copy_from_slice(&(start as u32).to_le_bytes());
                bytes.extend_from_slice(&template);
                for index in 0..fields {
                    let offset = start + 2 + index * 12;
                    let value = match u16_at(&bytes, offset) {
                        256 => width,
                        257 => height,
                        513 => (start + ifd_bytes) as u32,
                        514 => jpeg.len() as u32,
                        _ => continue,
                    };
                    bytes[offset + 8..offset + 12].copy_from_slice(&value.to_le_bytes());
                }
                bytes.extend_from_slice(&jpeg);
                link = start + 2 + 12 * fields;
            }
            let file = tempfile::Builder::new().suffix(".dng").tempfile().unwrap();
            std::fs::write(file.path(), bytes).unwrap();
            let mut raw = RawFile::open(file.path()).unwrap();
            let thumbnail = raw.embedded_thumbnail().unwrap().unwrap();
            let image = image::load_from_memory(&thumbnail.jpeg).unwrap();
            assert_eq!([image.width(), image.height()], expected);
            assert!(!raw.unpacked);
            assert!(raw.camera_image().is_err());
        }
    }

    #[test]
    fn bridge_layout_matches_native_static_assertions() {
        assert_eq!(size_of::<NativeMetadata>(), 656);
        assert_eq!(size_of::<SonyLensData>(), 160);
        assert_eq!(size_of::<SensorDiagnostics>(), 112);
        assert_eq!(size_of::<NativeView>(), 64);
        #[cfg(feature = "editor")]
        assert_eq!(size_of::<NativeThumbnail>(), 32);
    }

    #[cfg(feature = "project")]
    #[test]
    fn native_capture_flags_match_thresholds_and_survive_development() {
        let file = fixture::bayer_dng();
        let mut bytes = std::fs::read(file.path()).unwrap();
        let offset = bytes.len() - fixture::SIDE as usize * fixture::SIDE as usize * 2;
        let sites = [
            (0u32, 0u32, 0u16),
            (31, 1, 65535),
            (32, 0, 0),
            (63, 63, 65535),
        ];
        for (x, y, value) in sites {
            let start = offset + (y * fixture::SIDE + x) as usize * 2;
            bytes[start..start + 2].copy_from_slice(&value.to_le_bytes());
        }
        std::fs::write(file.path(), bytes).unwrap();
        let mut raw = RawFile::open_uncorrected(file.path()).unwrap();
        raw.ensure_unpacked().unwrap();
        let before = raw.capture_mask().unwrap();
        for y in 0..fixture::SIDE {
            for x in 0..fixture::SIDE {
                let area = Rect::from([x, y, 1, 1]);
                let expected = CaptureFlags::from(raw.region_sensor_diagnostics(area).unwrap());
                assert_eq!(before.flags(area).unwrap(), expected);
                let value = sites
                    .iter()
                    .find(|&&(sx, sy, _)| sx == x && sy == y)
                    .map(|&(_, _, value)| value);
                assert_eq!(expected.at_white, value == Some(65535));
                assert_eq!(expected.below_black, value == Some(0));
            }
        }
        raw.develop_camera([1.; 4]).unwrap();
        let after = raw.capture_mask().unwrap();
        for (x, y, _) in sites {
            assert_eq!(before.at(x, y), after.at(x, y));
        }
    }

    #[cfg(feature = "editor")]
    #[test]
    #[ignore = "read-only local RAW embedded extraction and unchanged camera development"]
    fn embedded_jpeg_does_not_unpack_or_change_camera_development() {
        let path = Path::new("samples/DSC04378.ARW");
        let sample = |extract: bool| {
            let mut raw = RawFile::open(path).unwrap();
            let metadata = raw.metadata;
            if extract {
                let jpeg = raw.embedded_thumbnail().unwrap().unwrap();
                assert!(jpeg.jpeg.starts_with(&[0xff, 0xd8]));
                println!(
                    "embedded_native bytes={} flip={} color_space={}",
                    jpeg.jpeg.len(),
                    jpeg.flip,
                    jpeg.color_space
                );
                assert!(!raw.unpacked && raw.camera_view.is_none() && raw.diagnostics.is_none());
                assert_eq!(format!("{:?}", raw.metadata), format!("{metadata:?}"));
            }
            let image = raw.develop_camera(metadata.cam_mul).unwrap();
            let coordinates = [(0, 0), (100, 100), (1000, 1000), (4000, 2600), (8000, 5500)];
            coordinates.map(|(x, y)| image.pixel_codes(x, y).unwrap())
        };
        assert_eq!(sample(true), sample(false));
    }

    #[test]
    fn metadata_inset_is_not_a_user_crop_and_invalid_geometry_is_rejected() {
        let mut m = metadata_fixture();
        assert_eq!(
            m.image_area().unwrap(),
            Rect {
                x: 12,
                y: 12,
                width: 8640,
                height: 5760
            }
        );
        m.left_margin = 4;
        assert_eq!(m.image_area().unwrap().x, 8);
        m.inset_left = 0;
        assert!(m.image_area().is_err());
        m.inset_width = 0;
        m.inset_height = 0;
        assert_eq!(
            m.image_area().unwrap(),
            Rect {
                x: 0,
                y: 0,
                width: 8704,
                height: 6144
            }
        );
    }

    #[test]
    fn d65_to_d60_preserves_neutral_and_rejects_missing_camera_matrix() {
        let mut m = metadata_fixture();
        let neutral = m.camera_to_ap1().unwrap() * DVec3::ONE;
        assert!((neutral - DVec3::ONE).abs().max_element() < 2e-7);
        m.rgb_cam = [0.0; 9];
        assert!(m.camera_to_ap1().is_err());
    }

    #[test]
    fn region_sampling_reads_every_source_pixel() {
        let pixels: Vec<u16> = (0..16)
            .flat_map(|v| [v * 1000, v * 2000, v * 3000, 0])
            .collect();
        let developed = Developed {
            pixels: &pixels,
            continuous: None,
            continuous_scale: 1.,
            width: 4,
            height: 4,
            maximum: 0,
            data_maximum: 0,
            warnings: 0,
            pre_mul: [1.0; 4],
            _owner: PhantomData,
        };
        let region = Rect {
            x: 1,
            y: 1,
            width: 2,
            height: 2,
        };
        let actual = developed.sample_mean(region, DMat3::IDENTITY).unwrap();
        let expected = DVec3::new(7500.0, 15000.0, 22500.0) / 65535.0;
        assert!((actual - expected).abs().max_element() < 1e-12);
        assert!(
            developed
                .sample_mean(Rect { width: 0, ..region }, DMat3::IDENTITY)
                .is_err()
        );
        assert_eq!(
            developed.pixel_codes(1, 1).unwrap(),
            [5000, 10000, 15000, 0]
        );
        assert!(developed.pixel_codes(4, 0).is_err());
        assert!(developed.pixel(4, 0).is_err());
    }

    #[test]
    fn development_threads_are_bounded_and_leave_foreground_capacity() {
        use crate::resources::MemoryPressure::*;
        for cpus in 0..=128 {
            for priority in [
                DevelopmentPriority::Foreground,
                DevelopmentPriority::Background,
            ] {
                for pressure in [Normal, Warning, Critical] {
                    let count = priority.threads(cpus, pressure);
                    assert!((1..=8).contains(&count));
                    assert!(count as usize <= cpus.saturating_sub(2).max(1));
                }
            }
        }
        assert_eq!(DevelopmentPriority::Foreground.threads(18, Normal), 8);
        assert_eq!(DevelopmentPriority::Foreground.threads(18, Warning), 2);
        assert_eq!(DevelopmentPriority::Background.threads(18, Normal), 2);
        assert_eq!(DevelopmentPriority::Background.threads(18, Warning), 1);
        assert_eq!(DevelopmentPriority::Foreground.threads(18, Critical), 1);
        // Concurrent owners split the budget without ever dropping below one thread.
        assert_eq!(
            DevelopmentPriority::Foreground.shared_threads(18, Normal, 1),
            8
        );
        assert_eq!(
            DevelopmentPriority::Foreground.shared_threads(18, Normal, 2),
            4
        );
        assert_eq!(
            DevelopmentPriority::Foreground.shared_threads(18, Critical, 2),
            1
        );
        for share in 0..=8 {
            for pressure in [Normal, Warning, Critical] {
                let count = DevelopmentPriority::Foreground.shared_threads(18, pressure, share);
                assert!((1..=8).contains(&count));
            }
        }
    }

    #[test]
    #[ignore = "read-only local Canon/Nikon/Olympus/Panasonic RAW samples; decode, development and eligibility invariants"]
    fn vendor_samples_develop_through_the_accepted_camera_domain() {
        // Three-colour integer Bayer bodies across four vendors and every accepted
        // container variant: 12-bit compressed NEF, Olympus hi-res ORF, Panasonic
        // crop modes and both current RW2 decoders, plus actual camera-scanned negatives.
        let samples = [
            (
                "samples/raw-downloads/canon/5dmk2-raw.CR2",
                ("Canon", "EOS 5D Mark II", 14),
            ),
            (
                "samples/raw-downloads/canon/eosr-raw-iso100.CR3",
                ("Canon", "EOS R", 14),
            ),
            (
                "samples/raw-downloads/canon/eosr-craw-iso100.CR3",
                ("Canon", "EOS R", 14),
            ),
            (
                "samples/raw-downloads/canon/r6-raw-nocrop-nodual.CR3",
                ("Canon", "EOS R6", 14),
            ),
            (
                "samples/raw-downloads/nikon/d750-lossless-14bit.NEF",
                ("Nikon", "D750", 14),
            ),
            (
                "samples/raw-downloads/nikon/d750-compressed-12bit.NEF",
                ("Nikon", "D750", 12),
            ),
            (
                "samples/raw-downloads/nikon/d850-lossless-14bit.NEF",
                ("Nikon", "D850", 14),
            ),
            (
                "samples/raw-downloads/nikon/z7-lossless-14bit.NEF",
                ("Nikon", "Z 7", 14),
            ),
            (
                "samples/raw-downloads/negative-scans/negadoctor-challenge-canondslr.CR2",
                ("Canon", "EOS 600D", 14),
            ),
            (
                "samples/raw-downloads/negative-scans/negadoctor-snowy-day-d780.NEF",
                ("Nikon", "D780", 14),
            ),
            (
                "samples/raw-downloads/olympus/em1-16mp-12bit.ORF",
                ("Olympus", "E-M1", 12),
            ),
            (
                "samples/raw-downloads/olympus/em1m3-standard-20mp.ORF",
                ("Olympus", "E-M1MarkIII", 12),
            ),
            (
                "samples/raw-downloads/olympus/em1x-20mp.ORF",
                ("Olympus", "E-M1X", 12),
            ),
            (
                "samples/raw-downloads/panasonic/gh4-standard-16mp.RW2",
                ("Panasonic", "DMC-GH4", 12),
            ),
            (
                "samples/raw-downloads/panasonic/g9-standard-20mp.RW2",
                ("Panasonic", "DC-G9", 12),
            ),
            (
                "samples/raw-downloads/panasonic/gh5-standard-20mp.RW2",
                ("Panasonic", "DC-GH5", 12),
            ),
            (
                "samples/raw-downloads/panasonic/s5-fullframe.RW2",
                ("Panasonic", "DC-S5", 14),
            ),
            (
                "samples/raw-downloads/panasonic/gh6-25mp.RW2",
                ("Panasonic", "DC-GH6", 16),
            ),
            (
                "samples/raw-downloads/panasonic/s5m2-fullframe.RW2",
                ("Panasonic", "DC-S5M2", 14),
            ),
            (
                "samples/raw-downloads/panasonic/g9m2-25mp.RW2",
                ("Panasonic", "DC-G9M2", 16),
            ),
        ];
        for (path, (make, model, bps)) in samples {
            let mut raw = RawFile::open(Path::new(path)).unwrap();
            let metadata = raw.metadata;
            assert_eq!(metadata.make(), *make, "{path}");
            assert_eq!(metadata.model(), *model, "{path}");
            assert_eq!(metadata.raw_bps, bps, "{path}");
            // The acceptance domain: three-colour integer Bayer, no repeated black pattern.
            assert_eq!(metadata.colors, 3);
            assert_ne!(metadata.filters, 0);
            assert_eq!(
                metadata.vendor.sony_lens.distortion_status(),
                SonyLensDataStatus::Absent,
                "{path}"
            );
            let wb = metadata.as_shot_wb();
            assert!(wb.iter().all(|v| v.is_finite() && *v > 0.), "{path}");
            let area = metadata.image_area().unwrap();
            assert!(area.width > 0 && area.height > 0, "{path}");
            // Development is repeatable and keeps the metadata-declared area.
            let first = raw.develop_camera(wb).unwrap();
            assert_eq!(
                [first.width, first.height],
                metadata.source_dimensions().unwrap(),
                "{path}"
            );
            let picks = [
                (area.x, area.y),
                (area.x + area.width / 2, area.y + area.height / 2),
                (area.x + area.width - 1, area.y + area.height - 1),
            ];
            let expected: Vec<_> = picks
                .into_iter()
                .map(|(x, y)| first.pixel_samples(x, y).unwrap())
                .collect();
            let second = raw.develop_camera(wb).unwrap();
            for ((x, y), value) in picks.into_iter().zip(expected) {
                assert_eq!(second.pixel_samples(x, y).unwrap(), value, "{path}");
            }
            // Eligibility of the whole active area matches its sensor diagnostics.
            let flags = raw.capture_eligibility(area).unwrap();
            let diagnostics = raw.region_sensor_diagnostics(area).unwrap();
            assert_eq!(flags, CaptureFlags::from(diagnostics), "{path}");
        }
        // sRAW/mRAW is decimated pre-demosaiced output, not CFA sensor data: rejected.
        assert!(RawFile::open(Path::new("samples/raw-downloads/canon/6d-sraw.CR2")).is_err());
    }

    #[test]
    #[ignore = "read-only local portrait Canon RAW sample; composition-seed invariant"]
    fn portrait_sources_seed_the_composition_orientation_setting() {
        let path =
            Path::new("samples/raw-downloads/negative-scans/negadoctor-challenge-canondslr.CR2");
        let mut raw = RawFile::open(path).unwrap();
        assert_eq!(raw.metadata.flip, 6);
        assert_eq!(
            raw.metadata.photo_orientation().unwrap(),
            crate::geometry::PhotoOrientation {
                turns: crate::geometry::QuarterTurns::One,
                flips: crate::geometry::Flips::default(),
            }
        );
        // The developed raster keeps native sensor orientation; only the setting rotates.
        let image = raw.develop_camera(raw.metadata.as_shot_wb()).unwrap();
        assert!(image.width > image.height);
        assert_eq!(
            [image.width, image.height],
            raw.metadata.source_dimensions().unwrap()
        );
    }

    #[test]
    #[ignore = "read-only local RAW lazy/full/region diagnostics and cached-development invariants"]
    fn development_and_region_picks_do_not_compute_or_invalidate_full_diagnostics() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/DSC04393.ARW");
        let mut raw = RawFile::open(&path).unwrap();
        let wb = raw.metadata.cam_mul;
        let area = Rect {
            x: 4000,
            y: 2600,
            width: 128,
            height: 128,
        };
        let before = raw.region_sensor_diagnostics(area).unwrap();
        assert!(raw.unpacked && raw.diagnostics.is_none());
        let data = raw.develop_camera(wb).unwrap();
        let pointer = data.pixels.as_ptr();
        let pixel = data.pixel_codes(4000, 2600).unwrap();
        assert!(raw.diagnostics.is_none());
        assert_eq!(before, raw.region_sensor_diagnostics(area).unwrap());
        assert!(raw.diagnostics.is_none());
        let full = raw.unpack().unwrap();
        assert_eq!(raw.unpack().unwrap(), full);
        assert_eq!(
            full.sites.iter().sum::<u64>(),
            raw.metadata.width as u64 * raw.metadata.height as u64
        );
        assert_eq!(raw.camera_image().unwrap().pixels.as_ptr(), pointer);
        assert_eq!(
            raw.camera_image().unwrap().pixel_codes(4000, 2600).unwrap(),
            pixel
        );
        raw.set_development_priority(DevelopmentPriority::Background);
        assert_eq!(raw.camera_image().unwrap().pixels.as_ptr(), pointer);
        assert!(
            raw.region_sensor_diagnostics(Rect { width: 0, ..area })
                .is_err()
        );
        assert_eq!(
            raw.camera_image().unwrap().pixel_codes(4000, 2600).unwrap(),
            pixel
        );
        assert_eq!(before, raw.region_sensor_diagnostics(area).unwrap());
    }

    #[cfg(feature = "project")]
    #[test]
    fn libraw_freeze_requires_valid_camera_development_and_keeps_the_original_allocation() {
        let file = fixture::bayer_dng();
        let raw = RawFile::open_uncorrected(file.path()).unwrap();
        assert_eq!(raw.metadata.width, fixture::SIDE);
        assert!(raw.freeze().is_err());
        let mut raw = RawFile::open_uncorrected(file.path()).unwrap();
        raw.ensure_unpacked().unwrap();
        assert!(raw.freeze().is_err());
        let mut raw = RawFile::open_uncorrected(file.path()).unwrap();
        raw.develop_srgb_reference([1.0; 4]).unwrap();
        assert!(raw.freeze().is_err());
        let mut raw = RawFile::open_uncorrected(file.path()).unwrap();
        raw.develop_camera([1.0; 4]).unwrap();
        raw.metadata.width -= 1;
        assert!(raw.freeze().is_err());
        let mut raw = RawFile::open_uncorrected(file.path()).unwrap();
        raw.develop_camera([1.0; 4]).unwrap();
        let pointer = raw.camera_image().unwrap().pixels.as_ptr();
        assert!(raw.diagnostics.is_none());
        let frozen = raw.freeze().unwrap();
        assert!(frozen.raw.diagnostics.is_none());
        assert_eq!(frozen.camera_image().unwrap().pixels.as_ptr(), pointer);
        let a = Rect {
            x: 8,
            y: 8,
            width: 16,
            height: 16,
        };
        let before = frozen.region_sensor_diagnostics(a).unwrap();
        frozen.sensor_diagnostics().unwrap();
        assert_eq!(frozen.region_sensor_diagnostics(a).unwrap(), before);
        assert!(frozen.raw.diagnostics.is_none());
        assert_eq!(frozen.camera_image().unwrap().pixels.as_ptr(), pointer);
    }

    #[test]
    fn failed_open_returns_a_diagnostic_without_a_handle() {
        assert!(RawFile::open(Path::new(".cache/probes/not-a-source-file.ARW")).is_err());
    }
}
