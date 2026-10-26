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

/// What a stage can offer the editor. Scan-based stages share the film tools; the
/// interpretation controls differ per stage. UI reads this instead of matching variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputCapabilities {
    /// The stage needs one shared scanning setup across its sources.
    pub scan_sources: bool,
    /// Roll analysis (film-scale fitting) is meaningful.
    pub roll_analysis: bool,
    /// Film-gate auto-crop is meaningful.
    pub auto_crop: bool,
    /// A flat-field reference can correct the scan.
    pub flat_field: bool,
    /// Dmin/Dmax (or equivalent film references) are measured by picks.
    pub calibration: bool,
    /// Global gamma and per-channel balance apply to interpretation.
    pub gamma_balance: bool,
    /// Toe/shoulder response applies to interpretation.
    pub response: bool,
}

/// The interpretation curve's toe and shoulder. Toe 1 with no shoulder removes their
/// shaping, leaving gamma's contrast between the anchors.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "project", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "project", serde(deny_unknown_fields))]
pub struct Response {
    pub toe: f64,
    pub shoulder: f64,
}

impl Response {
    /// The shared film prior over measured anchors: a default toe and no shoulder.
    pub const fn film() -> Self {
        Self {
            toe: DEFAULT_TOE,
            shoulder: 0.0,
        }
    }

