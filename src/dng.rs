//! Direct Adobe DNG SDK camera development and decode-free metadata inspection.
//! Original CFA and conservative dependency flags remain separate capture authority.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::path::Path;
use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use glam::DMat3;

/// SDK build and stage/WB/precision/capture interpretation, not a source-file revision.
pub const INTERPRETATION: &str = "adobe-dng-1.7.1.2768-float-bilinear-post-op-wb-quantum-v2";

#[derive(Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);

impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Cancelled,
    Rejected(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => f.write_str("DNG development cancelled"),
            Self::Rejected(reason) => write!(f, "DNG development rejected: {reason}"),
        }
    }
}
impl std::error::Error for Error {}

/// Limits the main sensor, not total SDK metadata/scratch or process residency.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub max_pixels: u32,
    /// None selects AsShotNeutral. Post-opcode gains normalize by their minimum.
    pub fixed_wb: Option<[f64; 3]>,
}

/// Header/opcode-derived interpretation; inspecting it does not decode sensor pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Header {
    pub dimensions: [u32; 2],
    pub original_dimensions: [u32; 2],
    pub active_origin: [u32; 2],
    pub default_crop: [u32; 4],
    pub orientation: u32,
    pub cfa_pattern: u32,
    pub capture_limits: [f64; 2],
    pub fixed_wb: [f64; 3],
    pub as_shot_wb: [f64; 3],
    pub camera_to_xyz_d50: DMat3,
    pub daylight_to_xyz_d50: DMat3,
    pub as_shot_to_xyz_d50: DMat3,
    pub input_floor: f64,
    pub interpretation_id: [u8; 32],
}

impl Header {
    pub fn read(path: &Path, options: Options, cancel: &Cancellation) -> Result<Self, Error> {
        let (path, options) = native_request(path, options, cancel, 1)?;
        let result = NativeAllocation::call(&path, &options, antitone_dng_inspect)?;
        Ok(Self::from_view(&result.view))
    }

    fn from_view(v: &NativeView) -> Self {
        Self {
            dimensions: [v.width, v.height],
            original_dimensions: [v.raw_width, v.raw_height],
            active_origin: [v.active_left, v.active_top],
            default_crop: [v.crop_left, v.crop_top, v.crop_width, v.crop_height],
            orientation: v.orientation,
            cfa_pattern: v.cfa_pattern,
            capture_limits: [v.black, v.white],
            fixed_wb: v.wb,
            as_shot_wb: v.as_shot_wb,
            camera_to_xyz_d50: DMat3::from_cols_array(&v.camera_to_xyz_d50).transpose(),
            daylight_to_xyz_d50: DMat3::from_cols_array(&v.daylight_to_xyz_d50).transpose(),
            as_shot_to_xyz_d50: DMat3::from_cols_array(&v.as_shot_to_xyz_d50).transpose(),
            input_floor: v.input_floor,
            interpretation_id: hash_interpretation(v),
        }
    }
}

/// Conservative reasons a developed pixel is not eligible for capture measurements.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureState(u8);
impl CaptureState {
    pub fn original_white(self) -> bool {
        self.0 & 1 != 0
    }
    pub fn original_black(self) -> bool {
        self.0 & 2 != 0
    }
    pub fn repeated_border(self) -> bool {
        self.0 & 4 != 0
    }
    /// Intermediate opcode/linearization limits, independent of original sensor limits.
    pub fn processing_limit(self) -> bool {
        self.0 & 8 != 0
    }
    pub fn eligible(self) -> bool {
        self.0 == 0
    }
}

/// Diagnostic wall times, not processing choices or interpretation identity.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct Timings {
    pub sdk_area_seconds: f64,
    pub warp_sdk_seconds: f64,
    pub bad_pixel_mask_seconds: f64,
    pub bilinear_mask_seconds: f64,
    pub warp_mask_seconds: f64,
    pub total_seconds: f64,
    pub sdk_workers: u32,
    pub mask_workers: u32,
    pub warp_tile_scratch_bytes: u64,
}

