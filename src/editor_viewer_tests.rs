use super::*;

fn frame(
    editor: &mut Editor,
    context: &egui::Context,
    size: egui::Vec2,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    context.enable_accesskit();
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            events,
            ..Default::default()
        },
        |ui| {
            egui::CentralPanel::default().show(ui, |ui| host::App::ui(editor, ui));
        },
    );
    output.textures_delta.clear();
    output
}

fn labels(
    editor: &mut Editor,
    context: &egui::Context,
    size: egui::Vec2,
    events: Vec<egui::Event>,
) -> Vec<(String, egui::Rect)> {
    ui_labels(frame(editor, context, size, events))
}

fn pointer(pos: egui::Pos2, pressed: bool) -> Vec<egui::Event> {
    vec![
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        },
    ]
}

fn key(key: egui::Key) -> Vec<egui::Event> {
    vec![egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }]
}

fn click(editor: &mut Editor, context: &egui::Context, size: egui::Vec2, pos: egui::Pos2) {
    for pressed in [true, false] {
        frame(editor, context, size, pointer(pos, pressed));
    }
}

fn warm(editor: &mut Editor, context: &egui::Context, size: egui::Vec2) {
    for _ in 0..4 {
        frame(editor, context, size, vec![]);
    }
}

#[test]
fn painted_label_bounds_include_right_aligned_galley_offsets() {
    let context = egui::Context::default();
    let mut bounds = egui::Rect::NOTHING;
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800., 600.),
            )),
            ..Default::default()
        },
        |ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                bounds = ui.label("Trailing state").rect;
            });
        },
    );
    output.textures_delta.clear();
    let text = ui_labels(output)
        .into_iter()
        .find(|(s, _)| s == "Trailing state")
        .unwrap()
        .1;
    // Text painting snaps its origin to physical pixels.
    assert!(
        bounds.expand(0.5).contains_rect(text),
        "{text:?} outside {bounds:?}"
    );
}

#[test]
fn bar_and_canvas_are_stable_across_tools_source_quality_loading_and_errors() {
    for size in [egui::vec2(1100., 800.), egui::vec2(800., 600.)] {
        let (mut editor, _send, _receiver) = fixture();
        add_ready_fixture(&mut editor);
        let context = egui::Context::default();
        warm(&mut editor, &context, size);
        let canvas = editor.last_layout.unwrap().0.canvas;
        let bar = egui::containers::panel::PanelState::load(&context, egui::Id::new("viewer-bar"))
            .unwrap()
            .outer_rect;
        assert_eq!(bar.height(), 40.);
        let before = labels(&mut editor, &context, size, vec![]);
        let fit = before.iter().find(|(s, _)| s == "Fit").unwrap().1;
        assert!(fit.top() >= canvas.bottom());
        for pick in [Pick::Dmin, Pick::Dmax, Pick::Gamma, Pick::Wb, Pick::Flat] {
            editor.begin_pick(pick);
            warm(&mut editor, &context, size);
            assert_eq!(editor.last_layout.unwrap().0.canvas, canvas, "{pick:?}");
            assert_eq!(
                labels(&mut editor, &context, size, vec![])
                    .iter()
                    .find(|(s, _)| s == "Fit")
                    .unwrap()
                    .1,
                fit
            );
            editor.finish_pick();
        }
        editor.begin_crop();
        warm(&mut editor, &context, size);
        assert_eq!(
            editor.crop_edit.as_ref().unwrap().layout.unwrap().canvas,
            canvas
        );
        editor.finish_crop(false);
        editor.source = true;
        editor.source_exposure = 12.3;
        editor.status = "Rejected measurement; accepted processing unchanged".into();
        warm(&mut editor, &context, size);
        assert_eq!(editor.last_layout.unwrap().0.canvas, canvas);
        let ready = editor.ready.take();
        editor.busy = true;
        let busy = labels(&mut editor, &context, size, vec![]);
        assert_eq!(busy.iter().find(|(s, _)| s == "Fit").unwrap().1, fit);
        editor.busy = false;
        let empty = labels(&mut editor, &context, size, vec![]);
        assert_eq!(empty.iter().find(|(s, _)| s == "Fit").unwrap().1, fit);
        editor.ready = ready;
    }
}

