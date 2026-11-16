//! Photographic model comparisons at the codec-independent original-processing boundary.

use super::{OutputBlock, SampleEncoding, TILE, execute, test_support::OriginalControls};
use crate::{
    adjustment_layers::{
        AdjustmentLayer, AdjustmentLayerCoverage, AdjustmentLayerPipeline, AdjustmentLayerSettings,
        AdjustmentLayerSnapshot,
    },
    frame_raster::FrameRaster,
    geometry::{Composition, Flips, PhotoOrientation, QuarterTurns, RollOrientation, SourcePoint},
    gpu::{flat::GpuFlatField, submission::SubmissionGate},
    processing::{Calibration, RollSettings},
    renderer::GpuRenderer,
    rendering::OutputReference,
    scan::Scan,
    working_precision,
};
use glam::DVec3;
use std::sync::Arc;

fn source() -> (Scan, Calibration, AdjustmentLayerSnapshot) {
    let scan = Scan::open(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng"),
        None,
    )
    .unwrap();
    // The authored fixture declares these uniform Dmin/Dmax cores. Calibrate the
    // developed camera samples, not nominal sensor ratios: a tiny base mismatch
    // can disappear in f32 and is amplified by the film response and contrast.
    let calibration = Calibration {
        dmin: scan.pixel(24, 24).unwrap(),
        dmax: scan.pixel(88, 24).unwrap(),
    };
    calibration.validate().unwrap();
    let grade =
        AdjustmentLayerSnapshot::new(vec![AdjustmentLayer::global(AdjustmentLayerSettings {
            contrast: 0.8,
            saturation: 1.3,
            ..Default::default()
        })])
        .unwrap();
    (scan, calibration, grade)
}

fn local_grade(base: &AdjustmentLayerSnapshot) -> AdjustmentLayerSnapshot {
    AdjustmentLayerSnapshot::new(vec![
        AdjustmentLayer::global(base.layers()[0].photo),
        AdjustmentLayer {
            mask: AdjustmentLayerCoverage::Linear {
                start: SourcePoint([32., 24.]),
                end: SourcePoint([224., 168.]),
            },
            photo: AdjustmentLayerSettings {
                exposure: 0.5,
                ..Default::default()
            },
            enabled: true,
            invert: false,
            strength: 0.7,
        },
        AdjustmentLayer {
            mask: AdjustmentLayerCoverage::Radial {
                center: SourcePoint([128., 96.]),
                radii: [96., 64.],
                angle: 0.3,
                feather: 0.65,
            },
            photo: AdjustmentLayerSettings {
                contrast: 1.02,
                saturation: 1.1,
                ..Default::default()
            },
            enabled: true,
            invert: true,
            strength: 0.8,
        },
    ])
    .unwrap()
}

fn correction(scan: &mut Scan, device: &wgpu::Device, enabled: bool) -> Option<Arc<GpuFlatField>> {
    let field = enabled.then(|| {
        Arc::new(
            crate::flat_field::FlatField::from_nodes(
                [scan.area.x, scan.area.y, scan.area.width, scan.area.height],
                [2, 2],
                vec![
                    DVec3::new(0.9, 1.1, 1.),
                    DVec3::new(1.2, 0.8, 1.1),
                    DVec3::new(0.95, 1.2, 0.85),
                    DVec3::new(1.1, 0.95, 1.2),
                ],
            )
            .unwrap(),
        )
    });
    scan.set_flat_field(field.clone());
    field.map(|field| Arc::new(GpuFlatField::new(device, field).unwrap()))
}

fn oriented_crop(scan: &Scan, angle: f64) -> Composition {
    Composition::with_crop(
        scan.area,
        RollOrientation {
            turns: QuarterTurns::One,
            horizontal: true,
            ..Default::default()
        },
        PhotoOrientation {
            turns: QuarterTurns::Two,
            flips: Flips {
                horizontal: false,
                vertical: true,
            },
        },
        Some(crate::crop::Crop {
            center: [128., 96.],
            size: [128, 96],
            angle,
        }),
    )
    .unwrap()
}