#[repr(C)]
struct NativeOptions {
    max_pixels: u64,
    threads: u32,
    wb: [f64; 3],
    cancelled: extern "C" fn(*const c_void) -> c_int,
    cancel_context: *const c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NativeView {
    rgb: *const f32,
    original: *const u16,
    flags: *const u8,
    program: *const u8,
    program_size: usize,
    width: u32,
    height: u32,
    raw_width: u32,
    raw_height: u32,
    active_left: u32,
    active_top: u32,
    crop_left: u32,
    crop_top: u32,
    crop_width: u32,
    crop_height: u32,
    orientation: u32,
    cfa_pattern: u32,
    applied: [u32; 3],
    skipped: [u32; 3],
    black: f64,
    white: f64,
    wb: [f64; 3],
    as_shot_wb: [f64; 3],
    camera_to_xyz_d50: [f64; 9],
    input_floor: f64,
    daylight_to_xyz_d50: [f64; 9],
    as_shot_to_xyz_d50: [f64; 9],
    retained_bytes: u64,
    timings: Timings,
}

type NativeCall = unsafe extern "C" fn(
    *const c_char,
    *const NativeOptions,
    *mut *mut c_void,
    *mut NativeView,
    *mut c_char,
    usize,
) -> c_int;

unsafe extern "C" {
    fn antitone_dng_develop(
        path: *const c_char,
        options: *const NativeOptions,
        owner: *mut *mut c_void,
        view: *mut NativeView,
        error: *mut c_char,
        size: usize,
    ) -> c_int;
    fn antitone_dng_inspect(
        path: *const c_char,
        options: *const NativeOptions,
        owner: *mut *mut c_void,
        view: *mut NativeView,
        error: *mut c_char,
        size: usize,
    ) -> c_int;
    fn antitone_dng_close(owner: *mut c_void);
}

struct NativeAllocation {
    owner: NonNull<c_void>,
    view: NativeView,
}

impl NativeAllocation {
    fn call(path: &CStr, options: &NativeOptions, operation: NativeCall) -> Result<Self, Error> {
        let mut owner = std::ptr::null_mut();
        let mut view = std::mem::MaybeUninit::<NativeView>::uninit();
        let mut error = [0 as c_char; 512];
        // Both entry points publish an owner only on complete success. Header owners
        // contain metadata/program buffers but deliberately no sensor or RGB allocation.
        let status = unsafe {
            operation(
                path.as_ptr(),
                options,
                &mut owner,
                view.as_mut_ptr(),
                error.as_mut_ptr(),
                error.len(),
            )
        };
        if status != 0 {
            debug_assert!(owner.is_null());
            return Err(if status == 1 {
                Error::Cancelled
            } else {
                Error::Rejected(
                    unsafe { CStr::from_ptr(error.as_ptr()) }
                        .to_string_lossy()
                        .into_owned(),
                )
            });
        }
        Ok(Self {
            owner: NonNull::new(owner)
                .ok_or_else(|| Error::Rejected("missing native owner".into()))?,
            // Successful native completion initializes all fields.
            view: unsafe { view.assume_init() },
        })
    }
}

impl Drop for NativeAllocation {
    fn drop(&mut self) {
        // The completed owner contains only plain vectors, not SDK state.
        unsafe { antitone_dng_close(self.owner.as_ptr()) };
    }
}

/// Immutable completed camera RGB, original CFA and eligibility. No live SDK state remains.
pub struct Source {
    allocation: NativeAllocation,
    header: Header,
}

// Native completion leaves immutable plain buffers; reads/destruction need no creator thread.
unsafe impl Send for Source {}
unsafe impl Sync for Source {}

impl std::fmt::Debug for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DngSource")
            .field("header", &self.header)
            .finish_non_exhaustive()
    }
}

extern "C" fn cancelled(context: *const c_void) -> c_int {
    // The synchronous call borrows the atomic token and never retains its address.
    c_int::from(unsafe { &*context.cast::<AtomicBool>() }.load(Ordering::Relaxed))
}

fn native_request(
    path: &Path,
    options: Options,
    cancel: &Cancellation,
    threads: u32,
) -> Result<(CString, NativeOptions), Error> {
    let path = CString::new(path.as_os_str().as_encoded_bytes())
        .map_err(|_| Error::Rejected("path contains NUL".into()))?;
    if !(1..=8).contains(&threads)
        || options.max_pixels == 0
        || options
            .fixed_wb
            .is_some_and(|wb| wb.iter().any(|v| !v.is_finite() || *v <= 0.))
    {
        return Err(Error::Rejected(
            "invalid pixel bound, execution budget or fixed WB".into(),
        ));
    }
    Ok((
        path,
        NativeOptions {
            max_pixels: u64::from(options.max_pixels),
            threads,
            wb: options.fixed_wb.unwrap_or([0.; 3]),
            cancelled,
            cancel_context: Arc::as_ptr(&cancel.0).cast(),
        },
    ))
}

impl Source {
    pub fn develop(path: &Path, options: Options, cancel: &Cancellation) -> Result<Self, Error> {
        Self::develop_with_threads(path, options, cancel, 1)
    }