#[test]
fn every_picker_keeps_coherent_identity_and_shared_actions_in_the_bar_on_every_tab() {
    for size in [
        egui::vec2(1100., 800.),
        egui::vec2(800., 600.),
        egui::vec2(700., 600.),
    ] {
        let (mut editor, _send, _receiver) = fixture();
        add_ready_fixture(&mut editor);
        let context = egui::Context::default();
        for pick in [
            Pick::Dmin,
            Pick::Dmax,
            Pick::Gamma,
            Pick::RollWb,
            Pick::Wb,
            Pick::Analysis,
        ] {
            editor.begin_pick(pick);
            for tab in 0..3 {
                editor.tab = tab;
                warm(&mut editor, &context, size);
                let canvas = editor
                    .crop_edit
                    .as_ref()
                    .and_then(|edit| edit.layout)
                    .or(editor.last_layout.map(|(layout, _)| layout))
                    .unwrap()
                    .canvas;
                let output = frame(&mut editor, &context, size, vec![]);
                for name in ["Cancel", "Apply"] {
                    let bounds = output
                        .platform_output
                        .accesskit_update
                        .as_ref()
                        .unwrap()
                        .nodes
                        .iter()
                        .find(|(_, node)| node.label() == Some(name))
                        .unwrap()
                        .1
                        .bounds()
                        .unwrap();
                    assert_eq!(bounds.y1 - bounds.y0, 28.);
                    assert!(bounds.y0 as f32 >= canvas.bottom());
                    assert!(bounds.x1 as f32 <= canvas.right() + 4.);
                }
                let text = ui_labels(output);
                let title = match pick {
                    Pick::Dmin => "Dmin",
                    Pick::Dmax => "Dmax",
                    Pick::Gamma => "Gamma neutral",
                    Pick::RollWb => "Roll WB neutral",
                    Pick::Wb => "WB neutral",
                    Pick::Analysis => "Picture area",
                    Pick::Flat => unreachable!(),
                };
                let marking = title.to_uppercase();
                let identity = text
                    .iter()
                    .find(|(s, rect)| s == &marking && rect.top() >= canvas.bottom())
                    .unwrap()
                    .1;
                assert!(
                    text.iter().find(|(s, _)| s == "Cancel").unwrap().1.left() - identity.right()
                        >= 11.5
                );
                assert!(!text.iter().any(|(s, _)| s.starts_with("Move tool toolbar")));
                assert!(
                    !text
                        .iter()
                        .any(|(s, rect)| s == &marking && canvas.contains_rect(*rect))
                );
                for name in ["Cancel", "Apply"] {
                    let rect = text.iter().find(|(s, _)| s == name).unwrap().1;
                    assert!(
                        rect.top() >= canvas.bottom(),
                        "{size:?} {pick:?} {tab}: {name} {rect:?} / {canvas:?}"
                    );
                    assert_eq!(text.iter().filter(|(s, _)| s == name).count(), 1);
                }
                for name in ["Details", "Choose reference RAW…", "current.ARW"] {
                    assert!(
                        !text
                            .iter()
                            .any(|(s, rect)| s == name && canvas.contains_rect(*rect))
                    );
                }
                let cancel = text.iter().find(|(s, _)| s == "Cancel").unwrap().1;
                let apply = text.iter().find(|(s, _)| s == "Apply").unwrap().1;
                assert_eq!(cancel.center().y, apply.center().y);
                assert!(cancel.right() < apply.left());
                assert!(apply.right() - cancel.left() < 120.);
                assert_eq!(identity.center().y, cancel.center().y);
                let tint = context
                    .global_style()
                    .visuals
                    .selection
                    .bg_fill
                    .linear_multiply(0.15);
                let output = frame(&mut editor, &context, size, vec![]);
                assert!(
                    output
                        .platform_output
                        .accesskit_update
                        .as_ref()
                        .unwrap()
                        .nodes
                        .iter()
                        .any(|(_, node)| node.value()
                            == Some(format!("Active tool: {title}").as_str()))
                );
                assert!(output.shapes.iter().any(|shape| match &shape.shape {
                    egui::Shape::Rect(rect) => {
                        rect.fill == tint
                            && rect.rect.contains_rect(identity)
                            && rect.rect.contains_rect(apply)
                    }
                    _ => false,
                }));
            }
            editor.finish_pick();
        }
    }
}

#[test]
fn filmstrip_selection_guidance_is_available_in_explicit_viewer_help() {
    for size in [egui::vec2(1100., 800.), egui::vec2(800., 600.)] {
        let (mut editor, _send, _receiver) = fixture();
        add_ready_fixture(&mut editor);
        let context = egui::Context::default();
        warm(&mut editor, &context, size);
        let before = editor.document.project.clone();
        let text = labels(&mut editor, &context, size, vec![]);
        let help = text
            .iter()
            .find(|(s, _)| s == "Viewer help")
            .unwrap()
            .1
            .center();
        click(&mut editor, &context, size, help);
        let text = labels(&mut editor, &context, size, vec![]);
        for instruction in [
            "Filmstrip selection",
            "Cmd-click toggles selection · Shift-click selects a range",
            "Shift–Cmd–A: select active photo only",
        ] {
            let bounds = text.iter().find(|(s, _)| s == instruction).unwrap().1;
            assert!(egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(bounds));
        }
        frame(&mut editor, &context, size, key(egui::Key::Escape));
        assert!(!egui::Popup::is_any_open(&context));
        assert_eq!(editor.document.project, before);
    }
}

