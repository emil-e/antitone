use super::*;
use crate::{processing::InputSettings, rendering::CpuRenderer};

use crate::exposure_fixture;

#[test]
fn prepared_tiff_and_exr_use_resolved_starting_exposure_without_changing_manual_controls() {
    let exposure = |ev| {
        crate::exposure_reference::ReferenceExposure::Matched(
            crate::exposure_reference::StartingExposure {
                ev,
                samples: 1024,
                spread: 0.,
            },
        )
    };
    use crate::exposure_reference::{ReferenceExposure, UnavailableReference};

    for (repaired, known, expected_ev) in [
        (false, None, 1.3),
        (true, None, 1.3),
        (false, Some(exposure(2.75)), 2.75),
        (
            false,
            Some(ReferenceExposure::Unavailable(
                UnavailableReference::NoUsablePreview,
            )),
            0.,
        ),
    ] {
        let path = if repaired {
            exposure_fixture::neutral_dng_with_defect(1.3)
        } else {
            exposure_fixture::neutral_dng(1.3)
        };
        let (_device, queue, renderer) = super::super::super::tests::context();
        let directory = tempfile::tempdir().unwrap();
        let scene = 6000. / 65535. * 2_f64.powf(expected_ev);
        let reference = CpuRenderer::new(OutputTarget::Srgb)
            .unwrap()
            .render(&[DVec3::splat(scene)])
            .unwrap()[0];
        for format in [
            Format::Tiff(OutputTarget::Srgb),
            Format::Exr(crate::export::ExrPrecision::Full),
        ] {
            let scan = DevelopedSource::open(path.path(), None).unwrap();
            let (mut project, id) = fixture(&scan);
            project.input = InputSettings::Digital;
            project.source_mut(id).unwrap().development_mode =
                crate::project::DevelopmentMode::AsShot;
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
                        [31, 31],
                    )
                    .unwrap();
            }
            // A contrasting look must neither affect scene EXR pixels nor allocate a GPU LUT.
            if matches!(format, Format::Exr(_)) {
                use crate::looks::{LookAsset, LookOrigin, Lut3D};

                let lut = Lut3D::new(2, [0.; 3], [1.; 3], vec![[0.; 3]; 8]).unwrap();
                project.output.lut = Some(Arc::new(
                    LookAsset::new(
                        "EXR bypass control".into(),
                        LookOrigin::Imported,
                        Arc::new(lut),
                    )
                    .unwrap(),
                ));
            }
            let before = project.clone();
            let output = directory.path().join(match format {
                Format::Tiff(_) => "cold.tif",
                Format::Exr(_) => "cold.exr",
            });
            let job = |recipe| Job {
                source: &scan,
                field: None,
                recipe,
                render: fixture_render(format, &renderer),
            };
            let unresolved =
                (!repaired && known.is_none()).then(|| project.image_recipe(id).unwrap());
            if let Some(unresolved) = &unresolved {
                let result = write(
                    renderer.evaluation(),
                    &queue,
                    job(unresolved),
                    (&output, Overwrite::Refuse),
                    &mut Execution::default(),
                    || Ok(()),
                );
                assert!(matches!(
                    result,
                    Err(Error::InvalidParameter(
                        "export requires resolved Digital starting exposure"
                    ))
                ));
                assert!(!output.exists());
            }

            let point = [
                scan.area().x + scan.area().width / 2,
                scan.area().y + scan.area().height / 2,
            ];
            let camera = scan.pixel(point[0], point[1]).unwrap();
            let memo = known
                .clone()
                .map_or_else(std::sync::OnceLock::new, std::sync::OnceLock::from);
            let prepared =
                crate::photo_preparation::native(None, &memo, project.input, &scan).unwrap();
            let recipe = project
                .image_recipe_with_reference(project.source(id).unwrap(), prepared.reference)
                .unwrap();
            write(
                renderer.evaluation(),
                &queue,
                job(&recipe),
                (&output, Overwrite::Refuse),
                &mut Execution::default(),
                || Ok(()),
            )
            .unwrap();
            assert_eq!(
                scan.pixel(point[0], point[1]).unwrap(),
                camera,
                "export must not change camera light"
            );
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
                    // Fresh estimates include JPEG quantization; a recorded gain is exact.
                    for point in [[16, 16], [32, 32], [48, 48]] {
                        let offset = (point[0] + point[1] * 64) * 3;
                        for channel in 0..3 {
                            assert!(
                                data[offset + channel].abs_diff(expected[channel])
                                    <= if known.is_some() { 2 } else { 160 },
                                "{} vs {}",
                                data[offset + channel],
                                expected[channel]
                            );
                        }
                    }
                }
                Format::Exr(_) => {
                    assert_eq!(renderer.looks.uploads(), 0);
                    let image = exr::prelude::read_first_flat_layer_from_file(&output).unwrap();
                    assert_eq!(image.layer_data.channel_data.list.len(), 3);
                    for channel in &image.layer_data.channel_data.list {
                        let exr::prelude::FlatSamples::F32(data) = &channel.sample_data else {
                            panic!("full precision EXR expected");
                        };
                        for point in [[16, 16], [32, 32], [48, 48]] {
                            let actual = f64::from(data[point[0] + point[1] * 64]);
                            assert!(
                                (actual / scene - 1.).abs()
                                    < if known.is_some() { 1e-5 } else { 0.03 },
                                "{actual} vs {scene}"
                            );
                        }
                    }
                }
            }
        }
    }
}
