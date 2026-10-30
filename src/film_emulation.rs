//! Film Emulation settings and source-scale parameter resolution.
//!
//! Independent image-MTF and grain stages execute after the complete photographic grade,
//! before the separate creative look and output transform.

use crate::Error;

pub const DEFAULT_SIZE: f64 = 0.040;
pub const DEFAULT_IMAGE_SIGMA: f64 = 0.040;

pub const DEFAULT_AMOUNT: f64 = 0.065;
pub const MIN_AMOUNT: f64 = 0.;
pub const MAX_AMOUNT: f64 = 0.20;
pub const MIN_SIZE: f64 = 0.005;
pub const MAX_SIZE: f64 = 0.100;
pub const DEFAULT_COLOUR: f64 = 0.5;
pub const MIN_COLOUR: f64 = 0.;
pub const MAX_COLOUR: f64 = 1.;

pub use crate::grain::{GRAIN_BASIS, GrainBasis};
/// 35mm reference image sigma per grain-basis Size unit; authored sigma is independent.
pub const MTF_SIGMA: f64 = 1.0;
/// Current grain spectrum scale relative to Size; both lengths follow the same adjustment.
pub const GRAIN_SCALE: f64 = 1.375;

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "project", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "project",
    serde(try_from = "SavedSettings", into = "SavedSettings")
)]
pub struct FilmEmulationSettings {
    pub enabled: bool,
    /// Green log-light RMS before pixel integration; averaging reduces visible variance.
    pub amount: f64,
    /// Grain basis scale as a percentage of the cropped frame's shorter side.
    pub size: f64,
    /// Independent image-response sigma in the same crop-relative units.
    pub image_sigma: f64,
    /// Monochrome at zero, fitted scan colour at 0.5, and stronger chroma above it.
    pub colour: f64,
}

/// Identifies the crop-relative filter, RGB spectrum and pre-integration RMS interpretation.
pub const CONTRACT: &str = "frame-mtf-grain-gaussian-rgb-v4";

#[cfg(feature = "project")]
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedSettings {
    contract: String,
    enabled: bool,
    amount: f64,
    size: f64,
    image_sigma: f64,
    colour: f64,
}

#[cfg(feature = "project")]
impl From<FilmEmulationSettings> for SavedSettings {
    fn from(settings: FilmEmulationSettings) -> Self {
        Self {
            contract: CONTRACT.into(),
            enabled: settings.enabled,
            amount: settings.amount,
            size: settings.size,
            image_sigma: settings.image_sigma,
            colour: settings.colour,
        }
    }
}

#[cfg(feature = "project")]
impl TryFrom<SavedSettings> for FilmEmulationSettings {
    type Error = Error;

    fn try_from(saved: SavedSettings) -> Result<Self, Error> {
        if saved.contract != CONTRACT {
            return Err(Error::InvalidParameter(
                "unsupported film emulation contract",
            ));
        }
        let settings = Self {
            enabled: saved.enabled,
            amount: saved.amount,
            size: saved.size,
            image_sigma: saved.image_sigma,
            colour: saved.colour,
        };
        settings.validate()?;
        Ok(settings)
    }
}

impl Default for FilmEmulationSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            amount: DEFAULT_AMOUNT,
            size: DEFAULT_SIZE,
            image_sigma: DEFAULT_IMAGE_SIGMA,
            colour: DEFAULT_COLOUR,
        }
    }
}

/// Starting-value provenance, not a dependency of saved rendering parameters.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "project", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "project", serde(rename_all = "snake_case"))]
pub enum FilmPreset {
    #[default]
    ThirtyFiveMm,
    MediumFormat,
}

impl FilmPreset {
    pub const fn label(self) -> &'static str {
        match self {
            Self::ThirtyFiveMm => "35mm",
            Self::MediumFormat => "Medium format",
        }
    }

    fn spatial_scale(self) -> f64 {
        match self {
            Self::ThirtyFiveMm => 1.,
            Self::MediumFormat => 24. / 56.,
        }
    }

    pub fn settings(self) -> FilmEmulationSettings {
        let scale = self.spatial_scale();
        FilmEmulationSettings {
            enabled: true,
            amount: DEFAULT_AMOUNT,
            size: DEFAULT_SIZE * scale,
            image_sigma: DEFAULT_IMAGE_SIGMA * scale,
            colour: DEFAULT_COLOUR,
        }
    }

    pub fn halation(self) -> crate::halation::HalationSettings {
        crate::halation::HalationSettings {
            size: crate::halation::DEFAULT_SIZE * self.spatial_scale(),
            ..Default::default()
        }
    }

    pub fn matches(
        self,
        texture: FilmEmulationSettings,
        halo: crate::halation::HalationSettings,
    ) -> bool {
        let reference = self.settings();
        let reference_halo = self.halation();
        texture.amount == reference.amount
            && texture.size == reference.size
            && texture.image_sigma == reference.image_sigma
            && texture.colour == reference.colour
            && halo.size == reference_halo.size
            && halo.strength == reference_halo.strength
    }
}

