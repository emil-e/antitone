mod graph;
#[cfg(all(test, feature = "editor"))]
mod graph_tests;
#[cfg(test)]
mod layout_tests;

use crate::gpu::{
    parameters::{CameraMatrix, ProcessingParameters},
    source::SourceGeometry,
};
use crate::{
    Error,
    adjustment_layers::{AdjustmentLayer, AdjustmentLayerCoverage, AdjustmentLayerSnapshot},
    color::{AP1_TO_XYZ, Balance, ap1_to_cones, oklab},
    geometry::SourceRegion,
    photographic::{Grade, TONAL_WIDTH, encoded_grey},
    processing::RollSettings,
};
use bytemuck::Zeroable;
use glam::{DVec2, Vec2, Vec3};

// Host layouts for grading/bindings.wgsl; shader layout tests check the member offsets.
#[repr(C, align(16))]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct AdjustmentLayerParameters {
    mask_origin: Vec2,
    mask_x_axis: Vec2,
    mask_y_axis: Vec2,
    mask_radii: Vec2,
    wb_log_cone_offsets: Vec3,
    log_exposure_gain: f32,
    mask_feather: f32,
    strength: f32,
    contrast: f32,
    encoded_pivot: f32,
    preserve: f32,
    saturation: f32,
    vibrance: f32,
    shadows_ev: f32,
    highlights_ev: f32,
    mask_kind: u32,
    invert_mask: u32,
    has_wb: u32,
}

#[repr(C, align(16))]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct AdjustmentLayerContext {
    ap1_to_bradford: CameraMatrix,
    bradford_to_ap1: CameraMatrix,
    ap1_to_oklab_lms: CameraMatrix,
    oklab_lms_to_ap1: CameraMatrix,
    oklab_lightness: Vec3,
    inverse_tonal_width: f32,
    ap1_luminance: Vec3,
    encoded_middle_grey: f32,
    source_origin: [u32; 2],
    source_extent: [u32; 2],
    layer_count: u32,
    _padding: [u32; 3],
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
        let context = context_parameters(active, area)?;
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
        use super::evaluation::{Function, pointwise};

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
        let mask_radii = DVec2::from_array(radii).as_vec2();
        if !mask_radii.is_finite() {
            return Err(Error::Gpu(
                "adjustment mask radii exceed the f32 range".into(),
            ));
        }

        let photo = adjustment.photo;
        let grade = Grade::new(photo)?;
        Ok(Self {
            mask_origin: origin.as_vec2(),
            mask_x_axis: x_axis.as_vec2(),
            mask_y_axis: y_axis.as_vec2(),
            mask_radii,
            wb_log_cone_offsets: photo.wb.offsets()?.as_vec3(),
            log_exposure_gain: (photo.exposure * std::f64::consts::LN_2) as f32,
            mask_feather: feather as f32,
            strength: adjustment.strength as f32,
            contrast: photo.contrast as f32,
            encoded_pivot: grade.qp as f32,
            preserve: photo.contrast_colour_mix as f32,
            saturation: photo.saturation as f32,
            vibrance: photo.vibrance as f32,
            shadows_ev: photo.shadows as f32,
            highlights_ev: photo.highlights as f32,
            mask_kind: kind,
            invert_mask: u32::from(adjustment.invert),
            has_wb: u32::from(photo.wb != Balance::default()),
        })
    }
}

fn context_parameters(
    layer_count: u32,
    area: SourceRegion,
) -> Result<AdjustmentLayerContext, Error> {
    let cones = ap1_to_cones();
    Ok(AdjustmentLayerContext {
        ap1_to_bradford: CameraMatrix::new(cones)?,
        bradford_to_ap1: CameraMatrix::new(cones.inverse())?,
        ap1_to_oklab_lms: CameraMatrix::new(*oklab::AP1_TO_LMS)?,
        oklab_lms_to_ap1: CameraMatrix::new(*oklab::LMS_TO_AP1)?,
        oklab_lightness: oklab::LIGHTNESS.as_vec3(),
        inverse_tonal_width: 1. / TONAL_WIDTH as f32,
        ap1_luminance: AP1_TO_XYZ.row(1).as_vec3(),
        encoded_middle_grey: encoded_grey() as f32,
        source_origin: [area.x, area.y],
        source_extent: [area.width, area.height],
        layer_count,
        _padding: [0; 3],
    })
}
