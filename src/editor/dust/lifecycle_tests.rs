use super::*;
use crate::{geometry::QuarterTurns, processing::InputPipeline};

mod persistence;

mod acceptance;

fn frame(
    editor: &mut Editor,
    context: &egui::Context,
    events: Vec<egui::Event>,
    cx: &UiContext<'_, Editor>,
) -> Vec<(String, egui::Rect)> {
    super::super::presentation::configure_fonts(context);
    context.enable_accesskit();
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1100., 800.),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            egui::CentralPanel::default().show(ui, |ui| editor.draw_ui(ui, cx));
        },
    );
    output.textures_delta.clear();
    super::super::tests::ui_labels(output)
}

fn editor() -> Editor {
    let mut editor = super::super::tests::fixture();
    super::super::tests::add_ready_fixture(&mut editor);
    editor.source = false;
    editor.tab = 1;
    let source = editor.current_source.unwrap();
    editor
        .dust_repairs_mut(source)
        .insert(
            SupportMask::new([50, 50], [3, 3], vec![true; 9]).unwrap(),
            RepairOrigin::Manual,
            [30, 0],
        )
        .unwrap();
    editor.select_dust_repair(RepairId(1));
    editor
}

fn captured_request(e: &Editor) -> Request {
    e.document
        .dust
        .pending
        .as_ref()
        .expect("pending evaluation")
        .evaluation
        .clone()
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

// Geometry/controller fixtures have synthetic display geometry. Resolve their
// queued searches geometrically; real source acquisition has separate workflow cases.
fn finish_searches(e: &mut Editor) {
    for request in e.dust_search_requests() {
        let result = crate::dust_repair::search_donor(
            &request.support,
            request.previous.as_ref().map(|r| r.offset),
            request.development.active_area.into(),
            &mut |_| panic!("expired geometry fixture must not read pixels"),
            std::time::Instant::now(),
            0.,
            &mut crate::dust_repair::Execution::default(),
        )
        .unwrap();
        e.receive_dust_search(request.id, Ok(result));
    }
}

fn gpu_editor() -> Editor {
    let mut e = editor();
    let instance = crate::gpu::instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    e.working = WorkingView::new(device, queue);
    e
}

fn high_response(e: &mut Editor) -> Response {
    let request = Request {
        request: e.request_id(),
        cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        loaded: e.id,
        source: e.current_source.unwrap(),
        signature: e.dust_signature().unwrap(),
        repair: e.dust_repairs(e.current_source.unwrap()).unwrap().repairs()[0].clone(),
        input: e.calibration().unwrap().into(),
    };
    let evaluated = Arc::new(
        crate::dust_repair::evaluate(
            &request.repair,
            e.document.visit.ready.as_ref().unwrap().area,
            |_| Ok([1e35; 3]),
        )
        .unwrap(),
    );
    e.document.dust.start(&request);
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
    let e = editor();
    crate::application::test_support::with_application(e, |app| {
        app.enter(|e, cx| {
            let source = e.current_source.unwrap();
            e.reset_dust_repairs(source);
            e.begin_dust_cleanup();
            e.document.dust.brush_size = 8;
            e.preferences.scopes_visible = false;
            let ctx = egui::Context::default();
            for _ in 0..3 {
                frame(e, &ctx, vec![], cx);
            }
            let project = e.document.project.clone();
            let layout = e.last_layout.unwrap().0;
            let start = layout.screen_point(egui::pos2(100., 200.));
            frame(
                e,
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
                cx,
            );
            for x in (125..=750).step_by(25) {
                let point = layout.screen_point(egui::pos2(x as f32, 200.));
                frame(e, &ctx, vec![egui::Event::PointerMoved(point)], cx);
                assert!(ctx.input(|i| i.pointer.primary_down()));
                assert!(
                    e.document.dust.mask_draft.is_some(),
                    "lost held stroke at source x={x}: {}",
                    e.status
                );
                assert!(e.dust_repairs(source).is_none());
                assert_eq!(e.document.project, project);
            }
            assert!(
                e.document
                    .dust
                    .mask_draft
                    .as_ref()
                    .unwrap()
                    .pixels
                    .contains(&[750, 200]),
                "held stroke stopped early: {}",
                e.status
            );
            let end = layout.screen_point(egui::pos2(750., 200.));
            frame(
                e,
                &ctx,
                vec![egui::Event::PointerButton {
                    pos: end,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                cx,
            );
            assert!(e.document.dust.mask_draft.is_none());
            assert!(e.dust_repairs(source).is_none());
            assert!(e.dust_search_pending());
            finish_searches(e);
            let repairs = e.dust_repairs(source).unwrap().repairs();
            assert_eq!(repairs.len(), 1);
            assert!(repairs[0].support.contains([750, 200]));
            assert_ne!(e.document.project, project);
            assert!(e.document.dirty());
        });
    });
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
        let e = editor();
        crate::application::test_support::with_application(e, |app| {
            app.enter(|e, cx| {
                let source = e.current_source.unwrap();
                if !edit_existing {
                    e.reset_dust_repairs(source);
                }
                e.begin_dust_cleanup();
                e.document.dust.brush_size = 64;
                e.preferences.scopes_visible = false;
                let before = e
                    .dust_repairs(source)
                    .map(|r| r.repairs().to_vec())
                    .unwrap_or_default();
                let ctx = egui::Context::default();
                for _ in 0..3 {
                    frame(e, &ctx, vec![], cx);
                }
                let project = e.document.project.clone();
                let layout = e.last_layout.unwrap().0;
                let modifiers = if edit_existing {
                    egui::Modifiers::SHIFT
                } else {
                    egui::Modifiers::NONE
                };
                let start = layout.screen_point(egui::pos2(200., 120.));
                frame(
                    e,
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
                    cx,
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
                        frame(
                            e,
                            &ctx,
                            vec![
                                egui::Event::ModifiersChanged(modifiers),
                                egui::Event::PointerMoved(end),
                            ],
                            cx,
                        );
                        assert!(ctx.input(|i| i.pointer.primary_down()));
                        if e.document
                            .dust
                            .mask_draft
                            .as_ref()
                            .is_some_and(|draft| draft.rejected)
                        {
                            break 'stroke;
                        }
                    }
                }
                assert!(e.document.dust.mask_draft.as_ref().unwrap().rejected);
                let labels = frame(e, &ctx, vec![egui::Event::ModifiersChanged(modifiers)], cx);
                assert!(ctx.input(|i| i.pointer.primary_down()));
                assert!(
                    labels
                        .iter()
                        .any(|(text, _)| text.contains("Stroke too large"))
                );
                assert_eq!(e.effective_dust_repairs(source), before);
                frame(
                    e,
                    &ctx,
                    vec![egui::Event::PointerButton {
                        pos: end,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers,
                    }],
                    cx,
                );
                assert!(e.document.dust.mask_draft.is_none());
                assert_eq!(
                    e.dust_repairs(source)
                        .map(|r| r.repairs().to_vec())
                        .unwrap_or_default(),
                    before
                );
                assert!(e.status.contains("no changes applied"), "{}", e.status);
                assert_eq!(e.document.project, project);
            });
        });
    }
}

#[test]
fn dust_cursor_matches_selection_paint_and_donor_action_precedence() {
    let mut e = editor();
    e.begin_dust_cleanup();
    let source = e.current_source.unwrap();
    let request = Request {
        request: 1,
        loaded: e.id,
        source,
        signature: e.dust_signature().unwrap(),
        repair: e.dust_repairs(source).unwrap().repairs()[0].clone(),
        cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        input: e.calibration().unwrap().into(),
    };
    e.document.dust.start(&request);
    e.receive_dust(
        1,
        Ok(Some(response(
            &request,
            e.document.visit.ready.as_ref().unwrap().area,
        ))),
    );
    let composition = Composition::new(
        e.document.visit.ready.as_ref().unwrap().area,
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
        .dust_repairs_mut(source)
        .insert(
            SupportMask::new([80, 50], [3, 3], vec![true; 9]).unwrap(),
            RepairOrigin::Manual,
            [64, 0],
        )
        .unwrap();
    e.select_dust_repair(RepairId(1));
    assert!(e.dust_donor_at(geometry, donor));
    assert_eq!(e.dust_repair_at(geometry, donor), Some(other));
    assert_eq!(
        e.dust_canvas_cursor(geometry, Some(donor), false),
        egui::CursorIcon::PointingHand
    );
    e.document.dust.show_masks = false;
    assert_eq!(
        e.dust_canvas_cursor(geometry, Some(target), false),
        egui::CursorIcon::Crosshair
    );
}

#[test]
fn repair_outline_fast_rejection_keeps_exact_hits_under_rotation_and_zoom() {
    let mut e = editor();
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
            e.document.visit.ready.as_ref().unwrap().area,
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
fn film_settings_do_not_change_neutral_repair_admission_or_trigger_rehealing() {
    let e = gpu_editor();
    crate::application::test_support::with_application(e, |app| {
        app.enter(|e, cx| {
            let response = high_response(e);
            let evaluated = response.evaluated.clone();
            e.receive_dust(response.request, Ok(Some(response)));
            let previous = e.dust_gpu_snapshot(None).unwrap();
            assert_eq!(previous.0.len(), 1);
            let controls = crate::editor::editing::ControlSettings::capture(e);
            e.film_emulation = FilmEmulationPreset::Medium.settings();
            e.validate_frame_edits(controls);
            assert!(e.film_emulation.enabled);
            let current = e.dust_gpu_snapshot(None).unwrap();
            assert_eq!(current.0.len(), 1);
            assert!(Arc::ptr_eq(&current.0[0].evaluated, &evaluated));
            e.schedule_dust(e.dust_signature().unwrap(), cx);
            assert!(e.document.dust.pending_request().is_none());
        });
    });
}

#[test]
fn grade_range_rejection_retains_neutral_evaluation_and_readmits_without_rehealing() {
    let e = gpu_editor();
    crate::application::test_support::with_application(e, |app| {
        app.enter(|e, cx| {
            e.adjustment_layer_settings.exposure = 16.;
            let response = high_response(e);
            let evaluated = response.evaluated.clone();
            e.receive_dust(response.request, Ok(Some(response)));
            assert!(e.document.dust.failed.is_empty());
            assert!(e.dust_gpu_snapshot(None).unwrap().0.is_empty());
            assert!(e.document.dust.render_error.is_some());
            e.adjustment_layer_settings.exposure = 0.;
            e.dust_validate_processing(
                e.calibration().unwrap(),
                e.roll,
                e.adjustment_layer_settings,
            )
            .unwrap();
            let snapshot = e.dust_gpu_snapshot(None).unwrap();
            assert_eq!(snapshot.0.len(), 1);
            assert!(Arc::ptr_eq(&snapshot.0[0].evaluated, &evaluated));
            assert!(e.document.dust.render_error.is_none());
            e.schedule_dust(e.dust_signature().unwrap(), cx);
            assert!(e.document.dust.pending_request().is_none());
        });
    });
}

#[test]
fn arriving_repairs_are_admitted_against_the_exact_view_grade_without_mutating_the_document() {
    let e = gpu_editor();
    crate::application::test_support::with_application(e, |app| {
        app.enter(|e, cx| {
            let before = e.document.project.clone();
            let response = high_response(e);
            let evaluated = response.evaluated.clone();
            e.receive_dust(response.request, Ok(Some(response)));
            let candidate = AdjustmentLayerSnapshot::new(
                std::iter::once(crate::adjustment_layers::AdjustmentLayer::global(
                    AdjustmentLayerSettings {
                        exposure: 16.,
                        ..e.adjustment_layer_settings
                    },
                ))
                .chain(Vec::new())
                .collect(),
            )
            .unwrap();
            assert!(e.dust_gpu_snapshot(Some(&candidate)).unwrap().0.is_empty());
            assert!(e.document.dust.render_error.is_some());
            let accepted = AdjustmentLayerSnapshot::new(
                std::iter::once(crate::adjustment_layers::AdjustmentLayer::global(
                    e.adjustment_layer_settings,
                ))
                .chain(Vec::new())
                .collect(),
            )
            .unwrap();
            let snapshot = e.dust_gpu_snapshot(Some(&accepted)).unwrap();
            assert_eq!(snapshot.0.len(), 1);
            assert!(Arc::ptr_eq(&snapshot.0[0].evaluated, &evaluated));
            let mut source_candidate = e
                .document
                .project
                .source(e.current_source.unwrap())
                .unwrap()
                .clone();
            source_candidate.adjustment_layers.as_mut().unwrap().layers[0]
                .adjustment
                .photo
                .exposure = 16.;
            assert!(e.validate_source_candidate(&source_candidate).is_err());
            assert_eq!(e.document.project, before);
            e.schedule_dust(e.dust_signature().unwrap(), cx);
            assert!(
                e.document.dust.pending_request().is_none(),
                "grade changes do not re-evaluate ungraded repairs"
            );
        });
    });
}

#[test]
fn photo_widget_rejects_an_unsafe_repaired_value_before_accepting_the_grade() {
    let e = gpu_editor();
    crate::application::test_support::with_application(e, |app| {
        app.enter(|e, cx| {
            let response = high_response(e);
            e.receive_dust(response.request, Ok(Some(response)));
            assert_eq!(e.dust_gpu_snapshot(None).unwrap().0.len(), 1);
            for section in ["photo-white-balance", "photo-contrast", "photo-saturation"] {
                e.preferences.sections.insert(section.into(), false);
            }
            let ctx = egui::Context::default();
            for _ in 0..4 {
                frame(e, &ctx, vec![], cx);
            }
            let labels = frame(e, &ctx, vec![], cx);
            let heading = labels.iter().find(|(s, _)| s == "Exposure").unwrap().1;
            let point = labels
                .iter()
                .filter(|(s, r)| {
                    s == "0.00" && r.top() >= heading.top() && r.left() >= heading.left()
                })
                .min_by(|(_, a), (_, b)| a.top().total_cmp(&b.top()))
                .unwrap()
                .1
                .center();
            for pressed in [true, false] {
                frame(
                    e,
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
                    cx,
                );
            }
            assert!(ctx.text_edit_focused());
            frame(
                e,
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
                cx,
            );
            assert_eq!(e.adjustment_layer_settings.exposure, 0.);
            assert!(e.status.contains("rejected:"), "{}", e.status);
            assert_eq!(e.dust_gpu_snapshot(None).unwrap().0.len(), 1);
        });
    });
}

#[test]
fn disconnected_supports_share_the_stored_translation_before_evaluation_and_after_drag() {
    let mut e = editor();
    let source = e.current_source.unwrap();
    let support = SupportMask::new(
        [50, 50],
        [31, 3],
        (0..93).map(|i| i % 31 < 3 || i % 31 >= 28).collect(),
    )
    .unwrap();
    e.dust_repairs_mut(source)
        .set_support(RepairId(1), support.clone())
        .unwrap();
    e.dust_repairs_mut(source)
        .set_offset(RepairId(1), [64, 0])
        .unwrap();
    e.begin_dust_cleanup();
    assert!(e.document.dust.cached.is_empty());
    let regions = e.dust_donor_regions();
    let presented = regions
        .iter()
        .flat_map(|region| region.support.selected_points())
        .collect::<BTreeSet<_>>();
    assert_eq!(presented, support.selected_points().collect());
    let composition = Composition::new(
        e.document.visit.ready.as_ref().unwrap().area,
        Default::default(),
        Default::default(),
    )
    .unwrap();
    let geometry = e.viewport.layout_composition(
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(864., 576.)),
        composition,
        1.,
    );
    for region in &regions {
        assert_eq!(region.donor.x as i32 - region.target.x as i32, 64);
        assert_eq!(region.donor.y, region.target.y);
    }
    let point = screen_source_point(geometry, [143., 51.]);
    let hit = e.dust_donor_hit(geometry, point).unwrap();
    assert!(hit.support.contains([78, 50]));
    e.begin_dust_donor_drag(geometry, point);
    assert_eq!(e.document.dust.donor_draft.unwrap().offset, [64, 0]);
    e.drag_dust_donor(geometry, screen_source_point(geometry, [145., 52.]));
    e.end_dust_donor_drag();
    assert_eq!(e.dust_repairs(source).unwrap().repairs()[0].offset, [66, 1]);
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
        offset: [-45, 25],
    };
    let request = Request {
        request: 1,
        loaded: 1,
        source: SourceId(1),
        cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        signature: Default::default(),
        repair,
        input: calibration.into(),
    };
    let result = evaluate(&scan, &request).unwrap();
    let other = evaluate(&scan, &request).unwrap();
    assert_eq!(
        result.evaluated.pixels().collect::<Vec<_>>(),
        other.evaluated.pixels().collect::<Vec<_>>()
    );
    let pipeline = InputPipeline::new(request.input, scan.camera_to_ap1).unwrap();
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
    let mut e = editor();
    let before = e.dust_signature().unwrap();
    e.roll.exposure = 1.2;
    e.roll.wb = Balance {
        warmth: 0.1,
        tint: -0.1,
    };
    e.adjustment_layer_settings = AdjustmentLayerSettings {
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
    e.set_gamma(0.8);
    assert_ne!(e.dust_signature(), Some(before));
    e.set_gamma(crate::processing::DEFAULT_GAMMA);
    assert_eq!(e.dust_signature(), Some(before));
    let mut balance = e.balance();
    balance.warmth = 0.05;
    e.set_balance(balance);
    assert_ne!(e.dust_signature(), Some(before));
}

#[test]
fn obsolete_heal_cancels_without_releasing_admission_or_memoizing_failure() {
    for upstream in [false, true] {
        let e = editor();
        crate::application::test_support::with_application(e, |app| {
            app.enter(|e, cx| {
                let signature = e.dust_signature().unwrap();
                e.schedule_dust(signature, cx);
                let first = captured_request(e);
                if upstream {
                    e.set_gamma(0.8);
                } else {
                    let source = e.current_source.unwrap();
                    e.dust_repairs_mut(source)
                        .set_offset(RepairId(1), [31, 0])
                        .unwrap();
                }
                let next_signature = e.dust_signature().unwrap();
                e.schedule_dust(next_signature, cx);
                assert!(first.cancelled.load(std::sync::atomic::Ordering::Relaxed));
                assert_eq!(
                    e.document.dust.pending.as_ref().unwrap().evaluation.request,
                    first.request
                );
                // A genuine backend error arriving after retirement is not an unchanged failure.
                e.receive_dust(first.request, Err("obsolete failure".into()));
                assert!(e.document.dust.pending.is_none() && e.document.dust.failed.is_empty());
                e.schedule_dust(next_signature, cx);
                let second = captured_request(e);
                assert!(!second.cancelled.load(std::sync::atomic::Ordering::Relaxed));
                e.reset_dust_repairs(e.current_source.unwrap());
                assert!(second.cancelled.load(std::sync::atomic::Ordering::Relaxed));
            });
        });
    }
}

#[test]
fn navigation_and_downstream_grades_do_not_cancel_an_admitted_heal() {
    let e = editor();
    crate::application::test_support::with_application(e, |app| {
        app.enter(|e, cx| {
            e.schedule_dust(e.dust_signature().unwrap(), cx);
            let request = captured_request(e);
            e.viewport.zoom = Some(2.);
            e.adjustment_layer_settings.exposure = 1.;
            e.roll.exposure = 2.;
            e.schedule_dust(e.dust_signature().unwrap(), cx);
            assert!(!request.cancelled.load(std::sync::atomic::Ordering::Relaxed));
            assert_eq!(
                e.document.dust.pending.as_ref().unwrap().evaluation.request,
                request.request
            );
            e.document.dust = State::default();
            assert!(request.cancelled.load(std::sync::atomic::Ordering::Relaxed));
        });
    });
}

#[test]
fn cancellation_and_obsolete_replies_neither_fail_current_repairs_nor_steal_admission() {
    let e = editor();
    crate::application::test_support::with_application(e, |app| {
        app.enter(|e, cx| {
            let signature = e.dust_signature().unwrap();
            let source = e.current_source.unwrap();
            e.schedule_dust(signature, cx);
            let first = captured_request(e);
            e.receive_dust(first.request, Ok(None));
            assert!(e.document.dust.failed.is_empty());
            e.schedule_dust(signature, cx);
            let second = captured_request(e);
            let active = e.document.visit.ready.as_ref().unwrap().area;
            e.receive_dust(first.request, Ok(Some(response(&first, active))));
            assert_eq!(
                e.document.dust.pending.as_ref().unwrap().evaluation.request,
                second.request
            );
            assert!(e.document.dust.cached.is_empty());
            e.set_gamma(0.8);
            e.receive_dust(second.request, Ok(Some(response(&second, active))));
            assert!(e.document.dust.pending.is_none());
            assert!(e.document.dust.cached.is_empty());
            assert!(e.document.dust.failed.is_empty());
            let signature = e.dust_signature().unwrap();
            e.schedule_dust(signature, cx);
            let third = captured_request(e);
            e.receive_dust(third.request, Ok(Some(response(&third, active))));
            assert!(e.document.dust.cached.contains_key(&(source, RepairId(1))));
            assert!(e.document.dust.pending.is_none());
        });
    });
}

#[test]
fn failed_new_open_preserve_repairs_and_empty_replacements_retire_them_and_late_work() {
    for open in [false, true] {
        let e = editor();
        crate::application::test_support::with_application(e, |app| {
            app.enter(|e, cx| {
                let source = e.current_source.unwrap();
                let active = e.document.visit.ready.as_ref().unwrap().area;
                e.schedule_dust(e.dust_signature().unwrap(), cx);
                let request = captured_request(e);
                let work = || {
                    if open {
                        Preparation::Open("missing.json".into())
                    } else {
                        Preparation::New
                    }
                };
                e.start_document(work(), cx);
                let id = e.document_pending.unwrap();
                e.document_prepared(id, Err("not prepared".into()));
                assert_eq!(e.dust_repairs(source).unwrap().repairs().len(), 1);
                e.start_document(work(), cx);
                let id = e.document_pending.unwrap();
                e.document_prepared(
                    id,
                    Ok(Box::new(Prepared {
                        document: Document::default(),
                        ready: None,
                        selected: None,
                        reuse_ready: false,
                        correction: None,
                        preserve_view: false,
                    })),
                );
                assert!(e.dust_repairs(source).is_none());
                e.receive_dust(request.request, Ok(Some(response(&request, active))));
                assert!(e.dust_repairs(source).is_none());
                assert!(e.document.dust.cached.is_empty());
            });
        });
    }
}
