//! Ordered photographic adjustment layers in source-anchored AP1.
use crate::{
    Error,
    color::{Balance, wb_matrix},
    geometry::SourcePoint,
    photographic::Grade,
    processing::{InputPipeline, InputSettings, InvertedPixel, RollSettings, exposure_multiplier},
};
use glam::{DMat3, DVec2, DVec3};

pub const DEFAULT_PIVOT: f64 = 0.18;
pub const DEFAULT_COLOUR_MIX: f64 = 0.;

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "project", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "project", serde(deny_unknown_fields))]
pub struct AdjustmentLayerSettings {
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

impl Default for AdjustmentLayerSettings {
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

impl AdjustmentLayerSettings {
    pub fn validate(self) -> Result<Self, Error> {
        exposure_multiplier(self.exposure)?;
        wb_matrix(self.wb)?;
        crate::photographic::Grade::new(self)?;
        Ok(self)
    }
}

pub const CONTRACT: &str = "ap1-ordered-adjustment-layers-smoothstep-source-v1";

/// Analytic coverage in original developed-source pixel-edge coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "project", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "project",
    serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)
)]
pub enum AdjustmentLayerCoverage {
    // Struct form makes serde reject stray geometry on whole-image coverage.
    Global {},
    Linear {
        start: SourcePoint,
        end: SourcePoint,
    },
    Radial {
        center: SourcePoint,
        radii: [f64; 2],
        /// Clockwise radians in the source plane (x right, y down).
        angle: f64,
        /// Fraction of normalized radius occupied by the transition, in 0..1.
        feather: f64,
    },
}

impl AdjustmentLayerCoverage {
    pub fn validate(self) -> Result<Self, Error> {
        match self {
            Self::Global {} => {}
            Self::Linear { start, end } => {
                let start = source_point(start)?;
                let end = source_point(end)?;
                let delta = end - start;
                let length = delta.x.hypot(delta.y);
                if !length.is_finite() || length <= 0. {
                    return Err(Error::InvalidParameter(
                        "linear gradient endpoints must have finite, nonzero separation",
                    ));
                }
            }
            Self::Radial {
                center,
                radii,
                angle,
                feather,
            } => {
                source_point(center)?;
                if radii.iter().any(|r| !r.is_finite() || *r <= 0.) || !angle.is_finite() {
                    return Err(Error::InvalidParameter(
                        "radial gradient requires positive finite radii and a finite angle",
                    ));
                }
                unit_interval(feather, "radial feather must be within 0..1")?;
            }
        }
        Ok(self)
    }

    /// Coverage before adjustment inversion/strength; no image bounds are imposed.
    pub fn coverage(self, point: SourcePoint) -> Result<f64, Error> {
        self.validate()?.coverage_at(source_point(point)?)
    }

    fn coverage_at(self, point: DVec2) -> Result<f64, Error> {
        let coverage = match self {
            Self::Global {} => 1.,
            Self::Linear { start, end } => {
                let start = DVec2::from_array(start.0);
                let delta = DVec2::from_array(end.0) - start;
                let length = delta.x.hypot(delta.y);
                let t = ((point - start) / length).dot(delta / length);
                if !t.is_finite() {
                    return Err(Error::NumericalOverflow);
                }
                smoothstep(t)
            }
            Self::Radial {
                center,
                radii,
                angle,
                feather,
            } => {
                let delta = point - DVec2::from_array(center.0);
                let (sin, cos) = angle.sin_cos();
                let x = delta.dot(DVec2::new(cos, sin)) / radii[0];
                let y = delta.dot(DVec2::new(-sin, cos)) / radii[1];
                let radius = x.hypot(y);
                if !radius.is_finite() {
                    return Err(Error::NumericalOverflow);
                }
                if radius >= 1. {
                    0.
                } else if radius <= 1. - feather {
                    1.
                } else {
                    smoothstep((1. - radius) / feather)
                }
            }
        };
        Ok(coverage)
    }
}

fn source_point(point: SourcePoint) -> Result<DVec2, Error> {
    let point = DVec2::from_array(point.0);
    if !point.is_finite() {
        return Err(Error::InvalidParameter("nonfinite local-mask coordinate"));
    }
    Ok(point)
}

fn unit_interval(value: f64, reason: &'static str) -> Result<(), Error> {
    if !value.is_finite() || !(0. ..=1.).contains(&value) {
        return Err(Error::InvalidParameter(reason));
    }
    Ok(())
}

fn smoothstep(t: f64) -> f64 {
    let t = t.clamp(0., 1.);
    // Evaluate from the nearer endpoint to keep rounded coverage within 0..1.
    let u = t.min(1. - t);
    let value = u * u * (3. - 2. * u);
    if t <= 0.5 { value } else { 1. - value }
}

/// Attenuates layer amounts towards identity, retaining pivot and blend settings.
pub fn weighted_settings(
    photo: AdjustmentLayerSettings,
    weight: f64,
) -> Result<AdjustmentLayerSettings, Error> {
    photo.validate()?;
    unit_interval(weight, "adjustment layer weight must be within 0..1")?;
    Ok(weighted(photo, weight))
}

