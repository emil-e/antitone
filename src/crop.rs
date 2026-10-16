//! Source-anchored framing; crop interaction never edits the developed source.
use crate::{
    Error,
    coverage::Coverage,
    geometry::{SourcePoint, SourceRegion},
};

#[cfg(feature = "project")]
fn zero_angle(angle: &f64) -> bool {
    *angle == 0.
}

/// A rectangular crop in original developed-source pixel units.
/// Centre/extent storage leaves the framing independent of displayed orientation.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "project", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "project", serde(deny_unknown_fields))]
pub struct Crop {
    pub center: [f64; 2],
    pub size: [u32; 2],
    /// Clockwise source-plane frame basis about `center`, in radians.
    #[cfg_attr(
        feature = "project",
        serde(default, skip_serializing_if = "zero_angle")
    )]
    pub angle: f64,
}

impl Crop {
    pub fn from_region(region: SourceRegion) -> Self {
        Self {
            center: [
                f64::from(region.x) + f64::from(region.width) * 0.5,
                f64::from(region.y) + f64::from(region.height) * 0.5,
            ],
            size: [region.width, region.height],
            angle: 0.,
        }
    }

    pub fn source_point(self, local: [f64; 2]) -> SourcePoint {
        let [x, y] = std::array::from_fn(|axis| local[axis] - f64::from(self.size[axis]) * 0.5);
        let (s, c) = self.angle.sin_cos();
        SourcePoint([
            self.center[0] + c * x - s * y,
            self.center[1] + s * x + c * y,
        ])
    }

    pub fn local_point(self, source: SourcePoint) -> [f64; 2] {
        let x = source.0[0] - self.center[0];
        let y = source.0[1] - self.center[1];
        let (s, c) = self.angle.sin_cos();
        [
            c * x + s * y + f64::from(self.size[0]) * 0.5,
            -s * x + c * y + f64::from(self.size[1]) * 0.5,
        ]
    }

    pub fn coverage(self) -> Result<Coverage, Error> {
        if self.size.contains(&0)
            || self.center.iter().any(|v| !v.is_finite())
            || !self.angle.is_finite()
            || self.angle.abs() > std::f64::consts::PI
        {
            return Err(Error::InvalidParameter(
                "invalid crop centre, dimensions or angle",
            ));
        }
        let [w, h] = self.size.map(f64::from);
        Coverage::new([[0., 0.], [w, 0.], [w, h], [0., h]].map(|p| self.source_point(p)))
    }

    /// Source iteration bounds, not a substitute for the rotated crop's actual coverage.
    pub fn region(self, active: SourceRegion) -> Result<SourceRegion, Error> {
        active.validate()?;
        if !self.angle.is_finite() {
            return Err(Error::InvalidParameter("nonfinite crop angle"));
        }
        if self.angle != 0. {
            return self.coverage()?.bounds(active);
        }
        let min = std::array::from_fn::<_, 2, _>(|axis| {
            self.center[axis] - f64::from(self.size[axis]) * 0.5
        });
        if self.size.contains(&0)
            || min
                .iter()
                .any(|v| !v.is_finite() || *v < 0. || v.fract() != 0. || *v > f64::from(u32::MAX))
        {
            return Err(Error::InvalidParameter(
                "crop must have finite integer source edges and positive dimensions",
            ));
        }
        let region = SourceRegion {
            x: min[0] as u32,
            y: min[1] as u32,
            width: self.size[0],
            height: self.size[1],
        };
        if !active.contains_region(region) {
            return Err(Error::InvalidParameter(
                "crop is outside the active source area",
            ));
        }
        Ok(region)
    }

    /// Turns the image beneath the frame about its source centre, shrinking to valid coverage.
    pub fn straightened(self, active: SourceRegion, angle: f64) -> Result<Self, Error> {
        self.region(active)?;
        let origin = [active.x, active.y];
        let bounds = [active.width, active.height];
        let available = std::array::from_fn(|i| {
            2. * (self.center[i] - f64::from(origin[i]))
                .min(f64::from(origin[i] + bounds[i]) - self.center[i])
        });
        // Respect the existing pivot: reject angles for which even a one-pixel frame cannot fit.
        let size = fit_rotated_size(self.size, angle, available)?;
        Self {
            size,
            angle,
            ..self
        }
        .constrained(active)
    }

