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

/// Gains entering LibRaw's normalization after undoing any capture-time balance.
#[cfg(feature = "raw")]
pub(crate) fn libraw_effective_wb(
    requested: [f64; 4],
    pre_applied: Option<[f64; 3]>,
) -> Result<[f64; 4], Error> {
    let mut gains = requested.map(|value| value as f32);
    if let Some(applied) = pre_applied {
        for c in 0..3 {
            let applied = applied[c] as f32;
            if !applied.is_finite() || applied <= 0.00001 {
                return Err(Error::InvalidParameter("invalid pre-applied camera WB"));
            }
            gains[c] /= applied;
        }
        gains[3] = gains[1];
    }
    if gains.iter().any(|value| !value.is_finite() || *value <= 0.) {
        return Err(Error::InvalidParameter("invalid effective development WB"));
    }
    Ok(gains.map(f64::from))
}

/// Restores green-relative light from pinned LibRaw's highlight-safe WB normalization.
#[cfg(any(feature = "raw", feature = "gpu"))]
pub(crate) fn libraw_green_scale(wb: [f64; 4]) -> Result<f64, Error> {
    let wb = wb.map(|value| value as f32);
    if wb.iter().any(|value| !value.is_finite() || *value <= 0.) {
        return Err(Error::InvalidParameter(
            "development WB must be positive finite f32",
        ));
    }
    let maximum = wb.into_iter().fold(0_f32, f32::max);
    if maximum <= 0.00001 {
        // LibRaw skips gain normalization in this degenerate branch.
        return Ok(1.);
    }
    Ok(f64::from(maximum) / f64::from(wb[1]))
}

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
    /// Digital film emulation (image MTF, halation later, grain) is available.
    pub grain: bool,
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
}

