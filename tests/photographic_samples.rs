//! Read-only photographic evidence through the production export engine and scalar/OCIO model.
#![cfg(all(
    feature = "raw",
    feature = "gpu",
    feature = "export",
    feature = "ocio-reference",
    target_os = "macos"
))]
use antitone as api;
use antitone::adjustment_layers::AdjustmentLayerSettings;
use antitone::{
    adjustment_layers::{
        AdjustmentLayer, AdjustmentLayerCoverage, AdjustmentLayerPipeline, AdjustmentLayerSnapshot,
    },
    crop::Crop,
    export::{self, Overwrite},
    flat_field::FlatField,
    geometry::{Composition, SourcePoint, SourceRegion},
    gpu::{GpuFlatField, SubmissionGate},
    gpu_export::{self, Format, Request},
    processing::{InputSettings, RollSettings},
    renderer::GpuRenderer,
    rendering::OutputTarget,
    scan::Scan,
};
use glam::DVec3;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};
use tiff::decoder::{Decoder, DecodingResult};
#[path = "support/train_raw.rs"]
mod train_raw;

fn read(path: &Path) -> ([u32; 2], Vec<u16>) {
    let mut decoder = Decoder::new(std::fs::File::open(path).unwrap()).unwrap();
    let (w, h) = decoder.dimensions().unwrap();
    let DecodingResult::U16(pixels) = decoder.read_image().unwrap() else {
        panic!("not RGB16")
    };
    ([w, h], pixels)
}

