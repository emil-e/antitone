use super::*;
use crate::geometry::QuarterTurns;

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
        wake: egui::Context::default(),
        cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
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
        evaluated,
    }
}

#[test]
fn held_dust_stroke_keeps_painting_past_512_source_pixels_until_primary_release() {
    let (mut e, _send, _commands) = editor();
    let source = e.current_source.unwrap();
    e.dust.clear_repairs(source);
    e.begin_dust_cleanup();
    e.dust.brush_size = 8;
    e.preferences.scopes_visible = false;
    let ctx = egui::Context::default();
    for _ in 0..3 {
        super::super::tests::app_frame(&mut e, &ctx, vec![]);
    }
    let project = e.document.project.clone();
    let layout = e.last_layout.unwrap().0;
    let start = layout.screen_point(egui::pos2(100., 200.));
    super::super::tests::app_frame(
        &mut e,
        &ctx,
        vec![
            egui::Event::PointerMoved(start),
            egui::Event::PointerButton {
                pos: start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    for x in (125..=750).step_by(25) {
        let point = layout.screen_point(egui::pos2(x as f32, 200.));
        super::super::tests::app_frame(&mut e, &ctx, vec![egui::Event::PointerMoved(point)]);
        assert!(ctx.input(|i| i.pointer.primary_down()));
        assert!(
            e.dust.mask_draft.is_some(),
            "lost held stroke at source x={x}: {}",
            e.status
        );
        assert!(e.dust.repairs(source).is_none());
    }
    assert!(
        e.dust
            .mask_draft
            .as_ref()
            .unwrap()
            .pixels
            .contains(&[750, 200]),
        "held stroke stopped early: {}",
        e.status
    );
    let end = layout.screen_point(egui::pos2(750., 200.));
    super::super::tests::app_frame(
        &mut e,
        &ctx,
        vec![egui::Event::PointerButton {
            pos: end,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    assert!(e.dust.mask_draft.is_none());
    let repairs = e.dust.repairs(source).unwrap().repairs();
    assert_eq!(repairs.len(), 1);
    assert!(repairs[0].support.contains([750, 200]));
    assert_eq!(e.document.project, project);
}

#[test]
fn mask_aspect_ratio_budget_preserves_original_storage_matching_and_automatic_work_bounds() {
    assert!(mask_within_budget([512, 512], MAX_MASK_PIXELS));
    assert!(mask_within_budget([700, 8], 5600));
    assert!(!mask_within_budget([513, 512], MAX_MASK_PIXELS));
    assert!(!mask_within_budget([8, 8], MAX_MASK_PIXELS + 1));
    assert!(!mask_within_budget([u32::MAX, u32::MAX], 1));
    let context_padding = 2 * u64::from(CONTEXT_PADDING);
    let padding = context_padding + 2 * u64::from(AUTOMATIC_PADDING);
    let max_width = MAX_MASK_WORK_PIXELS / (1 + padding) - padding;
    for width in 1..=max_width {
        let Some(height) = (MAX_MASK_WORK_PIXELS / (width + padding))
            .checked_sub(padding)
            .filter(|h| *h > 0)
        else {
            continue;
        };
        assert!(mask_within_budget([width as u32, height as u32], 1));
        assert!(width * height <= 512 * 512);
        assert!(
            (width + context_padding) * (height + context_padding)
                <= (512 + context_padding).pow(2)
        );
        assert!((width + padding) * (height + padding) <= MAX_MASK_WORK_PIXELS);
        const {
            assert!(MAX_MASK_WORK_PIXELS <= 4_000_000);
        }
    }
}

#[test]
fn over_budget_held_strokes_are_visible_and_discarded_without_partial_new_or_existing_repairs() {
    for edit_existing in [false, true] {
        let (mut e, _send, _commands) = editor();
        let source = e.current_source.unwrap();
        if !edit_existing {
            e.dust.clear_repairs(source);
        }
        e.begin_dust_cleanup();
        e.dust.brush_size = 64;
        e.preferences.scopes_visible = false;
        let before = e
            .dust
            .repairs(source)
            .map(|r| r.repairs().to_vec())
            .unwrap_or_default();
        let ctx = egui::Context::default();
        for _ in 0..3 {
            super::super::tests::app_frame(&mut e, &ctx, vec![]);
        }
        let project = e.document.project.clone();
        let layout = e.last_layout.unwrap().0;
        let modifiers = if edit_existing {
            egui::Modifiers::SHIFT
        } else {
            egui::Modifiers::NONE
        };
        let start = layout.screen_point(egui::pos2(200., 120.));
        super::super::tests::app_frame(
            &mut e,
            &ctx,
            vec![
                egui::Event::ModifiersChanged(modifiers),
                egui::Event::PointerMoved(start),
                egui::Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers,
                },
            ],
        );
        let mut end = start;
        'stroke: for row in 0..7 {
            for step in 0..=10 {
                let x = if row % 2 == 0 {
                    200 + step * 25
                } else {
                    450 - step * 25
                };
                end = layout.screen_point(egui::pos2(x as f32, (120 + row * 45) as f32));
                super::super::tests::app_frame(
                    &mut e,
                    &ctx,
                    vec![
                        egui::Event::ModifiersChanged(modifiers),
                        egui::Event::PointerMoved(end),
                    ],
                );
                assert!(ctx.input(|i| i.pointer.primary_down()));
                if e.dust
                    .mask_draft
                    .as_ref()
                    .is_some_and(|draft| draft.rejected)
                {
                    break 'stroke;
                }
            }
        }
        assert!(e.dust.mask_draft.as_ref().unwrap().rejected);
        let labels = super::super::tests::app_frame(
            &mut e,
            &ctx,
            vec![egui::Event::ModifiersChanged(modifiers)],
        );
        assert!(ctx.input(|i| i.pointer.primary_down()));
        assert!(
            labels
                .iter()
                .any(|(text, _)| text.contains("Stroke too large"))
        );
        assert_eq!(e.effective_dust_repairs(source), before);
        super::super::tests::app_frame(
            &mut e,
            &ctx,
            vec![egui::Event::PointerButton {
                pos: end,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers,
            }],
        );
        assert!(e.dust.mask_draft.is_none());
        assert_eq!(
            e.dust
                .repairs(source)
                .map(|r| r.repairs().to_vec())
                .unwrap_or_default(),
            before
        );
        assert!(e.status.contains("no changes applied"), "{}", e.status);
        assert_eq!(e.document.project, project);
    }
}

#[test]
fn dust_cursor_matches_selection_paint_and_donor_action_precedence() {
    let (mut e, _send, _commands) = editor();
    e.begin_dust_cleanup();
    let source = e.current_source.unwrap();
    let request = Request {
        request: 1,
        loaded: e.id,
        source,
        signature: e.dust_signature().unwrap(),
        repair: e.dust.repairs(source).unwrap().repairs()[0].clone(),
        wake: egui::Context::default(),
        cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        calibration: e.calibration().unwrap(),
        roll: e.roll.inversion_only(),
    };
    e.dust.start(&request);
    e.receive_dust(
        1,
        Ok(Some(response(&request, e.ready.as_ref().unwrap().area))),
    );
    let composition = Composition::new(
        e.ready.as_ref().unwrap().area,
        Default::default(),
        Default::default(),
    )
    .unwrap();
    let geometry = e.viewport.layout_composition(
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1728., 1152.)),
        composition,
        1.,
    );
    let target = screen_source_point(geometry, [51.5, 51.5]);
    let donor = screen_source_point(geometry, [81.5, 51.5]);
    let empty = screen_source_point(geometry, [20.5, 20.5]);
    let outline = screen_source_point(geometry, [53., 51.5]) + egui::vec2(4., 0.);
    assert_eq!(e.dust_repair_at(geometry, outline), Some(RepairId(1)));
    assert_eq!(
        e.dust_canvas_cursor(geometry, Some(target), false),
        egui::CursorIcon::PointingHand
    );
    assert_eq!(
        e.dust_canvas_cursor(geometry, Some(outline), false),
        egui::CursorIcon::PointingHand
    );
    assert_eq!(
        e.dust_canvas_cursor(geometry, Some(empty), false),
        egui::CursorIcon::Crosshair
    );
    assert_eq!(
        e.dust_canvas_cursor(geometry, Some(donor), false),
        egui::CursorIcon::Grab
    );
    for point in [target, outline, donor] {
        assert_eq!(
            e.dust_canvas_cursor(geometry, Some(point), true),
            egui::CursorIcon::Crosshair
        );
    }
    e.begin_dust_mask(None);
    assert_eq!(
        e.dust_canvas_cursor(geometry, Some(target), false),
        egui::CursorIcon::Crosshair
    );
    e.cancel_dust_mask();
    e.begin_dust_donor_drag(geometry, donor);
    assert!(e.dust_donor_dragging());
    assert_eq!(
        e.dust_canvas_cursor(geometry, Some(target), true),
        egui::CursorIcon::Grabbing
    );
    assert_eq!(
        e.dust_canvas_cursor(geometry, None, false),
        egui::CursorIcon::Grabbing
    );
    e.end_dust_donor_drag();
    let other = e
        .dust
        .repairs_mut(source)
        .insert(
            SupportMask::new([80, 50], [3, 3], vec![true; 9]).unwrap(),
            RepairOrigin::Manual,
            DonorChoice::Automatic,
        )
        .unwrap();
    e.dust
        .repairs_mut(source)
        .select(Some(RepairId(1)))
        .unwrap();
    assert!(e.dust_donor_at(geometry, donor));
    assert_eq!(e.dust_repair_at(geometry, donor), Some(other));
    assert_eq!(
        e.dust_canvas_cursor(geometry, Some(donor), false),
        egui::CursorIcon::PointingHand
    );
    e.dust.show_masks = false;
    assert_eq!(
        e.dust_canvas_cursor(geometry, Some(target), false),
        egui::CursorIcon::Crosshair
    );
}

#[test]
fn repair_outline_fast_rejection_keeps_exact_hits_under_rotation_and_zoom() {
    let (mut e, _send, _commands) = editor();
    let support = SupportMask::new(
        [50, 50],
        [7, 5],
        (0..35)
            .map(|i| i % 7 == 0 || i % 7 == 6 || i / 7 == 0 || i / 7 == 4)
            .collect(),
    )
    .unwrap();
    for turns in [
        QuarterTurns::Zero,
        QuarterTurns::One,
        QuarterTurns::Two,
        QuarterTurns::Three,
    ] {
        let composition = Composition::with_crop(
            e.ready.as_ref().unwrap().area,
            RollOrientation {
                turns,
                ..Default::default()
            },
            PhotoOrientation {
                flips: crate::geometry::Flips {
                    horizontal: true,
                    vertical: false,
                },
                ..Default::default()
            },
            Some(Crop {
                center: [110., 95.],
                size: [160, 120],
                angle: 0.17,
            }),
        )
        .unwrap();
        for zoom in [None, Some(2.)] {
            e.viewport.zoom = zoom;
            let geometry = e.viewport.layout_composition(
                egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900., 600.)),
                composition,
                1.,
            );
            let segments = support_boundary_segments(&support);
            for point in segments
                .iter()
                .flat_map(|segment| segment.iter())
                .map(|p| screen_source_point(geometry, *p))
            {
                for delta in [
                    egui::Vec2::ZERO,
                    egui::vec2(7.9, 0.),
                    egui::vec2(-7.9, 0.),
                    egui::vec2(0., 7.9),
                    egui::vec2(0., -7.9),
                    egui::vec2(30., 30.),
                ] {
                    let point = point + delta;
                    let exact = segments.iter().any(|segment| {
                        distance_to_segment(
                            point,
                            screen_source_point(geometry, segment[0]),
                            screen_source_point(geometry, segment[1]),
                        ) <= 8.
                    });
                    assert_eq!(repair_outline_hit(geometry, &support, point), exact);
                }
            }
        }
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
    assert!(e.dust_gpu_snapshot(None).unwrap().0.is_empty());
    assert!(e.dust.render_error.is_some());
    e.photo.exposure = 0.;
    e.dust_validate_processing(e.calibration().unwrap(), e.roll, e.photo)
        .unwrap();
    let snapshot = e.dust_gpu_snapshot(None).unwrap();
    assert_eq!(snapshot.0.len(), 1);
    assert!(Arc::ptr_eq(&snapshot.0[0].evaluated, &evaluated));
    assert!(e.dust.render_error.is_none());
    e.schedule_dust(e.dust_signature().unwrap());
    assert!(commands.try_recv().is_err());
}

#[test]
fn arriving_repairs_are_admitted_against_the_exact_view_grade_without_mutating_the_document() {
    let (mut e, _, commands) = gpu_editor();
    let before = e.document.project.clone();
    let response = high_response(&mut e);
    let evaluated = response.evaluated.clone();
    e.receive_dust(response.request, Ok(Some(response)));
    let candidate = PhotoGradeSnapshot::new(
        PhotoSettings {
            exposure: 16.,
            ..e.photo
        },
        Vec::new(),
    )
    .unwrap();
    assert!(e.dust_gpu_snapshot(Some(&candidate)).unwrap().0.is_empty());
    assert!(e.dust.render_error.is_some());
    let accepted = PhotoGradeSnapshot::new(e.photo, Vec::new()).unwrap();
    let snapshot = e.dust_gpu_snapshot(Some(&accepted)).unwrap();
    assert_eq!(snapshot.0.len(), 1);
    assert!(Arc::ptr_eq(&snapshot.0[0].evaluated, &evaluated));
    let mut source_candidate = e
        .document
        .project
        .source(e.current_source.unwrap())
        .unwrap()
        .clone();
    source_candidate.photo.exposure = 16.;
    assert!(e.validate_source_candidate(&source_candidate).is_err());
    assert_eq!(e.document.project, before);
    e.schedule_dust(e.dust_signature().unwrap());
    assert!(
        commands.try_recv().is_err(),
        "grade changes do not re-evaluate ungraded repairs"
    );
}

#[test]
fn photo_widget_rejects_an_unsafe_repaired_value_before_accepting_the_grade() {
    let (mut e, _, _commands) = gpu_editor();
    let response = high_response(&mut e);
    e.receive_dust(response.request, Ok(Some(response)));
    assert_eq!(e.dust_gpu_snapshot(None).unwrap().0.len(), 1);
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
    assert_eq!(e.dust_gpu_snapshot(None).unwrap().0.len(), 1);
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
        wake: egui::Context::default(),
        cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
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
        wake: egui::Context::default(),
        cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
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
    let expected = crate::dust_repair::evaluate(&request.repair, scan.area, |p| {
        Ok(pipeline
            .apply(scan.pixel(p[0], p[1])?)?
            .rgb
            .as_vec3()
            .to_array())
    })
    .unwrap();
    assert_eq!(
        result.evaluated.pixels().collect::<Vec<_>>(),
        expected.pixels().collect::<Vec<_>>()
    );
    let original = request
        .repair
        .support
        .selected_points()
        .map(|p| {
            pipeline
                .apply(scan.pixel(p[0], p[1]).unwrap())
                .unwrap()
                .rgb
                .as_vec3()
                .to_array()
        })
        .collect::<Vec<_>>();
    assert!(original.iter().flatten().any(|v| *v < 0.));
    assert!(original.iter().flatten().any(|v| *v > 1.));
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
fn obsolete_heal_cancels_without_releasing_admission_or_memoizing_failure() {
    for upstream in [false, true] {
        let (mut e, _, commands) = editor();
        let signature = e.dust_signature().unwrap();
        e.schedule_dust(signature);
        let Command::Dust(first) = commands.try_recv().unwrap() else {
            panic!("expected dust")
        };
        if upstream {
            e.roll.gamma = 0.8;
        } else {
            let source = e.current_source.unwrap();
            e.dust
                .repairs_mut(source)
                .set_donor(RepairId(1), DonorChoice::Offset([31, 0]))
                .unwrap();
        }
        let next_signature = e.dust_signature().unwrap();
        e.schedule_dust(next_signature);
        assert!(first.cancelled.load(std::sync::atomic::Ordering::Relaxed));
        assert_eq!(e.dust.pending.as_ref().unwrap().request, first.request);
        assert!(commands.try_recv().is_err());
        // A genuine backend error arriving after retirement is not an unchanged failure.
        e.receive_dust(first.request, Err("obsolete failure".into()));
        assert!(e.dust.pending.is_none() && e.dust.failed.is_empty());
        e.schedule_dust(next_signature);
        let Command::Dust(second) = commands.try_recv().unwrap() else {
            panic!("expected retry")
        };
        assert!(!second.cancelled.load(std::sync::atomic::Ordering::Relaxed));
        e.dust.clear_repairs(e.current_source.unwrap());
        assert!(second.cancelled.load(std::sync::atomic::Ordering::Relaxed));
    }
}

#[test]
fn navigation_and_downstream_grades_do_not_cancel_an_admitted_heal() {
    let (mut e, _, commands) = editor();
    e.schedule_dust(e.dust_signature().unwrap());
    let Command::Dust(request) = commands.try_recv().unwrap() else {
        panic!("expected dust")
    };
    e.viewport.zoom = Some(2.);
    e.photo.exposure = 1.;
    e.roll.exposure = 2.;
    e.schedule_dust(e.dust_signature().unwrap());
    assert!(!request.cancelled.load(std::sync::atomic::Ordering::Relaxed));
    assert_eq!(e.dust.pending.as_ref().unwrap().request, request.request);
    e.dust = State::default();
    assert!(request.cancelled.load(std::sync::atomic::Ordering::Relaxed));
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
