use super::*;

fn editor() -> (Editor, mpsc::Sender<Message>, mpsc::Receiver<Command>) {
    let (mut editor, send, commands) = super::super::tests::fixture();
    super::super::tests::add_ready_fixture(&mut editor);
    editor.source = false;
    editor.tab = 1;
    let source = editor.current_source.unwrap();
    editor
        .dust
        .repairs_mut(source)
        .insert(
            SupportMask::new([50, 50], [3, 3], vec![true; 9]).unwrap(),
            RepairOrigin::Manual,
            DonorChoice::Offset([30, 0]),
        )
        .unwrap();
    (editor, send, commands)
}
fn response(request: &Request, active: SourceRegion) -> Response {
    let evaluated = Arc::new(
        crate::dust_repair::evaluate(&request.repair, active, |_| Ok([0.1, 0.2, 2.])).unwrap(),
    );
    Response {
        request: request.request,
        loaded: request.loaded,
        source: request.source,
        signature: request.signature,
        repair: request.repair.clone(),
        original: vec![[0.1, 0.2, 2.]; evaluated.pixels().count()],
        evaluated,
    }
}

#[test]
fn corrected_repair_evaluation_uses_only_the_ungraded_positive_and_retains_signed_hdr() {
    let mut scan = Scan::open(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng"),
        None,
    )
    .unwrap();
    let area = scan.area;
    let field = Arc::new(
        crate::flat_field::FlatField::from_nodes(
            [area.x, area.y, area.width, area.height],
            [2, 2],
            vec![
                DVec3::new(1.2, 0.9, 1.1),
                DVec3::new(0.95, 1.3, 1.05),
                DVec3::new(1.1, 1.05, 0.88),
                DVec3::new(1.25, 0.98, 1.15),
            ],
        )
        .unwrap(),
    );
    scan.set_flat_field(Some(field)).unwrap();
    let calibration = Calibration {
        dmin: DVec3::splat(0.2),
        dmax: DVec3::splat(0.16),
        input_floor: crate::processing::CAMERA_INPUT_FLOOR,
    };
    let repair = Repair {
        id: RepairId(1),
        enabled: true,
        origin: RepairOrigin::Manual,
        // Independently include the authored Dmax and above-base patch centres.
        support: SupportMask::new(
            [94, 30],
            [68, 68],
            (0..68 * 68)
                .map(|i| {
                    let (x, y) = (i % 68, i / 68);
                    (x < 3 && y < 3) || (x >= 65 && y >= 65)
                })
                .collect(),
        )
        .unwrap(),
        donor: DonorChoice::Offset([-45, 25]),
    };
    let mut request = Request {
        request: 1,
        loaded: 1,
        source: SourceId(1),
        signature: [0; ProcessingParameters::BYTE_LEN],
        repair,
        calibration,
        roll: RollSettings {
            exposure: 3.,
            wb: Balance {
                warmth: 0.1,
                tint: -0.1,
            },
            ..RollSettings::new_grade()
        },
    };
    let result = evaluate(&scan, &request).unwrap();
    request.roll.exposure = -2.;
    request.roll.wb = Balance::default();
    let other = evaluate(&scan, &request).unwrap();
    assert_eq!(
        result.evaluated.pixels().collect::<Vec<_>>(),
        other.evaluated.pixels().collect::<Vec<_>>()
    );
    let pipeline = Pipeline::ungraded(
        scan.processing_calibration(calibration).unwrap(),
        request.roll,
        scan.camera_to_ap1,
    )
    .unwrap();
    for ((point, _), original) in result.evaluated.pixels().zip(&result.original) {
        assert_eq!(
            *original,
            pipeline
                .apply(scan.pixel(point[0], point[1]).unwrap())
                .unwrap()
                .rgb
                .as_vec3()
                .to_array()
        );
    }
    assert!(result.original.iter().flatten().any(|v| *v < 0.));
    assert!(result.original.iter().flatten().any(|v| *v > 1.));
}

