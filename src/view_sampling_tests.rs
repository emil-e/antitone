//! CPU sampling oracles only: approximation errors are not GPU agreement tolerances.
use crate::{
    color::{Balance, wb_matrix},
    crop::Crop,
    flat_field::FlatField,
    frame_raster::FrameRaster,
    geometry::{
        Composition, Flips, FramePoint, PhotoOrientation, QuarterTurns, RollOrientation,
        SourcePoint, SourceRegion,
    },
    local_adjustments::{
        GradientMask, LocalAdjustment, LocalGrade, PhotoGradeSnapshot, SpatialPipeline,
    },
    photographic::Grade,
    processing::{
        Calibration, Inversion, InvertedPixel, PhotoSettings, RollSettings, exposure_multiplier,
    },
    rendering::{CpuRenderer, OutputTarget},
};
use glam::{DMat3, DVec3};

struct Source {
    area: SourceRegion,
    camera: Vec<DVec3>,
    calibration: Calibration,
    matrix: DMat3,
}

impl Source {
    fn index(&self, [x, y]: [u32; 2]) -> usize {
        assert!(x >= self.area.x && x < self.area.x + self.area.width);
        assert!(y >= self.area.y && y < self.area.y + self.area.height);
        ((y - self.area.y) * self.area.width + x - self.area.x) as usize
    }

    fn pixels(&self) -> impl Iterator<Item = [u32; 2]> + '_ {
        (self.area.y..self.area.y + self.area.height)
            .flat_map(move |y| (self.area.x..self.area.x + self.area.width).map(move |x| [x, y]))
    }

    fn corrected(mut self) -> Self {
        let a = self.area;
        let field = FlatField::from_nodes(
            [a.x, a.y, a.width, a.height],
            [2, 2],
            vec![
                DVec3::new(1.1, 0.9, 1.05),
                DVec3::new(0.95, 1.2, 1.1),
                DVec3::new(1.05, 1.1, 0.94),
                DVec3::new(1.2, 0.98, 1.05),
            ],
        )
        .unwrap();
        self.calibration = field.processing_calibration(self.calibration).unwrap();
        let points: Vec<_> = self.pixels().collect();
        for (camera, point) in self.camera.iter_mut().zip(points) {
            *camera *= field.gain(point.map(|v| f64::from(v) + 0.5)).unwrap();
        }
        self
    }
}

fn synthetic(kind: &str) -> Source {
    let area = SourceRegion {
        x: 10,
        y: 20,
        width: 64,
        height: 48,
    };
    let calibration = Calibration {
        dmin: DVec3::new(0.19, 0.14, 0.12),
        dmax: DVec3::new(0.008, 0.004, 0.002),
        input_floor: 1. / 65535.,
    };
    let mut camera = Vec::new();
    for y in 0..area.height {
        for x in 0..area.width {
            let dense = DVec3::new(0.005, 0.004, 0.003);
            let thin = DVec3::new(0.14, 0.09, 0.08);
            let value = match kind {
                "smooth" => {
                    dense.lerp(thin, f64::from(x + y) / f64::from(area.width + area.height))
                }
                "checker" => {
                    if (x + y) % 2 == 0 {
                        dense
                    } else {
                        thin
                    }
                }
                "edge" => {
                    if 3 * x + 2 * y < 3 * area.width / 2 + area.height {
                        dense
                    } else {
                        thin
                    }
                }
                "signed-hdr-floor" => match (x + 3 * y) % 4 {
                    0 => DVec3::new(0., 0.002, 0.003),
                    1 => calibration.dmin * DVec3::new(1.03, 1.06, 1.02),
                    2 => DVec3::new(0.002, 0.01, 0.05),
                    _ => thin,
                },
                _ => panic!("unknown fixture"),
            };
            camera.push((value * 65535.).round() / 65535.);
        }
    }
    Source {
        area,
        camera,
        calibration,
        matrix: DMat3::from_cols(
            DVec3::new(0.6062, 0.0161, -0.0031),
            DVec3::new(0.3802, 1.1927, -0.1490),
            DVec3::new(0.0136, -0.2088, 1.1521),
        ),
    }
}

