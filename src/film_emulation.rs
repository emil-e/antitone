//! Film Emulation settings and source-scale parameter resolution.
//!
//! Independent image-MTF and grain stages execute after the complete photographic grade,
//! before the separate creative look and output transform.

use crate::Error;

/// Fine/Medium/Coarse are frame-relative presets. They are not a per-stock catalogue.
pub const FINE_SIZE: f64 = 0.025;
pub const MEDIUM_SIZE: f64 = 0.040;
pub const COARSE_SIZE: f64 = 0.065;
pub const DEFAULT_SIZE: f64 = MEDIUM_SIZE;

pub const DEFAULT_AMOUNT: f64 = 0.055;
pub const MIN_AMOUNT: f64 = 0.;
pub const MAX_AMOUNT: f64 = 0.20;
pub const MIN_SIZE: f64 = 0.005;
pub const MAX_SIZE: f64 = 0.100;
pub const DEFAULT_COLOUR: f64 = 0.5;
pub const MIN_COLOUR: f64 = 0.;
pub const MAX_COLOUR: f64 = 1.;

pub use crate::grain::{GRAIN_BASIS, GrainBasis};
/// Image-response sigma relative to source-resolved Size.
pub const MTF_SIGMA: f64 = 1.0;

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "project", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "project",
    serde(try_from = "SavedSettings", into = "SavedSettings")
)]
pub struct FilmEmulationSettings {
    pub enabled: bool,
    /// Green log-light RMS at the original source-pixel footprint; reduction loses variance.
    pub amount: f64,
    /// MTF/grain scale as a percentage of the cropped frame's shorter side.
    pub size: f64,
    /// Monochrome at zero, fitted scan colour at 0.5, and stronger chroma above it.
    pub colour: f64,
}

/// Identifies the accepted source-scale filter, RGB spectrum and seeded field interpretation.
pub const CONTRACT: &str = "source-mtf-grain-gaussian-rgb-v1";

#[cfg(feature = "project")]
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedSettings {
    contract: String,
    enabled: bool,
    amount: f64,
    size: f64,
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
            colour: DEFAULT_COLOUR,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilmEmulationPreset {
    Fine,
    Medium,
    Coarse,
}

impl FilmEmulationPreset {
    pub const fn settings(self) -> FilmEmulationSettings {
        match self {
            Self::Fine => FilmEmulationSettings {
                enabled: true,
                amount: 0.040,
                size: FINE_SIZE,
                colour: DEFAULT_COLOUR,
            },
            Self::Medium => FilmEmulationSettings {
                enabled: true,
                amount: 0.055,
                size: MEDIUM_SIZE,
                colour: DEFAULT_COLOUR,
            },
            Self::Coarse => FilmEmulationSettings {
                enabled: true,
                amount: 0.065,
                size: COARSE_SIZE,
                colour: DEFAULT_COLOUR,
            },
        }
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
        if !self.amount.is_finite() || !self.size.is_finite() || !self.colour.is_finite() {
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
        if !(MIN_COLOUR..=MAX_COLOUR).contains(&self.colour) {
            return Err(Error::InvalidParameter(
                "film emulation colour must be within 0..=1",
            ));
        }
        Ok(())
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
        let grain_size = self.size / 100.0 * f64::from(frame_extent);
        if !grain_size.is_finite() || grain_size <= 0. {
            return Err(Error::InvalidParameter(
                "film emulation grain scale is not finite",
            ));
        }
        let image_mtf_sigma = MTF_SIGMA * grain_size;
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
    fn default_settings_are_disabled_and_valid() {
        let settings = FilmEmulationSettings::default();
        settings.validate().unwrap();
        assert!(!settings.enabled);
        assert_eq!(settings.parameters([6000; 2]).unwrap(), None);
    }

    #[test]
    fn presets_enable_and_resolve_to_source_scale() {
        for preset in [
            FilmEmulationPreset::Fine,
            FilmEmulationPreset::Medium,
            FilmEmulationPreset::Coarse,
        ] {
            let settings = preset.settings();
            settings.validate().unwrap();
            let parameters = settings.parameters([6000; 2]).unwrap().unwrap();
            assert_eq!(parameters.frame_extent, 6000);
            assert!((parameters.grain_size - settings.size / 100.0 * 6000.).abs() < 1e-9);
            assert!((parameters.image_mtf_sigma - MTF_SIGMA * parameters.grain_size).abs() < 1e-12);
        }
    }

    #[test]
    fn tighter_crop_reduces_source_blur_and_preserves_the_frame_fraction() {
        let settings = FilmEmulationPreset::Medium.settings();
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
        assert!((landscape.grain_size - 1.2).abs() < 1e-12);
        assert!((landscape.image_mtf_sigma - 1.2).abs() < 1e-12);
    }

    fn grain_psd(f: f64) -> f64 {
        GRAIN_BASIS
            .iter()
            .map(|basis| {
                let s = basis.sigma;
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
    fn fitted_covariance_has_scan_like_channel_statistics() {
        let parameters = FilmEmulationPreset::Medium
            .settings()
            .parameters([6000; 2])
            .unwrap()
            .unwrap();
        let c = parameters.covariance(1.);
        assert!((c[1][1] - 1.).abs() < 1e-12);
        let rg = c[0][1] / (c[0][0] * c[1][1]).sqrt();
        assert!((0.5..0.8).contains(&rg));
        assert!((1.3..1.9).contains(&c[2][2].sqrt()));
        let reduced = parameters.channel_variance(4.);
        assert!((0.4..0.7).contains(&reduced[1].sqrt()));
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
        for colour in [0., 0.5, 1.] {
            let settings = FilmEmulationSettings {
                enabled: true,
                colour,
                ..Default::default()
            };
            let parameters = settings.parameters([6000; 2]).unwrap().unwrap();
            let covariance = parameters.covariance(1.);
            assert!((covariance[1][1] - 1.).abs() < 1e-12);
            if colour == 0. {
                for row in covariance {
                    for value in row {
                        assert!((value - 1.).abs() < 1e-12);
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
        let settings = FilmEmulationPreset::Fine.settings();
        assert!(settings.parameters([0; 2]).is_err());
        assert!(settings.parameters([4000, 0]).is_err());
    }
}