    /// Constrains an interactive proposal explicitly, snapping zero-angle crop edges to full pixels.
    pub fn constrained(self, active: SourceRegion) -> Result<Self, Error> {
        active.validate()?;
        if self.center.iter().any(|v| !v.is_finite()) {
            return Err(Error::InvalidParameter("nonfinite crop centre"));
        }
        if self.angle != 0. {
            let size = fit_rotated_size(
                self.size,
                self.angle,
                [f64::from(active.width), f64::from(active.height)],
            )?;
            let (s, c) = self.angle.sin_cos();
            let half = [
                (c.abs() * f64::from(size[0]) + s.abs() * f64::from(size[1])) * 0.5,
                (s.abs() * f64::from(size[0]) + c.abs() * f64::from(size[1])) * 0.5,
            ];
            let origin = [active.x, active.y];
            let bounds = [active.width, active.height];
            let center = std::array::from_fn(|i| {
                self.center[i].clamp(
                    f64::from(origin[i]) + half[i],
                    f64::from(origin[i] + bounds[i]) - half[i],
                )
            });
            let crop = Self {
                center,
                size,
                angle: self.angle,
            };
            crop.region(active)?;
            return Ok(crop);
        }
        let origin = [active.x, active.y];
        let bounds = [active.width, active.height];
        let size: [u32; 2] = std::array::from_fn(|axis| self.size[axis].clamp(1, bounds[axis]));
        let min: [u32; 2] = std::array::from_fn(|axis| {
            (self.center[axis] - f64::from(size[axis]) * 0.5)
                .round()
                .clamp(
                    f64::from(origin[axis]),
                    f64::from(origin[axis] + bounds[axis] - size[axis]),
                ) as u32
        });
        Ok(Self::from_region(SourceRegion {
            x: min[0],
            y: min[1],
            width: size[0],
            height: size[1],
        }))
    }
}

fn fit_rotated_size(size: [u32; 2], angle: f64, bounds: [f64; 2]) -> Result<[u32; 2], Error> {
    if !angle.is_finite() || angle.abs() > std::f64::consts::PI {
        return Err(Error::InvalidParameter("invalid straighten angle"));
    }
    let (s, c) = angle.sin_cos();
    let coefficients = [[c.abs(), s.abs()], [s.abs(), c.abs()]];
    if bounds
        .iter()
        .any(|v| !v.is_finite() || *v < c.abs() + s.abs())
    {
        return Err(Error::InvalidParameter(
            "no positive frame fits at this angle and pivot",
        ));
    }
    let size = size.map(|v| f64::from(v.max(1)));
    let mut scale = 1_f64;
    for i in 0..2 {
        scale =
            scale.min(bounds[i] / (coefficients[i][0] * size[0] + coefficients[i][1] * size[1]));
    }
    // The one-pixel minimum must be part of the constraint, not added after uniform shrinking.
    for small in 0..2 {
        if size[small] * scale < 1. {
            let large = 1 - small;
            for i in 0..2 {
                if coefficients[i][large] > 0. {
                    scale = scale.min(
                        (bounds[i] - coefficients[i][small])
                            / (coefficients[i][large] * size[large]),
                    );
                }
            }
        }
    }
    Ok(size.map(|v| (v * scale).floor().max(1.) as u32))
}

#[cfg(test)]
mod tests {
    use super::*;
    const AREA: SourceRegion = SourceRegion {
        x: 12,
        y: 7,
        width: 51,
        height: 33,
    };

    #[test]
    fn extreme_rotated_aspects_fit_without_losing_the_minimum_frame_or_pivot() {
        for angle in [-89_f64, -45., -13.7, 0.1, 19.3, 44., 89.] {
            for size in [[u32::MAX, 1], [1, u32::MAX], [u32::MAX, u32::MAX], [1, 1]] {
                let crop = Crop {
                    center: [-10000., 10000.],
                    size,
                    angle: angle.to_radians(),
                }
                .constrained(AREA)
                .unwrap();
                crop.region(AREA).unwrap();
            }
        }
        let tiny = SourceRegion {
            x: 0,
            y: 0,
            width: 2,
            height: 2,
        };
        let edge = Crop::from_region(SourceRegion {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        });
        assert!(edge.straightened(tiny, 0.3).is_err());
    }

    #[test]
    fn crop_roundtrips_source_edges_and_rejects_invalid_or_unsupported_framing() {
        let crop = Crop::from_region(AREA);
        assert_eq!(crop.region(AREA).unwrap(), AREA);
        for invalid in [
            Crop {
                center: [f64::NAN, 12.],
                ..crop
            },
            Crop {
                center: [14.25, 12.],
                size: [3, 3],
                angle: 0.,
            },
            Crop {
                size: [0, 1],
                ..crop
            },
            Crop {
                center: [-2., 12.],
                size: [1, 1],
                angle: 0.,
            },
            Crop {
                size: [u32::MAX, 2],
                ..crop
            },
        ] {
            assert!(invalid.region(AREA).is_err());
        }
        let edge = Crop {
            center: [1000., -100.],
            size: [12, 9],
            angle: 0.,
        }
        .constrained(AREA)
        .unwrap();
        assert_eq!(
            edge.region(AREA).unwrap(),
            SourceRegion {
                x: 51,
                y: 7,
                width: 12,
                height: 9
            }
        );
    }
}
