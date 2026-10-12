use crate::{Error, rendering::OutputTarget};
use glam::{DVec2, DVec3};

pub const BINS: u32 = 256;
pub const PLANE_LEN: usize = (BINS * BINS) as usize;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Plot {
    #[default]
    Rgb,
    Luma,
    Vector,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Basis {
    pub luma: DVec3,
    pub blue_direction: DVec2,
    pub extent: f64,
}

impl Basis {
    pub fn new(target: OutputTarget) -> Self {
        let matrix = target.rgb_to_xyz();
        let weights = DVec3::new(matrix.x_axis.y, matrix.y_axis.y, matrix.z_axis.y);
        let luma = weights / weights.element_sum();
        let chroma = |rgb: DVec3| {
            let y = luma.dot(rgb);
            DVec2::new(
                (rgb.z - y) / (2. * (1. - luma.z)),
                (rgb.x - y) / (2. * (1. - luma.x)),
            )
        };
        let blue_direction = chroma(DVec3::Z).normalize();
        let extent = references()
            .into_iter()
            .map(|(_, rgb)| chroma(rgb).length())
            .fold(0., f64::max)
            * 1.08;
        Self {
            luma,
            blue_direction,
            extent,
        }
    }

    /// Legacy positive-I, Q=0 hue projected as a target-native encoded-RGB heuristic, not
    /// NTSC colourimetry or a universal skin colour. See the scope contract for the convention.
    pub fn skin_reference_direction(self) -> DVec2 {
        let y = DVec3::new(0.299, 0.587, 0.114);
        let q = DVec3::new(0.2115, -0.5227, 0.3112);
        self.vector(y.cross(q)).normalize()
    }

    /// Right/up display coordinates; the scope raster performs the screen-y inversion.
    pub fn vector(self, encoded: DVec3) -> DVec2 {
        let y = self.luma.dot(encoded);
        let cb = (encoded.z - y) / (2. * (1. - self.luma.z));
        let cr = (encoded.x - y) / (2. * (1. - self.luma.x));
        DVec2::new(
            self.blue_direction.x * cb + self.blue_direction.y * cr,
            -self.blue_direction.y * cb + self.blue_direction.x * cr,
        )
    }
}

pub fn references() -> [(&'static str, DVec3); 6] {
    [
        ("R", DVec3::X),
        ("M", DVec3::new(1., 0., 1.)),
        ("B", DVec3::Z),
        ("C", DVec3::new(0., 1., 1.)),
        ("G", DVec3::Y),
        ("Y", DVec3::new(1., 1., 0.)),
    ]
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub target: OutputTarget,
    pub plot: Plot,
    pub amplification: u32,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            target: OutputTarget::DisplayP3,
            plot: Plot::Rgb,
            amplification: 1,
        }
    }
}
impl Settings {
    pub fn validate(self, size: [u32; 2]) -> Result<Self, Error> {
        if ![1, 2, 4, 8].contains(&self.amplification)
            || size[0].checked_mul(size[1]).is_none_or(|count| count == 0)
        {
            return Err(Error::InvalidParameter(
                "invalid scope raster or amplification",
            ));
        }
        Ok(self)
    }
    pub fn half_extent(self) -> f64 {
        Basis::new(self.target).extent / self.amplification as f64
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub valid: u32,
    pub invalid: u32,
    pub outside: u32,
}

/// Scalar oracle for a whole-frame linear-target raster, not viewer canvas/UV coordinates.
pub fn reference(
    size: [u32; 2],
    linear: &[[f32; 4]],
    settings: Settings,
) -> Result<(Vec<u32>, [u32; BINS as usize], Counts), Error> {
    settings.validate(size)?;
    if u64::from(size[0]) * u64::from(size[1]) != linear.len() as u64 {
        return Err(Error::InvalidParameter(
            "scope pixels do not match raster dimensions",
        ));
    }
    let mut bins = vec![0; PLANE_LEN * 4];
    let mut columns = [0; BINS as usize];
    let mut counts = Counts::default();
    let basis = Basis::new(settings.target);
    let half_extent = basis.extent / settings.amplification as f64;
    for (i, pixel) in linear.iter().enumerate() {
        let mut encoded = DVec3::ZERO;
        let mut invalid = false;
        for channel in 0..3 {
            match settings.target.encode(f64::from(pixel[channel])) {
                Ok(value) => encoded[channel] = value,
                Err(_) => invalid = true,
            }
        }
        if invalid {
            counts.invalid += 1;
            continue;
        }
        counts.valid += 1;
        let x = i as u32 % size[0];
        let column = ((x as f64 + 0.5) / size[0] as f64 * BINS as f64).floor() as usize;
        columns[column] += 1;
        if settings.plot == Plot::Vector {
            let point = basis.vector(encoded) / (2. * half_extent) + DVec2::splat(0.5);
            if point.min_element() < 0. || point.max_element() > 1. {
                counts.outside += 1;
            } else {
                let bx = bin(point.x);
                let by = bin(1. - point.y);
                bins[by * BINS as usize + bx] += 1;
            }
        } else {
            let values = [encoded.x, encoded.y, encoded.z, basis.luma.dot(encoded)];
            for (plane, value) in values.into_iter().enumerate() {
                bins[plane * PLANE_LEN + bin(1. - value) * BINS as usize + column] += 1;
            }
        }
    }
    Ok((bins, columns, counts))
}

fn bin(value: f64) -> usize {
    ((value * BINS as f64).floor() as usize).min(BINS as usize - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_weights_neutrals_and_hue_directions() {
        for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
            let basis = Basis::new(target);
            assert!((basis.luma.element_sum() - 1.).abs() < 1e-15);
            assert!(basis.luma.min_element() > 0.);
            for grey in [0., 0.01, 0.18, 0.5, 1.] {
                assert!(basis.vector(DVec3::splat(grey)).length() < 1e-15);
            }
            let blue = basis.vector(DVec3::Z);
            let yellow = basis.vector(DVec3::new(1., 1., 0.));
            assert!(blue.x > 0. && blue.y.abs() < 1e-15);
            assert!((yellow + blue).length() < 1e-15);
            let vertices = references().map(|(_, rgb)| basis.vector(rgb));
            for i in 0..6 {
                assert!(vertices[i].perp_dot(vertices[(i + 1) % 6]) < 0.);
                assert!(vertices[i].length() < basis.extent);
            }
        }
        let srgb = Basis::new(OutputTarget::Srgb);
        assert!((srgb.luma - DVec3::new(0.212639, 0.715169, 0.072192)).length() < 1e-6);
        let p3 = Basis::new(OutputTarget::DisplayP3);
        assert!((p3.luma - DVec3::new(0.228975, 0.691739, 0.079287)).length() < 1e-6);
        assert!((srgb.luma - p3.luma).length() > 0.01);
    }

    #[test]
    fn skin_reference_projects_positive_i_with_zero_q_in_each_native_basis() {
        for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
            let basis = Basis::new(target);
            let direction = basis.skin_reference_direction();
            assert!((direction.length() - 1.).abs() < 1e-15);
            assert!(direction.x < 0. && direction.y > 0.);
            let red = basis.vector(DVec3::X);
            let yellow = basis.vector(DVec3::new(1., 1., 0.));
            assert!(red.perp_dot(direction) > 0. && direction.perp_dot(yellow) > 0.);
            // Independent Q=0 RGB fixtures; adding neutral or scaling chroma preserves the ray.
            for (r, b) in [(0.6, 0.39), (0.8, 0.2), (0.4, 0.3)] {
                let g = (0.2115 * r + 0.3112 * b) / 0.5227;
                let rgb = DVec3::new(r, g, b);
                assert!(DVec3::new(0.5959, -0.2746, -0.3213).dot(rgb) > 0.);
                assert!((basis.vector(rgb).normalize() - direction).length() < 1e-14);
            }
        }
        assert!(
            (Basis::new(OutputTarget::Srgb).skin_reference_direction()
                - Basis::new(OutputTarget::DisplayP3).skin_reference_direction())
            .length()
                > 1e-4
        );
    }