#[test]
fn bar_inspector_and_help_own_pointer_drag_wheel_and_escape_before_the_canvas() {
    let size = egui::vec2(800., 600.);
    for cropping in [false, true] {
        let (mut editor, _send, _receiver) = fixture();
        add_ready_fixture(&mut editor);
        let context = egui::Context::default();
        if cropping {
            editor.tab = 1;
            editor.scopes.visible = false;
            editor.begin_crop();
        } else {
            editor.begin_pick(Pick::Wb);
        }
        editor.viewport.zoom = Some(2.);
        warm(&mut editor, &context, size);
        let viewport = editor.viewport;
        let draft = editor.crop_edit.as_ref().map(|edit| edit.draft);
        let region = editor.region;
        let text = labels(&mut editor, &context, size, vec![]);
        let title = text
            .iter()
            .find(|(s, _)| {
                s == if cropping {
                    "Crop & straighten"
                } else {
                    "WB NEUTRAL"
                }
            })
            .unwrap()
            .1
            .center();
        frame(&mut editor, &context, size, pointer(title, true));
        frame(
            &mut editor,
            &context,
            size,
            vec![egui::Event::PointerMoved(title + egui::vec2(-100., -100.))],
        );
        frame(
            &mut editor,
            &context,
            size,
            pointer(title + egui::vec2(-100., -100.), false),
        );
        assert_eq!(editor.viewport, viewport);
        assert_eq!(editor.region, region);
        assert_eq!(editor.crop_edit.as_ref().map(|edit| edit.draft), draft);
        let help = text
            .iter()
            .find(|(s, _)| s == "Viewer help")
            .unwrap()
            .1
            .center();
        click(&mut editor, &context, size, help);
        assert!(egui::Popup::is_any_open(&context));
        let canvas = editor
            .crop_edit
            .as_ref()
            .and_then(|edit| edit.layout)
            .or(editor.last_layout.map(|(layout, _)| layout))
            .unwrap()
            .canvas;
        frame(
            &mut editor,
            &context,
            size,
            vec![
                egui::Event::PointerMoved(canvas.center()),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    phase: egui::TouchPhase::Move,
                    delta: egui::vec2(0., -80.),
                    modifiers: egui::Modifiers::COMMAND,
                },
            ],
        );
        assert_eq!(editor.viewport, viewport);
        assert_eq!(editor.region, region);
        frame(&mut editor, &context, size, key(egui::Key::Escape));
        assert!(!egui::Popup::is_any_open(&context));
        assert!(editor.crop_edit.is_some() || editor.pick.is_some());
        frame(&mut editor, &context, size, key(egui::Key::Escape));
        assert!(editor.crop_edit.is_none() && editor.pick.is_none());
    }
}

#[test]
fn viewer_requests_a_native_rotation_cursor_and_restores_it_on_popup_and_crop_exit() {
    let (mut editor, _send, _receiver) = fixture();
    add_ready_fixture(&mut editor);
    editor.scopes.visible = false;
    editor.begin_crop();
    let context = egui::Context::default();
    let size = egui::vec2(800., 600.);
    warm(&mut editor, &context, size);
    let layout = editor.crop_edit.as_ref().unwrap().layout.unwrap();
    let point = layout.image.right_center() + egui::vec2(18., 0.);
    assert!(layout.canvas.contains(point));
    let output = frame(
        &mut editor,
        &context,
        size,
        vec![egui::Event::PointerMoved(point)],
    );
    let expected = icons::rotation_cursor(
        &context,
        egui::Id::new("expected-rotation-cursor"),
        (point - layout.image.center()).angle() + std::f32::consts::FRAC_PI_2,
    );
    assert_eq!(
        output.platform_output.cursor_image.as_ref(),
        Some(&expected)
    );
    assert_eq!(
        output.platform_output.cursor_icon,
        egui::CursorIcon::Crosshair
    );
    let draft = editor.crop_edit.as_ref().unwrap().draft;
    egui::Popup::open_id(&context, egui::Id::new("rotation-cursor-popup"));
    let blocked = frame(&mut editor, &context, size, vec![]);
    assert!(blocked.platform_output.cursor_image.is_none());
    assert_ne!(blocked.platform_output.cursor_icon, egui::CursorIcon::None);
    assert_eq!(editor.crop_edit.as_ref().unwrap().draft, draft);
    egui::Popup::close_all(&context);
    editor.finish_crop(false);
    let exited = frame(&mut editor, &context, size, vec![]);
    assert!(exited.platform_output.cursor_image.is_none());
    assert_ne!(exited.platform_output.cursor_icon, egui::CursorIcon::None);
    assert!(editor.crop_edit.is_none());
}

