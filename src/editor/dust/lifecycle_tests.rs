use super::*;
use crate::{
    geometry::QuarterTurns,
    processing::{InputPipeline, InputSettings},
};

mod persistence;

mod acceptance;

fn frame(
    editor: &mut Editor,
    context: &egui::Context,
    events: Vec<egui::Event>,
    cx: &UiContext<'_, Editor>,
) -> Vec<(String, egui::Rect)> {
    super::super::ui::configure_fonts(context);
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
    editor.document.source = false;
    editor.tab = 1;
    let source = editor.document.visit.current_source.unwrap();
    editor
        .dust_repairs_mut(source)
        .insert(
            SupportMask::new([50, 50], [3, 3], vec![true; 9]).unwrap(),
            RepairOrigin::Manual,
            [30, 0],
        )
        .unwrap();
    editor.select_dust_repair(RepairId(1));
    editor.document.dust.search_worker = Some(search_testing::Search::Geometry(
        editor.document.visit.canvas.ready.as_ref().unwrap().area,
    ));
    editor
}

#[test]
fn viewer_recipes_freeze_effective_dust_drafts_without_changing_accepted_or_other_photo_intent() {
    let mut e = editor();
    let source = e.document.visit.current_source.unwrap();
    let mut other = e.document.project.source(source).unwrap().clone();
    other.path = "other-dust-photo.dng".into();
    let other = e.document.project.add_source(other).unwrap();
    let before = e.document.project.clone();
    let capture = |e: &Editor| {
        e.positive_recipe(e.input(), e.document.project.roll, e.photo_settings())
            .unwrap()
    };
    e.begin_dust_cleanup();
    let accepted = capture(&e);
    {
        let session = e.dust_session().unwrap();
        session.edit.borrow_mut().donor_draft = Some(DonorDraft {
            repair: RepairId(1),
            offset: [64, 0],
            drag: None,
        });
    }
    let donor = capture(&e);
    assert_eq!(donor.repairs[0].offset, [64, 0]);
    assert_eq!(accepted.repairs[0].offset, [30, 0]);
    assert_ne!(
        accepted, donor,
        "effective repair changes are photographic intent changes"
    );
    let other_recipe = e
        .positive_recipe_for_source(
            e.input(),
            e.document.project.roll,
            e.document.project.source(other).unwrap(),
            e.photo_settings(),
        )
        .unwrap();
    assert_eq!(
        other_recipe.repairs[0].offset,
        [30, 0],
        "a current-photo draft cannot leak to another source"
    );

    {
        let session = e.dust_session().unwrap();
        session
            .edit
            .borrow_mut()
            .donor_draft
            .as_mut()
            .unwrap()
            .offset = [0, 0];
    }
    let unfinished = capture(&e);
    assert_eq!(
        unfinished.repairs[0].offset,
        [0, 0],
        "capture and repair evaluation readiness are distinct"
    );
    e.finish_dust_cleanup();
    assert_eq!(capture(&e), accepted);

    e.begin_dust_mask(Some(RepairId(1)));
    e.document
        .dust_mask_draft_mut()
        .unwrap()
        .pixels
        .insert([54, 50]);
    let mask = capture(&e);
    assert!(mask.repairs[0].support.contains([54, 50]));
    assert!(!accepted.repairs[0].support.contains([54, 50]));
    assert_eq!(
        donor.repairs[0].offset,
        [64, 0],
        "later drafts cannot rewrite a frozen snapshot"
    );
    assert_eq!(
        e.document.state.image_recipe(source).unwrap().repairs,
        accepted.repairs
    );
    assert_eq!(e.document.project, before);
    e.cancel_dust_mask();
    e.finish_dust_cleanup();
    assert_eq!(capture(&e), accepted);
}

fn gpu_editor() -> Editor {
    let mut e = editor();
    let instance = crate::gpu::instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    e.document.visit.working_view = WorkingView::new(device, queue);
    e
}

