#![cfg(all(
    feature = "raw",
    feature = "gpu",
    feature = "export",
    feature = "ocio-reference",
    target_os = "macos"
))]
#[path = "support/output.rs"]
mod authored_export;
use antitone::adjustment_layers::AdjustmentLayerSettings;
use antitone::{
    adjustment_layers::{AdjustmentLayerPipeline, AdjustmentLayerSnapshot},
    export::{self, ExrPrecision, Overwrite},
    geometry::{Composition, PhotoOrientation, RollOrientation},
    gpu::submission::SubmissionGate,
    gpu_export::Format,
    looks::{LookAsset, LookOrigin, Lut3D},
    processing::{Calibration, InputSettings, RollSettings},
    renderer::GpuRenderer,
    rendering::OutputTarget,
    scan::Scan,
};
use authored_export::Request;
use glam::DVec3;
use std::{path::Path, sync::Arc};
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
fn fixture() -> (
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
    };
    let grade =
        AdjustmentLayerSnapshot::new(vec![antitone::adjustment_layers::AdjustmentLayer::global(
            AdjustmentLayerSettings::default(),
        )])
        .unwrap();
    let composition = Composition::new(
        scan.area,
        RollOrientation::default(),
        PhotoOrientation::default(),
    )
    .unwrap();
    (
        scan,
        calibration,
        RollSettings::default(),
        grade,
        composition,
    )
}
fn look(minimum: [f32; 3], maximum: [f32; 3]) -> Arc<LookAsset> {
    let mut nodes = vec![];
    for b in 0..2 {
        for g in 0..2 {
            for r in 0..2 {
                nodes.push(std::array::from_fn(|c| {
                    minimum[c] + [r, g, b][c] as f32 * (maximum[c] - minimum[c]) + 0.02
                }));
            }
        }
    }
    Arc::new(
        LookAsset::new(
            "Test scene look".into(),
            LookOrigin::Imported,
            Arc::new(Lut3D::new(2, minimum, maximum, nodes).unwrap()),
        )
        .unwrap(),
    )
}
fn repairs(
    scan: &Scan,
    calibration: Calibration,
) -> (
    Vec<antitone::dust_repair::Repair>,
    Vec<Arc<antitone::dust_repair::EvaluatedRepair>>,
) {
    use antitone::dust_repair::{Repair, RepairId, RepairOrigin, SupportMask};
    let repair = Repair {
        id: RepairId(1),
        enabled: true,
        origin: RepairOrigin::Manual,
        support: SupportMask::new([126, 62], [5, 5], vec![true; 25]).unwrap(),
        offset: [-64, 0],
    };
    let input = InputSettings::negative(calibration.validate().unwrap());
    let evaluated = vec![Arc::new(
        antitone::repair_source::evaluate(
            &antitone::scan::ImageSource::new(scan, input).unwrap(),
            &repair,
        )
        .unwrap(),
    )];
    (vec![repair], evaluated)
}

fn rgb16(path: &Path) -> Vec<u16> {
    let mut decoder = tiff::decoder::Decoder::new(std::fs::File::open(path).unwrap()).unwrap();
    let tiff::decoder::DecodingResult::U16(values) = decoder.read_image().unwrap() else {
        panic!("not RGB16")
    };
    values
}
#[test]
fn tiff_applies_one_scene_look_before_each_selected_output_and_records_identity() {
    compare_tiff(false, look([-2.; 3], [2.; 3]));
}

#[test]
fn repaired_tiff_applies_the_scene_look_after_neutral_replacement_and_matches_scalar_ocio() {
    compare_tiff(true, look([-2.; 3], [2.; 3]));
}

#[test]
fn every_colour_medium_paper_only_tiff_matches_scalar_for_both_outputs_without_another_negative() {
    use antitone::looks::spektrafilm::{Model, Paper, Selection};
    for paper in Paper::all() {
        let selection = Selection {
            negative: None,
            paper,
        };
        let asset = Model::new(selection, &std::sync::atomic::AtomicBool::new(false))
            .unwrap()
            .bake(
                antitone::looks::authoring::DEFAULT_GRID,
                &std::sync::atomic::AtomicBool::new(false),
            )
            .unwrap();
        compare_tiff(false, Arc::new(asset));
    }
}