fn capture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &GpuRenderer,
    controls: OriginalControls<'_>,
) -> Vec<[f32; 4]> {
    let size = controls.composition.size();
    let request = controls.lower(device, renderer).unwrap();
    let mut pixels = vec![[f32::NAN; 4]; (size[0] * size[1]) as usize];
    execute(
        renderer,
        device,
        (queue, &SubmissionGate::default()),
        request,
        |render, complete| {
            for y in (0..size[1]).step_by(TILE[1] as usize) {
                for x in (0..size[0]).step_by(TILE[0] as usize) {
                    let extent = [TILE[0].min(size[0] - x), TILE[1].min(size[1] - y)];
                    let OutputBlock::Linear(block) = render([x, y], extent)? else {
                        panic!("expected linear processing readback")
                    };
                    for row in 0..extent[1] as usize {
                        let destination = (y as usize + row) * size[0] as usize + x as usize;
                        let source = row * extent[0] as usize;
                        pixels[destination..destination + extent[0] as usize]
                            .copy_from_slice(&block[source..source + extent[0] as usize]);
                    }
                }
            }
            complete()
        },
        || Ok(()),
    )
    .unwrap();
    assert!(pixels.iter().flatten().all(|channel| channel.is_finite()));
    pixels
}

fn controls(
    scan: &Scan,
    calibration: Calibration,
    grade: AdjustmentLayerSnapshot,
    field: Option<Arc<GpuFlatField>>,
    composition: Composition,
    output: OutputReference,
) -> OriginalControls<'_> {
    OriginalControls {
        scan,
        field,
        input: calibration.into(),
        roll: RollSettings::default(),
        grade,
        repairs: &[],
        composition,
        stages: Default::default(),
        output,
        luts: Default::default(),
        encoding: SampleEncoding::LinearFloat,
    }
}

fn compare_scene(
    scan: &Scan,
    calibration: Calibration,
    grade: AdjustmentLayerSnapshot,
    composition: Composition,
    pixels: &[[f32; 4]],
) {
    let scalar = AdjustmentLayerPipeline::new(
        calibration,
        RollSettings::default(),
        grade,
        scan.camera_to_ap1,
    )
    .unwrap();
    let raster = FrameRaster::full(composition).unwrap();
    let [width, height] = composition.size();
    assert_eq!(pixels.len(), (width * height) as usize);
    let mut minimum = DVec3::splat(f64::INFINITY);
    let mut maximum = DVec3::splat(f64::NEG_INFINITY);
    for y in 0..height {
        for x in 0..width {
            let expected = crate::sampling::weighted_mean_iter(
                raster
                    .footprint([x, y])
                    .unwrap()
                    .pixels(scan.area)
                    .unwrap()
                    .map(|([sx, sy], weight)| {
                        scalar
                            .apply(scan.pixel(sx, sy)?, [sx, sy])
                            .map(|p| (p.rgb, weight))
                    }),
            )
            .unwrap();
            minimum = minimum.min(expected);
            maximum = maximum.max(expected);
            let actual = pixels[(y * width + x) as usize];
            assert!(
                actual[3] >= 0.,
                "invalid contributing support at {:?}",
                [x, y]
            );
            for c in 0..3 {
                assert!(
                    (f64::from(actual[c]) - expected[c]).abs()
                        <= working_precision::allowance(expected[c]),
                    "output pixel {:?}, channel {c}: {actual:?} vs {expected:?}",
                    [x, y]
                );
            }
        }
    }
    assert!((maximum - minimum).min_element() > 0.01);
}

#[test]
fn scene_processing_matches_scalar_model_before_storage_on_exact_and_rotated_patches() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let (scan, calibration, grade) = source();
    let exact = Composition::new(scan.area, Default::default(), Default::default()).unwrap();
    let rotated = Composition::with_crop(
        scan.area,
        Default::default(),
        Default::default(),
        Some(crate::crop::Crop {
            center: [128., 96.],
            size: [scan.area.width - 64, scan.area.height - 64],
            angle: 0.13,
        }),
    )
    .unwrap();
    for composition in [exact, rotated] {
        let pixels = capture(
            &device,
            &queue,
            &renderer,
            controls(
                &scan,
                calibration,
                grade.clone(),
                None,
                composition,
                OutputReference::SceneReferred,
            ),
        );
        compare_scene(&scan, calibration, grade.clone(), composition, &pixels);
    }
}

