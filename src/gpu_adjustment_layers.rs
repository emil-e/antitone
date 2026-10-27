use super::{
    CameraMatrix, ProcessingParameters, ProcessingSource, SourceGeometry, packed_constants,
};
use crate::{
    Error,
    adjustment_layers::{AdjustmentLayer, AdjustmentLayerCoverage, AdjustmentLayerSnapshot},
    color::{Balance, ap1_to_cones},
    geometry::SourceRegion,
    photographic::Grade,
    processing::{InputSettings, RollSettings},
};
use bytemuck::Zeroable;
use glam::{DMat3, DVec2, DVec3, Vec4};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct AdjustmentLayerParameters {
    values: [Vec4; 5],
    flags: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct AdjustmentLayerContext {
    cones: CameraMatrix,
    inverse: CameraMatrix,
    area: [u32; 4],
    count: [u32; 4],
}

/// Immutable adjustment-layer resources with f32 constant and intermediate-range validation.
/// Consumers own accepted-snapshot signatures and stale-result rejection.
pub struct GpuAdjustmentLayers {
    pub(super) parameters: ProcessingParameters,
    pub(super) buffer: wgpu::Buffer,
    pub(super) context: wgpu::Buffer,
    snapshot: AdjustmentLayerSnapshot,
    camera_to_ap1: DMat3,
    area: SourceRegion,
    spatial: bool,
}

struct Prepared {
    parameters: ProcessingParameters,
    packed: Vec<AdjustmentLayerParameters>,
    area: SourceRegion,
    active: u32,
}

impl GpuAdjustmentLayers {
    /// Check the complete candidate and adapter limits without allocating GPU resources.
    pub fn validate(
        device: &wgpu::Device,
        input: impl Into<InputSettings>,
        roll: RollSettings,
        snapshot: &AdjustmentLayerSnapshot,
        camera_to_ap1: DMat3,
        area: SourceRegion,
    ) -> Result<(), Error> {
        Self::validate_limits(&device.limits(), input, roll, snapshot, camera_to_ap1, area)
    }

    pub fn validate_limits(
        limits: &wgpu::Limits,
        input: impl Into<InputSettings>,
        roll: RollSettings,
        snapshot: &AdjustmentLayerSnapshot,
        camera_to_ap1: DMat3,
        area: SourceRegion,
    ) -> Result<(), Error> {
        Self::prepare(limits, &input.into(), roll, snapshot, camera_to_ap1, area).map(|_| ())
    }

    pub fn new(
        device: &wgpu::Device,
        input: impl Into<InputSettings>,
        roll: RollSettings,
        snapshot: AdjustmentLayerSnapshot,
        camera_to_ap1: DMat3,
        area: SourceRegion,
    ) -> Result<Self, Error> {
        let input = input.into();
        let Prepared {
            parameters,
            packed,
            area,
            active,
        } = Self::prepare(
            &device.limits(),
            &input,
            roll,
            &snapshot,
            camera_to_ap1,
            area,
        )?;
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("immutable photographic adjustment layers"),
            contents: bytemuck::cast_slice(&packed),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let context = context_buffer(device, active, [area.x, area.y, area.width, area.height])?;
        let spatial = snapshot
            .layers()
            .iter()
            .any(|layer| layer.is_active() && layer.mask != (AdjustmentLayerCoverage::Global {}));
        Ok(Self {
            parameters,
            buffer,
            context,
            snapshot,
            camera_to_ap1,
            area,
            spatial,
        })
    }

    #[cfg(any(feature = "editor", all(feature = "raw", feature = "export")))]
    pub(crate) fn validate_repaired(
        device: &wgpu::Device,
        input: impl Into<InputSettings>,
        roll: RollSettings,
        snapshot: &AdjustmentLayerSnapshot,
        camera_to_ap1: DMat3,
        area: SourceRegion,
        neutral_bound: DVec3,
    ) -> Result<(), Error> {
        Self::prepare_with_neutral_bound(
            &device.limits(),
            &input.into(),
            roll,
            snapshot,
            camera_to_ap1,
            area,
            neutral_bound,
        )
        .map(|_| ())
    }

    fn prepare(
        limits: &wgpu::Limits,
        input: &InputSettings,
        roll: RollSettings,
        snapshot: &AdjustmentLayerSnapshot,
        camera_to_ap1: DMat3,
        area: SourceRegion,
    ) -> Result<Prepared, Error> {
        Self::prepare_with_neutral_bound(
            limits,
            input,
            roll,
            snapshot,
            camera_to_ap1,
            area,
            DVec3::ZERO,
        )
    }

    fn prepare_with_neutral_bound(
        limits: &wgpu::Limits,
        input: &InputSettings,
        roll: RollSettings,
        snapshot: &AdjustmentLayerSnapshot,
        camera_to_ap1: DMat3,
        area: SourceRegion,
        neutral_bound: DVec3,
    ) -> Result<Prepared, Error> {
        if !neutral_bound.is_finite() || neutral_bound.min_element() < 0. {
            return Err(Error::Gpu("invalid neutral repair bound".into()));
        }
        let area = area.validate()?;
        let (parameters, mut bound) = ProcessingParameters::interpret_with_neutral_bound(
            *input,
            roll,
            camera_to_ap1,
            neutral_bound,
        )?;
        let active = u32::try_from(snapshot.layers().iter().filter(|a| a.is_active()).count())
            .map_err(|_| Error::Gpu("too many adjustment layers".into()))?;
        if active != 0 {
            SourceGeometry::new([area.x, area.y, area.width, area.height], [1, 1])?;
        }
        let bytes =
            u64::from(active.max(1)) * std::mem::size_of::<AdjustmentLayerParameters>() as u64;
        if bytes > limits.max_storage_buffer_binding_size || bytes > limits.max_buffer_size {
            return Err(Error::Gpu(
                "adjustment layers exceed GPU buffer limits".into(),
            ));
        }
        let mut packed = Vec::with_capacity(active.max(1) as usize);
        for adjustment in snapshot.layers().iter().copied().filter(|a| a.is_active()) {
            let (peak, output) = stage_bounds(adjustment, bound)?;
            supported(peak)?;
            bound = output;
            packed.push(AdjustmentLayerParameters::new(adjustment, area)?);
        }
        if packed.is_empty() {
            packed.push(AdjustmentLayerParameters::zeroed());
        }
        Ok(Prepared {
            parameters,
            packed,
            area,
            active,
        })
    }

    #[cfg(feature = "editor")]
    pub(crate) fn source_matrix(&self) -> DMat3 {
        self.camera_to_ap1
    }

    pub fn snapshot(&self) -> &AdjustmentLayerSnapshot {
        &self.snapshot
    }

    /// Retained local constants and context; excludes per-dispatch uniforms and driver overhead.
    #[cfg(feature = "editor")]
    pub(crate) fn byte_len_for(snapshot: &AdjustmentLayerSnapshot) -> u64 {
        let active = snapshot
            .layers()
            .iter()
            .filter(|local| local.is_active())
            .count()
            .max(1);
        (active * std::mem::size_of::<AdjustmentLayerParameters>()
            + std::mem::size_of::<AdjustmentLayerContext>()) as u64
    }

    pub fn byte_len(&self) -> u64 {
        self.buffer.size() + self.context.size()
    }

    pub(super) fn same_content(&self, other: &Self) -> bool {
        self.snapshot == other.snapshot
            && self.camera_to_ap1 == other.camera_to_ap1
            && self.area == other.area
    }

    pub(super) fn require_source(&self, source: &ProcessingSource) -> Result<(), Error> {
        if source.camera_to_ap1 != self.camera_to_ap1 {
            return Err(Error::Gpu(
                "adjustment layer grade and RGB16 source camera matrices differ".into(),
            ));
        }
        if self.spatial || source.geometry.is_some_and(|g| g.is_pair()) {
            let geometry = source.geometry.ok_or_else(|| {
                Error::Gpu("adjustment layer grade requires original-source geometry".into())
            })?;
            // Refinement can pad a final tile; every real member still starts in this source.
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

fn supported(bound: f64) -> Result<(), Error> {
    if !bound.is_finite() || bound > f32::MAX as f64 / 4. {
        Err(Error::Gpu(
            "adjustment layers can exceed the f32 intermediate working range".into(),
        ))
    } else {
        Ok(())
    }
}

fn absolute_product(matrix: DMat3, bound: DVec3) -> DVec3 {
    matrix.x_axis.abs() * bound.x + matrix.y_axis.abs() * bound.y + matrix.z_axis.abs() * bound.z
}

fn stage_bounds(adjustment: AdjustmentLayer, input: f64) -> Result<(f64, f64), Error> {
    let gain = (adjustment.photo.exposure * adjustment.strength)
        .max(0.)
        .exp2();
    let mut bound = input * gain;
    let mut peak = input.max(bound);
    if adjustment.photo.wb != Balance::default() {
        let cones = ap1_to_cones();
        let cone_input = absolute_product(cones, DVec3::splat(bound));
        let offsets = adjustment.photo.wb.offsets()? * adjustment.strength;
        let gains = offsets.max(DVec3::ZERO).exp();
        let cone_output = cone_input * gains;
        let inverse = cones.inverse();
        // M(w) = inverse*C + inverse*diag(expm1(w*offsets))*C.
        // Bound the change from the signed baseline, not |inverse|*|C|:
        // the latter discards the cancellation even for arbitrarily small WB.
        let baseline = absolute_product(inverse * cones, DVec3::ONE);
        let movement = absolute_product(
            inverse,
            absolute_product(cones, DVec3::ONE) * offsets.map(|v| v.exp_m1().abs()),
        );
        let formation = absolute_product(inverse, absolute_product(cones, DVec3::ONE));
        let gain = (baseline + movement + 8. * f64::EPSILON * formation).max_element();
        bound *= gain;
        peak = peak
            .max(cone_input.max_element())
            .max(cone_output.max_element())
            .max(bound);
    }
    let (creative_peak, output) =
        Grade::new(adjustment.photo)?.weighted_bounds(bound, adjustment.strength);
    Ok((peak.max(creative_peak), output))
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

pub(super) fn empty_buffer(device: &wgpu::Device) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("empty adjustment layer grade"),
        contents: bytemuck::bytes_of(&AdjustmentLayerParameters::zeroed()),
        usage: wgpu::BufferUsages::STORAGE,
    })
}

pub(super) fn context_buffer(
    device: &wgpu::Device,
    count: u32,
    area: [u32; 4],
) -> Result<wgpu::Buffer, Error> {
    let cones = ap1_to_cones();
    Ok(
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("adjustment layer grade cone basis and source bounds"),
            contents: bytemuck::bytes_of(&AdjustmentLayerContext {
                cones: CameraMatrix::new(cones)?,
                inverse: CameraMatrix::new(cones.inverse())?,
                area,
                count: [count, 0, 0, 0],
            }),
            usage: wgpu::BufferUsages::UNIFORM,
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adjustment_layers::AdjustmentLayerSettings;
    use crate::{
        adjustment_layers::{AdjustmentLayerStack, weighted_settings},
        geometry::SourcePoint,
        processing::InvertedPixel,
    };

    #[test]
    fn weighted_bounds_cover_interior_derivative_extrema_and_signed_intermediates() {
        for pivot in [0.01, 0.03, 0.18, 1.] {
            for contrast in [0.4, 0.65, 1.6] {
                for saturation in [0., 1., 2.] {
                    for input in [0.00001, 0.0078125, 0.1, 1., 100.] {
                        let photo = AdjustmentLayerSettings {
                            contrast,
                            pivot,
                            saturation,
                            ..AdjustmentLayerSettings::default()
                        };
                        let grade = Grade::new(photo).unwrap();
                        let (_, bound) = grade.weighted_bounds(input, 1.);
                        for i in 0..=200 {
                            let weighted =
                                Grade::new(weighted_settings(photo, i as f64 / 200.).unwrap())
                                    .unwrap();
                            // Opposed channels include large chroma at near-zero AP1 Y.
                            for rgb in [
                                DVec3::splat(input),
                                DVec3::splat(-input),
                                DVec3::new(input, -0.4038491080227416 * input, 0.),
                                DVec3::new(-input, input, input),
                            ] {
                                let output = weighted.apply(rgb).unwrap().abs().max_element();
                                assert!(
                                    output <= bound * (1. + 1e-12),
                                    "{photo:?} input={input} weight={i}: {output} > {bound}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn small_wb_bounds_preserve_the_signed_baseline_over_continuous_weights() {
        for warmth in [-1., -0.005, 0., 0.005, 1.] {
            for tint in [-1., -0.003, 0., 0.003, 1.] {
                let adjustment = AdjustmentLayer {
                    mask: AdjustmentLayerCoverage::Linear {
                        start: SourcePoint([0., 0.]),
                        end: SourcePoint([1., 0.]),
                    },
                    photo: AdjustmentLayerSettings {
                        wb: Balance { warmth, tint },
                        ..Default::default()
                    },
                    enabled: true,
                    invert: false,
                    strength: 0.6,
                };
                let (_, output) = stage_bounds(adjustment, 100.).unwrap();
                for i in 0..=200 {
                    let weighted =
                        weighted_settings(adjustment.photo, 0.6 * i as f64 / 200.).unwrap();
                    let matrix = crate::color::wb_matrix(weighted.wb).unwrap();
                    let norm = absolute_product(matrix, DVec3::ONE).max_element();
                    assert!(100. * norm <= output);
                }
                if warmth.abs() <= 0.005 && tint.abs() <= 0.003 {
                    assert!(
                        output < 102.,
                        "near-identity WB must not inherit cone-basis condition numbers"
                    );
                }
            }
        }
    }

    #[test]
    fn stage_bounds_cover_continuous_exposure_wb_and_creative_controls() {
        let adjustment = AdjustmentLayer {
            mask: AdjustmentLayerCoverage::Linear {
                start: SourcePoint([0., 0.]),
                end: SourcePoint([1., 0.]),
            },
            photo: crate::adjustment_layers::AdjustmentLayerSettings {
                exposure: 2.,
                wb: Balance {
                    warmth: -1.,
                    tint: 1.,
                },
                contrast: 0.4,
                saturation: 2.,
                ..Default::default()
            },
            enabled: true,
            invert: false,
            strength: 0.73,
        };
        let (_, bound) = stage_bounds(adjustment, 100.).unwrap();
        let grade = AdjustmentLayerStack::new(vec![adjustment]).unwrap();
        for i in 0..=200 {
            for rgb in [
                DVec3::splat(100.),
                DVec3::splat(-100.),
                DVec3::new(100., -100., 100.),
            ] {
                let actual = grade
                    .apply(
                        InvertedPixel {
                            rgb,
                            floored: [false; 3],
                        },
                        SourcePoint([i as f64 / 200., 0.]),
                    )
                    .unwrap();
                assert!(actual.rgb.abs().max_element() <= bound);
            }
        }
    }
}