fn compare_tiff(repaired: bool, asset: Arc<LookAsset>) {
    let (device, queue, renderer) = context();
    let (scan, calibration, roll, grade, composition) = fixture();
    let (instructions, repairs) = if repaired {
        repairs(&scan, calibration)
    } else {
        (vec![], vec![])
    };
    let pipeline = AdjustmentLayerPipeline::new(
        InputSettings::negative((calibration).validate().unwrap()),
        roll,
        grade.clone(),
        scan.camera_to_ap1,
    )
    .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let gate = SubmissionGate::default();
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        let actual = directory.path().join("gpu.tif");
        let expected = directory.path().join("cpu.tif");
        let bypass = directory.path().join("none.tif");
        for (path, selected) in [(&actual, Some(asset.clone())), (&bypass, None)] {
            authored_export::write(
                &renderer,
                &device,
                (&queue, &gate),
                Request {
                    pipeline: Default::default(),
                    scan: &scan,
                    field: None,
                    input: InputSettings::negative(calibration),
                    roll,
                    grade: grade.clone(),
                    composition,
                    format: Format::Tiff(target),
                    repairs: &instructions,
                    luts: selected.into(),
                },
                path,
                Overwrite::Confirmed,
            )
            .unwrap();
        }
        export::tiff_composed(
            &expected,
            composition,
            target,
            Overwrite::Confirmed,
            |x, y| {
                let value = if let Some(repair) = repairs
                    .iter()
                    .find(|repair| repair.support().contains([x, y]))
                {
                    pipeline
                        .grade_neutral(
                            antitone::processing::InvertedPixel {
                                rgb: DVec3::from_array(
                                    repair.pixel([x, y], [0.; 3]).map(f64::from),
                                ),
                                floored: [false; 3],
                            },
                            [x, y],
                        )?
                        .rgb
                } else {
                    pipeline.apply(scan.pixel(x, y)?, [x, y])?.rgb
                };
                asset.lut().apply(value)
            },
        )
        .unwrap();
        let actual_values = rgb16(&actual);
        let expected_values = rgb16(&expected);
        assert!(
            actual_values
                .iter()
                .zip(&expected_values)
                .all(|(a, b)| a.abs_diff(*b) <= 1)
        );
        assert_ne!(actual_values, rgb16(&bypass));
    }
}
#[test]
fn exr_is_identical_with_a_selected_look_even_when_rendered_inputs_would_be_clipped() {
    compare_exr(false);
}

#[test]
fn repaired_half_and_full_exr_bypass_the_look_in_pixels_and_attributes() {
    compare_exr(true);
}

