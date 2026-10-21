use super::*;

#[test]
fn branded_header_preserves_composed_menu_availability_and_processing_state() {
    for scale in [1., 2.] {
        for width in [720., 1100.] {
            for busy in [false, true] {
                let mut editor = fixture();
                editor.busy = busy;
                let before = (editor.roll, editor.photo, editor.document.dirty());
                let context = egui::Context::default();
                context.enable_accesskit();
                let mut input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 600.),
                    )),
                    ..Default::default()
                };
                input
                    .viewports
                    .get_mut(&egui::ViewportId::ROOT)
                    .unwrap()
                    .native_pixels_per_point = Some(scale);
                let mut output = context.run_ui(input, |ui| {
                    crate::editor::tests::paint_controller(&mut editor, ui)
                });
                assert_eq!(context.pixels_per_point(), scale);
                output.textures_delta.clear();
                let nodes = &output
                    .platform_output
                    .accesskit_update
                    .as_ref()
                    .unwrap()
                    .nodes;
                let mark = &nodes
                    .iter()
                    .find(|(_, node)| node.value() == Some("Antitone"))
                    .unwrap()
                    .1;
                let bounds = mark.bounds().unwrap();

                assert!(bounds.width() > 0. && bounds.height() > 0.);
                assert!(bounds.x0 >= 0. && bounds.x1 <= f64::from(width));
                assert!(bounds.y0 >= 0. && bounds.y1 <= 600.);

                for label in ["File", "Edit", "View"] {
                    let menu = &nodes
                        .iter()
                        .find(|(_, node)| node.label() == Some(label))
                        .unwrap()
                        .1;
                    assert!(!menu.is_disabled());
                    let menu_bounds = menu.bounds().unwrap();
                    assert!(menu_bounds.x0 >= bounds.x1);
                    assert!(menu_bounds.x1 <= f64::from(width));
                    assert!(menu_bounds.y0 >= 0. && menu_bounds.y1 <= 600.);
                }
                let availability = editor.application_menu_state();
                assert_eq!(
                    availability.enabled(crate::platform::menu::Action::AddScans),
                    !busy
                );
                assert!(!availability.enabled(crate::platform::menu::Action::Export));
                assert_eq!((editor.roll, editor.photo, editor.document.dirty()), before);
                assert!(editor.pick.is_none());
                assert!(editor.document.visit.crop_edit.is_none());
                assert!(editor.document_pending.is_none());
            }
        }
    }
}

#[test]
fn stage_tabs_have_readable_distinct_named_hit_regions_and_keyboard_activation() {
    let context = egui::Context::default();
    presentation::configure_fonts(&context);
    context.enable_accesskit();
    let mut selected = 0;
    let mut frame = |events| {
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(270., 600.),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                presentation::stage_tabs(ui, &mut selected);
            },
        );
        output.textures_delta.clear();
        output
    };
    let first = frame(vec![]);
    let mut nodes: Vec<_> = first
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .filter(|(_, node)| node.role() == egui::accesskit::Role::Tab)
        .collect();
    nodes.sort_by(|(_, a), (_, b)| a.bounds().unwrap().x0.total_cmp(&b.bounds().unwrap().x0));
    assert_eq!(nodes.len(), 3);
    let mut previous_right = 0.;
    for ((_, node), name) in nodes.into_iter().zip(["Roll", "Photo", "Rendering"]) {
        assert_eq!(node.label(), Some(name));
        assert_eq!(node.is_selected(), Some(name == "Roll"));

        let bounds = node.bounds().unwrap();
        assert!(bounds.height() > 0. && bounds.width() > 0.);
        assert!(bounds.x0 >= previous_right && bounds.x1 <= 270.);
        previous_right = bounds.x1;
        let text = first
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text.eq_ignore_ascii_case(name) => {
                    Some(text)
                }
                _ => None,
            })
            .unwrap();
        let hit = egui::Rect::from_min_max(
            egui::pos2(bounds.x0 as f32, bounds.y0 as f32),
            egui::pos2(bounds.x1 as f32, bounds.y1 as f32),
        );
        assert!(hit.expand(0.5).contains_rect(text.visual_bounding_rect()));
    }

    let key = |key| {
        vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]
    };
    frame(key(egui::Key::Tab));
    frame(key(egui::Key::Tab));
    frame(key(egui::Key::Space));
    drop(frame);
    assert_eq!(selected, 1);
}

