#![cfg(feature = "gpu")]
use antitone::{
    color::{AP1_TO_XYZ, BRADFORD, Balance},
    flat_field::FlatField,
    geometry::{SourcePoint, SourceRaster, SourceRegion},
    gpu::{
        GpuFlatField, GpuPhotoGrade, ProcessingKernel, ProcessingParameters, ProcessingSource,
        SourceGeometry,
    },
    local_adjustments::{GradientMask, LocalAdjustment, PhotoGradeSnapshot},
    processing::{Calibration, PhotoSettings, Pipeline, RollSettings},
};
use glam::{DMat3, DVec2, DVec3};
use std::sync::Arc;
#[path = "support/photographic_reference.rs"]
mod reference;

fn context() -> (wgpu::Device, wgpu::Queue, ProcessingKernel) {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let kernel = ProcessingKernel::new(&device, &queue).await.unwrap();
        (device, queue, kernel)
    })
}
fn calibration() -> Calibration {
    Calibration {
        dmin: DVec3::new(0.19, 0.14, 0.12),
        dmax: DVec3::new(0.008, 0.004, 0.002),
        input_floor: 1. / 65535.,
    }
}
fn matrix() -> DMat3 {
    DMat3::from_cols(
        DVec3::new(0.6062, 0.0161, -0.0031),
        DVec3::new(0.3802, 1.1927, -0.1490),
        DVec3::new(0.0136, -0.2088, 1.1521),
    )
}
fn adjustment(mask: GradientMask, photo: PhotoSettings) -> LocalAdjustment {
    LocalAdjustment {
        mask,
        photo,
        enabled: true,
        invert: false,
        strength: 1.,
    }
}
fn codes(point: [u32; 2]) -> [u16; 4] {
    match (point[0] + point[1]) % 9 {
        0 => [0; 4],
        1 => [65535, 0, 65535, 42],
        2 => [65535; 4],
        3 => [12452, 9175, 7864, 65535],
        _ => [
            point[0].wrapping_mul(1871) as u16,
            point[1].wrapping_mul(7919) as u16,
            (point[0] + point[1]).wrapping_mul(3413) as u16,
            123,
        ],
    }
}
fn pixels(raster: SourceRaster) -> Vec<[u16; 4]> {
    let [width, height] = raster.size();
    (0..width * height)
        .map(|i| codes(raster.source_pixel([i % width, i / width]).unwrap()))
        .collect()
}
fn source(
    device: &wgpu::Device,
    area: SourceRegion,
    size: [u32; 2],
    matrix: DMat3,
    field: Option<Arc<GpuFlatField>>,
) -> (ProcessingSource, Vec<[u16; 4]>) {
    let pixels = pixels(SourceRaster::new(area, size).unwrap());
    let mut source = ProcessingSource::new(device, &pixels, matrix).unwrap();
    source
        .set_flat_field(
            field,
            SourceGeometry::new([area.x, area.y, area.width, area.height], size).unwrap(),
        )
        .unwrap();
    assert_eq!(source.byte_len(), pixels.len() as u64 * 8);
    (source, pixels)
}
#[allow(clippy::too_many_arguments)]
fn read(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    kernel: &ProcessingKernel,
    source: &ProcessingSource,
    n: usize,
    grade: &GpuPhotoGrade,
    legacy: Option<ProcessingParameters>,
) -> Vec<[f32; 4]> {
    let size = n as u64 * 16;
    assert_eq!(size, source.byte_len() * 2);
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    if let Some(parameters) = legacy {
        kernel.encode(device, &mut encoder, source, &output, parameters);
    } else {
        kernel
            .encode_grade(device, &mut encoder, source, &output, grade)
            .unwrap();
    }
    encoder.copy_buffer_to_buffer(&output, 0, &read, 0, size);
    queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    read.map_async(wgpu::MapMode::Read, .., move |r| tx.send(r).unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    rx.recv().unwrap().unwrap();
    let mapped = read.get_mapped_range(..).unwrap();
    let result = bytemuck::cast_slice::<u8, [f32; 4]>(&mapped).to_vec();
    drop(mapped);
    read.unmap();
    result
}
fn smooth(t: f64) -> f64 {
    let t = t.clamp(0., 1.);
    let u = t.min(1. - t);
    let v = u * u * (3. - 2. * u);
    if t <= 0.5 { v } else { 1. - v }
}
fn coverage(mask: GradientMask, point: [f64; 2]) -> f64 {
    let point = DVec2::from_array(point);
    match mask {
        GradientMask::Linear { start, end } => {
            let start = DVec2::from_array(start.0);
            let delta = DVec2::from_array(end.0) - start;
            let length = delta.x.hypot(delta.y);
            smooth(((point - start) / length).dot(delta / length))
        }
        GradientMask::Radial {
            center,
            radii,
            angle,
            feather,
        } => {
            let delta = point - DVec2::from_array(center.0);
            let (sin, cos) = angle.sin_cos();
            let radius = ((cos * delta.x + sin * delta.y) / radii[0])
                .hypot((-sin * delta.x + cos * delta.y) / radii[1]);
            if radius >= 1. {
                0.
            } else if radius <= 1. - feather {
                1.
            } else {
                smooth((1. - radius) / feather)
            }
        }
    }
}
fn local_reference(mut rgb: DVec3, locals: &[LocalAdjustment], point: [f64; 2]) -> DVec3 {
    for a in locals {
        if !a.enabled || a.strength == 0. {
            continue;
        }
        let m = coverage(a.mask, point);
        let w = a.strength * if a.invert { 1. - m } else { m };
        if w == 0. {
            continue;
        }
        rgb *= (w * a.photo.exposure).exp2();
        if a.photo.wb != Balance::default() {
            let warmth = w * a.photo.wb.warmth / 2_f64.sqrt();
            let tint = w * a.photo.wb.tint / 6_f64.sqrt();
            let gains = DVec3::new(warmth + tint, -2. * tint, -warmth + tint).exp();
            let cones = BRADFORD * AP1_TO_XYZ;
            rgb = (cones.inverse() * DMat3::from_diagonal(gains) * cones) * rgb;
        }
        let c = 1. + w * (a.photo.contrast - 1.);
        rgb = reference::blend(
            reference::contrast(rgb, c, a.photo.pivot, reference::Contrast::Rgb).unwrap(),
            reference::contrast(
                rgb,
                c,
                a.photo.pivot,
                reference::Contrast::BlackOffsetRatios,
            )
            .unwrap(),
            a.photo.contrast_colour_mix,
        )
        .unwrap();
        let s = 1. + w * (a.photo.saturation - 1.);
        rgb = reference::blend(
            reference::saturation(rgb, s, reference::Saturation::ConstantY).unwrap(),
            reference::saturation(rgb, s, reference::Saturation::Maximum).unwrap(),
            a.photo.saturation_density,
        )
        .unwrap();
    }
    rgb
}
fn assert_reference(
    actual: &[[f32; 4]],
    pixels: &[[u16; 4]],
    raster: SourceRaster,
    snapshot: &PhotoGradeSnapshot,
    roll: RollSettings,
    matrix: DMat3,
    field: Option<&FlatField>,
) {
    let global = Pipeline::new(calibration(), roll, snapshot.global(), matrix).unwrap();
    let mut signed = false;
    let mut hdr = false;
    for (i, pixel) in pixels.iter().enumerate() {
        let xy = raster
            .source_pixel([i as u32 % raster.size()[0], i as u32 / raster.size()[0]])
            .unwrap();
        let point = [xy[0] as f64 + 0.5, xy[1] as f64 + 0.5];
        let scan = DVec3::new(pixel[0] as f64, pixel[1] as f64, pixel[2] as f64) / 65535.;
        let scan = scan
            * field
                .map(|field| field.gain(point).unwrap())
                .unwrap_or(DVec3::ONE);
        let inverted = global.apply(scan).unwrap();
        let expected = local_reference(inverted.rgb, snapshot.locals(), point);
        let flags = inverted
            .floored
            .iter()
            .enumerate()
            .fold(0, |bits, (c, flag)| bits | (u32::from(*flag) << c));
        assert_eq!(actual[i][3] as u32, flags);
        signed |= inverted.rgb.min_element() < 0.;
        hdr |= inverted.rgb.max_element() > 1.;
        for c in 0..3 {
            assert!(
                actual[i][c].is_finite()
                    && (actual[i][c] as f64 - expected[c]).abs() <= 2e-5 + 2e-4 * expected[c].abs(),
                "pixel={i} point={point:?} channel={c} GPU={:?} reference={expected:?} locals={:?}",
                actual[i],
                snapshot.locals()
            );
        }
    }
    assert!(
        signed && hdr,
        "fixture must exercise signed/HDR working data"
    );
}
#[test]
fn analytic_linear_radial_and_ordered_weighted_controls_match_independent_reference() {
    let (device, queue, kernel) = context();
    let area = SourceRegion::from([120, 80, 128, 96]);
    let size = [16, 12];
    let field = Arc::new(
        FlatField::from_nodes(
            [120, 80, 128, 96],
            [2, 2],
            vec![
                DVec3::new(0.81, 1.17, 0.97),
                DVec3::new(1.23, 0.78, 1.07),
                DVec3::new(1.11, 0.99, 0.89),
                DVec3::new(0.93, 1.03, 1.14),
            ],
        )
        .unwrap(),
    );
    let gpu_field = Arc::new(GpuFlatField::new(&device, field.clone()).unwrap());
    for corrected in [false, true] {
        let (source, pixels) = source(
            &device,
            area,
            size,
            matrix(),
            corrected.then(|| gpu_field.clone()),
        );
        for case in 0..18 {
            let mask = if case % 2 == 0 {
                GradientMask::Linear {
                    start: SourcePoint([140.5, 90.5]),
                    end: SourcePoint([180.5, 130.5]),
                }
            } else {
                GradientMask::Radial {
                    center: SourcePoint([184.5, 124.5]),
                    radii: [60., 32.],
                    angle: [0., 0.371, std::f64::consts::FRAC_PI_2][case / 2 % 3],
                    feather: [0., 0.5, 1.][case / 6 % 3],
                }
            };
            let photo = PhotoSettings {
                exposure: [0.713456, -1.7, 2.31][case % 3],
                wb: Balance {
                    warmth: [-1., 0., 1.][case / 3 % 3],
                    tint: [1., -0.123456789, -1.][case % 3],
                },
                contrast: [0.4, 0.71234567, 1.6][case / 2 % 3],
                pivot: [0.01, 0.18, 1.][case / 6 % 3],
                contrast_colour_mix: [0., 0.432156, 1.][case % 3],
                saturation: [0., 0.71234567, 2.][case % 3],
                saturation_density: [1., 0.21345678, 0.][case / 3 % 3],
            };
            let mut local = adjustment(mask, photo);
            local.invert = case % 4 == 0;
            local.strength = [1., 0.73, 0.31][case / 3 % 3];
            let mut locals = vec![local];
            if case >= 12 {
                let mut second = adjustment(
                    GradientMask::Linear {
                        start: SourcePoint([210.5, 84.5]),
                        end: SourcePoint([160.5, 150.5]),
                    },
                    PhotoSettings {
                        exposure: -0.7,
                        contrast: 1.6,
                        saturation: 1.3,
                        ..Default::default()
                    },
                );
                second.strength = 0.61;
                locals.push(second);
                if case % 2 == 0 {
                    locals.reverse();
                }
            }
            let snapshot = PhotoGradeSnapshot::new(
                PhotoSettings {
                    contrast: 1.21,
                    saturation: 1.2,
                    ..Default::default()
                },
                locals,
            )
            .unwrap();
            let grade = GpuPhotoGrade::new(
                &device,
                calibration(),
                RollSettings::new_grade(),
                snapshot.clone(),
                matrix(),
                area,
            )
            .unwrap();
            assert_eq!(grade.snapshot(), &snapshot);
            let actual = read(
                &device,
                &queue,
                &kernel,
                &source,
                pixels.len(),
                &grade,
                None,
            );
            assert_reference(
                &actual,
                &pixels,
                SourceRaster::new(area, size).unwrap(),
                &snapshot,
                RollSettings::new_grade(),
                matrix(),
                corrected.then_some(field.as_ref()),
            );
        }
    }
}
#[test]
fn roll_and_photo_wb_precede_local_grading_at_the_current_default_gamma() {
    let (device, queue, kernel) = context();
    let area = SourceRegion::from([120, 80, 27, 24]);
    let size = [11, 9];
    let roll = RollSettings {
        wb: Balance {
            warmth: 0.23,
            tint: -0.17,
        },
        ..RollSettings::new_grade()
    };
    let snapshot = PhotoGradeSnapshot::new(
        PhotoSettings {
            wb: Balance {
                warmth: -0.11,
                tint: 0.07,
            },
            contrast: 1.03,
            ..Default::default()
        },
        vec![adjustment(
            GradientMask::Linear {
                start: SourcePoint([125., 83.]),
                end: SourcePoint([141., 99.]),
            },
            PhotoSettings {
                exposure: 0.3,
                wb: Balance {
                    warmth: 0.13,
                    tint: 0.09,
                },
                contrast: 1.02,
                saturation: 1.1,
                ..Default::default()
            },
        )],
    )
    .unwrap();
    let grade = GpuPhotoGrade::new(
        &device,
        calibration(),
        roll,
        snapshot.clone(),
        matrix(),
        area,
    )
    .unwrap();
    let (source, pixels) = source(&device, area, size, matrix(), None);
    let actual = read(
        &device,
        &queue,
        &kernel,
        &source,
        pixels.len(),
        &grade,
        None,
    );
    assert_reference(
        &actual,
        &pixels,
        SourceRaster::new(area, size).unwrap(),
        &snapshot,
        roll,
        matrix(),
        None,
    );
}

#[test]
fn tightened_global_local_bounds_admit_photographic_stacks_with_independent_pixel_agreement() {
    let (device, queue, kernel) = context();
    let area = SourceRegion::from([12, 12, 8192, 5472]);
    let size = [31, 19];
    for (gamma, global, count) in [0.60, 0.65].into_iter().flat_map(|gamma| {
        [
            (gamma, PhotoSettings::default(), 3),
            (
                gamma,
                PhotoSettings {
                    exposure: -2.,
                    contrast: 1.05,
                    saturation: 1.2,
                    ..Default::default()
                },
                if gamma == 0.60 { 3 } else { 4 },
            ),
        ]
    }) {
        let roll = RollSettings {
            gamma,
            ..RollSettings::new_grade()
        };
        let locals = (0..count)
            .map(|i| LocalAdjustment {
                mask: if i % 2 == 1 {
                    GradientMask::Radial {
                        center: SourcePoint([4100., 2750.]),
                        radii: [3500., 2300.],
                        angle: 0.3,
                        feather: 0.8,
                    }
                } else {
                    GradientMask::Linear {
                        start: SourcePoint([900., 800.]),
                        end: SourcePoint([7000., 4400.]),
                    }
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
                invert: i % 3 == 0,
                strength: 0.6,
            })
            .collect();
        let snapshot = PhotoGradeSnapshot::new(global, locals).unwrap();
        for matrix in [DMat3::IDENTITY, matrix()] {
            let grade = GpuPhotoGrade::new(
                &device,
                calibration(),
                roll,
                snapshot.clone(),
                matrix,
                area,
            )
            .unwrap_or_else(|error| {
                panic!("gamma={gamma} global={global:?} count={count} matrix={matrix:?}: {error}")
            });
            let (source, pixels) = source(&device, area, size, matrix, None);
            let actual = read(
                &device,
                &queue,
                &kernel,
                &source,
                pixels.len(),
                &grade,
                None,
            );
            assert_reference(
                &actual,
                &pixels,
                SourceRaster::new(area, size).unwrap(),
                &snapshot,
                roll,
                matrix,
                None,
            );
        }
    }
}

#[test]
fn default_response_ordered_locals_preserve_corrected_noisy_shadows_and_floor_flags() {
    let (device, queue, kernel) = context();
    let area = SourceRegion::from([120, 80, 32, 8]);
    let raster = SourceRaster::new(area, [32, 8]).unwrap();
    let field = Arc::new(
        FlatField::from_nodes(
            [120, 80, 32, 8],
            [2, 2],
            vec![
                DVec3::new(0.81, 1.17, 0.97),
                DVec3::new(1.23, 0.78, 1.07),
                DVec3::new(1.11, 0.99, 0.89),
                DVec3::new(0.93, 1.03, 1.14),
            ],
        )
        .unwrap(),
    );
    let pixels: Vec<_> = (0..256)
        .map(|i| match i % 4 {
            0 => [0, 1, 0, 0],
            1 => [900, 300, 100, 0],
            2 => {
                let noise = (i % 7) as i32 - 3;
                [
                    (12452 + noise) as u16,
                    (9175 - noise) as u16,
                    (7864 + 2 * noise) as u16,
                    0,
                ]
            }
            _ => codes([i % 32, i / 32]),
        })
        .collect();
    let roll = RollSettings::new_grade();
    assert_eq!(roll.gamma, antitone::processing::DEFAULT_GAMMA);
    for corrected in [false, true] {
        let cal = if corrected {
            field.processing_calibration(calibration()).unwrap()
        } else {
            calibration()
        };
        let mut source = ProcessingSource::new(&device, &pixels, matrix()).unwrap();
        source
            .set_flat_field(
                corrected.then(|| Arc::new(GpuFlatField::new(&device, field.clone()).unwrap())),
                SourceGeometry::new([120, 80, 32, 8], [32, 8]).unwrap(),
            )
            .unwrap();
        let mut locals = vec![
            adjustment(
                GradientMask::Linear {
                    start: SourcePoint([121., 80.]),
                    end: SourcePoint([149., 87.]),
                },
                PhotoSettings {
                    exposure: 0.5,
                    wb: Balance {
                        warmth: 0.02,
                        tint: -0.01,
                    },
                    ..Default::default()
                },
            ),
            adjustment(
                GradientMask::Radial {
                    center: SourcePoint([135., 84.]),
                    radii: [14., 5.],
                    angle: 0.3,
                    feather: 0.8,
                },
                PhotoSettings {
                    exposure: -0.25,
                    contrast: 1.05,
                    saturation: 1.1,
                    ..Default::default()
                },
            ),
        ];
        let mut results = Vec::new();
        for _ in 0..2 {
            let snapshot =
                PhotoGradeSnapshot::new(PhotoSettings::default(), locals.clone()).unwrap();
            let grade = GpuPhotoGrade::new(&device, cal, roll, snapshot, matrix(), area).unwrap();
            let actual = read(
                &device,
                &queue,
                &kernel,
                &source,
                pixels.len(),
                &grade,
                None,
            );
            let global = Pipeline::new(cal, roll, PhotoSettings::default(), matrix()).unwrap();
            let mut signed = false;
            let mut hdr = false;
            for (i, pixel) in pixels.iter().enumerate() {
                let xy = raster.source_pixel([i as u32 % 32, i as u32 / 32]).unwrap();
                let point = [xy[0] as f64 + 0.5, xy[1] as f64 + 0.5];
                let scan = DVec3::new(pixel[0] as f64, pixel[1] as f64, pixel[2] as f64) / 65535.;
                let scan = scan
                    * if corrected {
                        field.gain(point).unwrap()
                    } else {
                        DVec3::ONE
                    };
                let inverted = global.apply(scan).unwrap();
                let expected = local_reference(inverted.rgb, &locals, point);
                let flags = inverted
                    .floored
                    .iter()
                    .enumerate()
                    .fold(0, |bits, (c, flag)| bits | (u32::from(*flag) << c));
                assert_eq!(actual[i][3] as u32, flags);
                signed |= expected.min_element() < 0.;
                hdr |= expected.max_element() > 1.;
                for c in 0..3 {
                    assert!(
                        actual[i][c].is_finite()
                            && (actual[i][c] as f64 - expected[c]).abs()
                                <= 2e-5 + 2e-4 * expected[c].abs(),
                        "corrected={corrected} point={point:?} channel={c} GPU={:?} reference={expected:?}",
                        actual[i]
                    );
                }
            }
            assert!(signed && hdr);
            results.push(actual);
            locals.reverse();
        }
        assert!(
            results[0].iter().zip(&results[1]).any(|(a, b)| a != b),
            "the ordered fixture must distinguish nonlinear processing order"
        );
    }
}

#[test]
fn no_locals_disabled_identity_and_zero_coverage_keep_legacy_bits_and_geometry_is_required_only_when_active()
 {
    let (device, queue, kernel) = context();
    let area = SourceRegion::from([0, 0, 8, 8]);
    let pixels = pixels(SourceRaster::new(area, [8, 8]).unwrap());
    let source = ProcessingSource::new(&device, &pixels, matrix()).unwrap();
    let global = PhotoSettings {
        contrast: 0.71,
        saturation: 1.8,
        ..Default::default()
    };
    let empty = PhotoGradeSnapshot::new(global, vec![]).unwrap();
    let prepared = GpuPhotoGrade::new(
        &device,
        calibration(),
        RollSettings::new_grade(),
        empty,
        matrix(),
        area,
    )
    .unwrap();
    let parameters =
        ProcessingParameters::positive(calibration(), RollSettings::new_grade(), global, matrix())
            .unwrap();
    let baseline = read(
        &device,
        &queue,
        &kernel,
        &source,
        pixels.len(),
        &prepared,
        Some(parameters),
    );
    let bits = |pixels: &[[f32; 4]]| {
        pixels
            .iter()
            .map(|p| p.map(f32::to_bits))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        bits(&read(
            &device,
            &queue,
            &kernel,
            &source,
            pixels.len(),
            &prepared,
            None
        )),
        bits(&baseline)
    );
    let mask = GradientMask::Linear {
        start: SourcePoint([100., 0.]),
        end: SourcePoint([200., 0.]),
    };
    for mode in 0..4 {
        let mut a = adjustment(
            mask,
            PhotoSettings {
                exposure: 3.,
                ..Default::default()
            },
        );
        match mode {
            0 => a.enabled = false,
            1 => a.strength = 0.,
            2 => {
                a.photo = PhotoSettings {
                    pivot: 1.,
                    contrast_colour_mix: 1.,
                    saturation_density: 1.,
                    ..Default::default()
                }
            }
            _ => {}
        }
        let snapshot = PhotoGradeSnapshot::new(global, vec![a]).unwrap();
        let prepared = GpuPhotoGrade::new(
            &device,
            calibration(),
            RollSettings::new_grade(),
            snapshot,
            matrix(),
            area,
        )
        .unwrap();
        let mut source = ProcessingSource::new(&device, &pixels, matrix()).unwrap();
        if mode == 3 {
            let output = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: pixels.len() as u64 * 16,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            assert!(
                kernel
                    .encode_grade(&device, &mut encoder, &source, &output, &prepared)
                    .is_err()
            );
            source
                .set_geometry(SourceGeometry::new([0, 0, 8, 8], [8, 8]).unwrap())
                .unwrap();
        }
        let actual = read(
            &device,
            &queue,
            &kernel,
            &source,
            pixels.len(),
            &prepared,
            None,
        );
        assert_eq!(bits(&actual), bits(&baseline));
    }
}
#[test]
fn source_centres_at_large_origins_preserve_subpixel_mask_geometry() {
    let (device, queue, kernel) = context();
    let area = SourceRegion::from([16_770_000, 16_770_000, 128, 96]);
    let size = [8, 6];
    let point = [f64::from(area.x) + 8.5, f64::from(area.y) + 8.5];
    let (source, pixels) = source(&device, area, size, matrix(), None);
    for mask in [
        GradientMask::Linear {
            start: SourcePoint([point[0] - 0.0001, point[1]]),
            end: SourcePoint([point[0] + 0.0001, point[1]]),
        },
        GradientMask::Radial {
            center: SourcePoint([point[0] - 0.00005, point[1]]),
            radii: [0.0001, 0.0002],
            angle: 0.71345,
            feather: 1.,
        },
    ] {
        let snapshot = PhotoGradeSnapshot::new(
            PhotoSettings::default(),
            vec![adjustment(
                mask,
                PhotoSettings {
                    exposure: 16.,
                    ..Default::default()
                },
            )],
        )
        .unwrap();
        let grade = GpuPhotoGrade::new(
            &device,
            calibration(),
            RollSettings::new_grade(),
            snapshot.clone(),
            matrix(),
            area,
        )
        .unwrap();
        let actual = read(
            &device,
            &queue,
            &kernel,
            &source,
            pixels.len(),
            &grade,
            None,
        );
        assert_reference(
            &actual,
            &pixels,
            SourceRaster::new(area, size).unwrap(),
            &snapshot,
            RollSettings::new_grade(),
            matrix(),
            None,
        );
    }
}
#[test]
fn proxy_and_padded_tiles_agree_with_same_full_source_pixels() {
    let (device, queue, kernel) = context();
    let area = SourceRegion::from([120, 80, 27, 24]);
    let snapshot = PhotoGradeSnapshot::new(
        PhotoSettings {
            exposure: 0.7,
            contrast: 1.13,
            ..Default::default()
        },
        vec![
            adjustment(
                GradientMask::Linear {
                    start: SourcePoint([120.5, 90.5]),
                    end: SourcePoint([140.5, 100.5]),
                },
                PhotoSettings {
                    exposure: 1.2,
                    wb: Balance {
                        warmth: 0.31,
                        tint: -0.67,
                    },
                    contrast: 0.41,
                    saturation: 1.73,
                    ..Default::default()
                },
            ),
            adjustment(
                GradientMask::Radial {
                    center: SourcePoint([135.5, 90.5]),
                    radii: [16., 12.],
                    angle: 0.713,
                    feather: 0.73,
                },
                PhotoSettings {
                    contrast: 1.41,
                    saturation: 0.31,
                    ..Default::default()
                },
            ),
        ],
    )
    .unwrap();
    let grade = GpuPhotoGrade::new(
        &device,
        calibration(),
        RollSettings::new_grade(),
        snapshot.clone(),
        matrix(),
        area,
    )
    .unwrap();
    let (full, full_pixels) = source(&device, area, [27, 24], matrix(), None);
    let full_output = read(
        &device,
        &queue,
        &kernel,
        &full,
        full_pixels.len(),
        &grade,
        None,
    );
    for (region, size) in [
        (area, [7, 5]),
        (SourceRegion::from([120, 80, 10, 24]), [10, 24]),
        (SourceRegion::from([130, 80, 10, 24]), [10, 24]),
        (SourceRegion::from([140, 80, 10, 24]), [10, 24]),
    ] {
        let raster = SourceRaster::new(region, size).unwrap();
        let (source, pixels) = source(&device, region, size, matrix(), None);
        let output = read(
            &device,
            &queue,
            &kernel,
            &source,
            pixels.len(),
            &grade,
            None,
        );
        for (i, pixel) in output.iter().enumerate() {
            let xy = raster
                .source_pixel([i as u32 % size[0], i as u32 / size[0]])
                .unwrap();
            if xy[0] < area.x + area.width {
                let index = ((xy[1] - area.y) * area.width + xy[0] - area.x) as usize;
                assert_eq!(
                    pixel.map(f32::to_bits),
                    full_output[index].map(f32::to_bits)
                );
            }
        }
    }
}
#[test]
fn full_and_fractional_weights_cover_extreme_ev_neutral_wb_and_zero_y() {
    let (device, queue, kernel) = context();
    let area = SourceRegion::from([0, 0, 8, 8]);
    let mask = GradientMask::Linear {
        start: SourcePoint([0., 0.]),
        end: SourcePoint([1., 0.]),
    };
    let roll = RollSettings {
        gamma: 0.65,
        ..RollSettings::new_grade()
    };
    let floor_rgb = Pipeline::new(
        calibration(),
        roll,
        PhotoSettings::default(),
        DMat3::IDENTITY,
    )
    .unwrap()
    .apply(DVec3::ZERO)
    .unwrap()
    .rgb;
    let weights = AP1_TO_XYZ.row(1);
    let cancel = (-weights.x * floor_rgb.x - weights.y * floor_rgb.y
        + 0.25 * weights.z * floor_rgb.z)
        / (weights.y * floor_rgb.z);
    let camera_matrix = DMat3::from_cols(DVec3::X, DVec3::Y, DVec3::new(0., cancel, -0.25));
    let (source, pixels) = source(&device, area, [8, 8], camera_matrix, None);
    for exposure in [-16., 0., 16.] {
        for contrast in [0.4, 1., 1.6] {
            let photo = PhotoSettings {
                exposure,
                contrast,
                pivot: 0.03,
                contrast_colour_mix: 1.,
                saturation: 1.73,
                saturation_density: 0.43,
                ..Default::default()
            };
            let snapshot =
                PhotoGradeSnapshot::new(PhotoSettings::default(), vec![adjustment(mask, photo)])
                    .unwrap();
            let prepared = GpuPhotoGrade::new(
                &device,
                calibration(),
                roll,
                snapshot.clone(),
                camera_matrix,
                area,
            )
            .unwrap();
            let actual = read(
                &device,
                &queue,
                &kernel,
                &source,
                pixels.len(),
                &prepared,
                None,
            );
            if exposure == 16. && contrast == 0.4 {
                let legacy =
                    ProcessingParameters::positive(calibration(), roll, photo, camera_matrix)
                        .unwrap();
                let old = read(
                    &device,
                    &queue,
                    &kernel,
                    &source,
                    pixels.len(),
                    &prepared,
                    Some(legacy),
                );
                let expected = Pipeline::new(calibration(), roll, photo, camera_matrix)
                    .unwrap()
                    .apply(DVec3::ZERO)
                    .unwrap();
                for c in 0..3 {
                    assert!(
                        (old[23][c] as f64 - expected.rgb[c]).abs()
                            <= 2e-5 + 2e-4 * expected.rgb[c].abs(),
                        "shared global-only zero-Y precision: GPU={:?} reference={:?}",
                        old[23],
                        expected.rgb
                    );
                }
            }
            assert_reference(
                &actual,
                &pixels,
                SourceRaster::new(area, [8, 8]).unwrap(),
                &snapshot,
                roll,
                camera_matrix,
                None,
            );
        }
    }
}

#[test]
fn attenuation_followed_by_amplification_rejects_observable_underflow_before_dispatch() {
    let (device, queue, kernel) = context();
    let area = SourceRegion::from([0, 0, 1, 1]);
    let scan = DVec3::new(12452., 9175., 7864.) / 65535.;
    let native = Pipeline::new(
        calibration(),
        RollSettings::new_grade(),
        PhotoSettings::default(),
        DMat3::IDENTITY,
    )
    .unwrap()
    .apply(scan)
    .unwrap()
    .rgb;
    let camera_matrix = DMat3::from_diagonal(DVec3::new(-5e-5 / native.x, 1., 1.));
    let mask = GradientMask::Linear {
        start: SourcePoint([-2., 0.]),
        end: SourcePoint([-1., 0.]),
    };
    let locals = [-16., 16.]
        .into_iter()
        .flat_map(|exposure| {
            std::iter::repeat_n(
                adjustment(
                    mask,
                    PhotoSettings {
                        exposure,
                        ..Default::default()
                    },
                ),
                7,
            )
        })
        .collect::<Vec<_>>();
    let snapshot = PhotoGradeSnapshot::new(PhotoSettings::default(), locals.clone()).unwrap();
    let result = GpuPhotoGrade::new(
        &device,
        calibration(),
        RollSettings::new_grade(),
        snapshot,
        camera_matrix,
        area,
    );
    assert!(
        matches!(result, Err(antitone::Error::Gpu(message)) if message.contains("normal GPU precision domain"))
    );
    let expected = Pipeline::new(
        calibration(),
        RollSettings::new_grade(),
        PhotoSettings::default(),
        camera_matrix,
    )
    .unwrap()
    .apply(scan)
    .unwrap()
    .rgb;
    assert!(expected.x.abs() > 2e-5);
    assert!(((expected.x * (-112_f64).exp2()) as f32).is_subnormal());
    // The same count with smaller amounts remains supported and restores the input.
    let locals = locals
        .into_iter()
        .map(|a| LocalAdjustment {
            strength: 0.25,
            ..a
        })
        .collect();
    let snapshot = PhotoGradeSnapshot::new(PhotoSettings::default(), locals).unwrap();
    let grade = GpuPhotoGrade::new(
        &device,
        calibration(),
        RollSettings::new_grade(),
        snapshot,
        camera_matrix,
        area,
    )
    .unwrap();
    let mut source =
        ProcessingSource::new(&device, &[[12452, 9175, 7864, 0]], camera_matrix).unwrap();
    source
        .set_geometry(SourceGeometry::new([0, 0, 1, 1], [1, 1]).unwrap())
        .unwrap();
    let actual = read(&device, &queue, &kernel, &source, 1, &grade, None)[0];
    for c in 0..3 {
        assert!((actual[c] as f64 - expected[c]).abs() <= 2e-5 + 2e-4 * expected[c].abs());
    }
}

#[test]
fn unsupported_ordered_ranges_mask_constants_and_camera_basis_reject_before_dispatch() {
    let (device, _queue, kernel) = context();
    let area = SourceRegion::from([0, 0, 8, 8]);
    let active = adjustment(
        GradientMask::Linear {
            start: SourcePoint([0., 0.]),
            end: SourcePoint([8., 0.]),
        },
        PhotoSettings {
            exposure: 16.,
            contrast: 1.6,
            ..Default::default()
        },
    );
    let stack = PhotoGradeSnapshot::new(PhotoSettings::default(), vec![active; 16]).unwrap();
    assert!(
        GpuPhotoGrade::new(
            &device,
            calibration(),
            RollSettings::new_grade(),
            stack,
            matrix(),
            area
        )
        .is_err()
    );
    let tiny = adjustment(
        GradientMask::Radial {
            center: SourcePoint([0., 0.]),
            radii: [f64::MIN_POSITIVE, 1.],
            angle: 0.,
            feather: 0.5,
        },
        PhotoSettings {
            exposure: 1.,
            ..Default::default()
        },
    );
    let snapshot = PhotoGradeSnapshot::new(PhotoSettings::default(), vec![tiny]).unwrap();
    assert!(
        GpuPhotoGrade::new(
            &device,
            calibration(),
            RollSettings::new_grade(),
            snapshot,
            matrix(),
            area
        )
        .is_err()
    );
    let grade = GpuPhotoGrade::new(
        &device,
        calibration(),
        RollSettings::new_grade(),
        PhotoGradeSnapshot::new(PhotoSettings::default(), vec![active]).unwrap(),
        matrix(),
        area,
    )
    .unwrap();
    let (source, pixels) = source(&device, area, [8, 8], DMat3::IDENTITY, None);
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: pixels.len() as u64 * 16,
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    assert!(
        kernel
            .encode_grade(&device, &mut encoder, &source, &output, &grade)
            .is_err()
    );
}
