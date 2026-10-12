use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::marker::PhantomData;
use std::path::Path;
use std::ptr::NonNull;

use glam::{DMat3, DVec3};

use crate::Error;
use crate::color::{AP1_TO_XYZ, BRADFORD};
use crate::sampling::mean;

const SRGB_TO_XYZ: DMat3 = DMat3::from_cols(
    DVec3::new(0.4124564, 0.2126729, 0.0193339),
    DVec3::new(0.3575761, 0.7151522, 0.1191920),
    DVec3::new(0.1804375, 0.0721750, 0.9503041),
);

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Metadata {
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
    pub sony_file_type: u32,
    pub black: u32,
    pub maximum: u32,
    pub cblack: [u32; 4],
    pub linear_max: [u32; 4],
    pub cam_mul: [f64; 4],
    pub daylight_mul: [f64; 4],
    rgb_cam: [f64; 9],
}

impl Metadata {
    pub fn make(&self) -> String {
        ffi_string(&self.make)
    }
    pub fn model(&self) -> String {
        ffi_string(&self.model)
    }
    pub fn decoder(&self) -> String {
        ffi_string(&self.decoder)
    }
    pub fn camera_to_srgb(&self) -> DMat3 {
        DMat3::from_cols_array(&self.rgb_cam)
    }

    pub fn camera_model(&self) -> String {
        format!("{} {}", self.make(), self.model())
    }
    pub fn camera_to_ap1(&self) -> Result<DMat3, Error> {
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

    pub fn image_area(&self) -> Result<Rect, Error> {
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

pub use crate::geometry::SourceRegion as Rect;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SensorDiagnostics {
    pub sites: [u64; 4],
    pub below_black: [u64; 4],
    pub at_white: [u64; 4],
    pub peak: [u32; 4],
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

unsafe extern "C" {
    fn antitone_raw_version() -> *const c_char;
    fn antitone_raw_open(
        path: *const c_char,
        metadata: *mut Metadata,
        error: *mut c_char,
        size: usize,
    ) -> *mut c_void;
    fn antitone_raw_unpack(
        handle: *mut c_void,
        threads: c_int,
        error: *mut c_char,
        size: usize,
    ) -> c_int;
    fn antitone_raw_sensor_diagnostics(
        handle: *mut c_void,
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
        handle: *mut c_void,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        diagnostics: *mut SensorDiagnostics,
        error: *mut c_char,
        size: usize,
    ) -> c_int;
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
    fn current_threads(self) -> c_int {
        self.threads(
            std::thread::available_parallelism().map_or(1, usize::from),
            crate::resources::memory_pressure(),
        )
    }
}

pub struct RawFile {
    handle: NonNull<c_void>,
    pub metadata: Metadata,
    unpacked: bool,
    priority: DevelopmentPriority,
    diagnostics: Option<SensorDiagnostics>,
    camera_view: Option<NativeView>,
}

impl RawFile {
    pub fn open(path: &Path) -> Result<Self, Error> {
        let path = CString::new(path.as_os_str().as_encoded_bytes())
            .map_err(|_| Error::Raw("source path contains NUL".into()))?;
        let mut metadata = std::mem::MaybeUninit::<Metadata>::zeroed();
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
        Ok(Self {
            handle,
            metadata: unsafe { metadata.assume_init() },
            unpacked: false,
            priority: DevelopmentPriority::default(),
            diagnostics: None,
            camera_view: None,
        })
    }

    /// Changes resource scheduling only, never development settings or cached pixels.
    pub fn set_development_priority(&mut self, priority: DevelopmentPriority) {
        self.priority = priority;
    }
    fn ensure_unpacked(&mut self) -> Result<(), Error> {
        if self.unpacked {
            return Ok(());
        }
        let mut error = [0; 512];
        // This uniquely owned handle decodes once, retaining the original sensor snapshot.
        let code = unsafe {
            antitone_raw_unpack(
                self.handle.as_ptr(),
                self.priority.current_threads(),
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
        let mut diagnostics = SensorDiagnostics::default();
        let mut error = [0; 512];
        // The handle is uniquely owned; the bridge does not retain the output buffers.
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
        self.diagnostics = Some(diagnostics);
        Ok(diagnostics)
    }

    pub fn region_sensor_diagnostics(&mut self, area: Rect) -> Result<SensorDiagnostics, Error> {
        self.ensure_unpacked()?;
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
        let threads = self.priority.current_threads();
        let unpack_seconds = started.elapsed().as_secs_f64();
        let tick = std::time::Instant::now();
        self.camera_view = None;
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
        println!(
            "raw_development width={} height={} priority={:?} threads={threads} unpack_seconds={unpack_seconds:.6} process_seconds={:.6}",
            view.width,
            view.height,
            self.priority,
            tick.elapsed().as_secs_f64()
        );
        if output_color == 0 {
            self.camera_view = Some(view);
        }
        self.borrow_view(view)
    }

    /// Borrows the cached camera development without rerunning LibRaw.
    pub fn camera_image(&self) -> Result<Developed<'_>, Error> {
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

pub struct Developed<'a> {
    pixels: &'a [u16],
    pub width: u32,
    pub height: u32,
    pub maximum: u32,
    pub data_maximum: u32,
    pub warnings: u32,
    pub pre_mul: [f64; 4],
    _owner: PhantomData<&'a RawFile>,
}

impl Developed<'_> {
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
        if x >= self.width || y >= self.height {
            return Err(Error::Raw("pixel outside developed image".into()));
        }
        let offset = (y as usize * self.width as usize + x as usize) * 4;
        Ok(self.pixels[offset..offset + 4].try_into().unwrap())
    }

    pub fn pixel(&self, x: u32, y: u32) -> Result<DVec3, Error> {
        let [r, g, b, _] = self.pixel_codes(x, y)?;
        Ok(DVec3::new(r as f64, g as f64, b as f64) / 65535.0)
    }
}

fn ffi_string(bytes: &[c_char]) -> String {
    // The bridge always NUL-terminates fixed-size text fields and error buffers.
    unsafe { CStr::from_ptr(bytes.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

fn native_error(operation: &str, error: &[c_char]) -> Error {
    Error::Raw(format!("{operation}: {}", ffi_string(error)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata_fixture() -> Metadata {
        // All fields are numeric or char arrays, so zero is a valid fixture representation.
        let mut m: Metadata = unsafe { std::mem::zeroed() };
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
    fn bridge_layout_matches_native_static_assertions() {
        assert_eq!(size_of::<Metadata>(), 496);
        assert_eq!(size_of::<SensorDiagnostics>(), 112);
        assert_eq!(size_of::<NativeView>(), 64);
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

    #[test]
    fn failed_open_returns_a_diagnostic_without_a_handle() {
        assert!(RawFile::open(Path::new(".cache/probes/not-a-source-file.ARW")).is_err());
    }
}