fn pointer(point: egui::Pos2, pressed: bool) -> Vec<egui::Event> {
    vec![
        egui::Event::PointerMoved(point),
        egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        },
    ]
}

#[test]
fn successful_new_from_photo_or_rendering_selects_roll_and_opens_calibration_only() {
    for tab in [1, 2] {
        let editor = fixture();
        crate::application::test_support::with_application(editor, |app| {
            app.enter(|editor, cx| {
                editor.tab = tab;
                editor
                    .preferences
                    .sections
                    .insert("roll-calibration".into(), false);
                editor
                    .preferences
                    .sections
                    .insert("photo-white-balance".into(), false);
                editor
                    .preferences
                    .sections
                    .insert("roll-gamma-balance".into(), false);
                let before = editor.preferences.clone();
                editor.start_document(Preparation::New, cx);
                let id = editor.document_pending.unwrap();
                assert_eq!(editor.tab, tab);
                assert!(editor.busy && editor.document_pending.is_some());
                editor.document_prepared(id + 1, Err("stale".into()));
                assert_eq!(editor.tab, tab);
                assert_eq!(editor.document_pending, Some(id));
                assert!(editor.new_project_pending);
                editor.document_prepared(
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
                assert_eq!(editor.tab, 0);
                assert!(!editor.new_project_pending);
                assert!(editor.preferences.sections["roll-calibration"]);
                assert!(!editor.preferences.sections["photo-white-balance"]);
                assert!(!editor.preferences.sections["roll-gamma-balance"]);
                assert_eq!(editor.preferences.filmstrip_height, before.filmstrip_height);
                assert_eq!(editor.preferences.scopes_visible, before.scopes_visible);
                assert!(!editor.document.dirty());
            });
        });
    }
}

#[test]
fn cancelling_guarded_new_from_rendering_preserves_tab_document_and_preferences() {
    let mut editor = fixture();
    editor.tab = 2;
    editor.sync_document();
    let project = editor.document.project.clone();
    let context = egui::Context::default();
    editor.file_action(FileAction::New, &UiContext::render_only());
    assert_eq!(editor.transition, Some(Transition::New));
    assert_eq!(editor.tab, 2);
    for _ in 0..3 {
        app_frame(&mut editor, &context, vec![]);
    }
    let preferences = editor.preferences.clone();
    let labels = app_frame(&mut editor, &context, vec![]);
    let cancel = labels
        .iter()
        .find(|(s, _)| s == "Cancel")
        .unwrap()
        .1
        .center();
    for pressed in [true, false] {
        app_frame(&mut editor, &context, pointer(cancel, pressed));
    }
    assert!(editor.transition.is_none());
    assert!(editor.document_pending.is_none());
    assert_eq!(editor.tab, 2);
    assert_eq!(editor.document.project, project);
    assert_eq!(editor.preferences, preferences);
    assert!(editor.document_pending.is_none() && editor.export_operation.pending.is_none());
}

#[test]
fn failed_new_and_open_or_import_do_not_reset_the_selected_tab() {
    let editor = fixture();
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.tab = 2;
            let before = editor.document.project.clone();
            editor.start_document(Preparation::New, cx);
            editor.document_prepared(editor.document_pending.unwrap(), Err("failed".into()));
            assert_eq!(editor.tab, 2);
            assert_eq!(editor.document.project, before);
            assert!(!editor.new_project_pending);
            for work in [
                Preparation::Open("project.json".into()),
                Preparation::Import(Document::default(), vec![], None),
            ] {
                editor.start_document(work, cx);
                editor.document_prepared(
                    editor.document_pending.unwrap(),
                    Ok(Box::new(Prepared {
                        document: Document::default(),
                        ready: None,
                        selected: None,
                        reuse_ready: false,
                        correction: None,
                        preserve_view: false,
                    })),
                );
                assert_eq!(editor.tab, 2);
            }
        });
    });
}

