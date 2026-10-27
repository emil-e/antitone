//! Camera-preview brightness initialization; independent of IO, RAW backends and user grading.

const INVERSE: &[u8; 257 * 4] = include_bytes!("../assets/ocio/exposure-neutral-log2.f32");
const QUANTILES: [f64; 5] = [0.2, 0.35, 0.5, 0.65, 0.8];

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "project", derive(serde::Serialize, serde::Deserialize))]
pub struct StartingExposure {
    pub ev: f64,
    pub samples: usize,
    /// Disagreement between percentile estimates, not a probability or a visibility threshold.
    pub spread: f64,
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "project", derive(serde::Serialize, serde::Deserialize))]
pub enum ReferenceExposure {
    Matched(StartingExposure),
    Unavailable(UnavailableReference),
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "project", derive(serde::Serialize, serde::Deserialize))]
pub enum UnavailableReference {
    NoUsablePreview,
    UnsupportedOrientation,
    IncomparableFraming,
    InsufficientSamples,
    UnsupportedExposure,
    ReadFailed,
}

impl ReferenceExposure {
    pub fn ev(&self) -> f64 {
        match self {
            Self::Matched(exposure) => exposure.ev,
            Self::Unavailable(_) => 0.,
        }
    }
}

/// Estimate one starting exposure from comparable source-AP1 and preview-display luminance.
/// Geometry/colour interpretation and capture eligibility belong to the sample producer.
pub fn estimate(samples: impl IntoIterator<Item = (f64, f64)>) -> Option<StartingExposure> {
    let mut source = Vec::new();
    let mut preview = Vec::new();
    for (a, b) in samples {
        if a.is_finite() && a > 0. && b.is_finite() && (0.015..0.8).contains(&b) {
            source.push(a);
            preview.push(b);
        }
    }
    if source.len() < 128 {
        return None;
    }
    source.sort_unstable_by(f64::total_cmp);
    preview.sort_unstable_by(f64::total_cmp);
    let mut shifts =
        QUANTILES.map(|q| inverse_neutral(quantile(&preview, q)) - quantile(&source, q).log2());
    if shifts.iter().any(|v| !v.is_finite()) {
        return None;
    }
    shifts.sort_unstable_by(f64::total_cmp);
    Some(StartingExposure {
        ev: shifts[2],
        samples: source.len(),
        spread: shifts[4] - shifts[0],
    })
}

fn quantile(values: &[f64], fraction: f64) -> f64 {
    let index = fraction * (values.len() - 1) as f64;
    let low = index.floor() as usize;
    let high = (low + 1).min(values.len() - 1);
    values[low] + (values[high] - values[low]) * (index - low as f64)
}

/// log2 scene luminance for the pinned 100-nit neutral ACES response.
fn inverse_neutral(display: f64) -> f64 {
    let index = ((display.log2() + 12.) * (256. / 12.)).clamp(0., 256.);
    let low = (index.floor() as usize).min(255);
    let value = |i: usize| {
        f64::from(f32::from_le_bytes(
            INVERSE[i * 4..i * 4 + 4].try_into().unwrap(),
        ))
    };
    let a = value(low);
    a + (value(low + 1) - a) * (index - low as f64)
}

#[cfg(test)]
mod tests;
