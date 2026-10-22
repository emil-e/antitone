use glam::{DMat3, DVec3};

use crate::Error;
use crate::color::{Balance, wb_matrix};

pub const CAMERA_INPUT_FLOOR: f64 = 1. / 65535.;
/// Continuous-source admission envelope, covering the full signed Sony shading table (< 2^8).
pub const MAX_SOURCE_GAIN: f64 = 256.;
pub const MAX_CAMERA_SOURCE_CODE: f32 = 65535. * MAX_SOURCE_GAIN as f32;
pub const DEFAULT_GAMMA: f64 = 0.60;
pub const MIN_GAMMA: f64 = 0.1;
pub const MAX_GAMMA: f64 = 3.0;
pub const MAX_EXPOSURE: f64 = 16.0;
pub const MIN_ANCHOR_LOG_SPAN: f64 = 1e-6;
pub const DEFAULT_TOE: f64 = 2.5;
pub const MIN_TOE: f64 = 1.0;
pub const MAX_TOE: f64 = 4.0;
pub const MAX_SHOULDER: f64 = 0.02;
pub const DEFAULT_PIVOT: f64 = 0.18;
pub const DEFAULT_COLOUR_MIX: f64 = 0.;

/// How source samples become a scene-referred positive. Persisted per roll.
/// Negative owns its calibration and response; Digital is already a positive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "project", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "project",
    serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)
)]
pub enum InputKind {
    /// Camera scans of developed negative film: invert between measured anchors.
    Negative,
    /// An ordinary scene-referred photograph: interpret as already positive.
    #[default]
    Digital,
}

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
#[cfg_attr(feature = "project", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "project", serde(deny_unknown_fields))]
pub struct Calibration {
    pub dmin: DVec3,
    pub dmax: DVec3,
    pub input_floor: f64,
}

impl Calibration {
    /// Derive a processing floor once, without changing the stored native-base calibration.
    pub(crate) fn for_source(
        mut self,
        source_floor: f64,
        residual_gain: f64,
    ) -> Result<Self, Error> {
        if self.input_floor != CAMERA_INPUT_FLOOR
            || !source_floor.is_finite()
            || source_floor < CAMERA_INPUT_FLOOR
            || !residual_gain.is_finite()
            || residual_gain < 1.
        {
            return Err(Error::InvalidParameter(
                "source calibration requires the native base floor and conservative gain bounds",
            ));
        }
        self.input_floor = source_floor * residual_gain;
        self.validate()
    }

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

/// Grading shared across a roll, applied to the positive after interpretation.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[cfg_attr(feature = "project", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "project", serde(deny_unknown_fields))]
pub struct RollSettings {
    #[cfg_attr(feature = "project", serde(default))]
    pub wb: Balance,
    pub exposure: f64,
}

/// A negative scan's interpretation inputs: measured anchors and the response curve.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NegativeInput {
    pub calibration: Calibration,
    pub response: Response,
    pub gamma: f64,
    pub balance: Balance,
}

impl NegativeInput {
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

/// How source samples become a scene-referred positive. Per input stage: Negative inverts
/// between measured anchors; Digital is already a positive. Calibration lives here, not in
/// the shared grade.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "project", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "project",
    serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)
)]
pub enum InputSettings {
    /// Camera scans of developed negative film: invert between measured anchors.
    /// Dmin and Dmax are picked one at a time; both are needed to interpret.
    Negative {
        dmin: Option<DVec3>,
        dmax: Option<DVec3>,
        input_floor: f64,
        response: Response,
        gamma: f64,
        balance: Balance,
    },
    /// An ordinary scene-referred photograph: already a scene-referred positive.
    Digital,
}

impl InputSettings {
    /// A fresh negative interpretation over measured anchors, with the shared response prior.
    pub fn negative(calibration: Calibration) -> Self {
        Self::Negative {
            dmin: Some(calibration.dmin),
            dmax: Some(calibration.dmax),
            input_floor: calibration.input_floor,
            response: Response::film(),
            gamma: DEFAULT_GAMMA,
            balance: Balance::default(),
        }
    }

    /// The complete calibration once both anchors are measured.
    pub fn calibration(&self) -> Option<Calibration> {
        match self {
            Self::Negative {
                dmin: Some(dmin),
                dmax: Some(dmax),
                input_floor,
                ..
            } => Some(Calibration {
                dmin: *dmin,
                dmax: *dmax,
                input_floor: *input_floor,
            }),
            _ => None,
        }
    }

    /// The complete negative interpretation for processing, once calibrated.
    pub fn negative_input(&self) -> Option<NegativeInput> {
        match self {
            Self::Negative {
                response, gamma, balance, ..
            } => Some(NegativeInput {
                calibration: self.calibration()?,
                response: *response,
                gamma: *gamma,
                balance: *balance,
            }),
            Self::Digital => None,
        }
    }