    /// Bounded synchronous execution (1..8 workers), not another development queue.
    /// Worker count affects neither interpretation identity nor completed samples.
    pub fn develop_with_threads(
        path: &Path,
        options: Options,
        cancel: &Cancellation,
        threads: u32,
    ) -> Result<Self, Error> {
        let (path, options) = native_request(path, options, cancel, threads)?;
        Self::call_native(&path, &options)
    }

    fn call_native(path: &CStr, options: &NativeOptions) -> Result<Self, Error> {
        let allocation = NativeAllocation::call(path, options, antitone_dng_develop)?;
        let v = &allocation.view;
        if v.width == 0
            || v.height == 0
            || v.raw_width == 0
            || v.raw_height == 0
            || v.rgb.is_null()
            || v.original.is_null()
            || v.flags.is_null()
            || v.program.is_null()
        {
            return Err(Error::Rejected("invalid completed native view".into()));
        }
        let header = Header::from_view(v);
        Ok(Self { allocation, header })
    }

    pub fn header(&self) -> Header {
        self.header
    }
    pub fn dimensions(&self) -> [u32; 2] {
        self.header.dimensions
    }
    pub fn original_dimensions(&self) -> [u32; 2] {
        self.header.original_dimensions
    }
    pub fn active_origin(&self) -> [u32; 2] {
        self.header.active_origin
    }
    /// Unrotated active camera raster, independent of default crop/orientation metadata.
    pub fn camera_rgb(&self) -> &[[f32; 3]] {
        let [w, h] = self.dimensions();
        // Completed vectors stay immutable until this owner is dropped.
        unsafe {
            std::slice::from_raw_parts(self.allocation.view.rgb.cast(), w as usize * h as usize)
        }
    }
    /// Pre-OpcodeList1 codes in original sensor coordinates, including masked margins.
    pub fn original_cfa(&self) -> &[u16] {
        let [w, h] = self.original_dimensions();
        // Original CFA has independent retained storage with the same owner lifetime.
        unsafe {
            std::slice::from_raw_parts(self.allocation.view.original, w as usize * h as usize)
        }
    }
    pub fn capture_state(&self, x: u32, y: u32) -> Option<CaptureState> {
        let [w, h] = self.dimensions();
        if x >= w || y >= h {
            return None;
        }
        let i = y as usize * w as usize + x as usize;
        // The completed mask contains one byte per developed pixel; coordinates were checked.
        Some(CaptureState(unsafe { *self.allocation.view.flags.add(i) }))
    }
    pub fn capture_limits(&self) -> [f64; 2] {
        self.header.capture_limits
    }
    /// [left, top, width, height], relative to the active raster.
    pub fn default_crop(&self) -> [u32; 4] {
        self.header.default_crop
    }
    pub fn tiff_orientation(&self) -> u32 {
        self.header.orientation
    }
    pub fn fixed_wb(&self) -> [f64; 3] {
        self.header.fixed_wb
    }
    /// XYZ D50 from this balanced camera basis, without a second application of WB.
    pub fn camera_to_xyz_d50(&self) -> DMat3 {
        self.header.camera_to_xyz_d50
    }
    pub fn input_floor(&self) -> f64 {
        self.header.input_floor
    }
    pub fn opcode_counts(&self) -> ([u32; 3], [u32; 3]) {
        (self.allocation.view.applied, self.allocation.view.skipped)
    }
    /// Explicit owner/vector storage, not SDK peak scratch or allocator/process residency.
    pub fn retained_bytes(&self) -> u64 {
        self.allocation.view.retained_bytes
    }
    pub fn timings(&self) -> Timings {
        self.allocation.view.timings
    }
    /// Processing content only, not source-file identity or freshness.
    pub fn interpretation_id(&self) -> [u8; 32] {
        self.header.interpretation_id
    }
}

fn hash_interpretation(v: &NativeView) -> [u8; 32] {
    let mut hash = blake3::Hasher::new();
    hash.update(INTERPRETATION.as_bytes());
    for field in [
        v.width,
        v.height,
        v.raw_width,
        v.raw_height,
        v.active_left,
        v.active_top,
        v.crop_left,
        v.crop_top,
        v.crop_width,
        v.crop_height,
        v.orientation,
        v.cfa_pattern,
    ] {
        hash.update(&field.to_le_bytes());
    }
    for field in [v.black, v.white, v.input_floor]
        .into_iter()
        .chain(v.wb)
        .chain(v.camera_to_xyz_d50)
    {
        hash.update(&field.to_bits().to_le_bytes());
    }
    // Both inspected and developed owners retain the same canonical length-framed program.
    hash.update(unsafe { std::slice::from_raw_parts(v.program, v.program_size) });
    *hash.finalize().as_bytes()
}

#[cfg(test)]
#[path = "dng_tests.rs"]
mod tests;