impl Calibration {
    pub fn validate(self) -> Result<Self, Error> {
        for c in 0..3 {
            let dmin = self.dmin[c];
            let dmax = self.dmax[c];
            let reason = if !dmin.is_finite() || !dmax.is_finite() {
                Some("references must be finite")
            } else if dmax <= 0. || dmin <= dmax {
                Some("require Dmin RGB > Dmax RGB > 0")
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
        response: Response,
        gamma: f64,
        balance: Balance,
    },
    /// An ordinary scene-referred photograph: already a scene-referred positive.
    Digital,
}

impl InputSettings {
    /// Negative preserves unbalanced camera light; Digital resolves the RAW's as-shot WB.
    pub fn development_wb(&self) -> Option<[f64; 4]> {
        self.is_negative().then_some([1.; 4])
    }

    /// A fresh negative interpretation over measured anchors, with the shared response prior.
    pub fn negative(calibration: Calibration) -> Self {
        Self::Negative {
            dmin: Some(calibration.dmin),
            dmax: Some(calibration.dmax),
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
                ..
            } => Some(Calibration {
                dmin: *dmin,
                dmax: *dmax,
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

    #[cfg(any(feature = "raw", feature = "project"))]
    pub(crate) fn positive_matrix(
        &self,
        matrix: DMat3,
        reference_ev: Option<f64>,
    ) -> Result<DMat3, Error> {
        let gain = if self.is_negative() {
            1.
        } else {
            exposure_multiplier(reference_ev.unwrap_or(0.))?
        };
        Ok(matrix * gain)
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
                grain: false,
            },
            Self::Digital => InputCapabilities {
                scan_sources: false,
                roll_analysis: false,
                auto_crop: false,
                flat_field: false,
                calibration: false,
                gamma_balance: false,
                response: false,
                grain: true,
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

/// Validated coefficients used to prepare the GPU response.
#[derive(Debug, Clone)]
pub(crate) struct NegativeParameters {
    base: DVec3,
    gammas: DVec3,
    anchor_exponents: DVec3,
    pub(crate) anchor_logs: DVec3,
    shape: (f64, f64),
    pub(crate) tail: [[f64; 3]; 3],
}

/// Continue shoulder acceleration with constant slope before its rational denominator can vanish.
fn response_exponent(density: f64, gamma: f64, toe: f64, shoulder: f64) -> f64 {
    let h = shoulder / std::f64::consts::LN_10;
    if h * density > gamma * 0.5 {
        4. * toe * density / gamma - toe / h
    } else {
        toe * density / (gamma - h * density)
    }
}

impl NegativeInput {
    pub(crate) fn parameters(self) -> Result<NegativeParameters, Error> {
        let negative = self;
        let calibration = negative.calibration;
        calibration.validate()?;
        let gammas = negative.channel_gammas()?;
        let (toe, shoulder) = negative.response.shape()?;
        let anchor_exponents = DVec3::from_array(std::array::from_fn(|c| {
            let density = log_ratio(calibration.dmin[c], calibration.dmax[c]);
            response_exponent(density, gammas[c], toe, shoulder)
        }));
        if !anchor_exponents.is_finite() {
            return Err(Error::NumericalOverflow);
        }
        let anchor_logs = anchor_exponents.map(log_abs_expm1);
        let tail = std::array::from_fn(|c| {
            // Keep the ordinary response through sixteen stops above the white anchor.
            // Below this transmission, its tangent in camera light reaches zero finitely.
            let log_numerator = anchor_logs[c] + toe * 16. * std::f64::consts::LN_2;
            let exponent = if log_numerator > 50. {
                log_numerator
            } else {
                log_numerator.exp().ln_1p()
            };
            let h = shoulder / std::f64::consts::LN_10;
            let density = if h * exponent <= toe {
                gammas[c] * exponent / (toe + h * exponent)
            } else {
                gammas[c] * (exponent + toe / h) / (4. * toe)
            };
            let exponent = response_exponent(density, gammas[c], toe, shoulder);
            let value = ((log_abs_expm1(exponent) - anchor_logs[c]) / toe).exp();
            let denominator = gammas[c] - h * density;
            let derivative = if denominator > gammas[c] * 0.5 {
                gammas[c] / (denominator * denominator)
            } else {
                4. / gammas[c]
            };
            [density, value, value * derivative / -(-exponent).exp_m1()]
        });
        Ok(NegativeParameters {
            base: calibration.dmin,
            gammas,
            anchor_exponents,
            anchor_logs,
            shape: (toe, shoulder),
            tail,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Inversion {
    parameters: NegativeParameters,
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
        Ok(Self {
            parameters: negative.into().parameters()?,
        })
    }

    /// Scan, calibration and output are native-camera RGB.
    pub fn apply(&self, scan: DVec3) -> Result<InvertedPixel, Error> {
        let mut rgb = DVec3::ZERO;
        let mut floored = [false; 3];
        let parameters = &self.parameters;
        let (toe, shoulder) = parameters.shape;
        for c in 0..3 {
            if !scan[c].is_finite() {
                return Err(Error::NonFinitePixel { channel: c });
            }
            let [join, value, slope] = parameters.tail[c];
            if scan[c] <= 0. {
                floored[c] = true;
                rgb[c] = value + slope;
                continue;
            }
            let l = log_ratio(parameters.base[c], scan[c]);
            floored[c] = l > join;
            if l >= join {
                rgb[c] = value + slope * (1. - (join - l).exp());
                continue;
            }
            rgb[c] = if toe == 1.0 && shoulder == 0.0 {
                // The general branch round-trips through log/exp; this setting is reachable
                // and is the neutral toe/shoulder choice, so evaluate the ratio directly.
                expm1_ratio(l / parameters.gammas[c], parameters.anchor_exponents[c])
            } else {
                let a = response_exponent(l, parameters.gammas[c], toe, shoulder);
                if a == 0.0 {
                    0.0
                } else {
                    // Numerical continuation for above-base noise/reference overshoots, not film radiometry.
                    a.signum() * ((log_abs_expm1(a) - parameters.anchor_logs[c]) / toe).exp()
                }
            };
            if !rgb[c].is_finite() {
                return Err(Error::NumericalOverflow);
            }
        }
        Ok(InvertedPixel { rgb, floored })
    }
}

/// Input interpretation and conversion to the working positive, before photographic grading.
#[derive(Debug, Clone)]
pub struct InputPipeline {
    interpretation: Interpretation,
    camera_to_working: DMat3,
}

impl InputPipeline {
    pub fn new(input: impl Into<InputSettings>, camera_to_working: DMat3) -> Result<Self, Error> {
        validate_camera_matrix(camera_to_working)?;
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
        })
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

/// Shared photographic balance/exposure on an already interpreted working positive.
#[derive(Debug, Clone)]
pub struct RollPipeline {
    transform: DMat3,
}

impl RollPipeline {
    pub fn new(roll: RollSettings) -> Result<Self, Error> {
        let transform = wb_matrix(roll.wb)? * exposure_multiplier(roll.exposure)?;
        if !transform.is_finite() {
            return Err(Error::NumericalOverflow);
        }
        Ok(Self { transform })
    }

    pub fn apply(&self, rgb: DVec3) -> Result<DVec3, Error> {
        let output = self.transform * rgb;
        if !output.is_finite() {
            return Err(Error::NumericalOverflow);
        }
        Ok(output)
    }
}

/// Scalar composition through Roll grading, without adjustment layers.
#[derive(Debug, Clone)]
pub struct Pipeline {
    input: InputPipeline,
    roll: RollPipeline,
}

impl Pipeline {
    pub fn new(
        input: impl Into<InputSettings>,
        roll: RollSettings,
        matrix: DMat3,
    ) -> Result<Self, Error> {
        Ok(Self {
            input: InputPipeline::new(input, matrix)?,
            roll: RollPipeline::new(roll)?,
        })
    }

    pub fn apply(&self, camera: DVec3) -> Result<InvertedPixel, Error> {
        self.apply_roll(self.input.apply(camera)?)
    }

    pub fn apply_roll(&self, mut positive: InvertedPixel) -> Result<InvertedPixel, Error> {
        positive.rgb = self.roll.apply(positive.rgb)?;
        Ok(positive)
    }
}

pub(crate) fn validate_camera_matrix(matrix: DMat3) -> Result<(), Error> {
    // Test independence in each column's own units. Fixed normalization and unequal
    // channel scales must not turn an independent, representable transform into a singular one.
    let normalized = |column: DVec3| column / column.abs().max_element();
    let determinant = DMat3::from_cols(
        normalized(matrix.x_axis),
        normalized(matrix.y_axis),
        normalized(matrix.z_axis),
    )
    .determinant();
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