#[test]
fn held_dust_stroke_keeps_painting_past_512_source_pixels_until_primary_release() {
    let e = editor();
    crate::application::test_support::with_application(e, |app| {
        let (source, project) = app.enter(|e, cx| {
            let source = e.document.visit.current_source.unwrap();
            e.reset_dust_repairs(source);
            e.begin_dust_cleanup();
            e.document.visit.dust.brush_size = 8;
            e.preferences.scopes_visible = false;
            let ctx = egui::Context::default();
            for _ in 0..3 {
                frame(e, &ctx, vec![], cx);
            }
            let project = e.document.state.project.clone();
            let layout = e.document.visit.canvas.last_layout.unwrap().0;
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
                    e.document.dust_mask_draft().is_some(),
                    "lost held stroke at source x={x}: {}",
                    e.status
                );
                assert!(e.dust_repairs(source).is_none());
                assert_eq!(e.document.state.project, project);
            }
            assert!(
                e.document
                    .dust_mask_draft()
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
            assert!(e.document.dust_mask_draft().is_none());
            assert!(e.dust_repairs(source).is_none());
            assert!(e.dust_search_pending());
            (source, project)
        });
        app.run_until(Duration::from_secs(5), |e| !e.dust_search_pending());
        app.read(|e| {
            let repairs = e.dust_repairs(source).unwrap().repairs();
            assert_eq!(repairs.len(), 1);
            assert!(repairs[0].support.contains([750, 200]));
            assert_ne!(e.document.state.project, project);
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
                let source = e.document.visit.current_source.unwrap();
                if !edit_existing {
                    e.reset_dust_repairs(source);
                }
                e.begin_dust_cleanup();
                e.document.visit.dust.brush_size = 64;
                e.preferences.scopes_visible = false;
                let before = e
                    .dust_repairs(source)
                    .map(|r| r.repairs().to_vec())
                    .unwrap_or_default();
                let ctx = egui::Context::default();
                for _ in 0..3 {
                    frame(e, &ctx, vec![], cx);
                }
                let project = e.document.state.project.clone();
                let layout = e.document.visit.canvas.last_layout.unwrap().0;
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
                            .dust_mask_draft()
                            .is_some_and(|draft| draft.rejected)
                        {
                            break 'stroke;
                        }
                    }
                }
                assert!(e.document.dust_mask_draft().unwrap().rejected);
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
                assert!(e.document.dust_mask_draft().is_none());
                assert_eq!(
                    e.dust_repairs(source)
                        .map(|r| r.repairs().to_vec())
                        .unwrap_or_default(),
                    before
                );
                assert!(e.status.contains("no changes applied"), "{}", e.status);
                assert_eq!(e.document.state.project, project);
            });
        });
    }
}