fn raster(area: SourceRegion, rotated: bool, output: [u32; 2]) -> FrameRaster {
    let composition = Composition::with_crop(
        area,
        RollOrientation::default(),
        PhotoOrientation {
            turns: if rotated {
                QuarterTurns::One
            } else {
                QuarterTurns::Zero
            },
            flips: Flips {
                horizontal: rotated,
                vertical: false,
            },
        },
        Some(Crop {
            center: [
                f64::from(area.x) + f64::from(area.width) / 2.,
                f64::from(area.y) + f64::from(area.height) / 2.,
            ],
            size: [area.width / 2, area.height / 2],
            angle: if rotated { 0.17 } else { 0. },
        }),
    )
    .unwrap();
    FrameRaster::new(
        composition,
        FramePoint([0., 0.]),
        FramePoint(composition.size().map(f64::from)),
        output,
    )
    .unwrap()
}

fn snapshot(area: SourceRegion, case: &str, edit: usize) -> PhotoGradeSnapshot {
    let global = if case == "identity" {
        PhotoSettings::default()
    } else {
        PhotoSettings {
            exposure: 0.25 + edit as f64 * 0.125,
            wb: Balance {
                warmth: 0.005 + edit as f64 * 0.001,
                tint: -0.003,
            },
            contrast: 1.01,
            contrast_colour_mix: 0.25,
            saturation: 1.02,
            saturation_density: 0.4,
            ..Default::default()
        }
    };
    let x = f64::from(area.x) + f64::from(area.width) / 2.;
    let y = f64::from(area.y) + f64::from(area.height) / 2.;
    let locals = if case == "locals" {
        vec![
            LocalAdjustment {
                mask: GradientMask::Linear {
                    start: SourcePoint([x - 2., y]),
                    end: SourcePoint([x + 2., y]),
                },
                photo: PhotoSettings {
                    exposure: 1.,
                    contrast: 1.01,
                    saturation: 1.02,
                    ..Default::default()
                },
                enabled: true,
                invert: false,
                strength: 1.,
            },
            LocalAdjustment {
                mask: GradientMask::Radial {
                    center: SourcePoint([x, y]),
                    radii: [8., 6.],
                    angle: 0.3,
                    feather: 0.5,
                },
                photo: PhotoSettings {
                    exposure: -0.5,
                    wb: Balance {
                        warmth: -0.005,
                        tint: 0.003,
                    },
                    ..Default::default()
                },
                enabled: true,
                invert: false,
                strength: 0.7,
            },
        ]
    } else {
        vec![]
    };
    PhotoGradeSnapshot::new(global, locals).unwrap()
}

struct Stages {
    inversion: Inversion,
    matrix: DMat3,
    global: Grade,
    locals: LocalGrade,
    full: SpatialPipeline,
}

impl Stages {
    fn new(source: &Source, grade: PhotoGradeSnapshot) -> Self {
        let roll = RollSettings {
            exposure: -0.125,
            wb: Balance {
                warmth: 0.002,
                tint: -0.001,
            },
        };
        let photo = grade.global();
        let gain = exposure_multiplier(roll.exposure).unwrap()
            * exposure_multiplier(photo.exposure).unwrap();
        Self {
            inversion: Inversion::new(source.calibration).unwrap(),
            // Match the scalar Pipeline's exposure and white-balance ordering.
            matrix: wb_matrix(photo.wb).unwrap()
                * wb_matrix(roll.wb).unwrap()
                * source.matrix
                * gain,
            global: Grade::new(photo).unwrap(),
            locals: LocalGrade::new(grade.locals().to_vec()).unwrap(),
            full: SpatialPipeline::new(source.calibration, roll, grade, source.matrix).unwrap(),
        }
    }

