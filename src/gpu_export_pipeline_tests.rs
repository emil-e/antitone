use super::*;
use crate::{
    crop::Crop,
    geometry::{PhotoOrientation, QuarterTurns, RollOrientation},
};

type TestReader =
    JobReader<fn([u32; 2]) -> Result<Vec<[f64; 4]>, Error>, fn() -> Result<(), Error>>;
fn load(origin: [u32; 2]) -> Result<Vec<[f64; 4]>, Error> {
    Ok((0..TILE[0] * TILE[1])
        .map(|i| {
            let x = origin[0] + i % TILE[0];
            let y = origin[1] + i / TILE[0];
            [
                f64::from(x) / 2048.,
                f64::from(y) / 2048.,
                f64::from(x + y) / 4096.,
                1.,
            ]
        })
        .collect())
}
fn reader() -> TestReader {
    JobReader::new(load, || Ok(()), true)
}
fn composition(turns: QuarterTurns, angle: Option<f64>) -> Composition {
    Composition::with_crop(
        SourceRegion::from([0, 0, 513, 385]),
        RollOrientation {
            turns,
            ..Default::default()
        },
        PhotoOrientation::default(),
        angle.map(|angle| Crop {
            center: [256., 192.],
            size: [384, 256],
            angle,
        }),
    )
    .unwrap()
}
fn metadata() -> ProcessingMetadata {
    crate::processing::Response::film().into()
}

#[test]
fn tiff_pipeline_preserves_serial_files_cache_order_and_partial_blocks() {
    for turns in [
        QuarterTurns::Zero,
        QuarterTurns::One,
        QuarterTurns::Two,
        QuarterTurns::Three,
    ] {
        for angle in [None, Some(0.015), Some(-0.13)] {
            for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
                let c = composition(turns, angle);
                let dir = tempfile::tempdir().unwrap();
                let a = dir.path().join("serial.tif");
                let b = dir.path().join("pipeline.tif");
                let (mut serial, mut pipelined) = (reader(), reader());
                encode_with(
                    c,
                    Format::Tiff(target),
                    metadata(),
                    (&a, Overwrite::Refuse),
                    |o, e| reconstruct_reader(c, o, e, true, &mut serial),
                )
                .unwrap();
                encode(
                    c,
                    target,
                    metadata(),
                    (&b, Overwrite::Refuse),
                    &mut pipelined,
                )
                .unwrap();
                assert_eq!(std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap());
                assert_eq!(serial.requests, pipelined.requests);
                assert_eq!(serial.peak_bytes(), pipelined.peak_bytes());
            }
        }
    }
}