#[test]
fn bottom_crop_handles_are_unobstructed_without_reframing_or_docking() {
    for size in [egui::vec2(1100., 800.), egui::vec2(800., 600.)] {
        let (mut editor, _send, _receiver) = fixture();
        add_ready_fixture(&mut editor);
        let context = egui::Context::default();
        editor.begin_crop();
        warm(&mut editor, &context, size);
        let layout = editor.crop_edit.as_ref().unwrap().layout.unwrap();
        let draft = editor.crop_edit.as_ref().unwrap().draft;
        let viewport = editor.viewport;
        let text = labels(&mut editor, &context, size, vec![]);
        assert!(!text.iter().any(|(s, _)| s.starts_with("Move tool toolbar")));
        for name in ["Apply", "Cancel"] {
            assert!(
                text.iter().find(|(s, _)| s == name).unwrap().1.top() >= layout.canvas.bottom()
            );
        }
        assert_eq!(editor.viewport, viewport);
        let handle = layout.image.center_bottom();
        frame(&mut editor, &context, size, pointer(handle, true));
        frame(
            &mut editor,
            &context,
            size,
            vec![egui::Event::PointerMoved(handle - egui::vec2(0., 20.))],
        );
        frame(
            &mut editor,
            &context,
            size,
            pointer(handle - egui::vec2(0., 20.), false),
        );
        assert_ne!(editor.crop_edit.as_ref().unwrap().draft, draft);
        editor.finish_crop(false);
    }
}

#[test]
fn numeric_enter_and_escape_are_handled_before_picker_or_crop_shortcuts() {
    let size = egui::vec2(800., 600.);
    for cropping in [false, true] {
        let (mut editor, _send, _receiver) = fixture();
        add_ready_fixture(&mut editor);
        let context = egui::Context::default();
        if cropping {
            editor.tab = 1;
            editor.scopes.visible = false;
            editor.begin_crop();
        } else {
            editor.begin_pick(Pick::Dmin);
            editor.region = Some(Coverage::rectangle(Rect {
                x: 20,
                y: 30,
                width: 32,
                height: 32,
            }));
        }
        warm(&mut editor, &context, size);
        if !cropping {
            let text = labels(&mut editor, &context, size, vec![]);
            click(
                &mut editor,
                &context,
                size,
                text.iter()
                    .find(|(s, _)| s == "Source inspection")
                    .unwrap()
                    .1
                    .center(),
            );
        }
        let numeric = |text: &[(String, egui::Rect)]| {
            text.iter()
                .find(|(s, _)| {
                    if cropping {
                        s.ends_with('°')
                    } else {
                        s.parse::<f64>().is_ok()
                    }
                })
                .unwrap_or_else(|| panic!("numeric missing: cropping={cropping} labels={text:?}"))
                .1
                .center()
        };
        let text = labels(&mut editor, &context, size, vec![]);
        click(&mut editor, &context, size, numeric(&text));
        assert!(context.egui_wants_keyboard_input());
        frame(
            &mut editor,
            &context,
            size,
            vec![egui::Event::Text("12".into())],
        );
        frame(&mut editor, &context, size, key(egui::Key::Enter));
        frame(&mut editor, &context, size, vec![]);
        assert!(!editor.busy);
        assert!(editor.crop_edit.is_some() || editor.pick.is_some());
        if cropping {
            assert!(editor.crop_edit.as_ref().unwrap().draft.angle.abs() > 0.1);
        } else {
            assert_eq!(editor.source_exposure, 12.);
        }
        let accepted = (
            editor.region,
            editor.crop_edit.as_ref().map(|edit| edit.draft),
            editor.source_exposure,
        );
        let text = labels(&mut editor, &context, size, vec![]);
        click(&mut editor, &context, size, numeric(&text));
        assert!(
            context.egui_wants_keyboard_input(),
            "second numeric click: cropping={cropping} labels={text:?}"
        );
        frame(
            &mut editor,
            &context,
            size,
            vec![egui::Event::Text("5".into())],
        );
        assert!(
            context.egui_wants_keyboard_input(),
            "numeric text lost focus: cropping={cropping}"
        );
        assert_eq!(
            (
                editor.region,
                editor.crop_edit.as_ref().map(|edit| edit.draft),
                editor.source_exposure
            ),
            accepted,
            "pending text changed state: cropping={cropping}"
        );
        frame(&mut editor, &context, size, key(egui::Key::Escape));
        frame(&mut editor, &context, size, vec![]);
        assert_eq!(
            (
                editor.region,
                editor.crop_edit.as_ref().map(|edit| edit.draft),
                editor.source_exposure
            ),
            accepted
        );
        assert!(editor.crop_edit.is_some() || editor.pick.is_some());
    }
}

