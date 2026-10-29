use super::*;
use egui::{Event, FullOutput, Key, Modifiers, Pos2};

fn frame(
    editor: &mut Editor,
    context: &egui::Context,
    events: Vec<Event>,
    cx: &UiContext<'_, Editor>,
) -> FullOutput {
    frame_with_modifiers(editor, context, events, Modifiers::NONE, cx)
}

fn frame_with_modifiers(
    editor: &mut Editor,
    context: &egui::Context,
    events: Vec<Event>,
    modifiers: Modifiers,
    cx: &UiContext<'_, Editor>,
) -> FullOutput {
    let mut events = events;
    events.insert(0, Event::ModifiersChanged(modifiers));
    context.enable_accesskit();
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                Pos2::ZERO,
                egui::vec2(700., 600.),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            editor.filmstrip(ui, cx);
            let _ = ui.button("Following");
        },
    );
    output.textures_delta.clear();
    output
}

fn node<'a>(
    output: &'a FullOutput,
    name: &str,
) -> (egui::accesskit::NodeId, &'a egui::accesskit::Node) {
    let (id, node) = output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .find(|(_, node)| node.label() == Some(name))
        .unwrap();
    (*id, node)
}

fn point(output: &FullOutput, name: &str) -> Pos2 {
    let bounds = node(output, name).1.bounds().unwrap();
    egui::pos2(
        ((bounds.x0 + bounds.x1) / 2.) as f32,
        ((bounds.y0 + bounds.y1) / 2.) as f32,
    )
}

fn truncate_filmstrip(editor: &mut Editor, keep: usize) {
    let removed: Vec<_> = editor
        .document
        .project
        .order
        .iter()
        .copied()
        .skip(keep)
        .collect();
    for id in removed {
        editor
            .document
            .state
            .project
            .remove_from_filmstrip(id)
            .unwrap();
    }
    if editor
        .document
        .visit
        .current_source
        .is_some_and(|id| !editor.document.state.project.order.contains(&id))
    {
        editor.document.visit.current_source = None;
    }
    editor.document.selected = editor
        .document
        .selected
        .min(editor.document.state.project.order.len().saturating_sub(1));
    editor.document.roll_selection.reconcile(
        &editor.document.state.project.order,
        editor.document.visit.current_source,
    );
}

fn pointer(point: Pos2, pressed: bool) -> Vec<Event> {
    vec![
        Event::PointerMoved(point),
        Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        },
    ]
}

fn key(key: Key) -> Vec<Event> {
    vec![Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }]
}

fn selected_outline(output: &FullOutput, point: Pos2, active: bool) {
    let selected = output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .find(|(_, node)| {
            node.toggled() == Some(egui::accesskit::Toggled::True)
                && node.bounds().is_some_and(|bounds| {
                    f64::from(point.x) >= bounds.x0
                        && f64::from(point.x) <= bounds.x1
                        && f64::from(point.y) >= bounds.y0
                        && f64::from(point.y) <= bounds.y1
                })
        })
        .expect("selected photo");
    assert_eq!(
        selected
            .1
            .description()
            .is_some_and(|description| description.contains("Active photo")),
        active
    );
    let bounds = selected.1.bounds().unwrap();
    let card = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Rect(rect)
                if f64::from(rect.rect.left()) == bounds.x0
                    && f64::from(rect.rect.top()) == bounds.y0
                    && f64::from(rect.rect.right()) == bounds.x1
                    && f64::from(rect.rect.bottom()) == bounds.y1 =>
            {
                Some(rect)
            }
            _ => None,
        })
        .expect("selected card outline");
    assert!(card.stroke.width > 0. && card.stroke.color.a() > 0);
    assert_eq!(card.fill.r(), card.fill.g());
    assert_eq!(card.fill.g(), card.fill.b());
}