    fn before_creative(&self, camera: DVec3) -> InvertedPixel {
        let mut pixel = self.inversion.apply(camera).unwrap();
        pixel.rgb = self.matrix * pixel.rgb;
        pixel
    }

    fn after_creative(&self, mut pixel: InvertedPixel, point: SourcePoint) -> InvertedPixel {
        pixel.rgb = self.global.apply(pixel.rgb).unwrap();
        self.locals.apply(pixel, point).unwrap()
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Support {
    // Test-only quadrature. Every point is an actual original pixel; duplicate selections retain their weight.
    samples: Vec<([u32; 2], f64)>,
}

fn support(frame: FrameRaster, pixel: [u32; 2], side: u32) -> Support {
    let mut samples = Vec::new();
    for y in 0..side {
        for x in 0..side {
            let point = frame.source_point([
                f64::from(pixel[0]) + (f64::from(x) + 0.5) / f64::from(side),
                f64::from(pixel[1]) + (f64::from(y) + 0.5) / f64::from(side),
            ]);
            samples.push((
                point.0.map(|v| v.floor() as u32),
                1. / f64::from(side * side),
            ));
        }
    }
    Support { samples }
}

fn mean(values: impl Iterator<Item = (DVec3, f64)>) -> DVec3 {
    crate::sampling::weighted_mean_iter(values.map(Ok)).unwrap()
}

fn flags(values: impl Iterator<Item = [bool; 3]>) -> [bool; 3] {
    values.fold([false; 3], |a, b| std::array::from_fn(|i| a[i] || b[i]))
}

struct Comparison {
    exact: Vec<DVec3>,
    candidates: Vec<(&'static str, Vec<DVec3>)>,
    floor_loss_pixels: usize,
    exact_contributions: usize,
    exact_unique_originals: usize,
}

fn compare(
    source: &Source,
    frame: FrameRaster,
    grade: PhotoGradeSnapshot,
    target: OutputTarget,
) -> Comparison {
    let stages = Stages::new(source, grade);
    let renderer = CpuRenderer::new(target).unwrap();
    let before: Vec<_> = source
        .camera
        .iter()
        .map(|&camera| stages.before_creative(camera))
        .collect();
    let full: Vec<_> = source
        .pixels()
        .map(|p| {
            stages
                .full
                .apply(source.camera[source.index(p)], p)
                .unwrap()
        })
        .collect();
    let rendered = renderer
        .render(&full.iter().map(|v| v.rgb).collect::<Vec<_>>())
        .unwrap();
    let mut exact = Vec::new();
    let mut candidates: Vec<_> = [
        "filtered-camera",
        "filtered-precreative-ap1",
        "filtered-graded-ap1",
        "support-1",
        "support-4",
        "support-16",
    ]
    .into_iter()
    .map(|name| (name, Vec::new()))
    .collect();
    let mut floor_loss_pixels = 0;
    let mut exact_contributions = 0;
    let mut exact_unique_originals = std::collections::BTreeSet::new();
    for y in 0..frame.output[1] {
        for x in 0..frame.output[0] {
            let coverage: Vec<_> = frame
                .footprint([x, y])
                .unwrap()
                .pixels(source.area)
                .unwrap()
                .collect();
            exact_contributions += coverage.len();
            exact_unique_originals.extend(coverage.iter().map(|&(p, _)| p));
            let reference = mean(
                coverage
                    .iter()
                    .map(|&(p, w)| (rendered[source.index(p)].as_dvec3(), w)),
            );
            exact.push(reference);
            let exact_flags = flags(coverage.iter().map(|&(p, _)| full[source.index(p)].floored));
            let camera_mean = mean(
                coverage
                    .iter()
                    .map(|&(p, w)| (source.camera[source.index(p)], w)),
            );
            // Filtering uses a continuous footprint centre for locals: intentionally approximate, not original provenance.
            let centre = frame.source_point([f64::from(x) + 0.5, f64::from(y) + 0.5]);
            let camera_filtered =
                stages.after_creative(stages.before_creative(camera_mean), centre);
            floor_loss_pixels += usize::from(camera_filtered.floored != exact_flags);
            let precreative_mean = mean(
                coverage
                    .iter()
                    .map(|&(p, w)| (before[source.index(p)].rgb, w)),
            );
            let ap1_filtered = stages.after_creative(
                InvertedPixel {
                    rgb: precreative_mean,
                    floored: exact_flags,
                },
                centre,
            );
            let graded_mean = mean(
                coverage
                    .iter()
                    .map(|&(p, w)| (full[source.index(p)].rgb, w)),
            );
            for (i, rgb) in [camera_filtered.rgb, ap1_filtered.rgb, graded_mean]
                .into_iter()
                .enumerate()
            {
                candidates[i]
                    .1
                    .push(renderer.render(&[rgb]).unwrap()[0].as_dvec3());
            }
            for (i, side) in [1, 2, 4].into_iter().enumerate() {
                let samples = support(frame, [x, y], side);
                candidates[3 + i].1.push(mean(
                    samples
                        .samples
                        .into_iter()
                        .map(|(p, w)| (rendered[source.index(p)].as_dvec3(), w)),
                ));
            }
        }
    }
    Comparison {
        exact,
        candidates,
        floor_loss_pixels,
        exact_contributions,
        exact_unique_originals: exact_unique_originals.len(),
    }
}

fn error(reference: &[DVec3], candidate: &[DVec3]) -> (f64, f64) {
    assert_eq!(reference.len(), candidate.len());
    let mut square = 0.;
    let mut max: f64 = 0.;
    for (a, b) in reference.iter().zip(candidate) {
        assert!(a.is_finite() && b.is_finite());
        let delta = (a - b).abs();
        square += delta.length_squared();
        max = max.max(delta.max_element());
    }
    ((square / (3 * reference.len()) as f64).sqrt(), max)
}

fn report(
    kind: &str,
    corrected: bool,
    rotated: bool,
    case: &str,
    target: OutputTarget,
    comparison: &Comparison,
) {
    for (name, output) in &comparison.candidates {
        let (rms, max) = error(&comparison.exact, output);
        let quadrature_samples = match *name {
            "support-1" => comparison.exact.len(),
            "support-4" => comparison.exact.len() * 4,
            "support-16" => comparison.exact.len() * 16,
            _ => 0,
        };
        println!(
            "view_sampling fixture={kind} corrected={corrected} rotated={rotated} grade={case} target={target:?} candidate={name} output_pixels={} rms_linear_target={rms:.9} max_linear_target={max:.9} camera_filter_floor_loss_pixels={} quadrature_samples={quadrature_samples} exact_contributions={} exact_unique_originals={} metric=approximation_error_not_implementation_tolerance",
            comparison.exact.len(),
            comparison.floor_loss_pixels,
            comparison.exact_contributions,
            comparison.exact_unique_originals
        );
    }
}

#[test]
fn split_scalar_stages_preserve_original_sample_processing() {
    let source = synthetic("signed-hdr-floor").corrected();
    let stages = Stages::new(&source, snapshot(source.area, "locals", 1));
    let mut saw_floor = false;
    let mut saw_signed = false;
    let mut saw_hdr = false;
    for p in source.pixels() {
        let camera = source.camera[source.index(p)];
        let expected = stages.full.apply(camera, p).unwrap();
        let before = stages.before_creative(camera);
        saw_signed |= before.rgb.min_element() < 0.;
        saw_hdr |= before.rgb.max_element() > 1.;
        let actual = stages.after_creative(before, SourcePoint(p.map(|v| f64::from(v) + 0.5)));
        assert_eq!(actual.floored, expected.floored);
        saw_floor |= actual.floored.contains(&true);
        for c in 0..3 {
            assert!(
                (actual.rgb[c] - expected.rgb[c]).abs() <= 2e-12 + 2e-11 * expected.rgb[c].abs()
            );
        }
    }
    assert!(saw_floor && saw_signed && saw_hdr);
}

#[test]
fn axis_aligned_complete_original_support_matches_exact_coverage() {
    let source = synthetic("checker");
    let frame = raster(source.area, false, [8, 6]);
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        let result = compare(&source, frame, snapshot(source.area, "locals", 0), target);
        let (rms, max) = error(&result.exact, &result.candidates[5].1);
        assert!(rms < 1e-14 && max < 1e-14);
        assert!(
            error(&result.exact, &result.candidates[3].1).1 > 1e-3,
            "one centre aliases the checker pattern"
        );
    }
}

#[test]
fn native_pixel_permutations_need_no_additional_support_for_grading() {
    let source = synthetic("edge").corrected();
    let composition = Composition::new(
        source.area,
        RollOrientation::default(),
        PhotoOrientation {
            turns: QuarterTurns::One,
            flips: Flips {
                horizontal: true,
                vertical: false,
            },
        },
    )
    .unwrap();
    let frame = FrameRaster::full(composition).unwrap();
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        let result = compare(&source, frame, snapshot(source.area, "locals", 1), target);
        for (_, output) in &result.candidates[3..] {
            let (rms, max) = error(&result.exact, output);
            assert!(rms < 1e-14 && max < 1e-14);
        }
    }
}

#[test]
fn filtered_representations_are_not_exact_and_can_lose_native_floor_evidence() {
    let source = synthetic("signed-hdr-floor").corrected();
    let frame = raster(source.area, true, [8, 6]);
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        let result = compare(&source, frame, snapshot(source.area, "locals", 0), target);
        assert!(result.floor_loss_pixels > 0);
        for (_, output) in &result.candidates[..3] {
            assert!(
                error(&result.exact, output).1 > 1e-4,
                "a mean does not preserve arbitrary future nonlinear grades/rendering"
            );
        }
    }
}

