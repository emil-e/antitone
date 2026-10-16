use super::*;
use crate::{
    crop::Crop,
    geometry::{Flips, PhotoOrientation, QuarterTurns, RollOrientation, SourceRegion},
};

fn composition(
    area: SourceRegion,
    turns: QuarterTurns,
    flips: Flips,
    angle: f64,
    size: [u32; 2],
) -> Composition {
    Composition::with_crop(
        area,
        RollOrientation {
            turns,
            ..Default::default()
        },
        PhotoOrientation {
            flips,
            ..Default::default()
        },
        Some(Crop {
            center: [
                area.x as f64 + area.width as f64 / 2.,
                area.y as f64 + area.height as f64 / 2.,
            ],
            size,
            angle,
        }),
    )
    .unwrap()
}

#[test]
fn forecast_admission_bounds_payload_and_work_without_narrowing_processing() {
    let c = composition(
        SourceRegion::from([0, 0, 8640, 5760]),
        QuarterTurns::One,
        Flips::default(),
        0.015,
        [8128, 5248],
    );
    let forecast = Forecast::new(c).unwrap();
    let [width, height] = c.size();
    let blocks = export::ordered_blocks(ImageSize { width, height }, forecast.order()).count();
    assert_eq!(forecast.blocks as usize, blocks);
    assert!(forecast.blocks <= MAX_BLOCKS);
    assert!(!forecast.uses.is_empty());
    assert!(forecast.payload_bytes() <= MAX_INDEX_BYTES);
    assert!(forecast.uses.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(Forecast::with_budget(c, 0).is_none());
    assert!(
        Forecast::with_budget(c, forecast.uses.len() * std::mem::size_of::<Use>() - 1).is_none()
    );
    let exact = Composition::new(c.area(), Default::default(), Default::default()).unwrap();
    assert!(Forecast::new(exact).is_none());
    let huge = composition(
        SourceRegion::from([0, 0, 20000, 20000]),
        QuarterTurns::Zero,
        Flips::default(),
        0.015,
        [18000, 18000],
    );
    assert!(Forecast::new(huge).is_none());
}

#[test]
fn corner_hints_cover_actual_originals_for_turns_flips_edges_and_large_origins() {
    for area in [
        SourceRegion::from([17, 23, 513, 385]),
        SourceRegion::from([1_000_000_007, 900_000_013, 513, 385]),
        SourceRegion::from([17, 23, 65, 513]),
    ] {
        for turns in [
            QuarterTurns::Zero,
            QuarterTurns::One,
            QuarterTurns::Two,
            QuarterTurns::Three,
        ] {
            for angle in [-0.13, 0., 0.13] {
                for flips in [
                    Flips::default(),
                    Flips {
                        horizontal: true,
                        vertical: false,
                    },
                    Flips {
                        horizontal: false,
                        vertical: true,
                    },
                    Flips {
                        horizontal: true,
                        vertical: true,
                    },
                ] {
                    let crop = if angle == 0. {
                        [area.width, area.height]
                    } else if area.width < 128 {
                        [17, 193]
                    } else {
                        [353, 257]
                    };
                    let c = composition(area, turns, flips, angle, crop);
                    let raster = FrameRaster::full(c).unwrap();
                    let [width, height] = c.size();
                    for (origin, extent) in
                        export::ordered_blocks(ImageSize { width, height }, ORDER)
                    {
                        let predicted = hints(raster, origin, extent).unwrap();
                        let actual = plan(c, origin, extent, true).unwrap().admitted().unwrap();
                        for (key, _) in actual.tiles {
                            assert!(predicted.origins[..predicted.len].contains(&key));
                        }
                    }
                    assert!(hints(raster, [0, 0], [0, 1]).is_none());
                }
            }
        }
    }
}

#[test]
fn next_use_and_admission_handle_unknown_keys_ties_and_exhaustion() {
    let a = [1, 0];
    let b = [2, 0];
    let incoming = [3, 0];
    let mut uses = vec![
        Use {
            origin: a,
            block: 7,
        },
        Use {
            origin: b,
            block: 7,
        },
        Use {
            origin: incoming,
            block: 1,
        },
        Use {
            origin: incoming,
            block: 4,
        },
    ];
    uses.sort_unstable();
    let forecast = Forecast { uses, blocks: 8 };
    assert_eq!(forecast.next(incoming, 1), 4);
    assert_eq!(forecast.next([2, 1], 0), u32::MAX);
    let mut cursor = Cursor::new(forecast);
    assert_eq!(cursor.victim(incoming, [a, b].into_iter()), Some(0));
    assert!(cursor.advance());
    assert_eq!(
        cursor.victim(incoming, [a, b].into_iter()),
        Some(0),
        "same-next-use ties keep the LRU victim"
    );
    assert_eq!(cursor.victim([9, 9], [a, b].into_iter()), None);
    for _ in 1..8 {
        assert!(cursor.advance());
    }
    assert!(!cursor.advance());
}

#[test]
fn forecasted_consumer_failures_cancel_and_panic_preserve_destination_and_release_owner() {
    use std::sync::Arc;
    let c = composition(
        SourceRegion::from([0, 0, 1024, 768]),
        QuarterTurns::One,
        Flips::default(),
        0.015,
        [800, 576],
    );
    let metadata = crate::processing::Response::film().into();
    let directory =
        tempfile::tempdir_in(Path::new(env!("CARGO_MANIFEST_DIR")).join(".cache")).unwrap();
    let path = directory.path().join("existing.tif");
    std::fs::write(&path, b"keep existing").unwrap();
    for stop in [1, 2, 3, 4] {
        let owner = Arc::new(());
        let weak = Arc::downgrade(&owner);
        let load = move |_origin| {
            let _ = &owner;
            Ok(vec![[0.25, 0.5, 0.75, 1.]; 128 * 128])
        };
        let mut checks = 0;
        let check = || {
            checks += 1;
            if checks == stop {
                Err(Error::Export("forecast cancellation".into()))
            } else {
                Ok(())
            }
        };
        let mut reader = JobReader::forecasted(load, check, Forecast::new(c).unwrap());
        assert!(
            pipeline::encode(
                c,
                OutputTarget::Srgb,
                metadata,
                (&path, Overwrite::Confirmed),
                &mut reader
            )
            .is_err()
        );
        drop(reader);
        assert!(weak.upgrade().is_none());
        assert_eq!(std::fs::read(&path).unwrap(), b"keep existing");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }
    for panic in [false, true] {
        let owner = Arc::new(());
        let weak = Arc::downgrade(&owner);
        let load = move |_| -> Result<Vec<[f64; 4]>, Error> {
            let _ = &owner;
            if panic {
                panic!("forecast source panic");
            }
            Err(Error::Gpu("forecast source error".into()))
        };
        let mut reader = JobReader::forecasted(load, || Ok(()), Forecast::new(c).unwrap());
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            pipeline::encode(
                c,
                OutputTarget::Srgb,
                metadata,
                (&path, Overwrite::Confirmed),
                &mut reader,
            )
        }));
        assert!(if panic {
            result.is_err()
        } else {
            result.unwrap().is_err()
        });
        drop(reader);
        assert!(weak.upgrade().is_none());
        assert_eq!(std::fs::read(&path).unwrap(), b"keep existing");
    }
    let load = |_| Ok(vec![[0.25, 0.5, 0.75, 1.]; 128 * 128]);
    let mut early = JobReader::forecasted(load, || Ok(()), Forecast::new(c).unwrap());
    assert!(
        pipeline::encode(
            c,
            OutputTarget::Srgb,
            metadata,
            (&directory.path().join("missing/out.tif"), Overwrite::Refuse),
            &mut early
        )
        .is_err()
    );
    assert_eq!(early.requests, 0);
    let mut late = JobReader::forecasted(load, || Ok(()), Forecast::new(c).unwrap());
    assert!(
        pipeline::encode(
            c,
            OutputTarget::Srgb,
            metadata,
            (&path, Overwrite::Refuse),
            &mut late
        )
        .is_err()
    );
    assert!(late.requests > 0);
    assert_eq!(std::fs::read(&path).unwrap(), b"keep existing");
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn forecasted_files_preserve_exact_reconstruction_and_public_order() {
    let area = SourceRegion::from([17, 23, 1024, 768]);
    let snapshot = PhotoGradeSnapshot::new(Default::default(), vec![]).unwrap();
    let metadata = ProcessingMetadata::with_grade(RollSettings::new_grade().response, &snapshot);
    let directory =
        tempfile::tempdir_in(Path::new(env!("CARGO_MANIFEST_DIR")).join(".cache")).unwrap();
    for turns in [
        QuarterTurns::Zero,
        QuarterTurns::One,
        QuarterTurns::Two,
        QuarterTurns::Three,
    ] {
        for angle in [-0.13, 0.015] {
            for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
                let c = composition(
                    area,
                    turns,
                    Flips {
                        horizontal: true,
                        vertical: false,
                    },
                    angle,
                    [800, 576],
                );
                let run = |path: &Path, forecast: Option<Forecast>| {
                    let mut reads = 0;
                    let load = |origin: Origin| {
                        reads += 1;
                        Ok((0..128 * 128)
                            .map(|index| {
                                let x = f64::from(origin[0] - area.x + index % 128)
                                    / f64::from(area.width);
                                let y = f64::from(origin[1] - area.y + index / 128)
                                    / f64::from(area.height);
                                [0.05 + 0.8 * x, 0.1 + 0.7 * y, 0.15 + 0.3 * (x + y), 1.]
                            })
                            .collect::<Vec<_>>())
                    };
                    let check = || Ok(());
                    let mut reader = if let Some(forecast) = forecast {
                        JobReader::forecasted(load, check, forecast)
                    } else {
                        JobReader::new(load, check, true)
                    };
                    encode_output(
                        c,
                        Format::Tiff(target),
                        metadata,
                        (path, Overwrite::Refuse),
                        &mut reader,
                    )
                    .unwrap();
                    let requests = reader.requests;
                    let peak = reader.peak_bytes();
                    drop(reader);
                    (requests, reads, peak)
                };
                let baseline = directory.path().join("baseline");
                let candidate = directory.path().join("candidate");
                let before = run(&baseline, None);
                let after = run(&candidate, Forecast::new(c));
                assert_eq!(
                    std::fs::read(&baseline).unwrap(),
                    std::fs::read(&candidate).unwrap()
                );
                assert_eq!(before.0, after.0);
                assert!(after.2 <= cache::MAX_PAYLOAD_BYTES);
                for path in [baseline, candidate] {
                    std::fs::remove_file(path).unwrap();
                }
            }
        }
    }
}
