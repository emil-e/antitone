use super::*;
#[path = "../support/working_precision.rs"]
mod working_precision;
use antitone::{
    adjustment_layers::{AdjustmentLayerPipeline, AdjustmentLayerSnapshot},
    export::{self, ExrPrecision, Overwrite},
    geometry::{Composition, PhotoOrientation, RollOrientation},
    gpu::SubmissionGate,
    gpu_export::{self, Format, Request},
    processing::{Calibration, InputSettings},
    renderer::GpuRenderer,
    rendering::OutputTarget,
};
use tiff::decoder::{Decoder, DecodingResult};

// The sole changed red site is one original code above black. A uniform half-gain
// puts its developed value below the original quantum without making it capture-black.
// The sensor's (24, 24) is (18, 20) after the authored active-area offset (6, 4).
fn camera_pixel(x: u32, y: u32) -> DVec3 {
    let tent = f64::from(2_u32.saturating_sub(x.abs_diff(18)))
        * f64::from(2_u32.saturating_sub(y.abs_diff(20)))
        / 4.;
    let rgb = DVec3::new(0.125 - 0.5 * 1023. / 4096. * tent, 0.1875, 0.25);
    // The shared continuous-camera ABI carries f32 camera-code units.
    (rgb * 65535.).as_vec3().as_dvec3() / 65535.
}

fn tiff_pixels(path: &Path) -> Vec<u16> {
    let mut decoder = Decoder::new(std::fs::File::open(path).unwrap()).unwrap();
    let DecodingResult::U16(values) = decoder.read_image().unwrap() else {
        panic!("expected RGB16 TIFF")
    };
    values
}

#[test]
fn cold_sdk_project_exports_reopen_as_scalar_corresponding_tiff_and_ap0_exr() {
    let mut fixture = Fixture {
        marker: Some((24, 24, 65)),
        ..Default::default()
    };
    fixture.lists[1].push(Opcode::gain(0.5, 1));
    let file = fixture.write();
    let scan = Scan::open(file.path(), None).unwrap();
    assert_eq!(scan.source().input_floor(), 1. / 4096.);
    let calibration = Calibration {
        dmin: DVec3::splat(0.75),
        dmax: DVec3::splat(0.01),
        input_floor: CAMERA_INPUT_FLOOR,
    };
    let mut project = Project {
        input: InputSettings::negative(calibration),
        ..Project::default()
    };
    let id = project.add_source(record(file.path(), &scan)).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let project_path = directory.path().join("sdk.antitone");
    project.save(&project_path, Save::New).unwrap();
    drop(scan);
    let loaded = Project::load(&project_path).unwrap();
    loaded.require_runtime().unwrap();
    let source = loaded.source(id).unwrap();
    let scan = Scan::open(&source.path, Some(source.development.decode.fixed_wb)).unwrap();
    assert_eq!(record(file.path(), &scan).development, source.development);
    assert_eq!(source.development.input_floor, 1. / 4096.);
    for y in scan.area.y..scan.area.y + scan.area.height {
        for x in scan.area.x..scan.area.x + scan.area.width {
            assert_eq!(
                scan.pixel(x, y).unwrap(),
                camera_pixel(x, y),
                "source at ({x}, {y})"
            );
        }
    }
    let calibration = loaded.calibration().unwrap().unwrap();
    let grade = AdjustmentLayerSnapshot::new(
        std::iter::once(antitone::adjustment_layers::AdjustmentLayer::global(
            source.adjustment_layers.as_ref().unwrap().layers[0]
                .adjustment
                .photo,
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
    let oracle_input = InputSettings::negative(Calibration {
        input_floor: 1. / 4096.,
        ..calibration
    });
    let oracle =
        AdjustmentLayerPipeline::new(oracle_input, loaded.roll, grade.clone(), scan.camera_to_ap1)
            .unwrap();
    let (device, queue, renderer) = pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let renderer = GpuRenderer::new(&device, &queue).await.unwrap();
        (device, queue, renderer)
    });
    for (extension, format) in [
        ("tif", Format::Tiff(OutputTarget::Srgb)),
        ("exr", Format::Exr(ExrPrecision::Full)),
    ] {
        let actual_path = directory.path().join(format!("gpu.{extension}"));
        let expected_path = directory.path().join(format!("scalar.{extension}"));
        gpu_export::write(
            &renderer,
            &device,
            (&queue, &SubmissionGate::default()),
            Request {
                scan: &scan,
                field: None,
                input: InputSettings::negative(calibration),
                roll: loaded.roll,
                grade: grade.clone(),
                repairs: &[],
                composition,
                pipeline: Default::default(),
                format,
                look: None,
            },
            &actual_path,
            Overwrite::Refuse,
        )
        .unwrap();
        match format {
            Format::Tiff(target) => {
                export::tiff_composed(
                    &expected_path,
                    composition,
                    target,
                    Overwrite::Refuse,
                    |x, y| oracle.apply(camera_pixel(x, y), [x, y]).map(|p| p.rgb),
                )
                .unwrap();
                let actual = tiff_pixels(&actual_path);
                let expected = tiff_pixels(&expected_path);
                assert_eq!(actual.len(), expected.len());
                assert!(actual.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 1));
            }
            Format::Exr(precision) => {
                use exr::prelude::*;
                export::exr_composed(
                    &expected_path,
                    composition,
                    precision,
                    Overwrite::Refuse,
                    |x, y| oracle.apply(camera_pixel(x, y), [x, y]).map(|p| p.rgb),
                )
                .unwrap();
                let actual = read_first_flat_layer_from_file(&actual_path).unwrap();
                let expected = read_first_flat_layer_from_file(&expected_path).unwrap();
                assert_eq!(actual.layer_data.size, expected.layer_data.size);
                assert!(actual.attributes.chromaticities.is_some());
                assert_eq!(
                    actual.attributes.chromaticities,
                    expected.attributes.chromaticities
                );
                assert_eq!(actual.layer_data.channel_data.list.len(), 3);
                for (a, b) in actual
                    .layer_data
                    .channel_data
                    .list
                    .iter()
                    .zip(&expected.layer_data.channel_data.list)
                {
                    assert_eq!(a.name, b.name);
                    let (FlatSamples::F32(actual), FlatSamples::F32(expected)) =
                        (&a.sample_data, &b.sample_data)
                    else {
                        panic!("expected full-precision scene-linear AP0")
                    };
                    assert_eq!(actual.len(), expected.len());
                    for (a, b) in actual.iter().zip(expected) {
                        let expected = f64::from(*b);
                        assert!(
                            (f64::from(*a) - expected).abs()
                                <= working_precision::allowance(expected)
                                    + f64::from(f32::EPSILON) * expected.abs()
                        );
                    }
                }
            }
        }
    }
}
