//! Embedded camera lens profiles in normalized coordinates, independent of raster framing.
//!
//! Pixel origin, optical centre and radius normalization belong to the development adapter.
//! These maps do not resample pixels or choose whether to apply an available camera profile.
//! Profiles arrive as radial samples decoded from a vendor payload.

use glam::{DVec2, DVec3};

use crate::{Error, geometry::SourceRegion};

/// Geometric components; shading is independent source-space radiometry.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LensCorrections {
    pub distortion: bool,
    pub ca: bool,
}

impl LensCorrections {
    pub const ALL: Self = Self {
        distortion: true,
        ca: true,
    };
}

/// Radial lens samples: distortion and lateral-chromatic-aberration scales plus shading
/// attenuation. The first sample sits half a knot beyond the optical centre and the last
/// beyond the unit radius (Sony's radial sampling convention). Shading describes attenuation
/// on the original image, before geometric reconstruction: interpolate attenuation, then
/// reciprocate for gains (darktable 5.6.2's v2 interpretation, agreeing with RawTherapee
/// 5.12 at the knots).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EmbeddedLensProfile {
    distortion: Vec<f64>,
    ca: Vec<DVec2>,
    attenuation: Vec<f64>,
}

impl EmbeddedLensProfile {
    /// Normalized knots. A missing component is an empty slice and acts as identity;
    /// distortion and CA must then share knot counts, while shading is independent.
    pub fn from_knots(
        distortion: &[f64],
        ca: &[DVec2],
        attenuation: &[f64],
    ) -> Result<Self, Error> {
        if !distortion.iter().all(|v| v.is_finite())
            || !ca.iter().all(|v| v.x.is_finite() && v.y.is_finite())
            || !attenuation.iter().all(|v| v.is_finite() && *v > 0.)
        {
            return Err(invalid(
                "embedded lens knots must be finite; shading must be positive",
            ));
        }
        if !distortion.is_empty() && !ca.is_empty() && ca.len() != distortion.len() {
            return Err(invalid(
                "embedded lens distortion and CA knot counts differ",
            ));
        }
        let profile = Self {
            distortion: distortion.to_vec(),
            ca: ca.to_vec(),
            attenuation: attenuation.to_vec(),
        };
        for corrections in [
            LensCorrections {
                distortion: true,
                ca: false,
            },
            LensCorrections {
                distortion: false,
                ca: true,
            },
            LensCorrections::ALL,
        ] {
            profile.validate_map(corrections)?;
        }
        Ok(profile)
    }

    #[cfg(feature = "raw")]
    pub(crate) fn from_inverse_distortion(
        samples: usize,
        scale: impl Fn(f64) -> f64,
    ) -> Result<Self, Error> {
        let distortion: Vec<_> = (0..samples)
            .map(|i| scale((i as f64 + 0.5) / (samples - 1) as f64))
            .collect();
        Self::from_knots(&distortion, &[], &[])
    }

    #[cfg(feature = "raw")]
    pub(crate) fn set_radial_gain(
        &mut self,
        samples: usize,
        gain: impl Fn(f64) -> f64,
    ) -> Result<(), Error> {
        let attenuation: Vec<_> = (0..samples)
            .map(|i| gain((i as f64 + 0.5) / (samples - 1) as f64).recip())
            .collect();
        if !attenuation.iter().all(|&v| v.is_finite() && v > 0.) {
            return Err(invalid("embedded lens gain must be finite and positive"));
        }
        self.attenuation = attenuation;
        Ok(())
    }

    pub fn has_distortion(&self) -> bool {
        self.distortion.iter().any(|&v| v != 1.)
    }

    pub fn has_ca(&self) -> bool {
        self.ca.iter().any(|&v| v != DVec2::ONE)
    }

    pub fn has_vignetting(&self) -> bool {
        !self.attenuation.is_empty()
    }

    /// Conservative amplification of the native code quantum; attenuation never lowers the floor.
    pub fn maximum_gain(&self) -> f64 {
        self.attenuation
            .iter()
            .map(|s| s.recip())
            .fold(1., f64::max)
    }

