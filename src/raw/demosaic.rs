use super::{NativeView, RawFile, antitone_raw_develop};
use crate::gpu::demosaic::Demosaicer;
use std::{
    ffi::{c_char, c_int, c_void},
    panic::{AssertUnwindSafe, catch_unwind},
};

#[repr(C)]
struct NativeDemosaicer {
    context: *mut c_void,
    run: unsafe extern "C" fn(*mut c_void, *mut u16, u32, u32, *const u8) -> c_int,
}

unsafe extern "C" {
    fn antitone_raw_develop_with_demosaic(
        handle: *mut c_void,
        wb: *const f64,
        output_color: c_int,
        threads: c_int,
        demosaicer: *const NativeDemosaicer,
        view: *mut NativeView,
        error: *mut c_char,
        size: usize,
    ) -> c_int;
}

/// The native bridge clears its callback before returning, including on C++ exceptions.
/// Neither the callback's context nor its prepared-image borrow can escape this call.
pub(super) unsafe fn develop(
    raw: &RawFile,
    wb: &[f64; 4],
    output_color: c_int,
    threads: c_int,
    view: *mut NativeView,
    error: &mut [c_char],
) -> c_int {
    let handle = raw.handle.as_ptr();
    let wb = wb.as_ptr();
    let size = error.len();
    let error = error.as_mut_ptr();
    let Some(gpu) = raw.demosaicer.as_deref() else {
        // SAFETY: forwards the same synchronous lend contract to the CPU-only entry.
        return unsafe {
            antitone_raw_develop(handle, wb, output_color, threads, view, error, size)
        };
    };
    let callback = NativeDemosaicer {
        context: std::ptr::from_ref(gpu).cast_mut().cast(),
        run,
    };
    // SAFETY: gpu and callback outlive this synchronous call; the bridge resets its borrow.
    unsafe {
        antitone_raw_develop_with_demosaic(
            handle,
            wb,
            output_color,
            threads,
            &callback,
            view,
            error,
            size,
        )
    }
}

#[cfg(all(test, feature = "project"))]
mod tests;

unsafe extern "C" fn run(
    context: *mut c_void,
    image: *mut u16,
    width: u32,
    height: u32,
    pattern: *const u8,
) -> c_int {
    // No Rust unwind crosses LibRaw. Rejection/panic leaves the prepared image for CPU DHT.
    match catch_unwind(AssertUnwindSafe(|| -> Result<(), String> {
        if context.is_null() || image.is_null() || pattern.is_null() {
            return Err("missing demosaic borrow".into());
        }
        let count = (width as usize)
            .checked_mul(height as usize)
            .filter(|&n| n > 0 && n <= isize::MAX as usize / 8)
            .ok_or("invalid demosaic borrow size")?;
        if !(image as usize).is_multiple_of(align_of::<u16>()) {
            return Err("unaligned prepared image".into());
        }
        // SAFETY: the bridge lends one contiguous RGBG16 raster and four CFA bytes only
        // during this callback. Context points to the live shared Demosaicer above.
        let (gpu, image, pattern) = unsafe {
            (
                &*context.cast::<Demosaicer>(),
                std::slice::from_raw_parts_mut(image.cast::<[u16; 4]>(), count),
                *pattern.cast::<[u8; 4]>(),
            )
        };
        let started = std::time::Instant::now();
        let result = gpu.demosaic(image, width, height, pattern);
        match &result {
            Ok(()) => log::debug!(width, height, seconds = started.elapsed().as_secs_f64();
                "gpu_bayer_demosaic"),
            Err(error) => log::debug!("Bayer demosaic uses CPU: {error}"),
        }
        result
    })) {
        Ok(Ok(())) => 0,
        _ => -1,
    }
}
