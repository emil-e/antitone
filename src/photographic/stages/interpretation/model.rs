//! Scalar camera response and working-light conversion.

use crate::{
    Error,
    processing::{InputSettings, NegativeInput},
};
use glam::{DMat3, DVec3};

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
            // flip or film response.
            Self::Digital => {
                for c in 0..3 {
                    if !scan[c].is_finite() {
                        return Err(Error::NonFinitePixel { channel: c });
                    }
                }
                Ok(InvertedPixel { rgb: scan })
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

    /// Input light, calibration and output are native-camera RGB.
    pub fn apply(&self, scan: DVec3) -> Result<InvertedPixel, Error> {
        let mut rgb = DVec3::ZERO;
        let parameters = &self.parameters;
        let (toe, shoulder) = parameters.shape;
        for c in 0..3 {
            if !scan[c].is_finite() {
                return Err(Error::NonFinitePixel { channel: c });
            }
            let [join, value, slope] = parameters.tail[c];
            if scan[c] <= 0. {
                rgb[c] = value + slope;
                continue;
            }
            let l = log_ratio(parameters.base[c], scan[c]);
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
        Ok(InvertedPixel { rgb })
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

fn log_abs_expm1(a: f64) -> f64 {
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
