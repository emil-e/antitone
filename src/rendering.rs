#[cfg(feature = "ocio-reference")]
use std::ffi::{CStr, c_char, c_void};
#[cfg(feature = "ocio-reference")]
use std::ptr::NonNull;

use glam::{DMat3, DVec3, Vec3};

use crate::Error;

#[cfg(feature = "ocio-reference")]
unsafe extern "C" {
    fn antitone_ocio_version() -> *const c_char;
    fn antitone_output_parameters(target: u32, parameters: *mut f64) -> i32;
    fn antitone_ocio_create(target: u32, error: *mut c_char, size: usize) -> *mut c_void;
    fn antitone_ocio_reference_create(
        parameters: *const f64,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum OutputTarget {
    Srgb = 0,
    DisplayP3 = 1,
}

impl OutputTarget {
    pub fn parameters(self) -> [f64; 9] {
        // Pinned target definitions; reference builds independently check the native parameters.
        match self {
            Self::Srgb => [100., 0.64, 0.33, 0.30, 0.60, 0.15, 0.06, 0.3127, 0.3290],
            Self::DisplayP3 => [100., 0.68, 0.32, 0.265, 0.69, 0.15, 0.06, 0.3127, 0.3290],
        }
    }

    pub fn rgb_to_xyz(self) -> DMat3 {
        let p = self.parameters();
        let xyz = |x: f64, y: f64| DVec3::new(x / y, 1.0, (1.0 - x - y) / y);
        let primaries = DMat3::from_cols(xyz(p[1], p[2]), xyz(p[3], p[4]), xyz(p[5], p[6]));
        primaries * DMat3::from_diagonal(primaries.inverse() * xyz(p[7], p[8]))
    }

    pub fn linear_to_srgb(self) -> DMat3 {
        if self == Self::Srgb {
            return DMat3::IDENTITY;
        }
        let source_white = self.rgb_to_xyz() * DVec3::ONE;
        let destination = Self::Srgb.rgb_to_xyz();
        let destination_white = destination * DVec3::ONE;
        let bradford = crate::color::BRADFORD;
        destination.inverse()
            * bradford.inverse()
            * DMat3::from_diagonal((bradford * destination_white) / (bradford * source_white))
            * bradford
            * self.rgb_to_xyz()
    }

    pub fn encode(self, linear: f64) -> Result<f64, Error> {
        validate_channel(linear)?;
        Ok(match self {
            Self::Srgb | Self::DisplayP3 if linear <= 0.0031308 => 12.92 * linear,
            Self::Srgb | Self::DisplayP3 => 1.055 * linear.powf(1.0 / 2.4) - 0.055,
        })
    }

    pub fn decode(self, encoded: f64) -> Result<f64, Error> {
        validate_channel(encoded)?;
        Ok(match self {
            Self::Srgb | Self::DisplayP3 if encoded <= 0.04045 => encoded / 12.92,
            Self::Srgb | Self::DisplayP3 => ((encoded + 0.055) / 1.055).powf(2.4),
        })
    }

    pub fn encode_u16(self, linear: &[Vec3]) -> Result<Vec<[u16; 3]>, Error> {
        self.encode_u16_pixels(linear.iter().copied())
    }

    pub(crate) fn encode_u16_pixels(
        self,
        linear: impl IntoIterator<Item = Vec3>,
    ) -> Result<Vec<[u16; 3]>, Error> {
        linear
            .into_iter()
            .map(|pixel| {
                let mut encoded = [0; 3];
                for (channel, value) in pixel.to_array().into_iter().enumerate() {
                    encoded[channel] = (self.encode(f64::from(value))? * 65535.0).round() as u16;
                }
                Ok(encoded)
            })
            .collect()
    }
}

#[cfg(feature = "ocio-reference")]
pub fn reference_parameters(target: OutputTarget) -> [f64; 9] {
    let mut parameters = [0.; 9];
    // Both enum values are supported; native code writes exactly nine doubles.
    assert_eq!(
        unsafe { antitone_output_parameters(target as u32, parameters.as_mut_ptr()) },
        0
    );
    parameters
}

fn validate_channel(value: f64) -> Result<(), Error> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(Error::InvalidParameter(
            "rendered/encoded output channel must be finite and within 0..1",
        ));
    }
    Ok(())
}

#[cfg(feature = "ocio-reference")]
pub fn ocio_version() -> String {
    // OCIO returns a process-lifetime, NUL-terminated version string.
    unsafe { CStr::from_ptr(antitone_ocio_version()) }
        .to_string_lossy()
        .into_owned()
}

#[cfg(feature = "ocio-reference")]
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
#[cfg(feature = "ocio-reference")]
pub struct CpuRenderer {
    handle: NonNull<c_void>,
}

#[cfg(feature = "ocio-reference")]
impl CpuRenderer {
    pub fn new(target: OutputTarget) -> Result<Self, Error> {
        let mut error = [0; 512];
        let handle =
            unsafe { antitone_ocio_create(target as u32, error.as_mut_ptr(), error.len()) };
        Ok(Self {
            handle: NonNull::new(handle).ok_or_else(|| diagnostic(&error))?,
        })
    }

    /// Developer-only fixed-reference forward transform, without selected-output range clamps.
    pub fn new_reference(parameters: [f64; 9]) -> Result<Self, Error> {
        let mut error = [0; 512];
        // The native constructor reads exactly nine doubles during this call.
        let handle = unsafe {
            antitone_ocio_reference_create(parameters.as_ptr(), error.as_mut_ptr(), error.len())
        };
        Ok(Self {
            handle: NonNull::new(handle).ok_or_else(|| diagnostic(&error))?,
        })
    }
    pub fn render(&self, working_ap1: &[DVec3]) -> Result<Vec<Vec3>, Error> {
        let mut pixels = Vec::with_capacity(working_ap1.len());
        for input in working_ap1 {
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
            for value in pixel {
                validate_channel(f64::from(*value))?;
            }
        }
        Ok(pixels.into_iter().map(Vec3::from_array).collect())
    }
}

#[cfg(feature = "ocio-reference")]
impl Drop for CpuRenderer {
    fn drop(&mut self) {
        // This handle is uniquely owned and released exactly once.
        unsafe { antitone_ocio_destroy(self.handle.as_ptr()) };
    }
}