/// Resolved source-scale parameters for one photo. All lengths are source pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FilmEmulationParameters {
    pub frame_extent: u32,
    pub grain_size: f64,
    pub image_mtf_sigma: f64,
    pub amplitude: f64,
    pub colour: f64,
}

impl FilmEmulationParameters {
    pub(crate) fn grain(self) -> crate::grain::GrainParameters {
        crate::grain::GrainParameters {
            grain_size: self.grain_size,
            amplitude: self.amplitude,
            colour: self.colour,
        }
    }

    #[cfg(test)]
    pub(crate) fn channel_variance(self, footprint: f64) -> [f64; 3] {
        self.grain().channel_variance(footprint)
    }

    #[cfg(test)]
    pub(crate) fn covariance(self, footprint: f64) -> [[f64; 3]; 3] {
        self.grain().covariance(footprint)
    }
}

impl FilmEmulationSettings {
    #[cfg(feature = "project")]
    pub(crate) fn is_default(&self) -> bool {
        *self == Self::default()
    }

    pub fn validate(self) -> Result<(), Error> {
        if !self.amount.is_finite()
            || !self.size.is_finite()
            || !self.image_sigma.is_finite()
            || !self.colour.is_finite()
        {
            return Err(Error::InvalidParameter(
                "film emulation settings must be finite",
            ));
        }
        if !(MIN_AMOUNT..=MAX_AMOUNT).contains(&self.amount) {
            return Err(Error::InvalidParameter(
                "film emulation amount must be within 0..=0.20",
            ));
        }
        if !(MIN_SIZE..=MAX_SIZE).contains(&self.size) {
            return Err(Error::InvalidParameter(
                "film emulation size must be within 0.005%..=0.100% of the cropped frame's shorter side",
            ));
        }
        if !(MIN_SIZE..=MAX_SIZE).contains(&self.image_sigma) {
            return Err(Error::InvalidParameter(
                "film image sigma outside supported range",
            ));
        }
        if !(MIN_COLOUR..=MAX_COLOUR).contains(&self.colour) {
            return Err(Error::InvalidParameter(
                "film emulation colour must be within 0..=1",
            ));
        }
        Ok(())
    }

    /// Resize the current pair without substituting a preset's grain/image relationship.
    pub fn resized(self, size: f64) -> Self {
        Self {
            size,
            image_sigma: self.image_sigma * (size / self.size),
            ..self
        }
    }

