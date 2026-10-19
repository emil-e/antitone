use super::{
    CameraMatrix, ProcessingParameters, ProcessingSource, SourceGeometry, split_constants,
};
use crate::{
    Error,
    color::{Balance, ap1_to_cones},
    geometry::SourceRegion,
    local_adjustments::{GradientMask, LocalAdjustment, PhotoGradeSnapshot},
    photographic::Grade,
    processing::{Calibration, RollSettings},
};
use bytemuck::Zeroable;
use glam::{DMat3, DVec2, DVec3, Vec4};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LocalParameters {
    high: [Vec4; 5],
    low: [Vec4; 5],
    tail: [Vec4; 5],
    flags: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LocalContext {
    cones: CameraMatrix,
    inverse: CameraMatrix,
    area: [u32; 4],
    count: [u32; 4],
}

/// Immutable global/local resources with constant, range and normal-precision validation.
/// Consumers own accepted-snapshot signatures and stale-result rejection.
pub struct GpuPhotoGrade {
    pub(super) parameters: ProcessingParameters,
    pub(super) buffer: wgpu::Buffer,
    pub(super) context: wgpu::Buffer,
    snapshot: PhotoGradeSnapshot,
    camera_to_ap1: DMat3,
    area: SourceRegion,
    active: u32,
}

struct Prepared {
    parameters: ProcessingParameters,
    packed: Vec<LocalParameters>,
    area: SourceRegion,
    active: u32,
}

impl GpuPhotoGrade {
    /// Check the complete candidate and adapter limits without allocating GPU resources.
    pub fn validate(
        device: &wgpu::Device,
        calibration: Calibration,
        roll: RollSettings,
        snapshot: &PhotoGradeSnapshot,
        camera_to_ap1: DMat3,
        area: SourceRegion,
    ) -> Result<(), Error> {
        Self::prepare(device, calibration, roll, snapshot, camera_to_ap1, area).map(|_| ())
    }

    pub fn new(
        device: &wgpu::Device,
        calibration: Calibration,
        roll: RollSettings,
        snapshot: PhotoGradeSnapshot,
        camera_to_ap1: DMat3,
        area: SourceRegion,
    ) -> Result<Self, Error> {
        let Prepared {
            parameters,
            packed,
            area,
            active,
        } = Self::prepare(device, calibration, roll, &snapshot, camera_to_ap1, area)?;
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("immutable local photographic stages"),
            contents: bytemuck::cast_slice(&packed),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let context = context_buffer(device, active, [area.x, area.y, area.width, area.height])?;
        Ok(Self {
            parameters,
            buffer,
            context,
            snapshot,
            camera_to_ap1,
            area,
            active,
        })
    }

    #[cfg(any(feature = "editor", all(feature = "raw", feature = "export")))]
    pub(crate) fn validate_repaired(
        device: &wgpu::Device,
        calibration: Calibration,
        roll: RollSettings,
        snapshot: &PhotoGradeSnapshot,
        camera_to_ap1: DMat3,
        area: SourceRegion,
        neutral_bound: DVec3,
    ) -> Result<(), Error> {
        Self::prepare_with_neutral_bound(
            device,
            calibration,
            roll,
            snapshot,
            camera_to_ap1,
            area,
            neutral_bound,
        )
        .map(|_| ())
    }

    fn prepare(
        device: &wgpu::Device,
        calibration: Calibration,
        roll: RollSettings,
        snapshot: &PhotoGradeSnapshot,
        camera_to_ap1: DMat3,
        area: SourceRegion,
    ) -> Result<Prepared, Error> {
        Self::prepare_with_neutral_bound(
            device,
            calibration,
            roll,
            snapshot,
            camera_to_ap1,
            area,
            DVec3::ZERO,
        )
    }

    fn prepare_with_neutral_bound(
        device: &wgpu::Device,
        calibration: Calibration,
        roll: RollSettings,
        snapshot: &PhotoGradeSnapshot,
        camera_to_ap1: DMat3,
        area: SourceRegion,
        neutral_bound: DVec3,
    ) -> Result<Prepared, Error> {
        if !neutral_bound.is_finite() || neutral_bound.min_element() < 0. {
            return Err(Error::Gpu("invalid neutral repair bound".into()));
        }
        let area = area.validate()?;
        let (mut parameters, mut bound) = ProcessingParameters::positive_with_neutral_bound(
            calibration,
            roll,
            snapshot.global(),
            camera_to_ap1,
            neutral_bound,
        )?;
        let active = u32::try_from(snapshot.locals().iter().filter(|a| a.is_active()).count())
            .map_err(|_| Error::Gpu("too many local adjustments".into()))?;
        if active != 0 {
            SourceGeometry::new([area.x, area.y, area.width, area.height], [1, 1])?;
        }
        let bytes = u64::from(active.max(1)) * std::mem::size_of::<LocalParameters>() as u64;
        if bytes > device.limits().max_storage_buffer_binding_size
            || bytes > device.limits().max_buffer_size
        {
            return Err(Error::Gpu(
                "local adjustments exceed GPU buffer limits".into(),
            ));
        }
        let mut packed = Vec::with_capacity(active.max(1) as usize);
        let mut sensitivities = Vec::with_capacity(active as usize);
        for adjustment in snapshot.locals().iter().copied().filter(|a| a.is_active()) {
            let (peak, output, sensitivity) = stage_bounds(adjustment, bound)?;
            supported(peak)?;
            bound = output;
            sensitivities.push(sensitivity);
            parameters.response_precision.x =
                (f64::from(parameters.response_precision.x) / sensitivity.max(1.)) as f32;
            packed.push(LocalParameters::new(adjustment, area)?);
        }
        require_normal_precision(&sensitivities)?;
        if packed.is_empty() {
            packed.push(LocalParameters::zeroed());
        }
        Ok(Prepared {
            parameters,
            packed,
            area,
            active,
        })
    }

    pub fn snapshot(&self) -> &PhotoGradeSnapshot {
        &self.snapshot
    }

    /// Retained local constants and context; excludes per-dispatch uniforms and driver overhead.
    #[cfg(feature = "editor")]
    pub(crate) fn byte_len_for(snapshot: &PhotoGradeSnapshot) -> u64 {
        let active = snapshot
            .locals()
            .iter()
            .filter(|local| local.is_active())
            .count()
            .max(1);
        (active * std::mem::size_of::<LocalParameters>() + std::mem::size_of::<LocalContext>())
            as u64
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
                "local grade and RGB16 source camera matrices differ".into(),
            ));
        }
        if self.active != 0 || source.geometry.is_some_and(|g| g.is_pair()) {
            let geometry = source.geometry.ok_or_else(|| {
                Error::Gpu("local grade requires original-source geometry".into())
            })?;
            // Refinement can pad a final tile; every real member still starts in this source.
            for region in geometry.regions() {
                if region.x < self.area.x
                    || region.y < self.area.y
                    || region.x >= self.area.x + self.area.width
                    || region.y >= self.area.y + self.area.height
                {
                    return Err(Error::Gpu(
                        "local grade and source geometry regions differ".into(),
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
            "local adjustments can exceed the f32 intermediate working range".into(),
        ))
    } else {
        Ok(())
    }
}

fn absolute_product(matrix: DMat3, bound: DVec3) -> DVec3 {
    matrix.x_axis.abs() * bound.x + matrix.y_axis.abs() * bound.y + matrix.z_axis.abs() * bound.z
}

fn require_normal_precision(sensitivities: &[f64]) -> Result<(), Error> {
    // Reserve part of the existing absolute tolerance for unsupported tiny
    // signals. Every potentially observable value must retain all 53 bits in
    // normal f32 words, including before a later stage amplifies it.
    let mut observable = 2e-5 / 4.;
    for &gain in sensitivities.iter().rev() {
        observable /= gain;
        if !gain.is_finite() || observable * (f64::EPSILON / 2.) < f64::from(f32::MIN_POSITIVE) {
            return Err(Error::Gpu(
                "local stack can amplify signals below the normal GPU precision domain".into(),
            ));
        }
    }
    Ok(())
}

fn stage_bounds(adjustment: LocalAdjustment, input: f64) -> Result<(f64, f64, f64), Error> {
    let mut sensitivity = (adjustment.photo.exposure * adjustment.strength)
        .max(0.)
        .exp2();
    let mut bound = input * sensitivity;
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
        sensitivity *= gain;
        bound *= gain;
        peak = peak
            .max(cone_input.max_element())
            .max(cone_output.max_element())
            .max(bound);
    }
    let (creative_peak, output, creative_sensitivity) =
        Grade::new(adjustment.photo)?.weighted_bounds(bound, adjustment.strength);
    Ok((
        peak.max(creative_peak),
        output,
        sensitivity * creative_sensitivity,
    ))
}

