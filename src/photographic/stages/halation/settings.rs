use super::HalationParameters;
use crate::Error;

// Appearance fit with Medium image MTF, rounded to control precision; not stock calibration.
pub const DEFAULT_SIZE: f64 = 0.109;
pub const DEFAULT_STRENGTH: f64 = 0.54;
pub const MIN_SIZE: f64 = 0.01;
pub const MAX_SIZE: f64 = 0.50;
pub const MAX_STRENGTH: f64 = 1.;
pub const CONTRACT: &str = "ap1-positive-redistribution-gaussian-crop-v1";

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "project", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "project",
    serde(try_from = "SavedSettings", into = "SavedSettings")
)]
pub struct HalationSettings {
    pub enabled: bool,
    /// Gaussian sigma as a percentage of the accepted crop's shorter side.
    pub size: f64,
    /// Red coefficient k, not the final redistributed fraction k / (1 + k).
    pub strength: f64,
}
impl Default for HalationSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            size: DEFAULT_SIZE,
            strength: DEFAULT_STRENGTH,
        }
    }
}
impl HalationSettings {
    #[cfg(feature = "project")]
    pub(crate) fn is_default(&self) -> bool {
        *self == Self::default()
    }

    pub fn active(self) -> bool {
        self.enabled && self.strength > 0.
    }

    pub fn validate(self) -> Result<(), Error> {
        if !self.size.is_finite()
            || !(MIN_SIZE..=MAX_SIZE).contains(&self.size)
            || !self.strength.is_finite()
            || !(0. ..=MAX_STRENGTH).contains(&self.strength)
        {
            return Err(Error::InvalidParameter(
                "halation size or strength outside supported range",
            ));
        }
        Ok(())
    }

    pub fn parameters(self, crop_size: [u32; 2]) -> Result<Option<HalationParameters>, Error> {
        self.validate()?;
        if !self.active() {
            return Ok(None);
        }
        let extent = crop_size[0].min(crop_size[1]);
        if extent == 0 {
            return Err(Error::InvalidParameter(
                "halation requires cropped frame dimensions",
            ));
        }
        Ok(Some(HalationParameters {
            sigma: self.size / 100. * f64::from(extent),
            strength: self.strength,
        }))
    }
}

#[cfg(feature = "project")]
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedSettings {
    contract: String,
    enabled: bool,
    size: f64,
    strength: f64,
}
#[cfg(feature = "project")]
impl From<HalationSettings> for SavedSettings {
    fn from(value: HalationSettings) -> Self {
        Self {
            contract: CONTRACT.into(),
            enabled: value.enabled,
            size: value.size,
            strength: value.strength,
        }
    }
}
#[cfg(feature = "project")]
impl TryFrom<SavedSettings> for HalationSettings {
    type Error = Error;
    fn try_from(saved: SavedSettings) -> Result<Self, Error> {
        if saved.contract != CONTRACT {
            return Err(Error::InvalidParameter("unsupported halation contract"));
        }
        let settings = Self {
            enabled: saved.enabled,
            size: saved.size,
            strength: saved.strength,
        };
        settings.validate()?;
        Ok(settings)
    }
}
