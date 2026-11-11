mod graph;
#[cfg(all(test, feature = "editor"))]
mod graph_tests;

use crate::gpu::{
    parameters::{CameraMatrix, ProcessingParameters, packed_constants},
    source::SourceGeometry,
};
use crate::{
    Error,
    adjustment_layers::{AdjustmentLayer, AdjustmentLayerCoverage, AdjustmentLayerSnapshot},
    color::{Balance, ap1_to_cones},
    geometry::SourceRegion,
    photographic::Grade,
    processing::RollSettings,
};
use bytemuck::Zeroable;
use glam::{DVec2, DVec3, Vec4};

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct AdjustmentLayerParameters {
    values: [Vec4; 5],
    flags: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct AdjustmentLayerContext {
    cones: CameraMatrix,
    inverse: CameraMatrix,
    area: [u32; 4],
    count: [u32; 4],
}

/// Prepared Roll/layer controls and image operations, independent of source interpretation.
#[derive(Clone)]
pub struct Grading {
    pub(super) parameters: ProcessingParameters,
    pub(super) roll: RollSettings,
    pub(super) snapshot: AdjustmentLayerSnapshot,
    pub(super) layers: std::sync::Arc<[u8]>,
    pub(super) context: AdjustmentLayerContext,
    pub(super) area: SourceRegion,
    pub(super) spatial: bool,
    pub(super) coverage: u32,
    pub(super) coordinates: Option<SourceGeometry>,
    pub(super) active_layers: bool,
}

impl Grading {
    pub fn validate(
        limits: &wgpu::Limits,
        roll: RollSettings,
        snapshot: &AdjustmentLayerSnapshot,
        area: SourceRegion,
    ) -> Result<(), Error> {
        ProcessingParameters::photographic(roll)?;
        prepare_layers(limits, snapshot, area.validate()?).map(|_| ())
    }

    pub fn new(
        limits: &wgpu::Limits,
        roll: RollSettings,
        snapshot: AdjustmentLayerSnapshot,
        area: SourceRegion,
    ) -> Result<Self, Error> {
        let parameters = ProcessingParameters::photographic(roll)?;
        let area = area.validate()?;
        let packed = prepare_layers(limits, &snapshot, area)?;
        let active = snapshot.layers().iter().filter(|a| a.is_active()).count() as u32;
        let layers = bytemuck::cast_slice(&packed).into();
        let context = context_parameters(active, [area.x, area.y, area.width, area.height])?;
        let spatial = snapshot
            .layers()
            .iter()
            .any(|layer| layer.is_active() && layer.mask != AdjustmentLayerCoverage::Global {});
        Ok(Self {
            parameters,
            roll,
            snapshot,
            layers,
            context,
            area,
            spatial,
            coverage: 0,
            coordinates: None,
            active_layers: active != 0,
        })
    }

    /// Evaluate the same effective layer weights used by photographic grading, even for an
    /// identity-valued layer omitted from the active grade traversal.
    #[cfg(feature = "editor")]
    pub(crate) fn weight_image(
        input: super::evaluation::Value,
        mut layer: AdjustmentLayer,
    ) -> Result<super::evaluation::Value, Error> {
        use super::evaluation::{Function, Meaning, pointwise};

        layer.validate()?;
        if !layer.enabled {
            layer.strength = 0.;
        }

        let parameters = AdjustmentLayerParameters::new(layer, input.domain().finite()?.source()?)?;
        let code = format!(
            "{}\n{}",
            Self::shader(),
            include_str!("grading/layer_weight.wgsl")
        );

        Ok(pointwise(
            input,
            Meaning::Weight,
            Function::new(code, "evaluation_layer_weight")
                .with_storage(6, bytemuck::bytes_of(&parameters)),
        ))
    }

    pub fn snapshot(&self) -> &AdjustmentLayerSnapshot {
        &self.snapshot
    }

    #[cfg(feature = "editor")]
    pub(crate) fn byte_len_for(layers: &AdjustmentLayerSnapshot) -> u64 {
        let active = layers
            .layers()
            .iter()
            .filter(|layer| layer.is_active())
            .count()
            .max(1);
        (active * std::mem::size_of::<AdjustmentLayerParameters>()
            + std::mem::size_of::<AdjustmentLayerContext>()) as u64
    }

    pub fn byte_len(&self) -> u64 {
        self.layers.len() as u64 + std::mem::size_of::<AdjustmentLayerContext>() as u64
    }

    pub(super) fn require_extent(&self, area: SourceRegion) -> Result<(), Error> {
        if self.spatial && self.area != area {
            return Err(Error::Gpu(
                "photographic grade and input coordinate extents differ".into(),
            ));
        }
        Ok(())
    }

    pub(super) fn require_geometry(&self, geometry: Option<SourceGeometry>) -> Result<(), Error> {
        if self.spatial {
            let geometry = geometry.ok_or_else(|| {
                Error::Gpu("adjustment layer grade requires original-source geometry".into())
            })?;
            for region in geometry.regions() {
                if region.x < self.area.x
                    || region.y < self.area.y
                    || region.x >= self.area.x + self.area.width
                    || region.y >= self.area.y + self.area.height
                {
                    return Err(Error::Gpu(
                        "adjustment layer grade and source geometry regions differ".into(),
                    ));
                }
            }
        }
        Ok(())
    }
}

fn prepare_layers(
    limits: &wgpu::Limits,
    snapshot: &AdjustmentLayerSnapshot,
    area: SourceRegion,
) -> Result<Vec<AdjustmentLayerParameters>, Error> {
    let active = u32::try_from(snapshot.layers().iter().filter(|a| a.is_active()).count())
        .map_err(|_| Error::Gpu("too many adjustment layers".into()))?;
    if active != 0 {
        SourceGeometry::new([area.x, area.y, area.width, area.height], [1, 1])?;
    }
    let bytes = u64::from(active.max(1)) * std::mem::size_of::<AdjustmentLayerParameters>() as u64;
    if bytes > limits.max_storage_buffer_binding_size || bytes > limits.max_buffer_size {
        return Err(Error::Gpu(
            "adjustment layers exceed GPU buffer limits".into(),
        ));
    }
    let mut packed = snapshot
        .layers()
        .iter()
        .copied()
        .filter(|a| a.is_active())
        .map(|layer| AdjustmentLayerParameters::new(layer, area))
        .collect::<Result<Vec<_>, _>>()?;
    if packed.is_empty() {
        packed.push(AdjustmentLayerParameters::zeroed());
    }
    Ok(packed)
}

pub(super) fn supported(bound: f64) -> Result<(), Error> {
    if !bound.is_finite() || bound > f32::MAX as f64 / 4. {
        Err(Error::Gpu(
            "adjustment mask coordinates exceed the f32 range".into(),
        ))
    } else {
        Ok(())
    }
}

impl AdjustmentLayerParameters {
    fn new(adjustment: AdjustmentLayer, area: SourceRegion) -> Result<Self, Error> {
        let (kind, origin, x_axis, y_axis, radii, feather) = match adjustment.mask {
            AdjustmentLayerCoverage::Global {} => {
                (0, DVec2::ZERO, DVec2::ZERO, DVec2::ZERO, [1., 1.], 0.)
            }
            AdjustmentLayerCoverage::Linear { start, end } => {
                let origin = DVec2::from_array(start.0);
                let delta = DVec2::from_array(end.0) - origin;
                let length = delta.x.hypot(delta.y);
                (1, origin, delta / length, DVec2::ZERO, [length, 1.], 0.)
            }
            AdjustmentLayerCoverage::Radial {
                center,
                radii,
                angle,
                feather,
            } => {
                let (sin, cos) = angle.sin_cos();
                (
                    2,
                    DVec2::from_array(center.0),
                    DVec2::new(cos, sin),
                    DVec2::new(-sin, cos),
                    radii,
                    feather,
                )
            }
        };
        // Bound coordinate subtraction and each projection product, not just coverage.
        let coordinates = DVec2::new(
            f64::from(area.x + area.width),
            f64::from(area.y + area.height),
        );
        let delta = coordinates + origin.abs();
        supported(delta.max_element())?;
        if kind == 1 {
            supported((delta / radii[0]).max_element())?;
        }
        for (axis, radius) in [x_axis, y_axis].into_iter().zip(radii) {
            supported(delta.dot(axis.abs()))?;
            supported(delta.dot(axis.abs()) / radius)?;
        }
        let offsets = adjustment.photo.wb.offsets()?;
        let qp = Grade::new(adjustment.photo)?.qp;
        let rows = [
            [origin.x, origin.y, x_axis.x, x_axis.y],
            [y_axis.x, y_axis.y, feather, adjustment.strength],
            [
                adjustment.photo.exposure * std::f64::consts::LN_2,
                offsets.x,
                offsets.y,
                offsets.z,
            ],
            [
                adjustment.photo.contrast,
                qp,
                adjustment.photo.contrast_colour_mix,
                adjustment.photo.saturation,
            ],
            [adjustment.photo.saturation_density, radii[0], radii[1], 0.],
        ];
        let mut result = Self::zeroed();
        for (i, row) in rows.into_iter().enumerate() {
            result.values[i] = packed_constants(DVec3::new(row[0], row[1], row[2]), row[3])?;
        }
        result.flags = [
            kind,
            u32::from(adjustment.invert),
            u32::from(adjustment.photo.wb != Balance::default()),
            0,
        ];
        Ok(result)
    }
}

fn context_parameters(count: u32, area: [u32; 4]) -> Result<AdjustmentLayerContext, Error> {
    let cones = ap1_to_cones();
    Ok(AdjustmentLayerContext {
        cones: CameraMatrix::new(cones)?,
        inverse: CameraMatrix::new(cones.inverse())?,
        area,
        count: [count, 0, 0, 0],
    })
}