#[test]
fn cold_loading_and_failed_selected_raw_retain_selection_and_filename() {
    let editor = crate::editor::tests::fixture();
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            *editor.document = super::tests::fixture();
            truncate_filmstrip(editor, 3);
            let selected = editor.document.state.project.order[1];
            let name = "京都_Long_Scan_Reference_Roll01_Frame_04397.ARW";
            editor
                .document
                .state
                .project
                .source_mut(selected)
                .unwrap()
                .path = name.into();
            editor.document.selected = 1;
            editor.load(editor.document.source_path(selected).unwrap(), cx);
            assert!(editor.document.visit.source.pending());
            assert!(
                editor.busy
                    && editor.document.visit.ready.is_none()
                    && editor.document.visit.pending_preview.is_none()
            );
            let before = editor.document.state.project.clone();
            let context = egui::Context::default();
            frame(editor, &context, vec![], cx);
            let output = frame(editor, &context, vec![], cx);
            assert_eq!(
                node(&output, name).1.toggled(),
                Some(egui::accesskit::Toggled::True)
            );
            assert!(
                !node(&output, name).1.is_disabled(),
                "loading a photo must not block navigation"
            );
            selected_outline(&output, point(&output, name), true);
            assert!(
                editor.document.visit.ready.is_none()
                    && editor.document.visit.pending_preview.is_none()
            );
            assert_eq!(editor.document.state.project, before);
            editor.source_result(editor.id, Err("RAW unavailable".into()));
            assert!(
                !editor.busy
                    && editor.document.visit.ready.is_none()
                    && editor.document.visit.pending_preview.is_none()
            );
            let output = frame(editor, &context, vec![], cx);
            selected_outline(&output, point(&output, name), true);
            assert_eq!(
                node(&output, name).1.toggled(),
                Some(egui::accesskit::Toggled::True)
            );
            assert!(
                !node(&output, name).1.is_disabled(),
                "allow retry after failure"
            );
            assert_eq!(editor.document.visit.current_source, Some(selected));
            assert_eq!(editor.document.state.project, before);
        });
    });
}

#[test]
fn filmstrip_availability_blocks_mouse_and_keyboard_and_releases_existing_focus() {
    let editor = crate::editor::tests::fixture();
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            *editor.document = super::tests::fixture();
            truncate_filmstrip(editor, 2);
            crate::editor::tests::add_ready_fixture(editor);
            editor.document.selected = 2;
            editor.sync_document();
            let before = editor.document.state.project.clone();
            let context = egui::Context::default();
            frame(editor, &context, vec![], cx);
            let output = frame(editor, &context, vec![], cx);
            let target = node(&output, "photo-0.ARW").0;
            let point = point(&output, "photo-0.ARW");
            for state in ["busy", "crop", "flat", "pick"] {
                let mut focused = false;
                for _ in 0..12 {
                    frame(editor, &context, key(Key::Tab), cx);
                    if context
                        .memory(|memory| memory.focused())
                        .map(|id| id.accesskit_id())
                        == Some(target)
                    {
                        focused = true;
                        break;
                    }
                }
                assert!(focused, "enabled card is keyboard reachable");
                match state {
                    "busy" => editor.busy = true,
                    "crop" => editor.begin_crop(),
                    "pick" => editor.document.visit.picking.display = Some(Pick::Gamma.display()),
                    "flat" => {
                        editor.document.flat_edit = Some(crate::editor::flat::FlatEdit {
                            working: None,
                            reference: None,
                            region: None,
                            viewport: editor.document.visit.viewport,
                            source: editor.document.source,
                            task: None,
                        })
                    }
                    _ => unreachable!(),
                }
                let output = frame(editor, &context, key(Key::Space), cx);
                assert_eq!(node(&output, "photo-0.ARW").0, target);
                assert!(node(&output, "photo-0.ARW").1.is_disabled());
                assert!(!node(&output, "Following").1.is_disabled());
                assert_ne!(
                    context
                        .memory(|memory| memory.focused())
                        .map(|id| id.accesskit_id()),
                    Some(target)
                );
                frame(editor, &context, pointer(point, true), cx);
                frame(editor, &context, pointer(point, false), cx);
                for _ in 0..5 {
                    frame(editor, &context, key(Key::Tab), cx);
                    assert_ne!(
                        context
                            .memory(|memory| memory.focused())
                            .map(|id| id.accesskit_id()),
                        Some(target)
                    );
                }
                assert!(
                    !editor.document.visit.source.pending(),
                    "disabled cards dispatch no work"
                );
                assert_eq!(editor.document.state.project, before);
                editor.busy = false;
                editor.finish_crop(false);
                editor.document.flat_edit = None;
                editor.document.visit.picking.display = None;
                frame(editor, &context, vec![], cx);
            }
            frame(editor, &context, pointer(point, true), cx);
            frame(editor, &context, pointer(point, false), cx);
            assert!(
                editor
                    .document
                    .source_path(editor.document.visit.current_source.unwrap())
                    .unwrap()
                    .ends_with("photo-0.ARW")
            );
            assert!(editor.document.visit.source.pending());
            assert_eq!(editor.document.selected, 0);
        });
    });
}

#[test]
fn document_work_still_blocks_navigation_when_a_selected_load_was_pending() {
    let editor = crate::editor::tests::fixture();
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            *editor.document = super::tests::fixture();
            truncate_filmstrip(editor, 2);
            editor.load(
                editor
                    .document
                    .source_path(editor.document.state.project.order[0])
                    .unwrap(),
                cx,
            );
            assert!(editor.document.visit.source.pending());
            editor.document_pending = Some(999);
            let context = egui::Context::default();
            frame(editor, &context, vec![], cx);
            let output = frame(editor, &context, vec![], cx);
            assert!(node(&output, "photo-1.ARW").1.is_disabled());
            let point = point(&output, "photo-1.ARW");
            frame(editor, &context, pointer(point, true), cx);
            frame(editor, &context, pointer(point, false), cx);
            assert_eq!(
                editor.document.visit.current_source,
                Some(editor.document.state.project.order[0])
            );
        });
    });
}