#[test]
fn section_scope_and_filmstrip_preferences_restore_in_a_fresh_context_without_project_changes() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ui-preferences.json");
    let mut editor = fixture();
    editor.roll.exposure = 1.23456789;
    editor.sync_document();
    *editor.document = editor
        .document
        .save(
            &directory.path().join("project.json"),
            crate::project::Save::New,
        )
        .unwrap();
    let project = serde_json::to_value(&editor.document.project).unwrap();
    let context = egui::Context::default();
    editor.preference_store = Some(preferences::Store::open(path.clone()).unwrap().0);
    click_section_header(&mut editor, &context, "Gamma & balance");
    editor.tab = 1;
    click_section_header(&mut editor, &context, "White balance");
    click_section_header(&mut editor, &context, "Exposure");
    editor.scopes.visible = false;
    editor.scopes.height_ratio = 1.125;
    editor.preferences.filmstrip_height = 210.;
    editor.source_exposure = 2.5;
    app_frame(&mut editor, &context, vec![]);
    editor.sync_document();
    assert_eq!(
        serde_json::to_value(&editor.document.project).unwrap(),
        project
    );
    assert!(!editor.document.dirty());
    host::App::on_exit(&mut editor);
    let (store, restored) = preferences::Store::open(path).unwrap();
    assert_eq!(restored, editor.preferences);
    let mut reopened = fixture();
    reopened.apply_preferences(restored);
    reopened.preference_store = Some(store);
    let fresh = egui::Context::default();
    let roll = controls_frame(&mut reopened, &fresh, 315., vec![]);
    assert!(!roll.iter().any(|(s, _)| s == "Gamma"));
    assert!(!reopened.scopes.visible);
    assert_eq!(reopened.scopes.height_ratio, 1.125);
    assert_eq!(reopened.preferences.filmstrip_height, 210.);
    assert_eq!(reopened.tab, 0);
    assert_eq!(reopened.source_exposure, 0.);
    reopened.tab = 1;
    let photo = controls_frame(&mut reopened, &fresh, 315., vec![]);
    assert!(!photo.iter().any(|(s, _)| s == "Warmth" || s == "EV"));
    assert!(!photo.iter().any(|(s, _)| s == "Gamma & balance"));
}

#[test]
fn all_three_stage_controls_are_reachable_at_normal_and_narrow_inspector_sizes() {
    for (width, height) in [(315., 800.), (270., 600.)] {
        for (tab, names) in [
            (
                0,
                vec![
                    "Reset scan orientation",
                    "Flat-field correction",
                    "Choose reference RAW…",
                    "Calibration",
                    "Dmin (film base)",
                    "Dmax",
                    "Gamma & balance",
                    "Gamma",
                    "Balance",
                    "Warmth",
                    "Tint",
                    "Analyze",
                    "White balance",
                    "Toe & shoulder",
                    "Toe",
                    "Shoulder",
                    "Exposure",
                    "EV",
                ],
            ),
            (
                1,
                vec![
                    "Crop & straighten",
                    "Dust repair",
                    "White balance",
                    "Warmth",
                    "Tint",
                    "Exposure",
                    "EV",
                    "Contrast",
                    "Pivot",
                    "Colour preservation",
                    "Saturation",
                    "Density",
                ],
            ),
            (2, vec!["Look", "None", "Target", "Display P3"]),
        ] {
            let mut editor = fixture();
            editor.tab = tab;
            for name in [
                "roll-orientation",
                "roll-flat-field",
                "roll-calibration",
                "roll-gamma-balance",
                "roll-response",
                "roll-white-balance",
                "roll-exposure",
                "photo-composition",
                "photo-white-balance",
                "photo-exposure",
                "photo-contrast",
                "photo-saturation",
                "rendering-target",
            ] {
                editor.preferences.sections.insert(name.into(), true);
            }
            let context = egui::Context::default();
            let mut seen = std::collections::HashSet::new();
            let first = controls_frame_sized(&mut editor, &context, width, height, vec![]);
            let tabs = first.iter().find(|(s, _)| s == "Roll").unwrap().1;
            for frame in 0..180 {
                let events = if frame > 0 && frame % 10 == 0 {
                    vec![
                        egui::Event::PointerMoved(egui::pos2(width * 0.5, height - 40.)),
                        egui::Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            phase: egui::TouchPhase::Move,
                            delta: egui::vec2(0., -100.),
                            modifiers: egui::Modifiers::NONE,
                        },
                    ]
                } else {
                    vec![]
                };
                let labels = controls_frame_sized(&mut editor, &context, width, height, events);
                assert_eq!(labels.iter().find(|(s, _)| s == "Roll").unwrap().1, tabs);
                for (label, rect) in labels {
                    if rect.top() > tabs.bottom()
                        && rect.bottom() < height - 8.
                        && rect.left() >= 0.
                        && rect.right() <= width
                    {
                        seen.insert(label);
                    }
                }
            }
            for name in names {
                assert!(
                    seen.contains(name),
                    "unreachable {name}: tab={tab} width={width} seen={seen:?}"
                );
            }
        }
    }
}

