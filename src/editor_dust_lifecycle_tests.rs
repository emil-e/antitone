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

fn gpu_editor() -> (Editor, mpsc::Sender<Message>, mpsc::Receiver<Command>) {
    let (mut e, send, commands) = editor();
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, _) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    e.working = WorkingView::new(device);
    (e, send, commands)
}
fn high_response(e: &mut Editor) -> Response {
    let request = Request {
        request: e.request_id(),
        loaded: e.id,
        source: e.current_source.unwrap(),
        signature: e.dust_signature().unwrap(),
        repair: e.dust.repairs(e.current_source.unwrap()).unwrap().repairs()[0].clone(),
        calibration: e.calibration().unwrap(),
        roll: e.roll.inversion_only(),
    };
    let evaluated = Arc::new(
        crate::dust_repair::evaluate(&request.repair, e.ready.as_ref().unwrap().area, |_| {
            Ok([1e35; 3])
        })
        .unwrap(),
    );
    e.dust.start(&request);
    Response {
        request: request.request,
        loaded: request.loaded,
        source: request.source,
        signature: request.signature,
        repair: request.repair,
        original: vec![[1e35; 3]; evaluated.pixels().count()],
        evaluated,
    }
}

#[test]
fn grade_range_rejection_retains_neutral_evaluation_and_readmits_without_rehealing() {
    let (mut e, _, commands) = gpu_editor();
    e.photo.exposure = 16.;
    let response = high_response(&mut e);
    let evaluated = response.evaluated.clone();
    e.receive_dust(response.request, Ok(Some(response)));
    assert!(e.dust.failed.is_empty());
    assert!(e.dust_gpu_snapshot().unwrap().0.is_empty());
    assert!(e.dust.render_error.is_some());
    e.photo.exposure = 0.;
    e.dust_validate_processing(e.calibration().unwrap(), e.roll, e.photo)
        .unwrap();
    let snapshot = e.dust_gpu_snapshot().unwrap();
    assert_eq!(snapshot.0.len(), 1);
    assert!(Arc::ptr_eq(&snapshot.0[0].evaluated, &evaluated));
    assert!(e.dust.render_error.is_none());
    e.schedule_dust(e.dust_signature().unwrap());
    assert!(commands.try_recv().is_err());
}

#[test]
fn photo_widget_rejects_an_unsafe_repaired_value_before_accepting_the_grade() {
    let (mut e, _, _commands) = gpu_editor();
    let response = high_response(&mut e);
    e.receive_dust(response.request, Ok(Some(response)));
    assert_eq!(e.dust_gpu_snapshot().unwrap().0.len(), 1);
    for section in ["photo-white-balance", "photo-contrast", "photo-saturation"] {
        e.preferences.sections.insert(section.into(), false);
    }
    let ctx = egui::Context::default();
    for _ in 0..4 {
        super::super::tests::app_frame(&mut e, &ctx, vec![]);
    }
    let labels = super::super::tests::app_frame(&mut e, &ctx, vec![]);
    let heading = labels.iter().find(|(s, _)| s == "Exposure").unwrap().1;
    let point = labels
        .iter()
        .filter(|(s, r)| s == "0.00" && r.top() >= heading.top() && r.left() >= heading.left())
        .min_by(|(_, a), (_, b)| a.top().total_cmp(&b.top()))
        .unwrap()
        .1
        .center();
    for pressed in [true, false] {
        super::super::tests::app_frame(
            &mut e,
            &ctx,
            vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    assert!(ctx.text_edit_focused());
    super::super::tests::app_frame(
        &mut e,
        &ctx,
        vec![
            egui::Event::Text("16".into()),
            egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    assert_eq!(e.photo.exposure, 0.);
    assert!(e.status.starts_with("Edit rejected:"), "{}", e.status);
    assert_eq!(e.dust_gpu_snapshot().unwrap().0.len(), 1);
}

#[test]
fn disconnected_automatic_donors_use_their_own_supports_offsets_and_drag_origin() {
    let (mut e, _, _) = editor();
    let source = e.current_source.unwrap();
    let support = SupportMask::new(
        [50, 50],
        [11, 3],
        (0..33).map(|i| i % 11 < 3 || i % 11 >= 8).collect(),
    )
    .unwrap();
    e.dust
        .repairs_mut(source)
        .set_support(RepairId(1), support.clone())
        .unwrap();
    e.dust
        .repairs_mut(source)
        .set_donor(RepairId(1), DonorChoice::Automatic)
        .unwrap();
    e.begin_dust_cleanup();
    let repair = e.dust.repairs(source).unwrap().repairs()[0].clone();
    let request = Request {
        request: 1,
        loaded: e.id,
        source,
        signature: e.dust_signature().unwrap(),
        repair: repair.clone(),
        calibration: e.calibration().unwrap(),
        roll: e.roll,
    };
    // Constant input permits distinct, equally valid donors; author their resolved geometry.
    let mut evaluated = crate::dust_repair::evaluate(
        &Repair {
            donor: DonorChoice::Offset([30, 0]),
            ..repair
        },
        e.ready.as_ref().unwrap().area,
        |_| Ok([0.1, 0.2, 2.]),
    )
    .unwrap();
    assert_eq!(evaluated.donors.len(), 2);
    for (matched, offset) in evaluated.donors.iter_mut().zip([[40, 30], [-20, 40]]) {
        matched.source[0] = (matched.target[0] as i32 + offset[0]) as u32;
        matched.source[1] = (matched.target[1] as i32 + offset[1]) as u32;
    }
    let original = vec![[0.1, 0.2, 2.]; evaluated.pixels().count()];
    e.dust.start(&request);
    e.receive_dust(
        request.request,
        Ok(Some(Response {
            request: request.request,
            loaded: request.loaded,
            source,
            signature: request.signature,
            repair: request.repair,
            evaluated: Arc::new(evaluated),
            original,
        })),
    );
    let regions = e.dust_donor_regions();
    assert_eq!(regions.len(), 2);
    assert_eq!(regions[0].support.selected_points().count(), 9);
    assert_eq!(regions[1].support.selected_points().count(), 9);
    assert!(regions[0].support.contains([50, 50]) && !regions[0].support.contains([58, 50]));
    assert!(regions[1].support.contains([58, 50]) && !regions[1].support.contains([50, 50]));
    let composition = Composition::new(
        e.ready.as_ref().unwrap().area,
        Default::default(),
        Default::default(),
    )
    .unwrap();
    let geometry = e.viewport.layout_composition(
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(864., 576.)),
        composition,
        1.,
    );
    let point = screen_source_point(geometry, [38.5, 90.5]);
    let hit = e.dust_donor_hit(geometry, point).unwrap();
    assert!(hit.support.contains([58, 50]));
    e.begin_dust_donor_drag(geometry, point);
    assert_eq!(e.dust.donor_draft.unwrap().offset, [-20, 40]);
    e.drag_dust_donor(geometry, screen_source_point(geometry, [40.5, 91.5]));
    e.end_dust_donor_drag();
    assert_eq!(
        e.dust.repairs(source).unwrap().repairs()[0].donor,
        DonorChoice::Offset([-18, 41])
    );
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