fn weighted(photo: AdjustmentLayerSettings, weight: f64) -> AdjustmentLayerSettings {
    if weight == 1. {
        return photo;
    }
    AdjustmentLayerSettings {
        exposure: weight * photo.exposure,
        wb: Balance {
            warmth: weight * photo.wb.warmth,
            tint: weight * photo.wb.tint,
        },
        contrast: 1. + weight * (photo.contrast - 1.),
        saturation: 1. + weight * (photo.saturation - 1.),
        ..photo
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "project", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "project", serde(deny_unknown_fields))]
pub struct AdjustmentLayer {
    pub mask: AdjustmentLayerCoverage,
    pub photo: AdjustmentLayerSettings,
    pub enabled: bool,
    pub invert: bool,
    pub strength: f64,
}

impl AdjustmentLayer {
    pub fn global(photo: AdjustmentLayerSettings) -> Self {
        Self {
            mask: AdjustmentLayerCoverage::Global {},
            photo,
            enabled: true,
            invert: false,
            strength: 1.,
        }
    }

    pub(crate) fn is_active(self) -> bool {
        self.enabled
            && self.strength != 0.
            && (self.photo.exposure != 0.
                || self.photo.wb != Balance::default()
                || self.photo.contrast != 1.
                || self.photo.saturation != 1.)
    }

    pub fn weight(self, point: SourcePoint) -> Result<f64, Error> {
        self.validate()?.weight_at(source_point(point)?)
    }

    fn weight_at(self, point: DVec2) -> Result<f64, Error> {
        if !self.enabled {
            return Ok(0.);
        }
        let coverage = self.mask.coverage_at(point)?;
        Ok((if self.invert { 1. - coverage } else { coverage }) * self.strength)
    }

    pub fn validate(self) -> Result<Self, Error> {
        self.mask.validate()?;
        self.photo.validate()?;
        unit_interval(
            self.strength,
            "adjustment layer strength must be within 0..1",
        )?;
        Ok(self)
    }
}

/// Owned processing values, independent of adjustment names/IDs and later document edits.
#[derive(Debug, Clone, PartialEq)]
pub struct AdjustmentLayerSnapshot {
    layers: AdjustmentLayerStack,
}

impl AdjustmentLayerSnapshot {
    pub fn new(layers: Vec<AdjustmentLayer>) -> Result<Self, Error> {
        Ok(Self {
            layers: AdjustmentLayerStack::new(layers)?,
        })
    }

    /// Explicit owned snapshot payload, excluding allocator bookkeeping.
    pub fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.layers.layers.capacity() * std::mem::size_of::<AdjustmentLayer>()
    }

    pub fn layers(&self) -> &[AdjustmentLayer] {
        &self.layers.layers
    }
}

/// Full scalar processing at original source pixel centres, before reconstruction.
#[derive(Debug, Clone)]
pub struct AdjustmentLayerPipeline {
    input: InputPipeline,
    layers: AdjustmentLayerStack,
}

impl AdjustmentLayerPipeline {
    pub fn new(
        input: impl Into<InputSettings>,
        roll: RollSettings,
        grade: AdjustmentLayerSnapshot,
        camera_to_ap1: DMat3,
    ) -> Result<Self, Error> {
        Ok(Self {
            input: InputPipeline::new(input, roll, camera_to_ap1)?,
            layers: grade.layers,
        })
    }

    /// Grade absolute neutral AP1 at an original source pixel, retaining floor provenance.
    pub fn grade_neutral(
        &self,
        pixel: InvertedPixel,
        source_pixel: [u32; 2],
    ) -> Result<InvertedPixel, Error> {
        self.layers.apply(
            self.input.apply_roll(pixel)?,
            SourcePoint(source_pixel.map(|v| f64::from(v) + 0.5)),
        )
    }

    pub fn apply(&self, scan: DVec3, source_pixel: [u32; 2]) -> Result<InvertedPixel, Error> {
        self.layers.apply(
            self.input.apply(scan)?,
            SourcePoint(source_pixel.map(|v| f64::from(v) + 0.5)),
        )
    }
}

/// An immutable validated scalar stack; persistence and GPU preflight are separate.
#[derive(Debug, Clone, PartialEq)]
pub struct AdjustmentLayerStack {
    layers: Vec<AdjustmentLayer>,
}

impl AdjustmentLayerStack {
    pub fn new(layers: Vec<AdjustmentLayer>) -> Result<Self, Error> {
        for adjustment in &layers {
            adjustment.validate()?;
        }
        Ok(Self { layers })
    }

    /// Takes Roll-graded AP1 at an original source pixel centre, preserving floor flags.
    pub fn apply(
        &self,
        mut pixel: InvertedPixel,
        point: SourcePoint,
    ) -> Result<InvertedPixel, Error> {
        if !pixel.rgb.is_finite() {
            return Err(Error::NumericalOverflow);
        }
        let point = source_point(point)?;
        for adjustment in &self.layers {
            let photo = adjustment.photo;
            if !adjustment.is_active() {
                continue;
            }
            let weight = adjustment.weight_at(point)?;
            if weight == 0. {
                continue;
            }
            let photo = weighted(photo, weight);
            let matrix = wb_matrix(photo.wb)? * exposure_multiplier(photo.exposure)?;
            pixel.rgb = Grade::new(photo)?.apply(matrix * pixel.rgb)?;
        }
        Ok(pixel)
    }
}