    /// Resolve source-unit lengths from the logical crop, never the viewport or padded raster.
    pub fn parameters(
        self,
        frame_size: [u32; 2],
    ) -> Result<Option<FilmEmulationParameters>, Error> {
        self.validate()?;
        if !self.enabled {
            return Ok(None);
        }
        let frame_extent = frame_size[0].min(frame_size[1]);
        if frame_extent == 0 {
            return Err(Error::InvalidParameter(
                "film emulation requires cropped frame dimensions",
            ));
        }
        let scale = self.size / 100.0 * f64::from(frame_extent);
        let grain_size = GRAIN_SCALE * scale;
        if !grain_size.is_finite() || grain_size <= 0. {
            return Err(Error::InvalidParameter(
                "film emulation grain scale is not finite",
            ));
        }
        let image_mtf_sigma = self.image_sigma / 100.0 * f64::from(frame_extent);
        Ok(Some(FilmEmulationParameters {
            frame_extent,
            grain_size,
            image_mtf_sigma,
            amplitude: self.amount,
            colour: self.colour,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_scales_lengths_without_changing_continuous_grain_or_halo_strength() {
        let small = FilmPreset::ThirtyFiveMm
            .settings()
            .parameters([5600; 2])
            .unwrap()
            .unwrap();
        let medium_settings = FilmPreset::MediumFormat.settings();
        let medium = medium_settings.parameters([5600; 2]).unwrap().unwrap();
        assert!((small.grain_size - 3.08).abs() < 1e-12);
        assert!((medium.grain_size - 1.32).abs() < 1e-12);
        assert!((medium.image_mtf_sigma - 0.96).abs() < 1e-12);
        assert_eq!(medium.amplitude, small.amplitude);
        assert_eq!(medium.colour, small.colour);
        assert_eq!(
            FilmPreset::MediumFormat.halation().strength,
            FilmPreset::ThirtyFiveMm.halation().strength
        );
        assert!((FilmPreset::MediumFormat.halation().size - 0.046714285714285715).abs() < 1e-14);
        assert!(
            medium.channel_variance(4.)[1] < small.channel_variance(4.)[1],
            "pixel averaging supplies the lower visible contrast, not an Amount reduction"
        );
    }

    #[test]
    fn saved_lengths_are_independent_and_size_edits_preserve_the_authored_relationship() {
        let original = FilmEmulationSettings {
            enabled: true,
            size: 0.025,
            image_sigma: 0.050,
            ..Default::default()
        };
        let resized = original.resized(0.040);
        assert!((resized.image_sigma - 0.080).abs() < 1e-14);
        assert_eq!(resized.amount, original.amount);
        let parameters = original.parameters([4000; 2]).unwrap().unwrap();
        assert!((parameters.image_mtf_sigma - 2.).abs() < 1e-14);
        assert!((parameters.grain_size - 1.375).abs() < 1e-14);
        assert!(
            FilmEmulationSettings {
                image_sigma: f64::NAN,
                ..original
            }
            .validate()
            .is_err()
        );
        assert!(
            FilmEmulationSettings {
                image_sigma: 0.,
                ..original
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn default_settings_are_disabled_and_valid() {
        let settings = FilmEmulationSettings::default();
        settings.validate().unwrap();
        assert!(!settings.enabled);
        assert_eq!(settings.parameters([6000; 2]).unwrap(), None);
    }

    #[test]
    fn presets_enable_and_resolve_to_source_scale() {
        for preset in [FilmPreset::ThirtyFiveMm, FilmPreset::MediumFormat] {
            let settings = preset.settings();
            settings.validate().unwrap();
            let parameters = settings.parameters([6000; 2]).unwrap().unwrap();
            assert_eq!(parameters.frame_extent, 6000);
            let scale = settings.size / 100.0 * 6000.;
            assert!((parameters.grain_size - GRAIN_SCALE * scale).abs() < 1e-9);
            assert!(
                (parameters.image_mtf_sigma - settings.image_sigma / 100. * 6000.).abs() < 1e-12
            );
        }
    }

    #[test]
    fn tighter_crop_reduces_source_blur_and_preserves_the_frame_fraction() {
        let settings = FilmPreset::ThirtyFiveMm.settings();
        let small = settings.parameters([4000; 2]).unwrap().unwrap();
        let large = settings.parameters([8000; 2]).unwrap().unwrap();
        assert!((small.grain_size / 4000. - large.grain_size / 8000.).abs() < 1e-12);
        assert!(large.grain_size > small.grain_size);
        assert!(large.image_mtf_sigma > small.image_mtf_sigma);
    }

    #[test]
    fn shorter_crop_dimension_controls_both_stages_without_orientation_dependence() {
        let settings = FilmEmulationSettings {
            enabled: true,
            size: 0.040,
            ..Default::default()
        };
        let landscape = settings.parameters([6000, 3000]).unwrap().unwrap();
        let portrait = settings.parameters([3000, 6000]).unwrap().unwrap();
        assert_eq!(landscape.frame_extent, 3000);
        assert_eq!(landscape.grain_size, portrait.grain_size);
        assert_eq!(landscape.image_mtf_sigma, portrait.image_mtf_sigma);
        assert!((landscape.grain_size - 1.65).abs() < 1e-12);
        assert!((landscape.image_mtf_sigma - 1.2).abs() < 1e-12);
    }

    fn grain_psd(f: f64) -> f64 {
        GRAIN_BASIS
            .iter()
            .map(|basis| {
                let s = basis.sigma * GRAIN_SCALE;
                let power: f64 = basis.factor[1].iter().map(|v| v * v).sum();
                power
                    * 4.
                    * std::f64::consts::PI
                    * s
                    * s
                    * (-4. * std::f64::consts::PI.powi(2) * s * s * f * f).exp()
            })
            .sum()
    }

    #[test]
    fn profile_covariance_preserves_channel_structure_and_aperture_loss() {
        let parameters = FilmPreset::ThirtyFiveMm
            .settings()
            .parameters([6000; 2])
            .unwrap()
            .unwrap();
        let c = parameters.covariance(1.);
        // Independent Gaussian-spectrum integral for this scale and one-pixel aperture.
        assert!((c[1][1] - 0.9232917615858444).abs() < 1e-12);
        let rg = c[0][1] / (c[0][0] * c[1][1]).sqrt();
        assert!((0.5..0.8).contains(&rg));
        assert!((1.3..1.9).contains(&(c[2][2] / c[1][1]).sqrt()));
        let reduced = parameters.channel_variance(4.);
        assert!((reduced[1] - 0.5089985772459611).abs() < 1e-12);
    }

    #[test]
    fn grain_has_more_normalized_high_frequency_energy_than_the_image_psf() {
        // Compare every normalized upper tail of the fitted grain POWER with the fixed image
        // point response's power. This protects their bandwidth ordering, not absolute contrast
        // of arbitrary image content; it does not assume a particular filter implementation.
        let mut image = Vec::new();
        let mut grain = Vec::new();
        for i in 0..12000 {
            let f = (i as f64 + 0.5) / 1000.;
            let q = 2. * std::f64::consts::PI.powi(2) * f * f;
            image.push(f * (-2. * q * MTF_SIGMA.powi(2)).exp());
            grain.push(f * grain_psd(f));
        }
        let image_total: f64 = image.iter().sum();
        let grain_total: f64 = grain.iter().sum();
        let mut image_tail = 0.;
        let mut grain_tail = 0.;
        for i in (0..image.len()).rev() {
            image_tail += image[i] / image_total;
            grain_tail += grain[i] / grain_total;
            assert!(
                grain_tail + 1e-12 >= image_tail,
                "tail at f={}",
                i as f64 / 1000.
            );
        }
        let rms = |energy: &[f64], total: f64| {
            (energy
                .iter()
                .enumerate()
                .map(|(i, e)| e * ((i as f64 + 0.5) / 1000.).powi(2))
                .sum::<f64>()
                / total)
                .sqrt()
        };
        eprintln!(
            "ideal normalized RMS frequency: image={} / L, grain={} / L",
            rms(&image, image_total),
            rms(&grain, grain_total)
        );
    }

    #[test]
    fn monochrome_is_shared_and_green_strength_does_not_depend_on_colour() {
        let green_variance = FilmPreset::ThirtyFiveMm
            .settings()
            .parameters([6000; 2])
            .unwrap()
            .unwrap()
            .channel_variance(1.)[1];
        for colour in [0., 0.5, 1.] {
            let settings = FilmEmulationSettings {
                enabled: true,
                colour,
                ..Default::default()
            };
            let parameters = settings.parameters([6000; 2]).unwrap().unwrap();
            let covariance = parameters.covariance(1.);
            assert!((covariance[1][1] - green_variance).abs() < 1e-12);
            if colour == 0. {
                for row in covariance {
                    for value in row {
                        assert!((value - green_variance).abs() < 1e-12);
                    }
                }
            }
        }
    }

    #[test]
    fn rejects_non_finite_and_out_of_range_values() {
        let base = FilmEmulationSettings::default();
        for settings in [
            FilmEmulationSettings {
                amount: f64::NAN,
                ..base
            },
            FilmEmulationSettings {
                amount: MAX_AMOUNT + 1e-9,
                ..base
            },
            FilmEmulationSettings {
                size: MIN_SIZE / 2.,
                ..base
            },
            FilmEmulationSettings {
                size: MAX_SIZE + 1e-9,
                ..base
            },
            FilmEmulationSettings {
                colour: MAX_COLOUR + 1e-9,
                ..base
            },
        ] {
            assert!(settings.validate().is_err());
        }
    }

    #[test]
    fn film_emulation_is_visible_only_for_digital_input() {
        use crate::color::Balance;
        use crate::processing::{DEFAULT_GAMMA, InputSettings, Response};

        let digital = InputSettings::Digital.capabilities();
        assert!(digital.film_emulation);
        assert!(!digital.scan_sources);

        let negative = InputSettings::Negative {
            dmin: None,
            dmax: None,
            input_floor: crate::processing::CAMERA_INPUT_FLOOR,
            response: Response::film(),
            gamma: DEFAULT_GAMMA,
            balance: Balance::default(),
        }
        .capabilities();
        assert!(!negative.film_emulation);
        assert!(negative.scan_sources);
    }

    #[test]
    fn enabled_settings_require_cropped_frame_dimensions() {
        let settings = FilmPreset::MediumFormat.settings();
        assert!(settings.parameters([0; 2]).is_err());
        assert!(settings.parameters([4000, 0]).is_err());
    }
}