#[test]
fn scene_locals_match_scalar_model_through_correction_permutations_and_rotated_coverage() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let (mut scan, calibration, base) = source();
    let grade = local_grade(&base);
    for corrected in [false, true] {
        let field = correction(&mut scan, &device, corrected);
        for angle in [0., 0.13] {
            let composition = oriented_crop(&scan, angle);
            let pixels = capture(
                &device,
                &queue,
                &renderer,
                controls(
                    &scan,
                    calibration,
                    grade.clone(),
                    field.clone(),
                    composition,
                    OutputReference::SceneReferred,
                ),
            );
            compare_scene(&scan, calibration, grade.clone(), composition, &pixels);
        }
    }
}

#[cfg(feature = "ocio-reference")]
fn compare_display(
    scan: &Scan,
    calibration: Calibration,
    grade: AdjustmentLayerSnapshot,
    composition: Composition,
    target: crate::rendering::OutputTarget,
    pixels: &[[f32; 4]],
    allowed_codes: u16,
) {
    let scalar = AdjustmentLayerPipeline::new(
        calibration,
        RollSettings::default(),
        grade,
        scan.camera_to_ap1,
    )
    .unwrap();
    let reference = crate::rendering::CpuRenderer::new(target).unwrap();
    let raster = FrameRaster::full(composition).unwrap();
    let [width, height] = composition.size();
    for y in 0..height {
        for x in 0..width {
            let expected = crate::sampling::weighted_mean_iter(
                raster
                    .footprint([x, y])
                    .unwrap()
                    .pixels(scan.area)
                    .unwrap()
                    .map(|([sx, sy], weight)| {
                        let working = scalar.apply(scan.pixel(sx, sy)?, [sx, sy])?.rgb;
                        reference
                            .render(&[working])
                            .map(|pixels| (pixels[0].as_dvec3(), weight))
                    }),
            )
            .unwrap();
            let actual = pixels[(y * width + x) as usize];
            let encoded = target
                .encode_u16(&[
                    glam::Vec3::new(actual[0], actual[1], actual[2]),
                    expected.as_vec3(),
                ])
                .unwrap();
            for c in 0..3 {
                assert!(
                    encoded[0][c].abs_diff(encoded[1][c]) <= allowed_codes,
                    "{target:?} pixel {:?}, channel {c}: {encoded:?}",
                    [x, y]
                );
            }
        }
    }
}

#[cfg(feature = "ocio-reference")]
#[test]
fn display_processing_matches_ocio_before_storage_within_one_rgb16_code() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let (scan, calibration, grade) = source();
    let composition = Composition::new(scan.area, Default::default(), Default::default()).unwrap();
    for target in [
        crate::rendering::OutputTarget::Srgb,
        crate::rendering::OutputTarget::DisplayP3,
    ] {
        let pixels = capture(
            &device,
            &queue,
            &renderer,
            controls(
                &scan,
                calibration,
                grade.clone(),
                None,
                composition,
                OutputReference::DisplayReferred(target),
            ),
        );
        compare_display(
            &scan,
            calibration,
            grade.clone(),
            composition,
            target,
            &pixels,
            1,
        );
    }
}

#[cfg(feature = "ocio-reference")]
#[test]
fn display_locals_match_ocio_through_correction_permutations_and_rotated_coverage() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let (mut scan, calibration, base) = source();
    let grade = local_grade(&base);
    for corrected in [false, true] {
        let field = correction(&mut scan, &device, corrected);
        for angle in [0., 0.13] {
            let composition = oriented_crop(&scan, angle);
            for target in [
                crate::rendering::OutputTarget::Srgb,
                crate::rendering::OutputTarget::DisplayP3,
            ] {
                let pixels = capture(
                    &device,
                    &queue,
                    &renderer,
                    controls(
                        &scan,
                        calibration,
                        grade.clone(),
                        field.clone(),
                        composition,
                        OutputReference::DisplayReferred(target),
                    ),
                );
                // Composed FP32/OCIO differences allow four RGB16 codes here;
                // exact file storage is checked with supplied samples separately.
                compare_display(
                    &scan,
                    calibration,
                    grade.clone(),
                    composition,
                    target,
                    &pixels,
                    4,
                );
            }
        }
    }
}