#[test]
fn view_support_is_grade_target_independent_but_navigation_changes_provenance() {
    let source = synthetic("edge");
    let frame = raster(source.area, true, [8, 6]);
    let retained: Vec<_> = (0..6)
        .flat_map(|y| (0..8).map(move |x| support(frame, [x, y], 2)))
        .collect();
    for edit in 0..3 {
        let stages = Stages::new(&source, snapshot(source.area, "locals", edit));
        let input: Vec<_> = retained
            .iter()
            .flat_map(|s| {
                s.samples.iter().map(|&(p, _)| {
                    stages
                        .full
                        .apply(source.camera[source.index(p)], p)
                        .unwrap()
                        .rgb
                })
            })
            .collect();
        for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
            let rendered = CpuRenderer::new(target).unwrap().render(&input).unwrap();
            assert_eq!(rendered.len(), 8 * 6 * 4);
        }
        let recomputed: Vec<_> = (0..6)
            .flat_map(|y| (0..8).map(move |x| support(frame, [x, y], 2)))
            .collect();
        assert_eq!(retained, recomputed);
    }
    let moved = FrameRaster::new(
        frame.composition,
        FramePoint([0.25, 0.5]),
        frame.max,
        frame.output,
    )
    .unwrap();
    let changed: Vec<_> = (0..6)
        .flat_map(|y| (0..8).map(move |x| support(moved, [x, y], 2)))
        .collect();
    assert_ne!(retained, changed);
    for s in &retained {
        assert!((s.samples.iter().map(|(_, w)| w).sum::<f64>() - 1.).abs() < 1e-14);
        for &(p, _) in &s.samples {
            source.index(p);
        }
    }
}