impl LocalParameters {
    fn new(adjustment: LocalAdjustment, area: SourceRegion) -> Result<Self, Error> {
        let (kind, origin, x_axis, y_axis, radii, feather) = match adjustment.mask {
            GradientMask::Linear { start, end } => {
                let origin = DVec2::from_array(start.0);
                let delta = DVec2::from_array(end.0) - origin;
                let length = delta.x.hypot(delta.y);
                (1, origin, delta / length, DVec2::ZERO, [length, 1.], 0.)
            }
            GradientMask::Radial {
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
            let split =
                split_constants(DVec3::new(row[0], row[1], row[2]), row[3]).map_err(|_| {
                    Error::Gpu(
                        "local controls or mask geometry cannot be represented precisely on the GPU"
                        .into(),
                    )
                })?;
            result.high[i] = split[0];
            result.low[i] = split[1];
            result.tail[i] = split[2];
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
        label: Some("empty local grade"),
        contents: bytemuck::bytes_of(&LocalParameters::zeroed()),
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
            label: Some("local grade cone basis and source bounds"),
            contents: bytemuck::bytes_of(&LocalContext {
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
    use crate::{
        geometry::SourcePoint,
        local_adjustments::{LocalGrade, weighted_settings},
        processing::{InvertedPixel, PhotoSettings},
    };

    #[test]
    fn weighted_bounds_cover_interior_derivative_extrema_and_signed_intermediates() {
        for pivot in [0.01, 0.03, 0.18, 1.] {
            for contrast in [0.4, 0.65, 1.6] {
                for saturation in [0., 1., 2.] {
                    for input in [0.00001, 0.0078125, 0.1, 1., 100.] {
                        let photo = PhotoSettings {
                            contrast,
                            pivot,
                            saturation,
                            ..PhotoSettings::default()
                        };
                        let grade = Grade::new(photo).unwrap();
                        let (_, bound, _) = grade.weighted_bounds(input, 1.);
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
    fn sensitivity_bounds_cover_fractional_controls_and_near_zero_y() {
        let weights = crate::color::AP1_TO_XYZ.row(1);
        for contrast in [0.4, 0.7, 1.6] {
            for pivot in [0.01, 0.03, 0.18, 1.] {
                for colour in [0., 0.5, 1.] {
                    for input in [0.001, 0.01, 1., 100.] {
                        let photo = PhotoSettings {
                            contrast,
                            pivot,
                            contrast_colour_mix: colour,
                            saturation: 1.73,
                            saturation_density: colour,
                            ..Default::default()
                        };
                        let (_, _, sensitivity) =
                            Grade::new(photo).unwrap().weighted_bounds(input, 0.73);
                        for i in 0..=20 {
                            let grade = Grade::new(
                                weighted_settings(photo, 0.73 * i as f64 / 20.).unwrap(),
                            )
                            .unwrap();
                            let epsilon = input * 1e-7;
                            for rgb in [
                                DVec3::splat(input * 0.8),
                                DVec3::splat(-input * 0.8),
                                DVec3::new(input * 0.8, -weights.x / weights.y * input * 0.8, 0.),
                            ] {
                                let base = grade.apply(rgb).unwrap();
                                for direction in
                                    [DVec3::X, DVec3::Y, DVec3::Z, DVec3::new(1., -1., 1.)]
                                {
                                    let delta = (grade.apply(rgb + epsilon * direction).unwrap()
                                        - base)
                                        .abs()
                                        .max_element();
                                    assert!(
                                        delta <= epsilon * sensitivity * 1.0001,
                                        "{photo:?} B={input} weight={i}: {delta} > {}",
                                        epsilon * sensitivity
                                    );
                                }
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
                let adjustment = LocalAdjustment {
                    mask: GradientMask::Linear {
                        start: SourcePoint([0., 0.]),
                        end: SourcePoint([1., 0.]),
                    },
                    photo: PhotoSettings {
                        wb: Balance { warmth, tint },
                        ..Default::default()
                    },
                    enabled: true,
                    invert: false,
                    strength: 0.6,
                };
                let (_, output, sensitivity) = stage_bounds(adjustment, 100.).unwrap();
                for i in 0..=200 {
                    let weighted =
                        weighted_settings(adjustment.photo, 0.6 * i as f64 / 200.).unwrap();
                    let matrix = crate::color::wb_matrix(weighted.wb).unwrap();
                    let norm = absolute_product(matrix, DVec3::ONE).max_element();
                    assert!(
                        norm <= sensitivity,
                        "WB={:?} weight={i}: {norm} > {sensitivity}",
                        adjustment.photo.wb
                    );
                    assert!(100. * norm <= output);
                }
                if warmth.abs() <= 0.005 && tint.abs() <= 0.003 {
                    assert!(
                        sensitivity < 1.02,
                        "near-identity WB must not inherit cone-basis condition numbers"
                    );
                }
            }
        }
    }

    #[test]
    fn modest_photographic_stack_remains_in_the_normal_precision_domain() {
        let calibration = Calibration {
            dmin: DVec3::new(0.19, 0.14, 0.12),
            dmax: DVec3::new(0.008, 0.004, 0.002),
            input_floor: 1. / 65535.,
        };
        let roll = RollSettings {
            gamma: 0.65,
            ..RollSettings::new_grade()
        };
        let (_, mut bound) = ProcessingParameters::positive_with_bound(
            calibration,
            roll,
            PhotoSettings::default(),
            DMat3::IDENTITY,
        )
        .unwrap();
        let adjustment = LocalAdjustment {
            mask: GradientMask::Linear {
                start: SourcePoint([0., 0.]),
                end: SourcePoint([1., 0.]),
            },
            photo: PhotoSettings {
                exposure: 0.25,
                wb: Balance {
                    warmth: 0.005,
                    tint: -0.003,
                },
                contrast: 1.01,
                contrast_colour_mix: 0.25,
                saturation: 1.02,
                saturation_density: 0.4,
                ..Default::default()
            },
            enabled: true,
            invert: false,
            strength: 0.6,
        };
        let mut sensitivities = Vec::new();
        for _ in 0..3 {
            let (peak, output, sensitivity) = stage_bounds(adjustment, bound).unwrap();
            supported(peak).unwrap();
            bound = output;
            sensitivities.push(sensitivity);
        }
        assert!(require_normal_precision(&sensitivities).is_ok());
        // A tighter bound does not waive the unchanged normal-precision gate.
        let (_, _, sensitivity) = stage_bounds(adjustment, bound).unwrap();
        sensitivities.push(sensitivity);
        assert!(require_normal_precision(&sensitivities).is_err());

        let global = PhotoSettings {
            exposure: -2.,
            contrast: 1.05,
            saturation: 1.2,
            ..Default::default()
        };
        let (_, input) = ProcessingParameters::positive_with_bound(
            calibration,
            roll,
            PhotoSettings {
                exposure: global.exposure,
                ..Default::default()
            },
            DMat3::IDENTITY,
        )
        .unwrap();
        let scratch = Grade::new(global).unwrap().intermediate_bound(input);
        let (_, output) =
            ProcessingParameters::positive_with_bound(calibration, roll, global, DMat3::IDENTITY)
                .unwrap();
        for (mut bound, accepted) in [(scratch, false), (output, true)] {
            let mut sensitivities = Vec::new();
            for _ in 0..4 {
                let (peak, next, sensitivity) = stage_bounds(adjustment, bound).unwrap();
                supported(peak).unwrap();
                bound = next;
                sensitivities.push(sensitivity);
            }
            assert_eq!(
                require_normal_precision(&sensitivities).is_ok(),
                accepted,
                "input bound={bound}, sensitivities={sensitivities:?}"
            );
        }
    }

    #[test]
    fn stage_bounds_cover_continuous_exposure_wb_and_creative_controls() {
        let adjustment = LocalAdjustment {
            mask: GradientMask::Linear {
                start: SourcePoint([0., 0.]),
                end: SourcePoint([1., 0.]),
            },
            photo: crate::processing::PhotoSettings {
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
        let (_, bound, _) = stage_bounds(adjustment, 100.).unwrap();
        let grade = LocalGrade::new(vec![adjustment]).unwrap();
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