#[test]
fn repair_identity_excludes_roll_photo_grades_but_tracks_negative_interpretation() {
    let (mut e, _, _) = editor();
    let before = e.dust_signature().unwrap();
    e.roll.exposure = 1.2;
    e.roll.wb = Balance {
        warmth: 0.1,
        tint: -0.1,
    };
    e.photo = PhotoSettings {
        exposure: 0.5,
        contrast: 1.2,
        saturation: 0.7,
        wb: Balance {
            warmth: -0.1,
            tint: 0.1,
        },
        ..Default::default()
    };
    e.target = OutputTarget::Srgb;
    e.viewport.zoom = Some(2.);
    assert_eq!(e.dust_signature(), Some(before));
    e.roll.gamma = 0.8;
    assert_ne!(e.dust_signature(), Some(before));
    e.roll.gamma = RollSettings::new_grade().gamma;
    assert_eq!(e.dust_signature(), Some(before));
    e.roll.balance.warmth = 0.05;
    assert_ne!(e.dust_signature(), Some(before));
}

#[test]
fn cancellation_and_obsolete_replies_neither_fail_current_repairs_nor_steal_admission() {
    let (mut e, _, commands) = editor();
    let signature = e.dust_signature().unwrap();
    let source = e.current_source.unwrap();
    e.schedule_dust(signature);
    let Command::Dust(first) = commands.try_recv().unwrap() else {
        panic!("expected dust")
    };
    e.receive_dust(first.request, Ok(None));
    assert!(e.dust.failed.is_empty());
    e.schedule_dust(signature);
    let Command::Dust(second) = commands.try_recv().unwrap() else {
        panic!("expected dust retry")
    };
    let active = e.ready.as_ref().unwrap().area;
    e.receive_dust(first.request, Ok(Some(response(&first, active))));
    assert_eq!(e.dust.pending.as_ref().unwrap().request, second.request);
    assert!(e.dust.cached.is_empty());
    e.roll.gamma = 0.8;
    e.receive_dust(second.request, Ok(Some(response(&second, active))));
    assert!(e.dust.pending.is_none());
    assert!(e.dust.cached.is_empty());
    assert!(e.dust.failed.is_empty());
    let signature = e.dust_signature().unwrap();
    e.schedule_dust(signature);
    let Command::Dust(third) = commands.try_recv().unwrap() else {
        panic!("expected new upstream state")
    };
    e.receive_dust(third.request, Ok(Some(response(&third, active))));
    assert!(e.dust.cached.contains_key(&(source, RepairId(1))));
    assert!(e.dust.pending.is_none());
}

#[test]
fn new_open_clear_repairs_only_after_success_and_late_work_cannot_restore_them() {
    for open in [false, true] {
        let (mut e, send, commands) = editor();
        let source = e.current_source.unwrap();
        let active = e.ready.as_ref().unwrap().area;
        e.schedule_dust(e.dust_signature().unwrap());
        let Command::Dust(request) = commands.try_recv().unwrap() else {
            panic!("expected dust")
        };
        let work = || {
            if open {
                DocumentWork::Open("missing.json".into())
            } else {
                DocumentWork::New
            }
        };
        e.start_document(work());
        let id = e.document_pending.unwrap();
        send.send(Message::Prepared(id, Err("not prepared".into())))
            .unwrap();
        e.receive();
        assert_eq!(e.dust.repairs(source).unwrap().repairs().len(), 1);
        e.start_document(work());
        let id = e.document_pending.unwrap();
        send.send(Message::Prepared(
            id,
            Ok(Box::new(Prepared {
                document: Document::default(),
                ready: None,
                selected: None,
                reuse_ready: false,
                correction: None,
                preserve_view: false,
            })),
        ))
        .unwrap();
        e.receive();
        assert!(e.dust.repairs(source).is_none());
        e.receive_dust(request.request, Ok(Some(response(&request, active))));
        assert!(e.dust.repairs(source).is_none());
        assert!(e.dust.cached.is_empty());
    }
}