    pub fn vignetting_gain(&self, radius: f64) -> Result<f64, Error> {
        if !radius.is_finite() || !(0. ..=1.).contains(&radius) {
            return Err(invalid("lens radius must be finite and in [0, 1]"));
        }
        Ok(self.gain(radius))
    }

    fn gain(&self, radius: f64) -> f64 {
        let n = self.attenuation.len();
        if n == 0 {
            return 1.;
        }
        let index = (radius * (n - 1) as f64 - 0.5).clamp(0., (n - 1) as f64);
        let low = index.floor() as usize;
        let high = (low + 1).min(n - 1);
        let a = self.attenuation[low];
        (a + (self.attenuation[high] - a) * (index - low as f64)).recip()
    }

    pub fn knot_count(&self) -> usize {
        self.distortion.len().max(self.ca.len())
    }

    fn maximum_scale(&self) -> f64 {
        (0..self.knot_count())
            .map(|i| self.factors(i, LensCorrections::ALL).max_element())
            .fold(1., f64::max)
    }

    #[cfg(feature = "raw")]
    pub(crate) fn retained_bytes(&self) -> usize {
        (self.distortion.capacity() + self.attenuation.capacity()) * size_of::<f64>()
            + self.ca.capacity() * size_of::<DVec2>()
    }

    /// Per-channel inverse radial scales on the calibrated unit disc. No extrapolation outside it.
    pub fn inverse_scales(
        &self,
        radius: f64,
        corrections: LensCorrections,
    ) -> Result<DVec3, Error> {
        if !radius.is_finite() || !(0. ..=1.).contains(&radius) {
            return Err(invalid("lens radius must be finite and in [0, 1]"));
        }
        let n = self.knot_count();
        if n == 0 {
            return Ok(DVec3::ONE);
        }
        // The profile locates the first sample half a knot beyond zero and the last beyond
        // radius one.
        let index = (radius * (n - 1) as f64 - 0.5).max(0.);
        let low = (index.floor() as usize).min(n - 1);
        let high = (low + 1).min(n - 1);
        Ok(self
            .factors(low, corrections)
            .lerp(self.factors(high, corrections), index - low as f64))
    }

    /// Maps an output offset from the optical centre to three input offsets, in the same units.
    pub fn inverse_map(
        &self,
        offset: DVec2,
        corrections: LensCorrections,
    ) -> Result<[DVec2; 3], Error> {
        let factors = self.inverse_scales(offset.length(), corrections)?;
        Ok(factors.to_array().map(|factor| offset * factor))
    }

    fn factors(&self, index: usize, corrections: LensCorrections) -> DVec3 {
        let distortion = if corrections.distortion && !self.distortion.is_empty() {
            self.distortion[index]
        } else {
            1.
        };
        let ca = if corrections.ca && !self.ca.is_empty() {
            self.ca[index]
        } else {
            DVec2::ONE
        };
        DVec3::new(ca.x, 1., ca.y) * distortion
    }

    fn validate_map(&self, corrections: LensCorrections) -> Result<(), Error> {
        let n = self.knot_count();
        if n == 0 {
            return Ok(());
        }
        if self.factors(0, corrections).min_element() <= 0. {
            return Err(invalid("embedded lens map has a nonpositive radial scale"));
        }
        for i in 0..n - 1 {
            let left = (i as f64 + 0.5) / (n - 1) as f64;
            let right = (i as f64 + 1.5) / (n - 1) as f64;
            let a = self.factors(i, corrections);
            let b = self.factors(i + 1, corrections);
            let slope = (b - a) / (right - left);
            // Between knots, R(r) = r * (a + slope * (r - left)); R' is linear.
            let derivative_left = a + slope * left;
            let derivative_right = a + slope * (2. * right.min(1.) - left);
            if derivative_left.min_element() <= 0. || derivative_right.min_element() <= 0. {
                return Err(invalid(
                    "embedded lens map folds within the calibrated radius",
                ));
            }
        }
        Ok(())
    }
}