#[test]
fn card_tooltips_reveal_only_the_source_path_even_when_selection_is_disabled() {
    for disabled in [false, true] {
        let editor = crate::editor::tests::fixture();
        crate::application::test_support::with_application(editor, |app| {
            app.enter(|editor, cx| {
                *editor.document = super::tests::fixture();
                truncate_filmstrip(editor, 1);
                editor.document.visit.current_source = Some(editor.document.state.project.order[0]);
                editor.busy = disabled;
                let path = editor
                    .document
                    .source_path(editor.document.visit.current_source.unwrap())
                    .unwrap()
                    .display()
                    .to_string();
                let context = egui::Context::default();
                context.style_mut_of(egui::Theme::Dark, |style| {
                    style.interaction.tooltip_delay = 0.
                });
                frame(editor, &context, vec![], cx);
                let output = frame(editor, &context, vec![], cx);
                let point = point(&output, "photo-0.ARW");
                let mut visible = false;
                for _ in 0..4 {
                    let output = frame(editor, &context, vec![Event::PointerMoved(point)], cx);
                    for shape in &output.shapes {
                        if let egui::Shape::Text(text) = &shape.shape
                            && text.galley.job.text.starts_with(&path)
                        {
                            assert_eq!(text.galley.job.text, path);
                            visible = true;
                        }
                    }
                }
                assert!(visible, "full source path must remain available on hover");
            });
        });
    }
}

#[test]
fn disabled_cards_keep_actionable_error_tooltips() {
    let editor = crate::editor::tests::fixture();
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
    *editor.document = super::tests::fixture();
    truncate_filmstrip(editor, 1);
    editor.document.visit.current_source = Some(editor.document.state.project.order[0]);
    editor.busy = true;
    let image_key = super::tests::key(&editor.document, editor.document.visit.current_source.unwrap());
    editor.document.thumbnails.cache.insert(
        image_key,
        Err("Source unavailable: actionable detail".into()),
    );
    let context = egui::Context::default();
    context.style_mut_of(egui::Theme::Dark, |style| {
        style.interaction.tooltip_delay = 0.
    });
    frame(editor, &context, vec![], cx);
    let output = frame(editor, &context, vec![], cx);
    let point = point(&output, "photo-0.ARW");
    let mut visible = false;
    for _ in 0..4 {
        let output = frame(editor, &context, vec![Event::PointerMoved(point)], cx);
        visible |= output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.job.text == "Source unavailable: actionable detail"));
    }
    assert!(visible, "disabled selection must not hide error detail");

});
    });
}

#[test]
fn ordinary_pickers_keep_the_current_photo_until_the_interaction_ends() {
    for pick in [Pick::Dmin, Pick::Dmax, Pick::Gamma, Pick::RollWb, Pick::Wb] {
        let editor = crate::editor::tests::fixture();
        crate::application::test_support::with_application(editor, |app| {
            let (before, working, viewport) = app.update(|editor| {
                *editor.document = super::tests::fixture();
                truncate_filmstrip(editor, 2);
                crate::editor::tests::add_ready_fixture(editor);
                editor.document.selected = 2;
                editor.sync_document();
                (
                    editor.document.state.project.clone(),
                    editor.document.visit.current_source,
                    editor.document.visit.viewport,
                )
            });
            app.enter(|editor, cx| editor.begin_pick(pick, cx));
            app.pump();
            app.enter(|editor, cx| {
                let context = egui::Context::default();
                frame(editor, &context, vec![], cx);
                let output = frame(editor, &context, vec![], cx);
                assert!(node(&output, "photo-0.ARW").1.is_disabled());
                let point = point(&output, "photo-0.ARW");
                frame(editor, &context, pointer(point, true), cx);
                frame(editor, &context, pointer(point, false), cx);
                assert_eq!(editor.document.visit.current_source, working);
                assert_eq!(editor.document.selected, 2);
                assert_eq!(editor.active_pick(), Some(pick.display()));
                assert!(!editor.document.visit.source.pending());
                assert_eq!(editor.document.state.project, before);
                editor.finish_pick(cx);
                assert_eq!(editor.document.visit.current_source, working);
                assert_eq!(editor.document.visit.viewport, viewport);
                assert!(editor.active_pick().is_none());
                let output = frame(editor, &context, vec![], cx);
                assert!(!node(&output, "photo-0.ARW").1.is_disabled());
                frame(editor, &context, pointer(point, true), cx);
                frame(editor, &context, pointer(point, false), cx);
                assert_eq!(
                    editor.document.visit.current_source,
                    Some(editor.document.project.order[0])
                );
                assert!(editor.document.visit.source.pending());
            });
        });
    }
}