    pub fn shape(self) -> Result<(f64, f64), Error> {
        if !self.toe.is_finite()
            || !(MIN_TOE..=MAX_TOE).contains(&self.toe)
            || !self.shoulder.is_finite()
            || !(0.0..=MAX_SHOULDER).contains(&self.shoulder)
        {
            return Err(Error::InvalidParameter(
                "toe must be within 1..4 and shoulder within 0..0.02",
            ));
        }
        Ok((self.toe, self.shoulder))
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

impl From<Calibration> for NegativeInput {
    /// The shared film prior over measured anchors (default response, gamma and neutral balance).
    fn from(calibration: Calibration) -> Self {
        Self {
            calibration,
            response: Response::film(),
            gamma: DEFAULT_GAMMA,
            balance: Balance::default(),
        }
    }
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
                response,
                gamma,
                balance,
                ..
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

    /// Whether switching interpretation would discard user-editable stage settings.
    pub fn has_stage_settings(&self) -> bool {
        match *self {
            InputSettings::Negative {
                dmin,
                dmax,
                response,
                gamma,
                balance,
                ..
            } => {
                dmin.is_some()
                    || dmax.is_some()
                    || response != crate::processing::Response::film()
                    || gamma != crate::processing::DEFAULT_GAMMA
                    || balance != Balance::default()
            }
            InputSettings::Digital => false,
        }
    }

    /// Whether this stage interprets a scan and therefore needs one shared scanning setup.
    pub fn requires_scan_sources(&self) -> bool {
        matches!(self, Self::Negative { .. })
    }

    /// Whether the stage has everything it needs to produce a positive.
    pub fn ready(&self) -> bool {
        match self {
            Self::Negative { .. } => self.calibration().is_some(),
            Self::Digital => true,
        }
    }

    /// What the editor may show for this stage.
    pub fn capabilities(&self) -> InputCapabilities {
        match self {
            Self::Negative { .. } => InputCapabilities {
                scan_sources: true,
                roll_analysis: true,
                auto_crop: true,
                flat_field: true,
                calibration: true,
                gamma_balance: true,
                response: true,
            },
            Self::Digital => InputCapabilities {
                scan_sources: false,
                roll_analysis: false,
                auto_crop: false,
                flat_field: false,
                calibration: false,
                gamma_balance: false,
                response: false,
            },
        }
    }

    /// The interpretation gamma (Digital is already a positive: no gamma).
    pub fn gamma(&self) -> f64 {
        match self {
            Self::Negative { gamma, .. } => *gamma,
            Self::Digital => DEFAULT_GAMMA,
        }
    }

    /// The interpretation balance (Digital is already a positive: neutral).
    pub fn balance(&self) -> Balance {
        match self {
            Self::Negative { balance, .. } => *balance,
            Self::Digital => Balance::default(),
        }
    }

    /// The processing-model version this interpretation maps to.
    pub fn processing_model(&self) -> u32 {
        crate::PROCESSING_MODEL_VERSION
    }

    /// The same input stage over a source-refined calibration (Digital ignores it).
    pub fn with_calibration(&self, calibration: Calibration) -> Self {
        match self {
            Self::Negative {
                response,
                gamma,
                balance,
                ..
            } => Self::Negative {
                dmin: Some(calibration.dmin),
                dmax: Some(calibration.dmax),
                input_floor: calibration.input_floor,
                response: *response,
                gamma: *gamma,
                balance: *balance,
            },
            Self::Digital => Self::Digital,
        }
    }
}

impl From<Calibration> for InputSettings {
    /// A negative interpretation over measured anchors with the shared film prior.
    fn from(calibration: Calibration) -> Self {
        Self::negative(calibration)
    }
}

impl From<NegativeInput> for InputSettings {
    fn from(negative: NegativeInput) -> Self {
        Self::Negative {
            dmin: Some(negative.calibration.dmin),
            dmax: Some(negative.calibration.dmax),
            input_floor: negative.calibration.input_floor,
            response: negative.response,
            gamma: negative.gamma,
            balance: negative.balance,
        }
    }
}

impl From<&InputSettings> for InputSettings {
    fn from(input: &Self) -> Self {
        *input
    }
}

impl Default for InputSettings {
    /// The film-first product default. Digital remains available for mixed cameras or ordinary
    /// photographs; an incompatible fresh scan import can select it without retaining a scan basis.
    fn default() -> Self {
        Self::Negative {
            dmin: None,
            dmax: None,
            input_floor: CAMERA_INPUT_FLOOR,
            response: Response::film(),
            gamma: DEFAULT_GAMMA,
            balance: Balance::default(),
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
    pub fn new(negative: impl Into<NegativeInput>) -> Result<Self, Error> {
        let negative = negative.into();
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
            toe * l / (gammas[c] - shoulder * l / std::f64::consts::LN_10)
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
                // The general branch round-trips through log/exp; this setting is reachable
                // and is the neutral toe/shoulder choice, so evaluate the ratio directly.
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

/// Source interpretation, camera conversion and Roll balance/exposure, before adjustment layers.
#[derive(Debug, Clone)]
pub struct InputPipeline {
    interpretation: Interpretation,
    camera_to_working: DMat3,
    roll_transform: DMat3,
}

impl InputPipeline {
    pub fn new(
        input: impl Into<InputSettings>,
        roll: RollSettings,
        camera_to_ap1: DMat3,
    ) -> Result<Self, Error> {
        validate_camera_matrix(camera_to_ap1)?;
        let roll_transform = wb_matrix(roll.wb)? * exposure_multiplier(roll.exposure)?;
        let camera_to_working = roll_transform * camera_to_ap1;
        if !camera_to_working.is_finite() {
            return Err(Error::NumericalOverflow);
        }
        let interpretation = match input.into() {
            InputSettings::Digital => Interpretation::Digital,
            input @ InputSettings::Negative { .. } => {
                let negative = input.negative_input().ok_or(Error::InvalidCalibration {
                    channel: 0,
                    reason: "measure Dmin and Dmax",
                })?;
                Interpretation::Negative(Inversion::new(negative)?)
            }
        };
        Ok(Self {
            interpretation,
            camera_to_working,
            roll_transform,
        })
    }

    /// Absolute neutral AP1, without Roll or adjustment layers.
    pub fn ungraded(input: impl Into<InputSettings>, camera_to_ap1: DMat3) -> Result<Self, Error> {
        Self::new(input, RollSettings::default(), camera_to_ap1)
    }

    pub fn apply_roll(&self, mut pixel: InvertedPixel) -> Result<InvertedPixel, Error> {
        pixel.rgb = self.roll_transform * pixel.rgb;
        if !pixel.rgb.is_finite() {
            return Err(Error::NumericalOverflow);
        }
        Ok(pixel)
    }

    pub fn apply(&self, scan: DVec3) -> Result<InvertedPixel, Error> {
        let mut pixel = self.interpretation.interpret(scan)?;
        pixel.rgb = self.camera_to_working * pixel.rgb;
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