/// Embedded lens mapping in source coordinates. The calibrated rectangle determines the
/// optical centre and sampling bounds; the output keeps native pitch and fits within its framing.
#[derive(Debug, Clone)]
pub struct EmbeddedLensGeometry {
    profile: EmbeddedLensProfile,
    original: SourceRegion,
    output: SourceRegion,
    radial_frame: Option<(DVec2, f64)>,
}

impl EmbeddedLensGeometry {
    pub fn new(profile: EmbeddedLensProfile, original: SourceRegion) -> Result<Self, Error> {
        Self::with_framing(profile, original, original)
    }

    pub(crate) fn with_framing(
        profile: EmbeddedLensProfile,
        original: SourceRegion,
        framing: SourceRegion,
    ) -> Result<Self, Error> {
        original.validate()?;
        if !original.contains_region(framing) || framing.width < 2 || framing.height < 2 {
            return Err(invalid("invalid embedded lens framing"));
        }
        let maximum = profile.maximum_scale();
        let support_trim = [original.width, original.height]
            .map(|length| ((length - 1) as f64 * 0.5) * (1. - 1. / maximum));
        let trim = if framing == original {
            support_trim.map(|v| v.ceil() as u32)
        } else {
            // Fit the requested aspect frame inside valid support without shifting its centre.
            let inset = [
                (framing.x - original.x).min(original.right() - framing.right()),
                (framing.y - original.y).min(original.bottom() - framing.bottom()),
            ];
            let half = [framing.width, framing.height].map(|length| (length - 1) as f64 * 0.5);
            let fraction = (0..2)
                .map(|i| (support_trim[i] - f64::from(inset[i])) / half[i])
                .fold(0., f64::max);
            half.map(|length| (length * fraction).ceil() as u32)
        };
        Self::framed(profile, original, framing, trim, None)
    }

    /// The profile law uses the centred sensor half-diagonal, but its optical centre
    /// moves with stabilization. The sampling grid must cover the farthest corner.
    #[cfg(feature = "raw")]
    pub(crate) fn with_shifted_profile(
        original: SourceRegion,
        shift: DVec2,
        build: impl FnOnce(f64) -> EmbeddedLensProfile,
    ) -> Result<Option<Self>, Error> {
        original.validate()?;
        let size = [original.width, original.height];
        let center_to_edge = DVec2::from_array(size.map(|n| (f64::from(n) - 1.) / 2.));
        if !shift.is_finite() || !shift.abs().cmplt(center_to_edge).all() {
            return Err(invalid("optical centre is outside the source"));
        }
        let origin = DVec2::new(original.x as f64, original.y as f64);
        let half = DVec2::from_array(size.map(|n| f64::from(n) / 2.));
        let center = origin + center_to_edge + shift;
        let last = origin + center_to_edge * 2.;
        let farthest = (center - origin).max(last - center);
        let radius = farthest.length();
        let radius_ratio = radius / half.length();
        let profile = build(radius_ratio);
        if !profile.has_distortion() && !profile.has_ca() && !profile.has_vignetting() {
            return Ok(None);
        }
        let expansion = 1. - profile.maximum_scale().recip();
        let trim = (farthest * expansion).to_array().map(|d| d.ceil() as u32);
        Self::framed(profile, original, original, trim, Some((center, radius))).map(Some)
    }

    fn framed(
        profile: EmbeddedLensProfile,
        original: SourceRegion,
        framing: SourceRegion,
        trim: [u32; 2],
        radial_frame: Option<(DVec2, f64)>,
    ) -> Result<Self, Error> {
        let width = framing.width.saturating_sub(trim[0].saturating_mul(2));
        let height = framing.height.saturating_sub(trim[1].saturating_mul(2));
        if width < 2 || height < 2 {
            return Err(invalid(
                "embedded lens profile leaves no usable rectangular image",
            ));
        }
        Ok(Self {
            profile,
            original,
            output: SourceRegion::from([framing.x + trim[0], framing.y + trim[1], width, height]),
            radial_frame,
        })
    }