#[test]
fn retaining_acquired_exact_support_preserves_detail_across_grade_and_target_edits() {
    let source = synthetic("checker").corrected();
    let frame = raster(source.area, true, [8, 6]);
    let retained: Vec<_> = (0..6)
        .flat_map(|y| {
            (0..8).map(move |x| Support {
                samples: frame
                    .footprint([x, y])
                    .unwrap()
                    .pixels(source.area)
                    .unwrap()
                    .collect(),
            })
        })
        .collect();
    assert!(retained.iter().all(|s| s.samples.len() > 1));
    for edit in 0..3 {
        let grade = snapshot(source.area, "locals", edit);
        let stages = Stages::new(&source, grade.clone());
        let original: Vec<_> = source
            .pixels()
            .map(|p| {
                stages
                    .full
                    .apply(source.camera[source.index(p)], p)
                    .unwrap()
                    .rgb
            })
            .collect();
        for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
            let rendered = CpuRenderer::new(target).unwrap().render(&original).unwrap();
            let reused: Vec<_> = retained
                .iter()
                .map(|s| {
                    mean(
                        s.samples
                            .iter()
                            .map(|&(p, w)| (rendered[source.index(p)].as_dvec3(), w)),
                    )
                })
                .collect();
            let reference = compare(&source, frame, grade.clone(), target);
            assert_eq!(reused, reference.exact);
            assert!(
                error(&reused, &reference.candidates[3].1).1 > 1e-3,
                "same-sized one-point raster still demotes already acquired support"
            );
        }
    }
}

