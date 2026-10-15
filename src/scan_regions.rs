//! Spatial sampling hypotheses, independent of grading, capture eligibility and output crops.
use crate::{
    Error,
    coverage::Coverage,
    geometry::{SourcePoint, SourceRegion},
};
use serde::{Deserialize, Serialize};

mod lines;
mod material;
mod opening;
mod rectangles;
pub use lines::{Capture, Evidence};
pub use material::RollModel;

pub type Quad = [[f64; 2]; 4];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Material {
    pub size: [u32; 2],
    pub xs: Vec<u32>,
    pub ys: Vec<u32>,
    pub labels: String,
}
impl Material {
    fn nearest(values: &[u32], point: f64) -> usize {
        let i = values.partition_point(|&v| f64::from(v) < point - 0.5);
        let a = i.saturating_sub(1).min(values.len() - 1);
        let b = i.min(values.len() - 1);
        if (f64::from(values[a]) + 0.5 - point).abs() <= (f64::from(values[b]) + 0.5 - point).abs()
        {
            a
        } else {
            b
        }
    }
    fn label(&self, p: [f64; 2]) -> u8 {
        if p.iter()
            .zip(self.size)
            .any(|(&v, n)| v < 0. || v >= f64::from(n))
        {
            return b'U';
        }
        self.labels.as_bytes()
            [Self::nearest(&self.ys, p[1]) * self.xs.len() + Self::nearest(&self.xs, p[0])]
    }
    fn validate(&self) -> Result<(), Error> {
        let valid = self.size.iter().all(|&v| (16..=768).contains(&v))
            && self.xs.len() <= 96
            && self.ys.len() <= 64
            && [&self.xs, &self.ys].iter().zip(self.size).all(|(axis, n)| {
                !axis.is_empty()
                    && axis.windows(2).all(|v| v[0] < v[1])
                    && axis.last().is_some_and(|&v| v < n)
            })
            && self.labels.len() == self.xs.len() * self.ys.len()
            && self
                .labels
                .bytes()
                .all(|v| b'F' == v || b'H' == v || b'L' == v || b'U' == v);
        if valid {
            Ok(())
        } else {
            Err(Error::InvalidParameter("invalid analysis material grid"))
        }
    }
}

/// Independent frame evidence retained when its opening-limited coverage is edited.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PictureGeometry {
    pub size: [u32; 2],
    pub angle: f64,
    pub sides: [Option<f64>; 4],
}