    pub fn original_area(&self) -> SourceRegion {
        self.original
    }
    pub fn size(&self) -> [u32; 2] {
        [self.output.width, self.output.height]
    }

    /// Original developed pixel-index coordinates of the red, green and blue samples.
    pub fn sample_positions(&self, x: u32, y: u32) -> Result<[DVec2; 3], Error> {
        if x >= self.output.width || y >= self.output.height {
            return Err(invalid("pixel outside the corrected image"));
        }
        let origin = DVec2::new(self.original.x as f64, self.original.y as f64);
        let half = DVec2::new(self.original.width as f64, self.original.height as f64) * 0.5;
        let (center, radius_limit) = self
            .radial_frame
            .unwrap_or_else(|| (origin + half - DVec2::splat(0.5), half.length()));
        let base = DVec2::new((self.output.x + x) as f64, (self.output.y + y) as f64);
        let delta = base - center;
        let radius = delta.length() / radius_limit;
        let scales = self.profile.inverse_scales(radius, LensCorrections::ALL)?;
        let minimum = origin;
        let maximum = origin + half * 2. - DVec2::ONE;
        let mut points = [DVec2::ZERO; 3];
        for c in 0..3 {
            let p = if scales[c] == 1. {
                base
            } else {
                center + delta * scales[c]
            };
            if !p.is_finite()
                || (p.cmplt(minimum - DVec2::splat(1e-9))).any()
                || (p.cmpgt(maximum + DVec2::splat(1e-9))).any()
            {
                return Err(invalid(
                    "lens correction reads outside valid source support",
                ));
            }
            // Only absorb arithmetic roundoff at an exact support boundary, not missing image data.
            points[c] = p.clamp(minimum, maximum);
        }
        Ok(points)
    }

    /// Gain at an original developed pixel, not at the distorted output coordinate.
    #[cfg(feature = "raw")]
    pub(crate) fn source_gain(&self, x: u32, y: u32) -> f64 {
        if !self.profile.has_vignetting() {
            return 1.;
        }
        let half = DVec2::new(self.original.width as f64, self.original.height as f64) * 0.5;
        let (center, radius_limit) = self.radial_frame.unwrap_or_else(|| {
            let origin = DVec2::new(self.original.x as f64, self.original.y as f64);
            (origin + half - DVec2::splat(0.5), half.length())
        });
        let radius = (DVec2::new(x as f64, y as f64) - center).length() / radius_limit;
        self.profile.gain(radius)
    }

    #[cfg(feature = "raw")]
    pub(crate) fn identity(&self, algorithm: &[u8]) -> String {
        let mut hash = blake3::Hasher::new();
        hash.update(algorithm);
        if let Some((center, radius)) = self.radial_frame {
            for value in [center.x, center.y, radius] {
                hash.update(&value.to_bits().to_le_bytes());
            }
        }
        hash.update(&(self.profile.attenuation.len() as u32).to_le_bytes());
        for value in &self.profile.attenuation {
            hash.update(&value.to_bits().to_le_bytes());
        }
        for value in [
            self.original.x,
            self.original.y,
            self.original.width,
            self.original.height,
            self.output.x - self.original.x,
            self.output.y - self.original.y,
            self.output.width,
            self.output.height,
            self.profile.knot_count() as u32,
        ] {
            hash.update(&value.to_le_bytes());
        }
        for corrections in [
            LensCorrections {
                distortion: true,
                ca: false,
            },
            LensCorrections {
                distortion: false,
                ca: true,
            },
        ] {
            for i in 0..self.profile.knot_count() {
                for v in self.profile.factors(i, corrections).to_array() {
                    hash.update(&v.to_bits().to_le_bytes());
                }
            }
        }
        hash.finalize().to_hex().to_string()
    }

    #[cfg(feature = "raw")]
    pub(crate) fn retained_bytes(&self) -> usize {
        self.profile.retained_bytes()
    }
}

fn invalid(reason: &'static str) -> Error {
    Error::Raw(reason.into())
}