#[test]
#[ignore = "sampling approximation comparison; synthetic fixtures are not photographic acceptance"]
fn synthetic_view_sampling_quality() {
    for kind in ["smooth", "checker", "edge", "signed-hdr-floor"] {
        for corrected in [false, true] {
            let source = if corrected {
                synthetic(kind).corrected()
            } else {
                synthetic(kind)
            };
            for rotated in [false, true] {
                let output = if rotated { [6, 8] } else { [8, 6] };
                let frame = raster(source.area, rotated, output);
                for case in ["identity", "global", "locals"] {
                    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
                        let comparison =
                            compare(&source, frame, snapshot(source.area, case, 0), target);
                        report(kind, corrected, rotated, case, target, &comparison);
                    }
                }
            }
        }
    }
}

#[cfg(all(feature = "raw", feature = "gpu"))]
#[test]
#[ignore = "private read-only Sony patches; numerical approximation comparison, not photographic acceptance"]
fn raw_view_sampling_quality() {
    let (scan, calibration) = crate::raw_test_support::train_raw::load();
    for (name, offset) in [
        ("centre", [0i32, 0]),
        ("left", [-1800, 0]),
        ("upper", [0, -1400]),
    ] {
        let area = SourceRegion {
            x: (scan.area.x as i32 + scan.area.width as i32 / 2 + offset[0]) as u32,
            y: (scan.area.y as i32 + scan.area.height as i32 / 2 + offset[1]) as u32,
            width: 64,
            height: 48,
        };
        for corrected in [false, true] {
            let camera = (area.y..area.y + area.height)
                .flat_map(|y| {
                    (area.x..area.x + area.width)
                        .map(|x| scan.pixel(x, y).unwrap())
                        .collect::<Vec<_>>()
                })
                .collect();
            let source = Source {
                area,
                camera,
                calibration,
                matrix: scan.camera_to_ap1,
            };
            let source = if corrected {
                source.corrected()
            } else {
                source
            };
            for rotated in [false, true] {
                let output = if rotated { [6, 8] } else { [8, 6] };
                for case in ["identity", "global", "locals"] {
                    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
                        let comparison = compare(
                            &source,
                            raster(area, rotated, output),
                            snapshot(area, case, 0),
                            target,
                        );
                        report(name, corrected, rotated, case, target, &comparison);
                    }
                }
            }
        }
    }
}