fn flat_fixture(
    editor: &mut Editor,
    send: &mpsc::Sender<Message>,
    receiver: &mpsc::Receiver<Command>,
) {
    let ready = editor.ready.as_ref().unwrap();
    let mut record = ready.record.clone();
    record.path = "light.ARW".into();
    record.reference_only = true;
    let reference = Ready {
        file: thumbnails::FileKey::new(&record.path),
        thumbnail: None,
        path: record.path.clone(),
        area: ready.area,
        analysis_support: None,
        floor: ready.floor,
        proxy: ready.proxy.clone(),
        record,
        development: ready.development.clone(),
    };
    editor.begin_flat_edit(reference.path.clone(), None);
    assert!(matches!(
        receiver.try_recv().unwrap(),
        Command::FlatPreview(..)
    ));
    send.send(Message::FlatPreview(
        editor.id,
        Ok(Box::new(FlatPreview {
            ready: reference,
            region: None,
        })),
    ))
    .unwrap();
    editor.receive();
}

#[test]
fn crop_numeric_cancellation_survives_hiding_and_reopening_composition() {
    let size = egui::vec2(800., 600.);
    let (mut editor, _send, _receiver) = fixture();
    add_ready_fixture(&mut editor);
    editor.tab = 1;
    editor.scopes.visible = false;
    editor.begin_crop();
    let area = editor.ready.as_ref().unwrap().area;
    editor.crop_edit.as_mut().unwrap().draft = Crop::from_region(area)
        .straightened(area, 0.123456789)
        .unwrap();
    let context = egui::Context::default();
    warm(&mut editor, &context, size);
    let draft = editor.crop_edit.as_ref().unwrap().draft;
    let viewport = editor.viewport;
    let project = editor.document.project.clone();
    let text = labels(&mut editor, &context, size, vec![]);
    click(
        &mut editor,
        &context,
        size,
        text.iter()
            .find(|(s, _)| s.ends_with('°'))
            .unwrap()
            .1
            .center(),
    );
    frame(
        &mut editor,
        &context,
        size,
        vec![egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        }],
    );
    frame(
        &mut editor,
        &context,
        size,
        vec![egui::Event::Text("23".into())],
    );
    frame(&mut editor, &context, size, key(egui::Key::Escape));
    editor.tab = 0;
    warm(&mut editor, &context, size);
    let text = labels(&mut editor, &context, size, vec![]);
    click(
        &mut editor,
        &context,
        size,
        text.iter().find(|(s, _)| s == "Photo").unwrap().1.center(),
    );
    warm(&mut editor, &context, size);
    assert_eq!(editor.tab, 1);
    assert_eq!(editor.crop_edit.as_ref().unwrap().draft, draft);
    assert_eq!(editor.viewport, viewport);
    assert_eq!(editor.document.project, project);
    let text = labels(&mut editor, &context, size, vec![]);
    click(
        &mut editor,
        &context,
        size,
        text.iter()
            .find(|(s, _)| s.ends_with('°'))
            .unwrap()
            .1
            .center(),
    );
    frame(
        &mut editor,
        &context,
        size,
        vec![egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        }],
    );
    frame(
        &mut editor,
        &context,
        size,
        vec![egui::Event::Text("8.125".into())],
    );
    frame(&mut editor, &context, size, key(egui::Key::Enter));
    let edited = editor.crop_edit.as_ref().unwrap().draft;
    let edited_viewport = editor.viewport;
    assert_eq!(
        -edited.angle
            * Composition::new(area, editor.roll_orientation, editor.photo_orientation)
                .unwrap()
                .handedness(),
        8.125_f64.to_radians()
    );
    let text = labels(&mut editor, &context, size, vec![]);
    click(
        &mut editor,
        &context,
        size,
        text.iter()
            .find(|(s, _)| s == "Rendering")
            .unwrap()
            .1
            .center(),
    );
    warm(&mut editor, &context, size);
    assert_eq!(editor.crop_edit.as_ref().unwrap().draft, edited);
    assert_eq!(editor.viewport, edited_viewport);
    assert_eq!(editor.document.project, project);
}