/// Accepted statistics geometry. Missing bounds are not physical aperture/frame observations.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Areas {
    pub opening: Option<Quad>,
    pub picture: Option<Quad>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<Material>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub picture_constraints: Vec<[f64; 3]>,
    pub opening_observed: [bool; 4],
    #[serde(default)]
    pub opening_shared: bool,
    #[serde(default)]
    pub opening_exception: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture_geometry: Option<PictureGeometry>,
    pub picture_observed: [bool; 4],
    #[serde(default)]
    pub picture_weak: [bool; 4],
    #[serde(default)]
    pub opening_edited: bool,
    #[serde(default)]
    pub picture_edited: bool,
}
impl Areas {
    pub fn coverage(quad: Quad) -> Result<Coverage, Error> {
        Coverage::new(quad.map(SourcePoint))
    }
    pub fn validate(&self, active: SourceRegion) -> Result<(), Error> {
        active.validate()?;
        for q in [self.opening, self.picture].into_iter().flatten() {
            let coverage = Self::coverage(q)?;
            if !coverage.area().is_finite()
                || q.iter().flatten().any(|v| v.abs() > f64::from(u32::MAX))
            {
                return Err(invalid("invalid analysis coverage extent"));
            }
            // Statistical rectangles may extend off-source; actual sampling clips to
            // the active raster. This is not an observed missing physical boundary.
            for (axis, origin, extent) in
                [(0, active.x, active.width), (1, active.y, active.height)]
            {
                let lo = q.iter().map(|p| p[axis]).fold(f64::INFINITY, f64::min);
                let hi = q.iter().map(|p| p[axis]).fold(f64::NEG_INFINITY, f64::max);
                if hi <= f64::from(origin) || lo >= f64::from(origin + extent) {
                    return Err(invalid("analysis coverage does not intersect source"));
                }
            }
        }
        if let Some(geometry) = &self.picture_geometry
            && (geometry.size.iter().any(|&v| !(16..=768).contains(&v))
                || !geometry.angle.is_finite()
                || geometry.angle.abs() > 6.
                || geometry
                    .sides
                    .iter()
                    .flatten()
                    .any(|v| !v.is_finite() || v.abs() > 2048.)
                || self.picture_edited
                || geometry
                    .sides
                    .iter()
                    .zip(self.picture_observed.iter().zip(self.picture_weak))
                    .any(|(side, (&observed, weak))| side.is_some() != (observed || weak)))
        {
            return Err(invalid("invalid retained picture geometry"));
        }
        if self.opening_exception && !self.opening_shared
            || self.opening_shared
                && (self.opening_edited || self.opening_observed.iter().any(|&v| v))
        {
            return Err(invalid("inconsistent shared opening provenance"));
        }
        if self
            .picture_observed
            .iter()
            .zip(self.picture_weak)
            .any(|(&observed, weak)| observed && weak)
            || self.opening_edited
                && (self.material.is_some() || self.opening_observed.iter().any(|&v| v))
            || self.picture_edited
                && (!self.picture_constraints.is_empty()
                    || self
                        .picture_observed
                        .iter()
                        .chain(self.picture_weak.iter())
                        .any(|&v| v))
        {
            return Err(invalid("inconsistent analysis-area provenance"));
        }
        if self.picture_constraints.len() > 4
            || self.picture_constraints.iter().any(|line| {
                line.iter().any(|v| !v.is_finite())
                    || !line[0].hypot(line[1]).is_finite()
                    || line[0].hypot(line[1]) < 1e-12
                    || Coverage::rectangle(active)
                        .corners()
                        .iter()
                        .any(|p| !(line[0] * p.0[0] + line[1] * p.0[1] - line[2]).is_finite())
            })
        {
            return Err(Error::InvalidParameter(
                "invalid analysis picture half-planes",
            ));
        }
        if let Some(material) = &self.material {
            material.validate()?;
        }
        Ok(())
    }
    pub fn ready(&self) -> bool {
        self.opening.is_some() && self.picture.is_some()
    }
    pub fn needs_inspection(&self) -> bool {
        !self.ready()
            || self.opening_exception
            || !self.opening_edited
                && !self.opening_shared
                && self.opening_observed.iter().any(|&v| !v)
            || !self.picture_edited
                && self
                    .picture_observed
                    .iter()
                    .zip(self.picture_weak)
                    .any(|(&observed, weak)| !observed && !weak)
    }
    pub fn edit_opening(&mut self, coverage: Coverage) {
        self.opening = Some(coverage.corners().map(|p| p.0));
        self.opening_observed = [false; 4];
        self.opening_shared = false;
        self.opening_exception = false;
        self.opening_edited = true;
        // Authored film support replaces the automatic material hypothesis.
        self.material = None;
    }
    pub fn edit_picture(&mut self, coverage: Coverage) {
        self.picture = Some(coverage.corners().map(|p| p.0));
        self.picture_constraints.clear();
        self.picture_geometry = None;
        self.picture_observed = [false; 4];
        self.picture_weak = [false; 4];
        self.picture_edited = true;
    }
    /// Change statistical coverage, not independently measured picture edges or authored edits.
    pub fn refit_picture(&mut self, active: SourceRegion) {
        if !self.picture_edited
            && let Some(geometry) = &self.picture_geometry
        {
            self.picture = self
                .opening
                .and_then(|opening| geometry.fit(active, opening));
        }
    }
    pub fn contains_footprint(
        &self,
        footprint: SourceRegion,
        active: SourceRegion,
        picture: bool,
    ) -> bool {
        let Some(opening) = self.opening else {
            return false;
        };
        let corners = [
            [footprint.x, footprint.y],
            [footprint.x + footprint.width - 1, footprint.y],
            [
                footprint.x + footprint.width - 1,
                footprint.y + footprint.height - 1,
            ],
            [footprint.x, footprint.y + footprint.height - 1],
        ]
        .map(|p| p.map(|v| f64::from(v) + 0.5));
        if corners.iter().any(|&p| !inside(p, &opening)) {
            return false;
        }
        if let Some(m) = &self.material {
            let proxy = |p: [f64; 2]| {
                [
                    (p[0] - f64::from(active.x)) * f64::from(m.size[0]) / f64::from(active.width),
                    (p[1] - f64::from(active.y)) * f64::from(m.size[1]) / f64::from(active.height),
                ]
            };
            let lo = proxy(corners[0]);
            let hi = proxy(corners[2]);
            if lo
                .iter()
                .zip(hi)
                .zip(m.size)
                .any(|((&l, h), n)| l < 0. || h >= f64::from(n))
            {
                return false;
            }
            for y in Material::nearest(&m.ys, lo[1])..=Material::nearest(&m.ys, hi[1]) {
                for x in Material::nearest(&m.xs, lo[0])..=Material::nearest(&m.xs, hi[0]) {
                    if m.labels.as_bytes()[y * m.xs.len() + x] != b'F' {
                        return false;
                    }
                }
            }
        }
        if picture {
            let Some(quad) = self.picture else {
                return false;
            };
            if corners.iter().any(|&p| {
                !inside(p, &quad)
                    || self
                        .picture_constraints
                        .iter()
                        .any(|[a, b, c]| a * p[0] + b * p[1] < *c)
            }) {
                return false;
            }
        }
        true
    }
}

fn inside(p: [f64; 2], polygon: &Quad) -> bool {
    let cross: [f64; 4] = std::array::from_fn(|i| {
        let a = polygon[i];
        let b = polygon[(i + 1) % 4];
        (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
    });
    cross.iter().all(|&v| v >= 0.) || cross.iter().all(|&v| v <= 0.)
}
fn quantile(values: impl IntoIterator<Item = f64>, fraction: f64) -> Option<f64> {
    let mut values: Vec<_> = values.into_iter().collect();
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    Some(values[((values.len() - 1) as f64 * fraction).floor() as usize])
}
fn median(values: impl IntoIterator<Item = f64>) -> f64 {
    let mut v: Vec<_> = values.into_iter().collect();
    v.sort_by(f64::total_cmp);
    if v.len() % 2 == 0 {
        (v[v.len() / 2 - 1] + v[v.len() / 2]) / 2.
    } else {
        v[v.len() / 2]
    }
}
fn centres(extent: u32, count: u32) -> Vec<u32> {
    (0..count)
        .map(|i| (((2 * u64::from(i) + 1) * u64::from(extent)) / (2 * u64::from(count))) as u32)
        .collect()
}
fn invalid(message: &'static str) -> Error {
    Error::InvalidParameter(message)
}

#[cfg(test)]
mod tests;
