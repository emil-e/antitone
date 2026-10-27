use super::*;
use crate::{processing::InputSettings, rendering::CpuRenderer};

use crate::exposure_fixture;

#[test]
fn cold_tiff_and_exr_resolve_starting_exposure_without_viewer_or_manual_control_changes() {
    for repaired in [false, true] {
        let path = if repaired {
            exposure_fixture::neutral_dng_with_defect(1.3)
        } else {
            exposure_fixture::neutral_dng(1.3)
        };
        let (device, queue, renderer) = super::super::super::tests::context();
        let gate = SubmissionGate::default();
        let directory = tempfile::tempdir().unwrap();
        let scene = 6000. / 65535. * 2_f64.powf(1.3);
        let reference = CpuRenderer::new(OutputTarget::Srgb)
            .unwrap()
            .render(&[DVec3::splat(scene)])
            .unwrap()[0];
        for format in [
            Format::Tiff(OutputTarget::Srgb),
            Format::Exr(crate::export::ExrPrecision::Full),
        ] {
            let scan = Scan::open(path.path(), Some([1.; 4])).unwrap();
            assert!(scan.source().reference_exposure.get().is_none());
            let (mut project, id) = fixture(&scan);
            project.input = InputSettings::Digital;
            project.source_mut(id).unwrap().crop = None;
            if repaired {
                project
                    .source_mut(id)
                    .unwrap()
                    .repairs
                    .insert(
                        crate::dust_repair::SupportMask::new(
                            [10, 10],
                            [13, 13],
                            vec![true; 13 * 13],
                        )
                        .unwrap(),
                        crate::dust_repair::RepairOrigin::Manual,
                        crate::dust_repair::DonorChoice::Offset([31, 31]),
                    )
                    .unwrap();
            }
            let before = project.clone();
            let recipe = project.image_recipe(id).unwrap();
            let output = directory.path().join(match format {
                Format::Tiff(_) => "cold.tif",
                Format::Exr(_) => "cold.exr",
            });
            write(
                &renderer,
                &device,
                (&queue, &gate),
                Job {
                    scan: &scan,
                    field: None,
                    recipe: &recipe,
                    format,
                },
                (&output, Overwrite::Refuse),
                &mut Execution::default(),
                || Ok(()),
            )
            .unwrap();
            let exposure = scan.source().reference_exposure.get().unwrap();
            assert!((exposure.ev() - 1.3).abs() < 0.03, "{exposure:?}");
            assert_eq!(project, before);
            assert_eq!(project.roll.exposure, 0.);
            assert!(
                project
                    .source(id)
                    .unwrap()
                    .grade_snapshot()
                    .unwrap()
                    .layers()
                    .iter()
                    .all(|layer| layer.photo.exposure == 0.)
            );
            match format {
                Format::Tiff(_) => {
                    let mut decoder =
                        tiff::decoder::Decoder::new(std::fs::File::open(&output).unwrap()).unwrap();
                    assert_eq!(decoder.dimensions().unwrap(), (64, 64));
                    let tiff::decoder::DecodingResult::U16(data) = decoder.read_image().unwrap()
                    else {
                        panic!("RGB16 TIFF expected");
                    };
                    let expected = reference.to_array().map(|v| {
                        (OutputTarget::Srgb.encode(f64::from(v)).unwrap() * 65535.).round() as u16
                    });
                    // The camera reference was quantized to8-bit JPEG before estimating gain.
                    for point in [[16, 16], [32, 32], [48, 48]] {
                        let offset = (point[0] + point[1] * 64) * 3;
                        for channel in 0..3 {
                            assert!(
                                data[offset + channel].abs_diff(expected[channel]) <= 160,
                                "{} vs {}",
                                data[offset + channel],
                                expected[channel]
                            );
                        }
                    }
                }
                Format::Exr(_) => {
                    let image = exr::prelude::read_first_flat_layer_from_file(&output).unwrap();
                    assert_eq!(image.layer_data.channel_data.list.len(), 3);
                    for channel in &image.layer_data.channel_data.list {
                        let exr::prelude::FlatSamples::F32(data) = &channel.sample_data else {
                            panic!("full precision EXR expected");
                        };
                        for point in [[16, 16], [32, 32], [48, 48]] {
                            let actual = f64::from(data[point[0] + point[1] * 64]);
                            assert!((actual / scene - 1.).abs() < 0.03, "{actual} vs {scene}");
                        }
                    }
                }
            }
        }
    }
}