#[test]
fn crop_inspector_keeps_the_shared_tool_shelf_without_owning_transaction_controls() {
    let mut editor = fixture();
    add_ready_fixture(&mut editor);
    editor.tab = 1;
    editor.begin_crop();
    let draft = editor.document.visit.crop_edit.as_ref().unwrap().draft;
    for (width, height) in [(315., 800.), (270., 600.)] {
        let context = egui::Context::default();
        let labels = controls_frame_sized(&mut editor, &context, width, height, vec![]);
        for action in [
            "Turn left",
            "Turn right",
            "Flip horizontal",
            "Flip vertical",
            "Crop & straighten",
            "Dust repair",
        ] {
            assert!(labels.iter().any(|(s, _)| s == action));
        }
        assert!(!labels.iter().any(|(s, _)| s == "Apply" || s == "Cancel"));
        assert_eq!(
            editor.document.visit.crop_edit.as_ref().unwrap().draft,
            draft
        );
    }
}

#[test]
fn rendering_target_selector_and_reset_retain_grade_and_draft_guards() {
    let mut editor = fixture();
    editor.tab = 2;
    let grade = (
        editor.roll,
        editor.photo,
        editor.dmin,
        editor.dmax,
        editor.crop,
    );
    let context = egui::Context::default();
    let labels = controls_frame(&mut editor, &context, 315., vec![]);
    let selector = labels
        .iter()
        .find(|(s, _)| s == "Display P3")
        .unwrap()
        .1
        .center();
    for pressed in [true, false] {
        controls_frame(&mut editor, &context, 315., pointer(selector, pressed));
    }
    let menu = controls_frame(&mut editor, &context, 315., vec![]);
    let srgb = menu.iter().find(|(s, _)| s == "sRGB").unwrap().1.center();
    for pressed in [true, false] {
        controls_frame(&mut editor, &context, 315., pointer(srgb, pressed));
    }
    assert_eq!(editor.target, OutputTarget::Srgb);
    editor.sync_document();
    assert_eq!(
        editor.document.project.output.tiff,
        crate::project::TiffTarget::Srgb
    );
    assert_eq!(
        (
            editor.roll,
            editor.photo,
            editor.dmin,
            editor.dmax,
            editor.crop
        ),
        grade
    );
    click_section_reset(&mut editor, &context, "Target");
    assert_eq!(editor.target, OutputTarget::DisplayP3);
    editor.target = OutputTarget::Srgb;
    add_ready_fixture(&mut editor);
    editor.begin_crop();
    let draft = editor.document.visit.crop_edit.as_ref().unwrap().draft;
    click_section_reset(&mut editor, &context, "Target");
    assert_eq!(editor.target, OutputTarget::Srgb);
    assert_eq!(
        editor.document.visit.crop_edit.as_ref().unwrap().draft,
        draft
    );
    editor.finish_crop(false);
    editor.tab = 1;
    let context = egui::Context::default();
    let photo = controls_frame(&mut editor, &context, 315., vec![]);
    assert!(!photo.iter().any(|(s, _)| s == "Target"));
}
