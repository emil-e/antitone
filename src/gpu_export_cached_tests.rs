use super::*;
use crate::{
    crop::Crop,
    geometry::{PhotoOrientation, QuarterTurns, RollOrientation, SourcePoint},
    local_adjustments::{GradientMask, LocalAdjustment},
    processing::PhotoSettings,
};

#[test]
fn cached_jobs_preserve_reopened_pixels_through_correction_locals_and_rotated_reconstruction() {
    let (device, queue, renderer) = super::tests::context();
    let gate = SubmissionGate::default();
    let mut scan = Scan::open(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng"),
        None,
    )
    .unwrap();
    let area = scan.area;
    let calibration = Calibration {
        dmin: DVec3::splat(3000. / 4031.),
        dmax: DVec3::splat(128. / 4031.),
        input_floor: scan.input_floor,
    };
    let roll = RollSettings::new_grade();
    let grade = PhotoGradeSnapshot::new(
        PhotoSettings {
            contrast: 0.8,
            saturation: 1.1,
            ..Default::default()
        },
        vec![LocalAdjustment {
            mask: GradientMask::Linear {
                start: SourcePoint([32., 24.]),
                end: SourcePoint([224., 168.]),
            },
            photo: PhotoSettings {
                exposure: 0.5,
                ..Default::default()
            },
            enabled: true,
            invert: false,
            strength: 0.7,
        }],
    )
    .unwrap();
    let directory = tempfile::tempdir().unwrap();
    for corrected in [false, true] {
        let field = corrected.then(|| {
            Arc::new(
                crate::flat_field::FlatField::from_nodes(
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
        let field = field.map(|f| Arc::new(GpuFlatField::new(&device, f).unwrap()));
        for angle in [0., 0.13] {
            let crop = if angle == 0. {
                None
            } else {
                Some(Crop {
                    center: [128., 96.],
                    size: [160, 128],
                    angle,
                })
            };
            let composition = Composition::with_crop(
                area,
                RollOrientation {
                    turns: QuarterTurns::One,
                    ..Default::default()
                },
                PhotoOrientation::default(),
                crop,
            )
            .unwrap();
            for format in [
                Format::Tiff(OutputTarget::Srgb),
                Format::Tiff(OutputTarget::DisplayP3),
                Format::Exr(ExrPrecision::Half),
                Format::Exr(ExrPrecision::Full),
            ] {
                let request = || Request {
                    look: None,
                    scan: &scan,
                    field: field.clone(),
                    calibration,
                    roll,
                    grade: grade.clone(),
                    composition,
                    format,
                };
                let baseline = directory.path().join("baseline");
                let cached = directory.path().join("cached");
                let plain_stats = write_job(
                    &renderer,
                    &device,
                    (&queue, &gate),
                    request(),
                    (&baseline, Overwrite::Confirmed),
                    false,
                    || Ok(()),
                )
                .unwrap();
                let mut cached_checks = 0;
                let cached_stats = write_job(
                    &renderer,
                    &device,
                    (&queue, &gate),
                    request(),
                    (&cached, Overwrite::Confirmed),
                    true,
                    || {
                        cached_checks += 1;
                        Ok(())
                    },
                )
                .unwrap();
                assert_eq!(
                    cached_checks,
                    1 + 2 * cached_stats.0,
                    "cache hits retain bounded stop checks"
                );
                assert_eq!(plain_stats.0, plain_stats.1);
                assert_eq!(plain_stats.0, cached_stats.0);
                assert!(
                    cached_stats.1 < plain_stats.1,
                    "fixture must exercise tile reuse"
                );
                assert!(cached_stats.2 <= super::cache::MAX_PAYLOAD_BYTES);
                if !corrected && angle == 0. && matches!(format, Format::Tiff(OutputTarget::Srgb)) {
                    let before = std::fs::read(&cached).unwrap();
                    for stop in [2, 3, 4, 5] {
                        let mut checks = 0;
                        let stopped = write_job(
                            &renderer,
                            &device,
                            (&queue, &gate),
                            request(),
                            (&cached, Overwrite::Confirmed),
                            true,
                            || {
                                checks += 1;
                                if checks == stop {
                                    Err(Error::Export("bounded diagnostic stop".into()))
                                } else {
                                    Ok(())
                                }
                            },
                        );
                        assert_eq!(
                            stopped,
                            Err(Error::Export("bounded diagnostic stop".into()))
                        );
                        assert_eq!(std::fs::read(&cached).unwrap(), before);
                        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 2);
                    }
                    let mut checks = 0;
                    assert!(
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| write_job(
                            &renderer,
                            &device,
                            (&queue, &gate),
                            request(),
                            (&cached, Overwrite::Confirmed),
                            true,
                            || {
                                checks += 1;
                                if checks == 3 {
                                    panic!("paired consumer stop");
                                }
                                Ok(())
                            }
                        )))
                        .is_err()
                    );
                    assert_eq!(std::fs::read(&cached).unwrap(), before);
                    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 2);
                }
                if matches!(format, Format::Tiff(_)) {
                    let serial = directory.path().join("serial");
                    let stats = super::pair_tests::write_single(
                        &renderer,
                        &device,
                        (&queue, &gate),
                        request(),
                        (&serial, Overwrite::Refuse),
                        true,
                        || Ok(()),
                    )
                    .unwrap();
                    assert_eq!(
                        stats, cached_stats,
                        "batching must not change original requests/cache misses/payload"
                    );
                    assert_eq!(
                        std::fs::read(&serial).unwrap(),
                        std::fs::read(&cached).unwrap(),
                        "single and paired originals must serialize the same complete TIFF"
                    );
                    std::fs::remove_file(serial).unwrap();
                }
                match format {
                    Format::Tiff(_) => {
                        use tiff::decoder::Decoder;
                        let mut a = Decoder::new(std::fs::File::open(&baseline).unwrap()).unwrap();
                        let mut b = Decoder::new(std::fs::File::open(&cached).unwrap()).unwrap();
                        assert_eq!(a.dimensions().unwrap(), b.dimensions().unwrap());
                        assert_eq!(
                            a.get_tag_u8_vec(tiff::tags::Tag::Unknown(34675)).unwrap(),
                            b.get_tag_u8_vec(tiff::tags::Tag::Unknown(34675)).unwrap()
                        );
                        let tiff::decoder::DecodingResult::U16(a) = a.read_image().unwrap() else {
                            panic!()
                        };
                        let tiff::decoder::DecodingResult::U16(b) = b.read_image().unwrap() else {
                            panic!()
                        };
                        assert_eq!(a, b);
                    }
                    Format::Exr(_) => {
                        let a = exr::prelude::read_first_flat_layer_from_file(&baseline).unwrap();
                        let b = exr::prelude::read_first_flat_layer_from_file(&cached).unwrap();
                        assert_eq!(a.layer_data.channel_data, b.layer_data.channel_data);
                        assert_eq!(a.layer_data.attributes, b.layer_data.attributes);
                        assert_eq!(a.attributes, b.attributes);
                    }
                }
            }
        }
    }
}