#[test]
fn geometry_tools_use_trailing_viewer_bar_actions_and_only_their_own_inspector() {
    for size in [
        egui::vec2(1100., 800.),
        egui::vec2(800., 600.),
        egui::vec2(700., 600.),
    ] {
        for cropping in [true, false] {
            for source in [false, true] {
                let (mut editor, send, receiver) = fixture();
                add_ready_fixture(&mut editor);
                editor.scopes.visible = false;
                editor.source = source;
                let context = egui::Context::default();
                warm(&mut editor, &context, size);
                let canvas = editor.last_layout.unwrap().0.canvas;
                let project = editor.document.project.clone();
                let section = if cropping {
                    "photo-composition"
                } else {
                    "roll-flat-field"
                };
                editor.preferences.sections.insert(section.into(), false);
                if cropping {
                    editor.begin_crop();
                } else {
                    flat_fixture(&mut editor, &send, &receiver);
                    let area = editor.ready.as_ref().unwrap().area;
                    editor.region = Some(Coverage::rectangle(Rect {
                        x: area.x + 20,
                        y: area.y + 30,
                        width: 32,
                        height: 32,
                    }));
                }
                let draft = editor.crop_edit.as_ref().map(|edit| edit.draft);
                let region = editor.region;
                let mut view = None;
                for tab in 0..3 {
                    editor.tab = tab;
                    warm(&mut editor, &context, size);
                    let output = frame(&mut editor, &context, size, vec![]);
                    for name in ["Cancel", "Apply"] {
                        let bounds = output
                            .platform_output
                            .accesskit_update
                            .as_ref()
                            .unwrap()
                            .nodes
                            .iter()
                            .find(|(_, node)| node.label() == Some(name))
                            .unwrap()
                            .1
                            .bounds()
                            .unwrap();
                        assert_eq!(bounds.y1 - bounds.y0, 28.);
                        assert!(bounds.y0 as f32 >= canvas.bottom());
                        assert!(bounds.x1 as f32 <= canvas.right() + 4.);
                    }
                    let text = ui_labels(output);
                    let current_canvas = editor
                        .crop_edit
                        .as_ref()
                        .and_then(|edit| edit.layout)
                        .or(editor.last_layout.map(|(layout, _)| layout))
                        .unwrap()
                        .canvas;
                    assert_eq!(current_canvas, canvas);
                    assert!(!text.iter().any(|(s, _)| s.starts_with("Move tool toolbar")));
                    let cancel = text.iter().find(|(s, _)| s == "Cancel").unwrap().1;
                    let apply = text.iter().find(|(s, _)| s == "Apply").unwrap().1;
                    assert!(cancel.top() >= canvas.bottom() && apply.top() >= canvas.bottom());
                    assert!(cancel.right() < apply.left());
                    assert!(
                        apply.right() <= canvas.right() + 4.,
                        "{size:?}: {apply:?} / {canvas:?}"
                    );
                    let identity = text
                        .iter()
                        .find(|(s, rect)| {
                            s == if cropping { "CROP" } else { "FLAT-FIELD" }
                                && rect.top() >= canvas.bottom()
                        })
                        .unwrap()
                        .1;
                    assert!(cancel.left() - identity.right() >= 11.5);
                    assert_eq!(identity.center().y, cancel.center().y);
                    let title = if cropping {
                        "Crop & straighten"
                    } else {
                        "Flat-field area"
                    };
                    let own_tab = if cropping { 1 } else { 0 };
                    assert_eq!(text.iter().any(|(s, _)| s == title), tab == own_tab);
                    if cropping && tab == 1 {
                        let aspect = text.iter().find(|(s, _)| s == "Aspect").unwrap().1;
                        let angle = text.iter().find(|(s, _)| s == "Angle").unwrap().1;
                        let guides = text.iter().find(|(s, _)| s == "Guides").unwrap().1;
                        assert_eq!(aspect.left(), angle.left());
                        assert_eq!(angle.left(), guides.left());
                        assert!(aspect.bottom() < angle.top() && angle.bottom() < guides.top());
                    }
                    assert_eq!(editor.crop_edit.as_ref().map(|edit| edit.draft), draft);
                    assert_eq!(editor.region, region);
                    assert_eq!(editor.viewport, *view.get_or_insert(editor.viewport));
                    assert_eq!(editor.document.project, project);
                    assert_eq!(editor.preferences.sections.get(section), Some(&false));
                }
                let text = labels(&mut editor, &context, size, vec![]);
                click(
                    &mut editor,
                    &context,
                    size,
                    text.iter().find(|(s, _)| s == "Cancel").unwrap().1.center(),
                );
                assert!(editor.crop_edit.is_none() && editor.pick.is_none());
                assert_eq!(editor.source, source);
                assert_eq!(editor.document.project, project);
                assert_eq!(editor.preferences.sections.get(section), Some(&false));
            }
        }
    }
}