    pub fn is_negative(&self) -> bool {
        matches!(self, Self::Negative { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "project", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "project", serde(deny_unknown_fields))]
pub struct PhotoSettings {
    pub exposure: f64,
    pub wb: Balance,
    #[cfg_attr(feature = "project", serde(default = "identity_amount"))]
    pub contrast: f64,
    #[cfg_attr(feature = "project", serde(default = "default_pivot"))]
    pub pivot: f64,
    #[cfg_attr(feature = "project", serde(default = "default_colour_mix"))]
    pub contrast_colour_mix: f64,
    #[cfg_attr(feature = "project", serde(default = "identity_amount"))]
    pub saturation: f64,
    #[cfg_attr(feature = "project", serde(default = "default_colour_mix"))]
    pub saturation_density: f64,
}

fn identity_amount() -> f64 {
    1.
}

fn default_pivot() -> f64 {
    DEFAULT_PIVOT
}

fn default_colour_mix() -> f64 {
    DEFAULT_COLOUR_MIX
}

impl Default for PhotoSettings {
    fn default() -> Self {
        Self {
            exposure: 0.,
            wb: Balance::default(),
            contrast: identity_amount(),
            pivot: default_pivot(),
            contrast_colour_mix: default_colour_mix(),
            saturation: identity_amount(),
            saturation_density: default_colour_mix(),
        }
    }
}

impl PhotoSettings {
    pub fn validate(self) -> Result<Self, Error> {
        exposure_multiplier(self.exposure)?;
        wb_matrix(self.wb)?;
        crate::photographic::Grade::new(self)?;
        Ok(self)
    }

    /// Measurement is before Photo WB and creative grading, retaining exposure.
    pub fn before_white_balance(self) -> Self {
        Self {
            exposure: self.exposure,
            ..Self::default()
        }
    }
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

/// The input stage: source sample → scene-referred native-camera positive.
/// Everything after this is grading and is blind to which stage ran.
#[derive(Debug, Clone)]
enum Interpretation {
    Negative(Inversion),
    Digital,
}

impl Interpretation {
    fn interpret(&self, scan: DVec3) -> Result<InvertedPixel, Error> {
        match self {
            Self::Negative(inversion) => inversion.apply(scan),
            // A digital source is already a scene-referred positive; no polarity
            // flip and no density flooring.
            Self::Digital => {
                for c in 0..3 {
                    if !scan[c].is_finite() {
                        return Err(Error::NonFinitePixel { channel: c });
                    }
                }
                Ok(InvertedPixel {
                    rgb: scan,
                    floored: [false; 3],
                })
            }
        }
    }
}

impl Inversion {
    pub fn new(negative: NegativeInput) -> Result<Self, Error> {
        let calibration = negative.calibration;
        calibration.validate()?;
        let gammas = negative.channel_gammas()?;
        let (toe, shoulder) = negative.response.shape()?;
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
    interpretation: Interpretation,
    /// Native-camera positive → pre-contrast graded AP1: camera→AP1, then roll/photo WB
    /// and exposure. Contrast/saturation follow in `grade`. Used by `apply`, which starts
    /// from a source sample. (Not named for "inversion": Digital uses it too.)
    to_graded_ap1: DMat3,
    /// Working AP1 → pre-contrast graded AP1: roll/photo WB and exposure, *without*
    /// camera→AP1 (already applied). Used by `grade_neutral`, which starts from an
    /// already-converted working positive such as a neutral repair replacement.
    working_to_graded: DMat3,
    grade: crate::photographic::Grade,
}

impl Pipeline {
    /// Build the graded pipeline for a source interpreted through `input`.
    pub fn new(
        input: &InputSettings,
        roll: RollSettings,
        photo: PhotoSettings,
        camera_to_ap1: DMat3,
    ) -> Result<Self, Error> {
        Self::build(Self::interpret(input)?, roll, photo, camera_to_ap1)
    }

    /// Full-resolution signed/HDR AP1 after correction, interpretation and camera conversion,
    /// before grading. The caller supplies camera samples corrected in the same basis.
    pub fn ungraded(input: &InputSettings, camera_to_ap1: DMat3) -> Result<Self, Error> {
        Self::new(
            input,
            RollSettings::default(),
            PhotoSettings::default(),
            camera_to_ap1,
        )
    }

    fn interpret(input: &InputSettings) -> Result<Interpretation, Error> {
        match input {
            InputSettings::Digital => Ok(Interpretation::Digital),
            InputSettings::Negative { .. } => {
                let negative = input.negative_input().ok_or_else(|| {
                    Error::InvalidCalibration {
                        channel: 0,
                        reason: "measure Dmin and Dmax",
                    }
                })?;
                Ok(Interpretation::Negative(Inversion::new(negative)?))
            }
        }
    }

    /// Shared grading for every input stage; only `interpret` differs.
    fn build(
        interpretation: Interpretation,
        roll: RollSettings,
        photo: PhotoSettings,
        camera_to_ap1: DMat3,
    ) -> Result<Self, Error> {
        validate_camera_matrix(camera_to_ap1)?;
        photo.validate()?;
        let gain = exposure_multiplier(roll.exposure)? * exposure_multiplier(photo.exposure)?;
        let to_graded_ap1 =
            wb_matrix(photo.wb)? * wb_matrix(roll.wb)? * camera_to_ap1 * gain;
        if !to_graded_ap1.is_finite() {
            return Err(Error::NumericalOverflow);
        }
        Ok(Self {
            interpretation,
            to_graded_ap1,
            working_to_graded: wb_matrix(photo.wb)? * wb_matrix(roll.wb)? * gain,
            grade: crate::photographic::Grade::new(photo)?,
        })
    }

    /// Grade an absolute ungraded AP1 replacement without repeating correction or inversion.
    pub fn grade_neutral(&self, mut pixel: InvertedPixel) -> Result<InvertedPixel, Error> {
        pixel.rgb = self.grade.apply(self.working_to_graded * pixel.rgb)?;
        Ok(pixel)
    }

    /// Returns unclipped AP1 working RGB; scene-linear AP0 export converts at its boundary.
    pub fn apply(&self, scan: DVec3) -> Result<InvertedPixel, Error> {
        let mut pixel = self.interpretation.interpret(scan)?;
        pixel.rgb = self.grade.apply(self.to_graded_ap1 * pixel.rgb)?;
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
