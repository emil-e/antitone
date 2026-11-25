//! Pinned ACES display rendering into target-linear RGB.

#[cfg(all(
    feature = "gpu",
    any(feature = "render", all(test, feature = "project"))
))]
pub(crate) mod aces;
#[cfg(all(feature = "gpu", feature = "render"))]
mod operation;
#[cfg(all(feature = "gpu", feature = "render"))]
pub use operation::{Target, Targets};

#[cfg(feature = "ocio-reference")]
mod reference;
#[cfg(feature = "ocio-reference")]
pub use reference::{CpuRenderer, ocio_version, reference_parameters};

#[cfg(feature = "render")]
pub(crate) fn validate_channel(value: f64) -> Result<(), crate::Error> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(crate::Error::InvalidParameter(
            "rendered/encoded output channel must be finite and within 0..1",
        ));
    }
    Ok(())
}