#[test]
fn picker_bar_actions_preserve_cancel_and_measurement_transactions() {
    let size = egui::vec2(800., 600.);
    for pick in [Pick::Dmin, Pick::Dmax, Pick::Gamma, Pick::Wb] {
        let (mut editor, _send, receiver) = fixture();
        add_ready_fixture(&mut editor);
        editor.viewport.zoom = Some(2.);
        let context = egui::Context::default();
        warm(&mut editor, &context, size);
        let project = editor.document.project.clone();
        let view = editor.viewport;
        editor.begin_pick(pick);
        warm(&mut editor, &context, size);
        let output = frame(&mut editor, &context, size, vec![]);
        assert!(
            output
                .platform_output
                .accesskit_update
                .unwrap()
                .nodes
                .iter()
                .find(|(_, node)| node.label() == Some("Apply"))
                .unwrap()
                .1
                .is_disabled()
        );
        let text = labels(&mut editor, &context, size, vec![]);
        click(
            &mut editor,
            &context,
            size,
            text.iter().find(|(s, _)| s == "Cancel").unwrap().1.center(),
        );
        assert!(editor.pick.is_none());
        assert_eq!(editor.viewport, view);
        assert_eq!(editor.document.project, project);
        editor.begin_pick(pick);
        let area = editor.ready.as_ref().unwrap().area;
        editor.region = Some(Coverage::rectangle(Rect {
            x: area.x + 20,
            y: area.y + 30,
            width: 32,
            height: 32,
        }));
        let text = labels(&mut editor, &context, size, vec![]);
        click(
            &mut editor,
            &context,
            size,
            text.iter().find(|(s, _)| s == "Apply").unwrap().1.center(),
        );
        assert!(editor.busy);
        assert_eq!(editor.document.project, project);
        match receiver.try_recv().unwrap() {
            Command::Pick(_, kind, ..) => assert_eq!(kind, pick),
            _ => panic!("unexpected picker command"),
        }
        let output = frame(&mut editor, &context, size, vec![]);
        for name in ["Cancel", "Apply"] {
            assert!(
                output
                    .platform_output
                    .accesskit_update
                    .as_ref()
                    .unwrap()
                    .nodes
                    .iter()
                    .find(|(_, node)| node.label() == Some(name))
                    .unwrap()
                    .1
                    .is_disabled()
            );
        }
    }
}

#[test]
fn geometry_apply_and_busy_guards_use_the_original_transactions() {
    let size = egui::vec2(800., 600.);
    for cropping in [true, false] {
        let (mut editor, send, receiver) = fixture();
        add_ready_fixture(&mut editor);
        let context = egui::Context::default();
        if cropping {
            editor.begin_crop();
        } else {
            flat_fixture(&mut editor, &send, &receiver);
        }
        warm(&mut editor, &context, size);
        if !cropping {
            let output = frame(&mut editor, &context, size, vec![]);
            assert!(
                output
                    .platform_output
                    .accesskit_update
                    .unwrap()
                    .nodes
                    .iter()
                    .find(|(_, node)| node.label() == Some("Apply"))
                    .unwrap()
                    .1
                    .is_disabled()
            );
            editor.region = Some(Coverage::rectangle(Rect {
                x: 20,
                y: 30,
                width: 32,
                height: 32,
            }));
        }
        let draft = editor.crop_edit.as_ref().map(|edit| edit.draft);
        editor.busy = true;
        let output = frame(&mut editor, &context, size, vec![]);
        for name in ["Cancel", "Apply"] {
            assert!(
                output
                    .platform_output
                    .accesskit_update
                    .as_ref()
                    .unwrap()
                    .nodes
                    .iter()
                    .find(|(_, node)| node.label() == Some(name))
                    .unwrap()
                    .1
                    .is_disabled()
            );
        }
        frame(&mut editor, &context, size, key(egui::Key::Escape));
        assert_eq!(editor.crop_edit.as_ref().map(|edit| edit.draft), draft);
        editor.busy = false;
        let text = labels(&mut editor, &context, size, vec![]);
        click(
            &mut editor,
            &context,
            size,
            text.iter().find(|(s, _)| s == "Apply").unwrap().1.center(),
        );
        if cropping {
            assert!(editor.crop_edit.is_none());
            assert!(editor.viewport.zoom.is_none());
            assert!(receiver.try_recv().is_err());
        } else {
            assert!(editor.busy);
            assert!(matches!(
                receiver.try_recv().unwrap(),
                Command::FlatAccept(..)
            ));
        }
    }
}

#[test]
fn source_inspection_is_hidden_outside_source_view_without_losing_its_value() {
    let size = egui::vec2(800., 600.);
    let (mut editor, _send, _receiver) = fixture();
    add_ready_fixture(&mut editor);
    editor.source = false;
    editor.source_exposure = 7.25;
    let context = egui::Context::default();
    warm(&mut editor, &context, size);
    let canvas = editor.last_layout.unwrap().0.canvas;
    let text = labels(&mut editor, &context, size, vec![]);
    assert!(
        !text
            .iter()
            .any(|(s, _)| s == "Source inspection" || s.starts_with("Source +"))
    );
    click(
        &mut editor,
        &context,
        size,
        text.iter()
            .find(|(s, _)| s == "View source")
            .unwrap()
            .1
            .center(),
    );
    let text = labels(&mut editor, &context, size, vec![]);
    assert!(text.iter().any(|(s, _)| s == "Source +7.25 EV"));
    click(
        &mut editor,
        &context,
        size,
        text.iter()
            .find(|(s, _)| s == "Source inspection")
            .unwrap()
            .1
            .center(),
    );
    assert!(egui::Popup::is_any_open(&context));
    editor.source = false;
    warm(&mut editor, &context, size);
    assert!(!egui::Popup::is_any_open(&context));
    assert_eq!(editor.source_exposure, 7.25);
    assert_eq!(editor.last_layout.unwrap().0.canvas, canvas);
}

