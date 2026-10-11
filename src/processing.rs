use glam::{DMat3, DVec3};

use crate::Error;
use crate::color::{Balance, wb_matrix};

pub const CAMERA_INPUT_FLOOR: f64 = 1. / 65535.;
pub const DEFAULT_GAMMA: f64 = 0.65;
pub const MIN_GAMMA: f64 = 0.1;
pub const MAX_GAMMA: f64 = 3.0;
pub const MAX_EXPOSURE: f64 = 16.0;
pub const MIN_ANCHOR_LOG_SPAN: f64 = 1e-6;
pub const DEFAULT_TOE: f64 = 2.5;
pub const MIN_TOE: f64 = 1.0;
pub const MAX_TOE: f64 = 4.0;
pub const MAX_SHOULDER: f64 = 0.02;

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "project", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "project",
    serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)
)]
pub enum Response {
    Original,
    Film { toe: f64, shoulder: f64 },
}

impl Response {
    pub const fn original() -> Self {
        Self::Original
    }
    pub const fn film() -> Self {
        Self::Film {
            toe: DEFAULT_TOE,
            shoulder: 0.0,
        }
    }
    pub fn is_original(&self) -> bool {
        matches!(self, Self::Original)
    }
    pub fn processing_model(self) -> u32 {
        crate::PROCESSING_MODEL_VERSION
    }
    pub fn shape(self) -> Result<(f64, f64), Error> {
        let Self::Film { toe, shoulder } = self else {
            return Ok((1.0, 0.0));
        };
        if !toe.is_finite()
            || !(MIN_TOE..=MAX_TOE).contains(&toe)
            || !shoulder.is_finite()
            || !(0.0..=MAX_SHOULDER).contains(&shoulder)
        {
            return Err(Error::InvalidParameter(
                "toe must be within 1..4 and shoulder within 0..0.02",
            ));
        }
        Ok((toe, shoulder))
    }
}

/// Linear native-camera RGB measurements, not optical densities.
/// Dmin (film base) transmits more light than Dmax: dmin > dmax per channel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Calibration {
    pub dmin: DVec3,
    pub dmax: DVec3,
    pub input_floor: f64,
}

impl Calibration {
    pub fn validate(self) -> Result<Self, Error> {
        if !self.input_floor.is_finite() || self.input_floor <= 0.0 {
            return Err(Error::InvalidParameter(
                "scan input floor must be finite and strictly positive",
            ));
        }
        for c in 0..3 {
            let dmin = self.dmin[c];
            let dmax = self.dmax[c];
            let reason = if !dmin.is_finite() || !dmax.is_finite() {
                Some("references must be finite")
            } else if dmax < self.input_floor || dmin <= dmax {
                Some("require Dmin RGB > Dmax RGB >= input floor > 0")
            } else if log_ratio(dmin, dmax) < MIN_ANCHOR_LOG_SPAN {
                Some("references are too close together")
            } else {
                None
            };
            if let Some(reason) = reason {
                return Err(Error::InvalidCalibration { channel: c, reason });
            }
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "project", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "project", serde(deny_unknown_fields))]
pub struct RollSettings {
    pub response: Response,
    pub gamma: f64,
    pub balance: Balance,
    pub exposure: f64,
}

impl Default for RollSettings {
    fn default() -> Self {
        Self {
            response: Response::Original,
            gamma: DEFAULT_GAMMA,
            balance: Balance::default(),
            exposure: 0.0,
        }
    }
}

impl RollSettings {
    pub fn new_grade() -> Self {
        Self {
            response: Response::film(),
            ..Self::default()
        }
    }