#[test]
fn pending_photo_loading_accepts_mouse_and_keyboard_replacement_and_ignores_old_ready() {
    for keyboard in [false, true] {
        let editor = crate::editor::tests::fixture();
        crate::application::test_support::with_application(editor, |app| {
            app.enter(|editor, cx| {
                *editor.document = super::tests::fixture();
                truncate_filmstrip(editor, 2);
                let first = editor.document.state.project.order[0];
                let second = editor.document.state.project.order[1];
                editor.load(editor.document.source_path(first).unwrap(), cx);
                let original = editor.id;
                assert!(editor.document.visit.source.pending());
                let context = egui::Context::default();
                frame(editor, &context, vec![], cx);
                let output = frame(editor, &context, vec![], cx);
                let target = node(&output, "photo-1.ARW").0;
                assert!(!node(&output, "photo-1.ARW").1.is_disabled());
                if keyboard {
                    let mut focused = false;
                    for _ in 0..12 {
                        frame(editor, &context, key(Key::Tab), cx);
                        if context.memory(|m| m.focused()).map(|id| id.accesskit_id())
                            == Some(target)
                        {
                            focused = true;
                            break;
                        }
                    }
                    assert!(focused);
                    frame(editor, &context, key(Key::Space), cx);
                } else {
                    let point = point(&output, "photo-1.ARW");
                    frame(editor, &context, pointer(point, true), cx);
                    frame(editor, &context, pointer(point, false), cx);
                }
                assert_eq!(editor.document.visit.current_source, Some(second));
                assert_eq!(editor.document.selected, 1);
                assert!(editor.busy && editor.document.visit.ready.is_none());
                assert!(editor.id > original);
                assert!(editor.document.visit.source.pending());
                editor.source_result(original, Err("obsolete failure".into()));
                assert!(editor.busy && editor.document.visit.current_source == Some(second));
                assert_ne!(editor.status, "obsolete failure");
                editor.begin_pick(Pick::Dmin, cx);
                editor.begin_crop();
                assert!(
                    editor.active_pick().is_none() && editor.document.visit.crop_edit.is_none()
                );
            });
        });
    }
}

#[test]
fn modifier_clicks_expose_selection_separately_from_the_active_photo() {
    let editor = crate::editor::tests::fixture();
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            *editor.document = super::tests::fixture();
            truncate_filmstrip(editor, 3);
            editor.document.visit.current_source = Some(editor.document.state.project.order[0]);
            let context = egui::Context::default();
            frame(editor, &context, vec![], cx);
            let output = frame(editor, &context, vec![], cx);
            let click = |editor: &mut Editor, index: usize, modifiers| {
                let point = point(&output, &format!("photo-{index}.ARW"));
                for pressed in [true, false] {
                    let mut events = pointer(point, pressed);
                    if let Event::PointerButton {
                        modifiers: event_modifiers,
                        ..
                    } = &mut events[1]
                    {
                        *event_modifiers = modifiers;
                    }
                    frame_with_modifiers(editor, &context, events, modifiers, cx);
                }
            };
            click(editor, 2, Modifiers::COMMAND);
            assert!(
                !editor.document.visit.source.pending(),
                "extending selection does not load a RAW"
            );
            let result = frame(editor, &context, vec![], cx);
            for index in 0..3 {
                let name = format!("photo-{index}.ARW");
                let (_, node) = node(&result, &name);
                if index != 1 {
                    selected_outline(&result, point(&result, &name), index == 0);
                }
                assert_eq!(
                    node.toggled(),
                    Some(if index == 1 {
                        egui::accesskit::Toggled::False
                    } else {
                        egui::accesskit::Toggled::True
                    })
                );
                assert_eq!(
                    node.description()
                        .is_some_and(|description| description.contains("Active photo")),
                    index == 0
                );
            }
            click(editor, 0, Modifiers::COMMAND);
            assert!(editor.document.visit.source.pending());
            assert_eq!(
                editor.document.visit.current_source,
                Some(editor.document.state.project.order[2])
            );
            editor.source_result(editor.id, Err("fixture unavailable".into()));
            click(editor, 2, Modifiers::COMMAND);
            assert!(
                !editor.document.visit.source.pending(),
                "sole selected card is a no-op"
            );
            click(editor, 1, Modifiers::NONE);
            assert!(editor.document.visit.source.pending());
            assert_eq!(
                editor.document.visit.current_source,
                Some(editor.document.state.project.order[1])
            );
        });
    });
}
