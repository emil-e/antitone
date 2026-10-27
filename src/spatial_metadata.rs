//! Derived source geometry shared by consumers; sampling and composition remain accepted edits.
use crate::{
    Error,
    crop::Crop,
    geometry::SourceRegion,
    project::{Development, Project, SourceDevelopment, SourceId},
    scan_regions::{PictureGeometry, Quad},
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub use crate::source::FileRevision;

mod geometry;
mod store;
pub use store::{Request, Store};

#[cfg(test)]
mod tests;

pub const VERSION: u32 = 3;

pub(crate) fn read_cached<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Metadata>, D::Error> {
    // Decode only the optional derived section permissively. Accepted project fields retain
    // their strict deserializers, and the project reader still bounds decoded JSON strings.
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value::<Metadata>(value)
        .ok()
        .filter(|metadata| metadata.validate().is_ok()))
}

pub(crate) fn empty_or_invalid(metadata: &Option<Metadata>) -> bool {
    metadata
        .as_ref()
        .is_none_or(|metadata| metadata.groups.is_empty() || metadata.validate().is_err())
}

fn invalid(message: &'static str) -> Error {
    Error::InvalidParameter(message)
}

/// Inward unit normal and source-pixel offset: `a*x + b*y >= c` is inside.
pub type Plane = [f64; 3];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Boundary {
    Measured {
        plane: Plane,
    },
    Inferred {
        plane: Plane,
    },
    /// The physical boundary is beyond available camera coverage; no position is fabricated.
    CameraClipped,
    Unknown,
}

impl Boundary {
    pub fn plane(&self) -> Option<Plane> {
        match self {
            Self::Measured { plane } | Self::Inferred { plane } => Some(*plane),
            Self::CameraClipped | Self::Unknown => None,
        }
    }