#[test]
fn uncalibrated_view_remains_source_only_with_the_positive_toggle_disabled() {
    let (mut editor, _send, _receiver) = fixture();
    editor.dmin = None;
    editor.dmax = None;
    editor.source = false;
    add_ready_fixture(&mut editor);
    let context = egui::Context::default();
    let size = egui::vec2(800., 600.);
    warm(&mut editor, &context, size);
    assert!(editor.source);
    assert!(editor.last_layout.is_some());
    let output = frame(&mut editor, &context, size, vec![]);
    let nodes = output.platform_output.accesskit_update.unwrap().nodes;
    assert!(
        nodes
            .iter()
            .find(|(_, node)| node.label() == Some("View source"))
            .unwrap()
            .1
            .is_disabled()
    );
    assert!(editor.calibration().is_none());
}

#[test]
fn sampling_rejects_sources_that_are_not_in_the_roll_without_changing_the_pick() {
    let (mut editor, _send, receiver) = fixture();
    add_ready_fixture(&mut editor);
    let mut reference = editor.ready.as_ref().unwrap().record.clone();
    reference.path = "flat-reference.ARW".into();
    let development = editor.ready.as_ref().unwrap().development.clone();
    let reference_id = editor
        .document
        .register_reference(reference, development)
        .unwrap();
    editor.begin_pick(Pick::Dmin);
    editor.region = Some(Coverage::rectangle(Rect {
        x: 20,
        y: 30,
        width: 32,
        height: 32,
    }));
    let before = (
        editor.current_source,
        editor.selected,
        editor.region,
        editor.viewport,
        editor.document.project.clone(),
    );
    for id in [reference_id, SourceId(u64::MAX)] {
        editor.pick_roll_source(id);
        assert_eq!(
            (
                editor.current_source,
                editor.selected,
                editor.region,
                editor.viewport,
                editor.document.project.clone()
            ),
            before
        );
        assert_eq!(editor.pick, Some(Pick::Dmin));
        assert!(!editor.busy);
        assert!(receiver.try_recv().is_err());
    }
}

#[test]
fn photo_tool_shelf_keeps_only_the_active_crop_button_available_during_a_crop() {
    let (mut editor, _send, _receiver) = fixture();
    add_ready_fixture(&mut editor);
    editor.tab = 1;
    editor.begin_crop();
    let context = egui::Context::default();
    let size = egui::vec2(800., 600.);
    warm(&mut editor, &context, size);
    let output = frame(&mut editor, &context, size, vec![]);
    let nodes = output.platform_output.accesskit_update.unwrap().nodes;
    for name in [
        "Turn left",
        "Turn right",
        "Flip horizontal",
        "Flip vertical",
        "Dust repair",
    ] {
        assert!(
            nodes
                .iter()
                .find(|(_, node)| node.label() == Some(name))
                .unwrap()
                .1
                .is_disabled()
        );
    }
    let crop = &nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Crop & straighten"))
        .unwrap()
        .1;
    assert!(!crop.is_disabled());
    assert_eq!(crop.is_selected(), Some(true));
}

#[test]
fn pick_apply_is_present_but_disabled_until_selection_and_while_measuring() {
    let size = egui::vec2(800., 600.);
    let (mut editor, _send, receiver) = fixture();
    add_ready_fixture(&mut editor);
    let context = egui::Context::default();
    editor.begin_pick(Pick::Gamma);
    warm(&mut editor, &context, size);
    let output = frame(&mut editor, &context, size, vec![]);
    let apply = output
        .platform_output
        .accesskit_update
        .unwrap()
        .nodes
        .into_iter()
        .find(|(_, node)| node.label() == Some("Apply"))
        .unwrap()
        .1;
    assert!(apply.is_disabled());
    frame(&mut editor, &context, size, key(egui::Key::Enter));
    assert!(!editor.busy);
    editor.region = Some(Coverage::rectangle(Rect {
        x: 20,
        y: 30,
        width: 32,
        height: 32,
    }));
    frame(&mut editor, &context, size, key(egui::Key::Enter));
    assert!(editor.busy);
    assert!(matches!(
        receiver.try_recv().unwrap(),
        Command::Pick(_, Pick::Gamma, ..)
    ));
    let output = frame(&mut editor, &context, size, vec![]);
    for name in ["Cancel", "Apply"] {
        let node = output
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some(name))
            .unwrap();
        assert!(node.1.is_disabled());
    }
    frame(&mut editor, &context, size, key(egui::Key::Escape));
    assert_eq!(editor.pick, Some(Pick::Gamma));
}