#[test]
#[ignore = "private read-only RAWs; writes paired photographic TIFFs into a fresh cache directory"]
fn photographic_tiff_pairs() {
    let root =
        PathBuf::from(std::env::var_os("ANTITONE_RAW_SAMPLES").expect("set ANTITONE_RAW_SAMPLES"));
    let output = PathBuf::from(
        std::env::var_os("ANTITONE_PHOTO_OUTPUT").expect("set fresh ANTITONE_PHOTO_OUTPUT"),
    );
    assert!(root.is_absolute() && output.is_absolute());
    std::fs::create_dir(&output).unwrap();
    let (train, calibration) = train_raw::load();
    let wb = train.fixed_wb;
    drop(train);
    let (device, queue, renderer) = pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let start = Instant::now();
        let renderer = GpuRenderer::new(&device, &queue).await.unwrap();
        println!(
            "photographic renderer_initialization_ms={:.3} adapter={:?}",
            start.elapsed().as_secs_f64() * 1000.,
            adapter.get_info()
        );
        (device, queue, renderer)
    });
    for (name, fraction) in [
        ("DSC04520.ARW", 0.25),
        ("DSC04522.ARW", 0.5),
        ("DSC04524.ARW", 0.75),
    ] {
        let mut scan = Scan::open(&root.join(name), Some(wb)).unwrap();
        let area =
            SourceRegion::from([scan.area.x, scan.area.y, scan.area.width, scan.area.height]);
        let region = SourceRegion {
            x: area.x + ((area.width - 512) as f64 * fraction) as u32,
            y: area.y + (area.height - 384) / 2,
            width: 512,
            height: 384,
        };
        let composition = Composition::with_crop(
            area,
            Default::default(),
            Default::default(),
            Some(Crop::from_region(region)),
        )
        .unwrap();
        for corrected in [false, true] {
            let field = corrected.then(|| {
                Arc::new(
                    FlatField::from_nodes(
                        [area.x, area.y, area.width, area.height],
                        [2, 2],
                        vec![
                            DVec3::new(0.8, 0.9, 1.),
                            DVec3::new(1.2, 1.1, 1.05),
                            DVec3::new(1., 1.1, 0.9),
                            DVec3::new(1.1, 0.95, 1.2),
                        ],
                    )
                    .unwrap(),
                )
            });
            scan.set_flat_field(field.clone()).unwrap();
            let gpu_field = field.map(|f| Arc::new(GpuFlatField::new(&device, f).unwrap()));
            let input = InputSettings::negative(scan.processing_calibration(calibration).unwrap());
            for (edited, colour, density) in [
                ("none", 0., 0.),
                ("rgb", 0., 0.),
                ("preserve", 1., 1.),
                ("mixed", 0.5, 0.5),
            ] {
                let photo = if edited != "none" {
                    AdjustmentLayerSettings {
                        exposure: 2.,
                        wb: antitone::color::Balance {
                            warmth: 0.15,
                            tint: -0.1,
                        },
                        contrast: 0.8,
                        contrast_colour_mix: colour,
                        saturation: 1.2,
                        saturation_density: density,
                        ..Default::default()
                    }
                } else {
                    AdjustmentLayerSettings::default()
                };
                let layers = if edited != "none" {
                    vec![AdjustmentLayer {
                        mask: AdjustmentLayerCoverage::Linear {
                            start: SourcePoint([region.x as f64, region.y as f64]),
                            end: SourcePoint([
                                (region.x + region.width) as f64,
                                (region.y + region.height) as f64,
                            ]),
                        },
                        photo: AdjustmentLayerSettings {
                            exposure: -0.5,
                            wb: antitone::color::Balance {
                                warmth: -0.1,
                                tint: 0.05,
                            },
                            contrast: 1.1,
                            contrast_colour_mix: colour,
                            saturation: 0.9,
                            saturation_density: density,
                            ..Default::default()
                        },
                        enabled: true,
                        invert: false,
                        strength: 0.7,
                    }]
                } else {
                    vec![]
                };
                let grade = AdjustmentLayerSnapshot::new(
                    std::iter::once(antitone::adjustment_layers::AdjustmentLayer::global(photo))
                        .chain(layers)
                        .collect(),
                )
                .unwrap();
                let roll = RollSettings::default();
                let pipeline =
                    AdjustmentLayerPipeline::new(input, roll, grade.clone(), scan.camera_to_ap1)
                        .unwrap();
                for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
                    let label = format!("{name}-{corrected}-{edited}-{target:?}");
                    let gpu_path = output.join(format!("{label}-f32.tif"));
                    let reference_path = output.join(format!("{label}-scalar.tif"));
                    let start = Instant::now();
                    gpu_export::write(
                        &renderer,
                        &device,
                        (&queue, &SubmissionGate::default()),
                        Request {
                            scan: &scan,
                            field: gpu_field.clone(),
                            input,
                            roll,
                            grade: grade.clone(),
                            repairs: &[],
                            composition,
                            pipeline: Default::default(),
                            format: Format::Tiff(target),
                            look: None,
                        },
                        &gpu_path,
                        Overwrite::Refuse,
                    )
                    .unwrap();
                    let gpu_ms = start.elapsed().as_secs_f64() * 1000.;
                    export::tiff_composed(
                        &reference_path,
                        composition,
                        target,
                        Overwrite::Refuse,
                        |x, y| pipeline.apply(scan.pixel(x, y)?, [x, y]).map(|p| p.rgb),
                    )
                    .unwrap();
                    let (dimensions, actual) = read(&gpu_path);
                    let (reference_dimensions, expected) = read(&reference_path);
                    assert_eq!(dimensions, [512, 384]);
                    assert_eq!(dimensions, reference_dimensions);
                    assert_eq!(actual.len(), expected.len());
                    assert!(
                        expected.iter().min() != expected.iter().max(),
                        "uniform fixture {label}"
                    );
                    let mut errors: Vec<_> = actual
                        .iter()
                        .zip(&expected)
                        .map(|(a, b)| a.abs_diff(*b))
                        .collect();
                    errors.sort_unstable();
                    let mean =
                        errors.iter().map(|v| f64::from(*v)).sum::<f64>() / errors.len() as f64;
                    println!(
                        "photographic case={label} gpu_ms={gpu_ms:.3} rgb16_mean={mean:.6} p99={} maximum={} above_one_8bit_code={} interior_reference_channels={}/{}",
                        errors[errors.len() * 99 / 100],
                        errors.last().unwrap(),
                        errors.iter().filter(|v| **v > 257).count(),
                        expected.iter().filter(|v| **v > 257 && **v < 65278).count(),
                        expected.len()
                    );
                }
            }
        }
    }
}