fn compare_exr(repaired: bool) {
    let (device, queue, renderer) = context();
    let (scan, calibration, roll, grade, composition) = fixture();
    let (instructions, _repairs) = if repaired {
        repairs(&scan, calibration)
    } else {
        (vec![], vec![])
    };
    let narrow = look([0.99; 3], [1.; 3]);
    let directory = tempfile::tempdir().unwrap();
    let gate = SubmissionGate::default();
    for precision in [ExrPrecision::Half, ExrPrecision::Full] {
        let plain = directory.path().join("plain.exr");
        let selected = directory.path().join("selected.exr");
        for (path, look) in [(&plain, None), (&selected, Some(narrow.clone()))] {
            authored_export::write(
                &renderer,
                &device,
                (&queue, &gate),
                Request {
                    pipeline: Default::default(),
                    scan: &scan,
                    field: None,
                    input: InputSettings::negative(calibration),
                    roll,
                    grade: grade.clone(),
                    composition,
                    format: Format::Exr(precision),
                    repairs: &instructions,
                    luts: look.into(),
                },
                path,
                Overwrite::Confirmed,
            )
            .unwrap();
        }
        // EXR attribute order is unspecified; compare all decoded metadata and exact
        // sample bits, not the serializer's randomized attribute iteration order.
        use exr::prelude::{FlatSamples, read_first_flat_layer_from_file};
        let plain = read_first_flat_layer_from_file(&plain).unwrap();
        let selected = read_first_flat_layer_from_file(&selected).unwrap();
        assert_eq!(plain, selected);
        for (a, b) in plain
            .layer_data
            .channel_data
            .list
            .iter()
            .zip(&selected.layer_data.channel_data.list)
        {
            match (&a.sample_data, &b.sample_data) {
                (FlatSamples::F16(a), FlatSamples::F16(b)) => {
                    assert!(a.iter().zip(b).all(|(a, b)| a.to_bits() == b.to_bits()))
                }
                (FlatSamples::F32(a), FlatSamples::F32(b)) => {
                    assert!(a.iter().zip(b).all(|(a, b)| a.to_bits() == b.to_bits()))
                }
                _ => panic!("EXR precision changed"),
            }
        }
    }
}
#[test]
fn narrow_lut_renders_crop_and_full_image_and_invalid_settings_preserve_destination() {
    let (device, queue, renderer) = context();
    let (scan, calibration, roll, grade, composition) = fixture();
    let pipeline = AdjustmentLayerPipeline::new(
        InputSettings::negative((calibration).validate().unwrap()),
        roll,
        grade.clone(),
        scan.camera_to_ap1,
    )
    .unwrap();
    let [x, y] = [
        scan.area.x + scan.area.width / 2,
        scan.area.y + scan.area.height / 2,
    ];
    let scene = pipeline
        .apply(scan.pixel(x, y).unwrap(), [x, y])
        .unwrap()
        .rgb;
    let encoded = scene.to_array().map(|v| {
        if v <= 0.0078125 {
            10.5402377416545 * v + 0.0729055341958355
        } else {
            (v.log2() + 9.72) / 17.52
        }
    });
    let asset = look(
        encoded.map(|v| (v - 0.001) as f32),
        encoded.map(|v| (v + 0.001) as f32),
    );
    let crop = Composition::with_crop(
        scan.area,
        Default::default(),
        Default::default(),
        Some(antitone::crop::Crop {
            center: [x as f64 + 0.5, y as f64 + 0.5],
            size: [1, 1],
            angle: 0.,
        }),
    )
    .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("crop.tif");
    let gate = SubmissionGate::default();
    authored_export::write(
        &renderer,
        &device,
        (&queue, &gate),
        Request {
            pipeline: Default::default(),
            scan: &scan,
            field: None,
            input: InputSettings::negative(calibration),
            roll,
            grade: grade.clone(),
            composition: crop,
            repairs: &[],
            format: Format::Tiff(OutputTarget::Srgb),
            luts: (Some(asset.clone())).into(),
        },
        &path,
        Overwrite::Refuse,
    )
    .unwrap();
    assert_eq!(rgb16(&path).len(), 3);
    authored_export::write(
        &renderer,
        &device,
        (&queue, &gate),
        Request {
            pipeline: Default::default(),
            scan: &scan,
            field: None,
            input: InputSettings::negative(calibration),
            roll,
            grade: grade.clone(),
            composition,
            repairs: &[],
            format: Format::Tiff(OutputTarget::Srgb),
            luts: (Some(asset.clone())).into(),
        },
        &path,
        Overwrite::Confirmed,
    )
    .unwrap();
    let expected = directory.path().join("reference.tif");
    export::tiff_composed(
        &expected,
        composition,
        OutputTarget::Srgb,
        Overwrite::Refuse,
        |x, y| {
            asset
                .lut()
                .apply(pipeline.apply(scan.pixel(x, y)?, [x, y])?.rgb)
        },
    )
    .unwrap();
    assert!(
        rgb16(&path)
            .iter()
            .zip(rgb16(&expected))
            .all(|(a, b)| a.abs_diff(b) <= 1)
    );
    let original = std::fs::read(&path).unwrap();
    let invalid_roll = RollSettings {
        exposure: f64::NAN,
        ..roll
    };
    assert!(
        authored_export::write(
            &renderer,
            &device,
            (&queue, &gate),
            Request {
                pipeline: Default::default(),
                scan: &scan,
                field: None,
                input: InputSettings::negative(calibration),
                roll: invalid_roll,
                repairs: &[],
                grade,
                composition,
                format: Format::Tiff(OutputTarget::Srgb),
                luts: Some(asset).into()
            },
            &path,
            Overwrite::Confirmed
        )
        .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 2);
}
