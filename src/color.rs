use glam::{DMat3, DVec2, DVec3};

use crate::Error;

pub const AP0_TO_XYZ: DMat3 = DMat3::from_cols(
    DVec3::new(0.9525523959381857, 0.3439664497650751, 0.0),
    DVec3::new(0.0, 0.7281660966134857, 0.0),
    DVec3::new(0.0000936786316605, -0.0721325463785608, 1.0088251843515859),
);
pub const AP1_TO_XYZ: DMat3 = DMat3::from_cols(
    DVec3::new(
        0.6624541811085053,
        0.27222871678091454,
        -0.005574649490394108,
    ),
    DVec3::new(
        0.13400420645643313,
        0.6740817658111483,
        0.004060733528982826,
    ),
    DVec3::new(0.1561876870049078, 0.05368951740793705, 1.0103391003129971),
);

pub fn ap0_to_ap1() -> DMat3 {
    AP1_TO_XYZ.inverse() * AP0_TO_XYZ
}

pub fn ap1_to_ap0() -> DMat3 {
    AP0_TO_XYZ.inverse() * AP1_TO_XYZ
}

pub const BRADFORD: DMat3 = DMat3::from_cols(
    DVec3::new(0.8951, -0.7502, 0.0389),
    DVec3::new(0.2664, 1.7135, -0.0685),
    DVec3::new(-0.1614, 0.0367, 1.0296),
);

pub fn opponent(x: f64, y: f64) -> DVec3 {
    let warm = x / 2.0_f64.sqrt();
    let tint = y / 6.0_f64.sqrt();
    DVec3::new(warm + tint, -2.0 * tint, -warm + tint)
}

pub fn opponent_coordinates(value: DVec3) -> DVec2 {
    DVec2::new(
        (value.x - value.z) / 2.0_f64.sqrt(),
        (value.x - 2.0 * value.y + value.z) / 6.0_f64.sqrt(),
    )
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
#[cfg_attr(feature = "project", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "project", serde(deny_unknown_fields))]
pub struct Balance {
    pub warmth: f64,
    pub tint: f64,
}

impl Balance {
    pub fn validate(self) -> Result<Self, Error> {
        let coordinates = DVec2::new(self.warmth, self.tint);
        if !coordinates.is_finite() || coordinates.abs().max_element() > 1.0 {
            return Err(Error::InvalidParameter(
                "balance coordinates must be within -1..1",
            ));
        }
        Ok(self)
    }

    pub fn offsets(self) -> Result<DVec3, Error> {
        self.validate()?;
        Ok(opponent(self.warmth, self.tint))
    }
}

pub fn ap1_to_cones() -> DMat3 {
    BRADFORD * AP1_TO_XYZ
}

pub fn wb_matrix(balance: Balance) -> Result<DMat3, Error> {
    let gains = balance.offsets()?.exp();
    if balance == Balance::default() {
        return Ok(DMat3::IDENTITY);
    }
    let cones = ap1_to_cones();
    Ok(cones.inverse() * DMat3::from_diagonal(gains) * cones)
}

pub fn neutral_wb(mean: DVec3) -> Result<Balance, Error> {
    if !mean.is_finite() {
        return Err(Error::UnusableSample("WB patch must have finite mean RGB"));
    }
    let cones = ap1_to_cones();
    let sample = cones * mean;
    let white = cones * DVec3::ONE;
    if !sample.is_finite() || sample.min_element() <= 0.0 {
        return Err(Error::UnusableSample(
            "WB patch has invalid Bradford cone responses",
        ));
    }
    let coordinates = opponent_coordinates((white / sample).ln());
    let result = Balance {
        warmth: coordinates.x,
        tint: coordinates.y,
    };
    result
        .validate()
        .map_err(|_| Error::UnusableSample("WB correction is outside the control range"))
}