    #[test]
    fn whole_raster_columns_endpoints_and_invalid_samples_are_accounted() {
        let settings = Settings::default();
        let pixels = [
            [0., 0., 0., 1.],
            [1., 1., 1., 1.],
            [f32::NAN, 0., 0., 1.],
            [-0.1, 0., 0., 1.],
        ];
        let (bins, columns, counts) = reference([2, 2], &pixels, settings).unwrap();
        assert_eq!(
            counts,
            Counts {
                valid: 2,
                invalid: 2,
                outside: 0
            }
        );
        assert_eq!(columns[64], 1);
        assert_eq!(columns[192], 1);
        for plane in bins.chunks(PLANE_LEN) {
            assert_eq!(plane.iter().sum::<u32>(), 2);
            assert_eq!(plane[255 * 256 + 64], 1);
            assert_eq!(plane[192], 1);
        }
        assert!(reference([0, 2], &[], settings).is_err());
        assert!(reference([u32::MAX, 2], &[], settings).is_err());
        assert!(reference([2, 2], &pixels[..3], settings).is_err());
    }

    #[test]
    fn amplification_rebins_around_neutral_and_keeps_offscreen_counts() {
        let pixels = [
            [0.2, 0.18, 0.18, 1.],
            [0.18, 0.18, 0.18, 1.],
            [1., 0., 0., 1.],
        ];
        for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
            let low = Settings {
                target,
                plot: Plot::Vector,
                amplification: 1,
            };
            let high = Settings {
                amplification: 8,
                ..low
            };
            let (a, _, ca) = reference([3, 1], &pixels, low).unwrap();
            let (b, _, cb) = reference([3, 1], &pixels, high).unwrap();
            assert_eq!(ca.valid, cb.valid);
            assert_eq!(ca.outside, 0);
            assert_eq!(cb.outside, 1);
            assert_ne!(a, b);
            assert_eq!(a.iter().sum::<u32>() + ca.outside, ca.valid);
            assert_eq!(b.iter().sum::<u32>() + cb.outside, cb.valid);
        }
    }
}
