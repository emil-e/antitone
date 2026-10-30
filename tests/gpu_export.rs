#![cfg(all(
    feature = "raw",
    feature = "gpu",
    feature = "export",
    feature = "ocio-reference",
    target_os = "macos"
))]
use antitone::adjustment_layers::AdjustmentLayerSettings;
use antitone::{
    adjustment_layers::{AdjustmentLayerPipeline, AdjustmentLayerSnapshot},
    export::{self, ExrPrecision, Overwrite},
    geometry::{Composition, PhotoOrientation, RollOrientation},
    gpu::SubmissionGate,
    gpu_export::{self, Format, Request},
    processing::{Calibration, InputSettings, RollSettings},
    renderer::GpuRenderer,
    rendering::OutputTarget,
    scan::Scan,
};
use glam::DVec3;
use std::{path::Path, sync::Arc};
use tiff::decoder::{Decoder, DecodingResult};
#[path = "support/working_precision.rs"]
mod working_precision;

fn context() -> (wgpu::Device, wgpu::Queue, Arc<GpuRenderer>) {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let renderer = GpuRenderer::new(&device, &queue).await.unwrap();
        (device, queue, renderer)
    })
}

fn source() -> (
    Scan,
    Calibration,
    RollSettings,
    AdjustmentLayerSnapshot,
    Composition,
) {
    let scan = Scan::open(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng"),
        None,
    )
    .unwrap();
    let calibration = Calibration {
        dmin: DVec3::splat(3000. / 4031.),
        dmax: DVec3::splat(128. / 4031.),
        input_floor: scan.input_floor,
    };
    let roll = RollSettings::default();
    let grade = AdjustmentLayerSnapshot::new(
        std::iter::once(antitone::adjustment_layers::AdjustmentLayer::global(
            AdjustmentLayerSettings {
                contrast: 0.8,
                saturation: 1.3,
                ..Default::default()
            },
        ))
        .chain(Vec::new())
        .collect(),
    )
    .unwrap();
    let composition = Composition::new(
        scan.area,
        RollOrientation::default(),
        PhotoOrientation::default(),
    )
    .unwrap();
    (scan, calibration, roll, grade, composition)
}

fn tiff(path: &Path) -> Vec<u16> {
    let mut decoder = Decoder::new(std::fs::File::open(path).unwrap()).unwrap();
    let DecodingResult::U16(values) = decoder.read_image().unwrap() else {
        panic!("not RGB16")
    };
    values
}

// GPU/scalar correspondence allows rounding differences; encoder serialization is exact.
#[test]
fn gpu_tiff_matches_scalar_ocio_within_one_rgb16_code_on_authored_patches() {
    let (device, queue, renderer) = context();
    let (scan, calibration, roll, grade, composition) = source();
    let directory = tempfile::tempdir().unwrap();
    let pipeline = AdjustmentLayerPipeline::new(
        InputSettings::negative(calibration),
        roll,
        grade.clone(),
        scan.camera_to_ap1,
    )
    .unwrap();
    let mut differences = 0;
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        let gpu = directory.path().join(format!("gpu-{target:?}.tif"));
        let cpu = directory.path().join(format!("cpu-{target:?}.tif"));
        gpu_export::write(
            &renderer,
            &device,
            (&queue, &SubmissionGate::default()),
            Request {
                pipeline: Default::default(),
                scan: &scan,
                field: None,
                input: InputSettings::negative(calibration),
                roll,
                grade: grade.clone(),
                repairs: &[],
                composition,
                format: Format::Tiff(target),
                look: None,
            },
            &gpu,
            Overwrite::Refuse,
        )
        .unwrap();
        export::tiff_composed(&cpu, composition, target, Overwrite::Refuse, |x, y| {
            pipeline.apply(scan.pixel(x, y)?, [x, y]).map(|p| p.rgb)
        })
        .unwrap();
        let actual = tiff(&gpu);
        let expected = tiff(&cpu);
        assert_eq!(actual.len(), expected.len());
        let changed = actual.iter().zip(&expected).filter(|(a, b)| a != b).count();
        let maximum = actual
            .iter()
            .zip(&expected)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        println!(
            "{target:?} differing RGB16 channels={changed}/{} max_code_difference={maximum}",
            actual.len()
        );
        assert!(
            maximum <= 1,
            "{target:?} output diverges beyond one RGB16 code"
        );
        differences += changed;
    }
    println!("total differing TIFF channels={differences}");
}