#[test]
fn producer_errors_panics_and_payload_rejection_preserve_destination_and_join() {
    let c = composition(QuarterTurns::One, Some(0.015));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("existing.tif");
    std::fs::write(&path, b"keep existing").unwrap();
    let mut seen = 0;
    let mut reads = reader();
    let result = encode_planned(
        c,
        OutputTarget::Srgb,
        metadata(),
        (&path, Overwrite::Confirmed),
        &mut reads,
        |o, e| {
            seen += 1;
            if seen == 2 {
                return Err(Error::InvalidParameter("injected geometry error"));
            }
            plan(c, o, e, true)
        },
    );
    assert_eq!(
        result,
        Err(Error::InvalidParameter("injected geometry error"))
    );
    assert!(reads.requests > 0);
    let mut seen = 0;
    let mut reads = reader();
    let result = encode_planned(
        c,
        OutputTarget::Srgb,
        metadata(),
        (&path, Overwrite::Confirmed),
        &mut reads,
        |o, e| {
            seen += 1;
            assert_ne!(seen, 2, "injected producer panic");
            plan(c, o, e, true)
        },
    );
    assert_eq!(
        result,
        Err(Error::Export("export geometry worker panicked".into()))
    );
    let mut reads = reader();
    let result = encode_planned(
        c,
        OutputTarget::Srgb,
        metadata(),
        (&path, Overwrite::Confirmed),
        &mut reads,
        |o, e| {
            let mut p = plan(c, o, e, true)?;
            p.samples = Vec::with_capacity(
                ReconstructionPlan::MAX_PAYLOAD_BYTES / std::mem::size_of::<(DVec3, f64)>() + 1,
            );
            Ok(p)
        },
    );
    assert_eq!(
        result,
        Err(Error::Export(
            "export geometry plan exceeds its payload bound".into()
        ))
    );
    assert_eq!(reads.requests, 0);
    assert_eq!(std::fs::read(&path).unwrap(), b"keep existing");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn consumer_failures_cancellation_and_panic_disconnect_before_joining() {
    let c = composition(QuarterTurns::One, Some(0.015));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("existing.tif");
    std::fs::write(&path, b"keep existing").unwrap();
    let mut failed = JobReader::new(
        |_| Err(Error::Gpu("injected read error".into())),
        || Ok(()),
        true,
    );
    assert_eq!(
        encode(
            c,
            OutputTarget::Srgb,
            metadata(),
            (&path, Overwrite::Confirmed),
            &mut failed
        ),
        Err(Error::Gpu("injected read error".into()))
    );
    let mut checks = 0;
    let mut canceled = JobReader::new(
        load,
        || {
            checks += 1;
            if checks > 5 {
                Err(Error::Export("injected cancellation".into()))
            } else {
                Ok(())
            }
        },
        true,
    );
    assert_eq!(
        encode(
            c,
            OutputTarget::Srgb,
            metadata(),
            (&path, Overwrite::Confirmed),
            &mut canceled
        ),
        Err(Error::Export("injected cancellation".into()))
    );
    let mut panicked = JobReader::new(
        |_| -> Result<Vec<[f64; 4]>, Error> { panic!("injected consumer panic") },
        || Ok(()),
        true,
    );
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| encode(
            c,
            OutputTarget::Srgb,
            metadata(),
            (&path, Overwrite::Confirmed),
            &mut panicked
        )))
        .is_err()
    );
    let mut early = reader();
    assert!(
        encode(
            c,
            OutputTarget::Srgb,
            metadata(),
            (&dir.path().join("missing/out.tif"), Overwrite::Refuse),
            &mut early
        )
        .is_err()
    );
    assert_eq!(early.requests, 0);
    let mut late = reader();
    assert!(
        encode(
            c,
            OutputTarget::Srgb,
            metadata(),
            (&path, Overwrite::Refuse),
            &mut late
        )
        .is_err()
    );
    assert!(late.requests > 0);
    assert_eq!(std::fs::read(&path).unwrap(), b"keep existing");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn full_resolution_plan_payload_bound_covers_rotation_edges_and_large_origins() {
    for base in [0u32, 2_147_000_000] {
        let area = SourceRegion::from([base, base, 4096, 4096]);
        for turns in [
            QuarterTurns::Zero,
            QuarterTurns::One,
            QuarterTurns::Two,
            QuarterTurns::Three,
        ] {
            for angle in [0., 1e-12, -1e-12, 0.015, -0.13, std::f64::consts::FRAC_PI_4] {
                let c = Composition::with_crop(
                    area,
                    RollOrientation {
                        turns,
                        horizontal: true,
                        ..Default::default()
                    },
                    PhotoOrientation::default(),
                    Some(Crop {
                        center: [f64::from(base) + 2048.5; 2],
                        size: [2049, 1537],
                        angle,
                    }),
                )
                .unwrap();
                let [w, h] = c.size();
                for origin in [
                    [0, 0],
                    [w - 1, h - 1],
                    [w - 128, 0],
                    [0, h - 128],
                    [w / 2, h / 2],
                ] {
                    let extent = [128.min(w - origin[0]), 128.min(h - origin[1])];
                    let p = plan(c, origin, extent, true).unwrap();
                    assert!(p.samples.len() <= extent[0] as usize * extent[1] as usize * 16);
                    assert!(p.payload_bytes().unwrap() <= ReconstructionPlan::MAX_PAYLOAD_BYTES);
                    p.admitted().unwrap();
                }
            }
        }
    }
}