    pub fn channel_gammas(self) -> Result<DVec3, Error> {
        self.response.shape()?;
        if !self.gamma.is_finite() || !(MIN_GAMMA..=MAX_GAMMA).contains(&self.gamma) {
            return Err(Error::InvalidParameter(
                "global gamma must be within 0.1..3",
            ));
        }
        Ok(self.balance.offsets()?.exp() * self.gamma)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
#[cfg_attr(feature = "project", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "project", serde(deny_unknown_fields))]
pub struct PhotoSettings {
    pub exposure: f64,
    pub wb: Balance,
}

pub fn exposure_multiplier(stops: f64) -> Result<f64, Error> {
    if !stops.is_finite() || stops.abs() > MAX_EXPOSURE {
        return Err(Error::InvalidParameter(
            "exposure must be within -16..16 stops",
        ));
    }
    Ok(stops.exp2())
}

#[derive(Debug, Clone)]
pub struct Inversion {
    calibration: Calibration,
    gammas: DVec3,
    pub(crate) anchor_exponents: DVec3,
    shape: (f64, f64),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InvertedPixel {
    pub rgb: DVec3,
    pub floored: [bool; 3],
}

impl Inversion {
    pub fn new(calibration: Calibration, roll: RollSettings) -> Result<Self, Error> {
        calibration.validate()?;
        let gammas = roll.channel_gammas()?;
        let (toe, shoulder) = roll.response.shape()?;
        for c in 0..3 {
            let density =
                log_ratio(calibration.dmin[c], calibration.input_floor) / std::f64::consts::LN_10;
            if gammas[c] - shoulder * density <= 0.0 {
                return Err(Error::InvalidParameter(
                    "shoulder pole falls within the scan-floor domain; reduce shoulder",
                ));
            }
        }
        let anchor_exponents = DVec3::from_array(std::array::from_fn(|c| {
            let l = log_ratio(calibration.dmin[c], calibration.dmax[c]);
            if toe == 1.0 && shoulder == 0.0 {
                l / gammas[c]
            } else {
                toe * l / (gammas[c] - shoulder * l / std::f64::consts::LN_10)
            }
        }));
        if !anchor_exponents.is_finite() {
            return Err(Error::NumericalOverflow);
        }
        Ok(Self {
            calibration,
            gammas,
            anchor_exponents,
            shape: (toe, shoulder),
        })
    }

    /// Scan, calibration and output are native-camera RGB.
    pub fn apply(&self, scan: DVec3) -> Result<InvertedPixel, Error> {
        let mut rgb = DVec3::ZERO;
        let mut floored = [false; 3];
        let (toe, shoulder) = self.shape;
        for c in 0..3 {
            if !scan[c].is_finite() {
                return Err(Error::NonFinitePixel { channel: c });
            }
            floored[c] = scan[c] < self.calibration.input_floor;
            let s = scan[c].max(self.calibration.input_floor);
            let l = log_ratio(self.calibration.dmin[c], s);
            rgb[c] = if toe == 1.0 && shoulder == 0.0 {
                expm1_ratio(l / self.gammas[c], self.anchor_exponents[c])
            } else {
                let a = toe * l / (self.gammas[c] - shoulder * l / std::f64::consts::LN_10);
                if a == 0.0 {
                    0.0
                } else {
                    // Numerical continuation for above-base noise/reference overshoots, not film radiometry.
                    a.signum()
                        * ((log_abs_expm1(a) - log_abs_expm1(self.anchor_exponents[c])) / toe).exp()
                }
            };
            if !rgb[c].is_finite() {
                return Err(Error::NumericalOverflow);
            }
        }
        Ok(InvertedPixel { rgb, floored })
    }
}

#[derive(Debug, Clone)]
pub struct Pipeline {
    inversion: Inversion,
    post_inversion_matrix: DMat3,
}

impl Pipeline {
    pub fn new(
        calibration: Calibration,
        roll: RollSettings,
        photo: PhotoSettings,
        camera_to_ap1: DMat3,
    ) -> Result<Self, Error> {
        validate_camera_matrix(camera_to_ap1)?;
        let gain = exposure_multiplier(roll.exposure)? * exposure_multiplier(photo.exposure)?;
        let post_inversion_matrix = wb_matrix(photo.wb)? * camera_to_ap1 * gain;
        if !post_inversion_matrix.is_finite() {
            return Err(Error::NumericalOverflow);
        }
        Ok(Self {
            inversion: Inversion::new(calibration, roll)?,
            post_inversion_matrix,
        })
    }

    /// Returns unclipped AP1 working RGB; scene-linear AP0 export converts at its boundary.
    pub fn apply(&self, scan: DVec3) -> Result<InvertedPixel, Error> {
        let mut pixel = self.inversion.apply(scan)?;
        pixel.rgb = self.post_inversion_matrix * pixel.rgb;
        if !pixel.rgb.is_finite() {
            return Err(Error::NumericalOverflow);
        }
        Ok(pixel)
    }
}

pub(crate) fn validate_camera_matrix(matrix: DMat3) -> Result<(), Error> {
    let determinant = matrix.determinant();
    if !matrix.is_finite() || !determinant.is_finite() || determinant.abs() <= 1e-12 {
        return Err(Error::InvalidParameter("invalid camera-to-AP1 matrix"));
    }
    Ok(())
}

pub(crate) fn log_abs_expm1(a: f64) -> f64 {
    if a > 0.0 {
        a + (-(-a).exp_m1()).ln()
    } else {
        (-a.exp_m1()).ln()
    }
}

pub(crate) fn log_ratio(a: f64, b: f64) -> f64 {
    let relative = (a - b) / b;
    if relative.abs() < 0.5 {
        relative.ln_1p()
    } else {
        a.ln() - b.ln()
    }
}

fn expm1_ratio(a: f64, b: f64) -> f64 {
    if a > 50.0 || b > 50.0 {
        if a >= 0.0 {
            (a - b).exp() * (-(-a).exp_m1()) / (-(-b).exp_m1())
        } else {
            a.exp_m1() * (-b).exp() / (-(-b).exp_m1())
        }
    } else {
        a.exp_m1() / b.exp_m1()
    }
}