#[test]
fn gpu_exr_uses_shared_working_precision_through_linear_ap0_on_authored_patches() {
    use exr::prelude::*;
    let (device, queue, renderer) = context();
    let (scan, calibration, roll, grade, composition) = source();
    let directory = tempfile::tempdir().unwrap();
    let pipeline = AdjustmentLayerPipeline::new(
        InputSettings::negative(calibration),
        roll,
        grade.clone(),
        scan.camera_to_ap1,
    )
    .unwrap();
    let mut differences = 0;
    for precision in [ExrPrecision::Half, ExrPrecision::Full] {
        let gpu = directory.path().join(format!("gpu-{precision:?}.exr"));
        let cpu = directory.path().join(format!("cpu-{precision:?}.exr"));
        gpu_export::write(
            &renderer,
            &device,
            (&queue, &SubmissionGate::default()),
            Request {
                pipeline: Default::default(),
                scan: &scan,
                field: None,
                input: InputSettings::negative(calibration),
                roll,
                grade: grade.clone(),
                repairs: &[],
                composition,
                format: Format::Exr(precision),
                look: None,
            },
            &gpu,
            Overwrite::Refuse,
        )
        .unwrap();
        export::exr_composed(&cpu, composition, precision, Overwrite::Refuse, |x, y| {
            pipeline.apply(scan.pixel(x, y)?, [x, y]).map(|p| p.rgb)
        })
        .unwrap();
        let rounded_path = directory.path().join(format!("rounded-{precision:?}.exr"));
        export::exr_composed(
            &rounded_path,
            composition,
            precision,
            Overwrite::Refuse,
            |x, y| {
                pipeline
                    .apply(scan.pixel(x, y)?, [x, y])
                    .map(|p| p.rgb.as_vec3().as_dvec3())
            },
        )
        .unwrap();
        let rounded = read_first_flat_layer_from_file(rounded_path).unwrap();
        let actual = read_first_flat_layer_from_file(gpu).unwrap();
        let expected = read_first_flat_layer_from_file(cpu).unwrap();
        let mut rounding_only_differences = 0;
        let mut gpu_vs_rounded_differences = 0;
        for ((a, b), r) in actual
            .layer_data
            .channel_data
            .list
            .iter()
            .zip(&expected.layer_data.channel_data.list)
            .zip(&rounded.layer_data.channel_data.list)
        {
            match (&a.sample_data, &b.sample_data, &r.sample_data) {
                (FlatSamples::F16(a), FlatSamples::F16(b), FlatSamples::F16(r)) => {
                    rounding_only_differences += r.iter().zip(b).filter(|(r, b)| r != b).count();
                    gpu_vs_rounded_differences += a.iter().zip(r).filter(|(a, r)| a != r).count();
                }
                (FlatSamples::F32(a), FlatSamples::F32(b), FlatSamples::F32(r)) => {
                    rounding_only_differences += r.iter().zip(b).filter(|(r, b)| r != b).count();
                    gpu_vs_rounded_differences += a.iter().zip(r).filter(|(a, r)| a != r).count();
                }
                _ => panic!("wrong EXR sample precision"),
            }
        }
        println!(
            "{precision:?} scalar-AP1-f32-rounding-only differing samples={rounding_only_differences}; GPU vs rounded-scalar differing samples={gpu_vs_rounded_differences}"
        );
        assert_eq!(actual.layer_data.size, expected.layer_data.size);
        let mut changed = 0;
        let mut maximum = 0_f64;
        for (a, b) in actual
            .layer_data
            .channel_data
            .list
            .iter()
            .zip(&expected.layer_data.channel_data.list)
        {
            assert_eq!(a.name, b.name);
            match (&a.sample_data, &b.sample_data) {
                (FlatSamples::F16(a), FlatSamples::F16(b)) => {
                    assert_eq!(a.len(), b.len());
                    changed += a.iter().zip(b).filter(|(a, b)| a != b).count();
                    for (actual, expected) in a.iter().zip(b) {
                        let expected = expected.to_f64();
                        let quantization = 0.0009765625 * expected.abs() + 5.960464477539063e-8;
                        assert!(
                            (actual.to_f64() - expected).abs()
                                <= working_precision::allowance(expected) + quantization
                        );
                    }
                    maximum = a
                        .iter()
                        .zip(b)
                        .map(|(a, b)| (a.to_f64() - b.to_f64()).abs())
                        .fold(maximum, f64::max);
                }
                (FlatSamples::F32(a), FlatSamples::F32(b)) => {
                    assert_eq!(a.len(), b.len());
                    changed += a.iter().zip(b).filter(|(a, b)| a != b).count();
                    for (actual, expected) in a.iter().zip(b) {
                        let expected = *expected as f64;
                        let quantization = f32::EPSILON as f64 * expected.abs();
                        assert!(
                            (*actual as f64 - expected).abs()
                                <= working_precision::allowance(expected) + quantization
                        );
                    }
                    maximum = a
                        .iter()
                        .zip(b)
                        .map(|(a, b)| (*a as f64 - *b as f64).abs())
                        .fold(maximum, f64::max);
                }
                _ => panic!("wrong EXR sample precision"),
            }
        }
        println!("{precision:?} differing AP0 samples={changed} max_absolute_difference={maximum}");
        differences += changed;
    }
    println!(
        "total differing EXR samples={differences}; supplied-sample serialization is tested independently"
    );
}