#[test]
fn dust_cursor_matches_selection_paint_and_donor_action_precedence() {
    let mut e = editor();
    e.begin_dust_cleanup();
    let source = e.document.visit.current_source.unwrap();
    let composition = Composition::new(
        e.document.visit.canvas.ready.as_ref().unwrap().area,
        Default::default(),
        Default::default(),
    )
    .unwrap();
    let geometry = e.document.visit.canvas.viewport.layout_composition(
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
    e.set_dust_outlines(false);
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
            e.document.visit.canvas.ready.as_ref().unwrap().area,
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
            e.document.visit.canvas.viewport.zoom = zoom;
            let geometry = e.document.visit.canvas.viewport.layout_composition(
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
fn film_and_grade_candidates_preserve_authored_repairs_without_preparing_pixels() {
    let e = gpu_editor();
    crate::application::test_support::with_application(e, |app| {
        app.enter(|e, _cx| {
            let source = e.document.visit.current_source.unwrap();
            let repairs = e.dust_repairs(source).unwrap().clone();
            let before = e.document.state.project.clone();
            let mut candidate = before.source(source).unwrap().clone();
            candidate.adjustment_layers.as_mut().unwrap().layers[0]
                .adjustment
                .photo
                .exposure = 16.;
            e.validate_source_candidate(&candidate).unwrap();
            assert_eq!(e.document.state.project, before);
            let mut edit = e.edit_project("Rendering settings").unwrap();
            edit.update(|project| project.output.grain = GrainPreset::ThirtyFiveMm.settings());
            e.commit_project(edit).unwrap();
            assert!(e.document.project.output.grain.enabled);
            assert_eq!(e.dust_repairs(source).unwrap(), &repairs);
            assert_eq!(
                e.positive_recipe(
                    e.input(),
                    e.document.project_snapshot().roll,
                    e.photo_settings()
                )
                .unwrap()
                .repairs,
                repairs.repairs()
            );
        });
    });
}

#[test]
fn photo_widget_accepts_exposure_edits_without_discarding_authored_repairs() {
    for gamma in [
        crate::processing::DEFAULT_GAMMA,
        crate::processing::MIN_GAMMA,
    ] {
        let e = gpu_editor();
        crate::application::test_support::with_application(e, |app| {
            app.enter(|e, cx| {
                e.set_gamma(gamma);
                let source = e.document.visit.current_source.unwrap();
                let repair = e.dust_repairs(source).unwrap().repairs()[0].clone();
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
                assert_eq!(e.photo_settings().exposure, 16.);
                assert!(!e.status.contains("rejected:"), "{}", e.status);
                assert_eq!(e.dust_repairs(source).unwrap().repairs(), &[repair]);
            });
        });
    }
}

#[test]
fn disconnected_supports_share_the_stored_translation_before_evaluation_and_after_drag() {
    let mut e = editor();
    let source = e.document.visit.current_source.unwrap();
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
    assert_eq!(e.dust_render_state(), "Repairs pending");
    let regions = e.dust_donor_regions();
    let presented = regions
        .iter()
        .flat_map(|region| region.support.selected_points())
        .collect::<BTreeSet<_>>();
    assert_eq!(presented, support.selected_points().collect());
    let composition = Composition::new(
        e.document.visit.canvas.ready.as_ref().unwrap().area,
        Default::default(),
        Default::default(),
    )
    .unwrap();
    let geometry = e.document.visit.canvas.viewport.layout_composition(
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
    assert_eq!(e.document.dust_donor_draft().unwrap().offset, [64, 0]);
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
    scan.set_flat_field(Some(field));
    let calibration = Calibration {
        dmin: DVec3::splat(0.2),
        dmax: DVec3::splat(0.16),
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
    let mut project = crate::project::Project {
        input: calibration.into(),
        ..Default::default()
    };
    let source = crate::source::source_metadata(
        &scan.source().file.path,
        &scan.source().metadata(),
        scan.source().white_balance(),
    )
    .unwrap();
    let id = project.add_source(source).unwrap();
    let mut recipe = project.image_recipe(id).unwrap();
    recipe.neutral.correction = scan.flat_field().cloned();
    let application = crate::editor::tests::authored_fixture(&["patches.dng"]);
    let services = application.source_services.as_ref().unwrap();
    let result = render_testing::pixels(&scan, &recipe, &repair, services);
    let mut graded = recipe.clone();
    graded.roll.exposure = 3.;
    let mut layers = graded.grade.layers().to_vec();
    layers[0].photo.exposure = 2.;
    graded.grade = AdjustmentLayerSnapshot::new(layers).unwrap();
    assert_eq!(
        result,
        render_testing::pixels(&scan, &graded, &repair, services)
    );
    let mut interpreted = recipe.clone();
    if let InputSettings::Negative { gamma, .. } = &mut interpreted.neutral.input {
        *gamma = 1.2;
    }
    let changed = render_testing::pixels(&scan, &interpreted, &repair, services);
    assert!(
        result
            .iter()
            .zip(changed.iter())
            .any(|((_, a), (_, b))| (0..3).any(|c| (a[c] - b[c]).abs() > 1e-4))
    );
    let pipeline = InputPipeline::new(calibration, scan.camera_to_ap1).unwrap();
    let expected = crate::dust_repair::evaluate(&repair, scan.area, |p| {
        Ok(pipeline
            .apply(scan.pixel(p[0], p[1])?)?
            .rgb
            .as_vec3()
            .to_array())
    })
    .unwrap();
    let expected = expected.pixels().collect::<Vec<_>>();
    assert_eq!(result.len(), expected.len());
    for ((point, actual), (reference_point, reference)) in result.iter().zip(expected) {
        assert_eq!(*point, reference_point);
        for channel in 0..3 {
            assert!(
                (actual[channel] - reference[channel]).abs()
                    < 2e-5 * reference[channel].abs().max(1.),
                "{point:?}: {actual:?} != {reference:?}"
            );
        }
    }
    let original = repair
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
    e.document.project.roll.exposure = 1.2;
    e.document.project.roll.wb = Balance {
        warmth: 0.1,
        tint: -0.1,
    };
    *e.photo_settings_mut() = AdjustmentLayerSettings {
        exposure: 0.5,
        contrast: 1.2,
        saturation: 0.7,
        wb: Balance {
            warmth: -0.1,
            tint: 0.1,
        },
        ..Default::default()
    };
    e.document.set_target(OutputTarget::Srgb);
    e.document.visit.canvas.viewport.zoom = Some(2.);
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
fn input_keeps_repair_instructions_and_new_retires_them() {
    for switch_first in [false, true] {
        crate::application::test_support::with_application(editor(), |app| {
            let source = app.read(|e| e.document.visit.current_source.unwrap());
            if switch_first {
                app.enter(|e, cx| {
                    e.request_input(crate::processing::InputSettings::Digital, cx);
                    if e.dialogs.is_open() {
                        e.dialogs.answer_for_test("Switch stage");
                    }
                });
                app.run_until(std::time::Duration::from_secs(5), |e| {
                    e.document.input_change.is_none()
                });
                app.read(|e| {
                    assert_eq!(
                        e.document.project.input,
                        crate::processing::InputSettings::Digital
                    );
                    assert_eq!(e.dust_repairs(source).unwrap().repairs().len(), 1);
                });
            }
            app.update(|e| {
                e.new_project();
                assert!(e.dust_repairs(source).is_none());
                assert!(e.document.visit.working_view.completed_capture().is_none());
            });
        });
    }
}
