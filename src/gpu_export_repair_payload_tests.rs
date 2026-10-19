use super::*;

#[test]
fn dense_repaired_unit_accounts_host_and_gpu_uploads_separately_from_retained_results() {
    let (device, queue, renderer) = super::super::tests::context();
    let gate = SubmissionGate::default();
    let scan = Scan::open(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng"),
        None,
    )
    .unwrap();
    let a = scan.area;
    let composition = Composition::with_crop(
        a,
        Default::default(),
        Default::default(),
        Some(crate::crop::Crop {
            center: [
                f64::from(a.x) + f64::from(a.width) / 2.,
                f64::from(a.y) + f64::from(a.height) / 2.,
            ],
            size: [a.width - 64, a.height - 64],
            angle: 0.17,
        }),
    )
    .unwrap();
    let centre = composition
        .frame_to_source(crate::geometry::FramePoint([64., 64.]))
        .0
        .map(|v| v.floor() as u32);
    let repair = crate::dust_repair::Repair {
        id: crate::dust_repair::RepairId(1),
        enabled: true,
        origin: crate::dust_repair::RepairOrigin::Manual,
        support: crate::dust_repair::SupportMask::new(
            [centre[0] - 8, centre[1] - 8],
            [16, 16],
            vec![true; 256],
        )
        .unwrap(),
        donor: crate::dust_repair::DonorChoice::Offset([64, 0]),
    };
    let evaluated =
        Arc::new(crate::dust_repair::evaluate(&repair, a, |_| Ok([0.25, 0.5, 0.75])).unwrap());
    let unit = Unit::new(composition, [0, 0], [128, composition.size()[1].min(128)]).unwrap();
    assert!(evaluated.pixels().all(|(p, _)| p[0] >= unit.bounds.x
        && p[1] >= unit.bounds.y
        && p[0] < unit.bounds.x + unit.bounds.width
        && p[1] < unit.bounds.y + unit.bounds.height));
    let snapshot = crate::gpu::repairs::Snapshot::logical(vec![evaluated.clone()]).unwrap();
    let upload_bytes = 256 * 32;
    assert_eq!(
        snapshot.upload_scratch_bytes(unit.bounds),
        upload_bytes * 2,
        "simultaneous explicit host materialization and GPU sparse buffer"
    );
    assert!(snapshot.byte_len() >= evaluated.retained_bytes() as u64);
    let calibration = Calibration {
        dmin: DVec3::splat(0.8),
        dmax: DVec3::splat(0.02),
        input_floor: scan.input_floor,
    };
    let grade = PhotoGradeSnapshot::new(Default::default(), vec![]).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("not-opened.tif");
    let run = |repairs| {
        let request = Request {
            scan: &scan,
            field: None,
            calibration,
            roll: RollSettings::new_grade(),
            grade: grade.clone(),
            repairs,
            composition,
            format: Format::Tiff(OutputTarget::Srgb),
            look: None,
        };
        let (result, pixels) = crate::export_timing::pixels::capture(|| {
            write(
                &renderer,
                &device,
                (&queue, &gate),
                request,
                (&path, Overwrite::Refuse),
                || Ok(()),
            )
        });
        assert!(pixels.is_some());
        result.unwrap()
    };
    let plain = run(&[]);
    let repaired = run(&[evaluated]);
    assert_eq!(plain.peak_repair_upload_bytes, 0);
    assert_eq!(repaired.peak_repair_upload_bytes, upload_bytes * 2);
    assert_eq!(
        repaired.peak_unit_payload,
        plain.peak_unit_payload + upload_bytes * 2
    );
    assert_eq!(repaired.retained_repair_payload_bytes, snapshot.byte_len());
}
