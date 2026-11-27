//! Independent OCIO transforms for testing and authoring; the GPU owns rendered output.

use std::ffi::{CStr, c_char, c_void};
use std::ptr::NonNull;

use super::validate_channel;
use crate::{Error, color::OutputTarget};
use glam::{DVec3, Vec3};

unsafe extern "C" {
    fn antitone_ocio_version() -> *const c_char;
    fn antitone_output_parameters(target: u32, parameters: *mut f64) -> i32;
    fn antitone_ocio_create(target: u32, error: *mut c_char, size: usize) -> *mut c_void;
    fn antitone_ocio_reference_create(
        parameters: *const f64,
        inverse: u32,
        error: *mut c_char,
        size: usize,
    ) -> *mut c_void;
    fn antitone_ocio_apply(
        handle: *mut c_void,
        rgb: *mut f32,
        count: usize,
        error: *mut c_char,
        size: usize,
    ) -> i32;
    fn antitone_ocio_destroy(handle: *mut c_void);
}

pub fn reference_parameters(target: OutputTarget) -> [f64; 9] {
    let mut parameters = [0.; 9];
    // The native reference writes exactly nine doubles for each target.
    assert_eq!(
        unsafe { antitone_output_parameters(target as u32, parameters.as_mut_ptr()) },
        0
    );
    parameters
}

pub fn ocio_version() -> String {
    // OCIO returns a process-lifetime, NUL-terminated version string.
    unsafe { CStr::from_ptr(antitone_ocio_version()) }
        .to_string_lossy()
        .into_owned()
}

fn diagnostic(buffer: &[c_char]) -> Error {
    // The C++ bridge uses snprintf into a nonempty, zero-initialized buffer.
    Error::Rendering(
        unsafe { CStr::from_ptr(buffer.as_ptr()) }
            .to_string_lossy()
            .into_owned(),
    )
}

/// Independent OCIO reference: linear AP1/D60 in, limited linear target RGB out.
/// Not a production export path. Does not encode or apply a monitor transform.
pub struct CpuRenderer {
    handle: NonNull<c_void>,
    peak: f64,
}

impl CpuRenderer {
    pub fn new(target: OutputTarget) -> Result<Self, Error> {
        let mut error = [0; 512];
        let handle =
            unsafe { antitone_ocio_create(target as u32, error.as_mut_ptr(), error.len()) };
        Ok(Self {
            handle: NonNull::new(handle).ok_or_else(|| diagnostic(&error))?,
            peak: target.linear_peak(),
        })
    }

    /// Developer-only fixed-reference forward transform, without selected-output range clamps.
    pub fn new_reference(parameters: [f64; 9]) -> Result<Self, Error> {
        Self::reference(parameters, false)
    }

    /// Developer-only inverse: linear reference RGB to AP1, not source-scan inversion.
    /// Use `apply_unbounded`; its AP1 results are not limited display channels.
    pub fn new_inverse_reference(parameters: [f64; 9]) -> Result<Self, Error> {
        Self::reference(parameters, true)
    }

    fn reference(parameters: [f64; 9], inverse: bool) -> Result<Self, Error> {
        let mut error = [0; 512];
        // The native constructor reads exactly nine doubles during this call.
        let handle = unsafe {
            antitone_ocio_reference_create(
                parameters.as_ptr(),
                u32::from(inverse),
                error.as_mut_ptr(),
                error.len(),
            )
        };
        Ok(Self {
            handle: NonNull::new(handle).ok_or_else(|| diagnostic(&error))?,
            peak: 1.,
        })
    }

    pub fn render(&self, working_ap1: &[DVec3]) -> Result<Vec<Vec3>, Error> {
        let values = self.apply_unbounded(working_ap1)?;
        for value in &values {
            for channel in value.to_array() {
                validate_channel(f64::from(channel), self.peak)?;
            }
        }
        Ok(values)
    }

    /// Apply a pinned diagnostic reference, preserving signed/HDR values but rejecting failures.
    pub fn apply_unbounded(&self, input: &[DVec3]) -> Result<Vec<Vec3>, Error> {
        let mut pixels = Vec::with_capacity(input.len());
        for input in input {
            for (channel, value) in input.to_array().into_iter().enumerate() {
                if !value.is_finite() {
                    return Err(Error::NonFinitePixel { channel });
                }
            }
            let input = input.as_vec3();
            if !input.is_finite() {
                return Err(Error::NumericalOverflow);
            }
            pixels.push(input.to_array());
        }
        let mut error = [0; 512];
        // The owned buffer contains tightly packed RGB triples and lives for the call.
        let status = unsafe {
            antitone_ocio_apply(
                self.handle.as_ptr(),
                pixels.as_mut_ptr().cast(),
                pixels.len(),
                error.as_mut_ptr(),
                error.len(),
            )
        };
        if status != 0 {
            return Err(diagnostic(&error));
        }
        for pixel in &pixels {
            for (channel, value) in pixel.iter().enumerate() {
                if !value.is_finite() {
                    return Err(Error::NonFinitePixel { channel });
                }
            }
        }
        Ok(pixels.into_iter().map(Vec3::from_array).collect())
    }
}

impl Drop for CpuRenderer {
    fn drop(&mut self) {
        // This handle is uniquely owned and released exactly once.
        unsafe { antitone_ocio_destroy(self.handle.as_ptr()) };
    }
}