    pub fn is_measured(&self) -> bool {
        matches!(self, Self::Measured { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Region {
    /// Clockwise source-plane basis in radians, independent of user orientation.
    pub angle: f64,
    /// Source Y-axis deviation from the perpendicular to X, in radians. Unequal
    /// raster scales can skew those transformed axes even when their bounds are absent.
    pub skew: f64,
    /// Left, right, top, bottom. Camera limits are censoring, not measured picture edges.
    pub boundaries: [Boundary; 4],
}

impl Region {
    pub fn validate(&self) -> Result<(), Error> {
        if !self.angle.is_finite()
            || self.angle.abs() > std::f64::consts::PI
            || !self.skew.is_finite()
            || self.skew.abs() >= std::f64::consts::FRAC_PI_2
        {
            return Err(invalid("invalid spatial region angle"));
        }
        let (s, c) = self.angle.sin_cos();
        let normals = [[c, s], [-c, -s], [-s, c], [s, -c]];
        for (boundary, normal) in self.boundaries.iter().zip(normals) {
            if let Some([a, b, offset]) = boundary.plane()
                && (!a.is_finite()
                    || !b.is_finite()
                    || !offset.is_finite()
                    || (a.hypot(b) - 1.).abs() > 1e-8
                    || a * normal[0] + b * normal[1] <= 0.)
            {
                return Err(invalid("invalid spatial region boundary"));
            }
        }
        if self.boundaries.iter().all(|b| b.plane().is_some()) {
            let quad = self
                .physical_quad()
                .ok_or(invalid("spatial boundaries do not enclose a region"))?;
            if !crate::scan_regions::Areas::coverage(quad)?
                .area()
                .is_finite()
            {
                return Err(invalid("spatial region area overflow"));
            }
            if self
                .boundaries
                .iter()
                .filter_map(Boundary::plane)
                .any(|[a, b, c]| quad.iter().any(|p| a * p[0] + b * p[1] < c - 1e-7))
            {
                return Err(invalid("inconsistent spatial region boundaries"));
            }
        }
        Ok(())
    }

    pub fn physical_quad(&self) -> Option<Quad> {
        let planes: Vec<_> = self
            .boundaries
            .iter()
            .map(Boundary::plane)
            .collect::<Option<_>>()?;
        let intersect = |a: Plane, b: Plane| {
            let det = a[0] * b[1] - a[1] * b[0];
            (det.abs() > 1e-12).then(|| {
                [
                    (a[2] * b[1] - a[1] * b[2]) / det,
                    (a[0] * b[2] - a[2] * b[0]) / det,
                ]
            })
        };
        Some([
            intersect(planes[0], planes[2])?,
            intersect(planes[1], planes[2])?,
            intersect(planes[1], planes[3])?,
            intersect(planes[0], planes[3])?,
        ])
    }

    /// Convert the detector's raster basis without losing unequal source scales or source origin.
    pub fn from_picture(
        frame: &PictureGeometry,
        active: SourceRegion,
        measured: [bool; 4],
    ) -> Result<Self, Error> {
        active.validate()?;
        if frame.size.contains(&0) || !frame.angle.is_finite() || frame.angle.abs() > 6. {
            return Err(invalid("invalid picture geometry for spatial metadata"));
        }
        let scale = [
            f64::from(active.width) / f64::from(frame.size[0]),
            f64::from(active.height) / f64::from(frame.size[1]),
        ];
        let (s, c) = frame.angle.to_radians().sin_cos();
        let normals = [[c, s], [-c, -s], [-s, c], [s, -c]];
        let boundaries = std::array::from_fn(|side| {
            let Some(bound) = frame.sides[side] else {
                return Boundary::Unknown;
            };
            let normal = [normals[side][0] / scale[0], normals[side][1] / scale[1]];
            let length = normal[0].hypot(normal[1]);
            let offset = if side.is_multiple_of(2) {
                bound
            } else {
                -bound
            };
            let plane = [
                normal[0] / length,
                normal[1] / length,
                (offset + normal[0] * f64::from(active.x) + normal[1] * f64::from(active.y))
                    / length,
            ];
            if measured[side] {
                Boundary::Measured { plane }
            } else {
                Boundary::Inferred { plane }
            }
        });
        let angle = (scale[1] * s).atan2(scale[0] * c);
        let region = Self {
            angle,
            skew: (scale[1] * c).atan2(-scale[0] * s) - angle - std::f64::consts::FRAC_PI_2,
            boundaries,
        };
        region.validate()?;
        Ok(region)
    }

    /// Available, inscribed geometry, before any consumer margin. Unknown sides do not certify it.
    pub fn visible_crop(&self, active: SourceRegion, limits: &[Plane]) -> Result<Crop, Error> {
        self.validate()?;
        if self
            .boundaries
            .iter()
            .any(|b| matches!(b, Boundary::Unknown))
            || !self.boundaries.iter().any(Boundary::is_measured)
        {
            return Err(Error::UnusableSample("picture extent is unresolved"));
        }
        geometry::visible_crop(
            active,
            self.angle,
            self.boundaries
                .iter()
                .filter_map(Boundary::plane)
                .chain(limits.iter().copied()),
        )
    }

    fn measured_extent(&self, active: SourceRegion, axis: usize) -> Option<f64> {
        let angle = self.angle
            + if axis == 0 {
                0.
            } else {
                std::f64::consts::FRAC_PI_2 + self.skew
            };
        let (s, c) = angle.sin_cos();
        let nominal = [c, s];
        // Rounded/anisotropic rasters map orthogonal detector axes to source parallelograms.
        // Use the other observed axis when available, rather than treating the source pose
        // normal as its tangent and silently changing the learned Euclidean extent.
        let other = (1 - axis) * 2;
        let direction = self.boundaries[other]
            .plane()
            .or(self.boundaries[other + 1].plane())
            .map_or(nominal, |[a, b, _]| {
                let mut tangent = if axis == 0 { [b, -a] } else { [-b, a] };
                let sign = if tangent[0] * nominal[0] + tangent[1] * nominal[1] < 0. {
                    -1.
                } else {
                    1.
                };
                let length = tangent[0].hypot(tangent[1]);
                tangent.iter_mut().for_each(|v| *v *= sign / length);
                tangent
            });
        let center = Crop::from_region(active).center;
        let position = |side| {
            let Boundary::Measured { plane: [a, b, k] } = self.boundaries[side] else {
                return None;
            };
            let denominator = a * direction[0] + b * direction[1];
            (denominator.abs() > 1e-12).then_some((k - a * center[0] - b * center[1]) / denominator)
        };
        let size = position(axis * 2 + 1)? - position(axis * 2)?;
        (size.is_finite() && size > 0.).then_some(size)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub source: SourceId,
    pub revision: FileRevision,
    pub development: SourceDevelopment,
    /// Identity only when correction participated in measurement; grades are never dependencies.
    pub correction: Option<[u8; 32]>,
    pub code_scale: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceRegions {
    pub input: Input,
    pub observation: Option<Region>,
    pub picture: Option<Region>,
    pub film: Option<Region>,
    pub holder_opening: Option<Region>,
    /// Coarse film/holder/light/unknown classification, not a precise physical contour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<crate::scan_regions::Material>,
}

impl SourceRegions {
    /// Nominal picture framing. Missing film or holder coverage does not move its edges.
    pub fn picture_crop(&self) -> Result<Crop, Error> {
        self.input.development.validate()?;
        let picture = self
            .picture
            .as_ref()
            .ok_or(Error::UnusableSample("picture region is unavailable"))?;
        picture.visible_crop(self.input.development.active_area.into(), &[])
    }

    /// Nominal picture sampling window; consumers still qualify film/material footprints.
    pub fn picture_sampling(&self, inset_source_pixels: f64) -> Result<Quad, Error> {
        let mut crop = self.picture_crop()?;
        if !inset_source_pixels.is_finite() || inset_source_pixels < 0. {
            return Err(invalid("invalid spatial sampling inset"));
        }
        let removed = 2. * inset_source_pixels.ceil();
        if crop.size.iter().any(|&n| removed >= f64::from(n)) {
            return Err(Error::UnusableSample(
                "no picture remains after sampling inset",
            ));
        }
        crop.size = crop.size.map(|n| n - removed as u32);
        Ok(crop.coverage()?.corners().map(|p| p.0))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommonSize {
    pub extent: [Option<f64>; 2],
    pub observations: [Vec<SourceId>; 2],
}

impl CommonSize {
    pub fn fit(sources: &[SourceRegions]) -> Self {
        let mut observations = [vec![], vec![]];
        let extent = std::array::from_fn(|axis| {
            let mut values: Vec<_> = sources
                .iter()
                .filter_map(|source| {
                    let size = source
                        .observation
                        .as_ref()?
                        .measured_extent(source.input.development.active_area.into(), axis)?;
                    Some((source.input.source, size))
                })
                .collect();
            values.sort_by_key(|(id, _)| id.0);
            observations[axis] = values.iter().map(|(id, _)| *id).collect();
            values.sort_by(|a, b| a.1.total_cmp(&b.1));
            (!values.is_empty())
                .then(|| (values[(values.len() - 1) / 2].1 + values[values.len() / 2].1) * 0.5)
        });
        Self {
            extent,
            observations,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Group {
    pub development: Development,
    pub code_scale: f64,
    pub sources: Vec<SourceRegions>,
    pub common_size: CommonSize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    pub version: u32,
    pub groups: Vec<Group>,
}

impl Default for Metadata {
    fn default() -> Self {
        Self {
            version: VERSION,
            groups: vec![],
        }
    }
}

impl Metadata {
    pub fn validate(&self) -> Result<(), Error> {
        if self.version != VERSION {
            return Err(invalid("unsupported spatial metadata version"));
        }
        let mut identities = HashSet::new();
        for group in &self.groups {
            group.development.validate()?;
            if group.sources.is_empty()
                || !group.code_scale.is_finite()
                || group.code_scale < 1.
                || group
                    .common_size
                    .extent
                    .iter()
                    .flatten()
                    .any(|v| !v.is_finite() || *v <= 0.)
            {
                return Err(invalid("invalid spatial metadata group"));
            }
            let basis = &group.sources[0].input.development;
            for source in &group.sources {
                source.input.development.validate()?;
                if source.input.code_scale != group.code_scale {
                    return Err(invalid("spatial input code basis differs from group"));
                }
                if let Some(material) = &source.material {
                    material.validate()?;
                }
                if !identities.insert(source.input.source)
                    || source.input.development.active_area != basis.active_area
                    || source.input.development.camera_basis() != basis.camera_basis()
                {
                    return Err(invalid("invalid spatial metadata source association"));
                }
                let active = SourceRegion::from(source.input.development.active_area);
                active.validate()?;
                if let Some(observation) = &source.observation {
                    let camera = crate::coverage::Coverage::rectangle(active).corners();
                    for boundary in &observation.boundaries {
                        if let Boundary::Measured { plane: [a, b, c] } = boundary {
                            let distances = camera.map(|p| a * p.0[0] + b * p.0[1] - c);
                            if distances.iter().all(|v| *v > 1e-8)
                                || distances.iter().all(|v| *v < -1e-8)
                            {
                                return Err(invalid(
                                    "measured spatial boundary is outside camera coverage",
                                ));
                            }
                        }
                    }
                }
                for region in [
                    &source.observation,
                    &source.picture,
                    &source.film,
                    &source.holder_opening,
                ]
                .into_iter()
                .flatten()
                {
                    region.validate()?;
                }
                if source.observation.as_ref().is_some_and(|r| {
                    r.boundaries
                        .iter()
                        .any(|b| matches!(b, Boundary::Inferred { .. }))
                }) {
                    return Err(invalid(
                        "inferred geometry cannot supply spatial dimensions",
                    ));
                }
            }
            if group.common_size != CommonSize::fit(&group.sources) {
                return Err(invalid(
                    "common dimensions do not match spatial observations",
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn for_project(&self, project: &Project) -> Self {
        if self.validate().is_err() {
            return Self::default();
        }
        let mut result = self.clone();
        result.groups.retain(|group| {
            group.sources.first().is_some_and(|source| {
                project
                    .scan_basis()
                    .is_some_and(|basis| basis.require_source(&source.input.development).is_ok())
            }) && group.sources.iter().all(|source| {
                project
                    .source(source.input.source)
                    .is_ok_and(|record| record.development == source.input.development)
            })
        });
        result
    }

    /// No file I/O here: callers capture fresh identities through the normal acquisition service.
    /// One changed supporting input invalidates its group, not unrelated cached groups.
    pub fn matching(&self, project: &Project, inputs: &[Input]) -> Self {
        let mut ids = HashSet::new();
        if inputs.iter().any(|input| !ids.insert(input.source)) {
            return Self::default();
        }
        let mut result = self.for_project(project);
        result.groups.retain(|group| {
            group
                .sources
                .iter()
                .all(|source| inputs.contains(&source.input))
        });
        result
    }
}