#[test]
fn corrected_ordered_locals_reopen_after_native_permutations_and_rotated_coverage() {
    use antitone::{
        adjustment_layers::{AdjustmentLayer, AdjustmentLayerCoverage},
        crop::Crop,
        flat_field::FlatField,
        frame_raster::FrameRaster,
        geometry::{Flips, QuarterTurns, SourcePoint},
        gpu::GpuFlatField,
    };
    use exr::prelude::*;
    let (device, queue, renderer) = context();
    let (mut scan, calibration, roll, base, _) = source();
    let directory = tempfile::tempdir().unwrap();
    let area = scan.area;
    let grade = AdjustmentLayerSnapshot::new(vec![
        antitone::adjustment_layers::AdjustmentLayer::global(base.layers()[0].photo),
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
    .unwrap();
    for corrected in [false, true] {
        let field = corrected.then(|| {
            Arc::new(
                FlatField::from_nodes(
                    [area.x, area.y, area.width, area.height],
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
        scan.set_flat_field(field.clone()).unwrap();
        let gpu_field = field.map(|f| Arc::new(GpuFlatField::new(&device, f).unwrap()));
        let scalar = AdjustmentLayerPipeline::new(
            scan.processing_calibration(calibration).unwrap(),
            roll,
            grade.clone(),
            scan.camera_to_ap1,
        )
        .unwrap();
        for angle in [0., 0.13] {
            let composition = Composition::with_crop(
                area,
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
                Some(Crop {
                    center: [128., 96.],
                    size: [128, 96],
                    angle,
                }),
            )
            .unwrap();
            let raster = FrameRaster::full(composition).unwrap();
            for format in [
                Format::Tiff(OutputTarget::Srgb),
                Format::Tiff(OutputTarget::DisplayP3),
                Format::Exr(ExrPrecision::Half),
                Format::Exr(ExrPrecision::Full),
            ] {
                let path = directory.path().join("gpu-output");
                gpu_export::write(
                    &renderer,
                    &device,
                    (&queue, &SubmissionGate::default()),
                    Request {
                        pipeline: Default::default(),
                        look: None,
                        scan: &scan,
                        field: gpu_field.clone(),
                        input: InputSettings::negative(calibration),
                        roll,
                        grade: grade.clone(),
                        repairs: &[],
                        composition,
                        format,
                    },
                    &path,
                    Overwrite::Confirmed,
                )
                .unwrap();
                match format {
                    Format::Tiff(target) => {
                        let expected = directory.path().join("reference.tif");
                        export::tiff_composed(
                            &expected,
                            composition,
                            target,
                            Overwrite::Confirmed,
                            |x, y| scalar.apply(scan.pixel(x, y)?, [x, y]).map(|p| p.rgb),
                        )
                        .unwrap();
                        let actual = tiff(&path);
                        let expected = tiff(&expected);
                        assert_eq!(actual.len(), expected.len());
                        let maximum = actual
                            .iter()
                            .zip(&expected)
                            .map(|(a, b)| a.abs_diff(*b))
                            .max()
                            .unwrap();
                        // This is a composition/model check, not bit-exact photographic arithmetic.
                        // Four RGB16 codes allow f32 processing/rendering rounding; encoder
                        // serialization has separate exact-sample coverage.
                        assert!(
                            maximum <= 4,
                            "corrected={corrected}, angle={angle}, target={target:?}, maximum RGB16 difference={maximum}"
                        );
                    }
                    Format::Exr(precision) => {
                        let image = read_first_flat_layer_from_file(&path).unwrap();
                        let [width, height] = composition.size();
                        assert_eq!(image.layer_data.size, Vec2(width as usize, height as usize));
                        for channel in &image.layer_data.channel_data.list {
                            let c = match channel.name.to_string().as_str() {
                                "R" => 0,
                                "G" => 1,
                                "B" => 2,
                                _ => panic!(),
                            };
                            for y in 0..height {
                                for x in 0..width {
                                    let mean = antitone::sampling::weighted_mean_iter(
                                        raster
                                            .footprint([x, y])
                                            .unwrap()
                                            .pixels(area)
                                            .unwrap()
                                            .map(|([sx, sy], weight)| {
                                                scalar
                                                    .apply(scan.pixel(sx, sy)?, [sx, sy])
                                                    .map(|p| (p.rgb, weight))
                                            }),
                                    )
                                    .unwrap();
                                    let expected = (antitone::color::ap1_to_ap0() * mean)[c];
                                    let index = (y * width + x) as usize;
                                    let (actual, quantization) = match &channel.sample_data {
                                        FlatSamples::F16(v) => (
                                            v[index].to_f64(),
                                            0.0009765625 * expected.abs() + 5.960464477539063e-8,
                                        ),
                                        FlatSamples::F32(v) => {
                                            (v[index] as f64, f32::EPSILON as f64 * expected.abs())
                                        }
                                        _ => panic!(),
                                    };
                                    assert!(
                                        (actual - expected).abs()
                                            <= working_precision::allowance(expected)
                                                + quantization,
                                        "corrected={corrected}, angle={angle}, precision={precision:?}, actual={actual}, expected={expected}"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
