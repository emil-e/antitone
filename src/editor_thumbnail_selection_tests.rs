use super::*;
use egui::{Event, FullOutput, Key, Modifiers, Pos2};

fn frame(editor: &mut Editor, context: &egui::Context, events: Vec<Event>) -> FullOutput {
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
            editor.filmstrip(ui);
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

fn selected_outline(output: &FullOutput, point: Pos2) {
    let card = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Rect(rect) if rect.rect.contains(point) && rect.stroke.width == 1.5 => {
                Some(rect)
            }
            _ => None,
        })
        .expect("selected card outline");
    assert_eq!(card.fill, egui::Color32::from_gray(30));
}

#[test]
fn cold_loading_and_failed_selected_raw_retain_selection_and_filename() {
    let (mut editor, send, commands) = crate::editor::tests::fixture();
    editor.document = super::tests::fixture();
    editor.document.project.order.truncate(3);
    let selected = editor.document.project.order[1];
    let name = "京都_Long_Scan_Reference_Roll01_Frame_04397.ARW";
    editor.document.project.source_mut(selected).unwrap().path = name.into();
    editor.selected = 1;
    editor.load(editor.document.source_path(selected).unwrap());
    assert!(matches!(commands.try_recv().unwrap(), Command::Load(..)));
    assert!(editor.busy && editor.ready.is_none() && editor.pending_preview.is_none());
    let before = editor.document.project.clone();
    let context = egui::Context::default();
    frame(&mut editor, &context, vec![]);
    let output = frame(&mut editor, &context, vec![]);
    assert_eq!(
        node(&output, name).1.toggled(),
        Some(egui::accesskit::Toggled::True)
    );
    assert!(
        !node(&output, name).1.is_disabled(),
        "loading a photo must not block navigation"
    );
    selected_outline(&output, point(&output, name));
    assert!(editor.ready.is_none() && editor.pending_preview.is_none());
    assert_eq!(editor.document.project, before);
    send.send(Message::Ready(editor.id, Err("RAW unavailable".into())))
        .unwrap();
    editor.receive();
    assert!(!editor.busy && editor.ready.is_none() && editor.pending_preview.is_none());
    let output = frame(&mut editor, &context, vec![]);
    selected_outline(&output, point(&output, name));
    assert_eq!(
        node(&output, name).1.toggled(),
        Some(egui::accesskit::Toggled::True)
    );
    assert!(
        !node(&output, name).1.is_disabled(),
        "allow retry after failure"
    );
    assert_eq!(editor.current_source, Some(selected));
    assert_eq!(editor.document.project, before);
}

#[test]
fn filmstrip_availability_blocks_mouse_and_keyboard_and_releases_existing_focus() {
    let (mut editor, _, commands) = crate::editor::tests::fixture();
    editor.document = super::tests::fixture();
    editor.document.project.order.truncate(2);
    crate::editor::tests::add_ready_fixture(&mut editor);
    editor.selected = 2;
    editor.sync_document();
    let before = editor.document.project.clone();
    let context = egui::Context::default();
    frame(&mut editor, &context, vec![]);
    let output = frame(&mut editor, &context, vec![]);
    let target = node(&output, "photo-0.ARW").0;
    let point = point(&output, "photo-0.ARW");
    for state in ["busy", "crop", "flat"] {
        let mut focused = false;
        for _ in 0..12 {
            frame(&mut editor, &context, key(Key::Tab));
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
            "flat" => {
                editor.flat_edit = Some(crate::editor::flat::FlatEdit {
                    id: editor.id,
                    ready: None,
                    viewport: editor.viewport,
                    source: editor.source,
                })
            }
            _ => unreachable!(),
        }
        let output = frame(&mut editor, &context, key(Key::Space));
        assert_eq!(node(&output, "photo-0.ARW").0, target);
        assert!(node(&output, "photo-0.ARW").1.is_disabled());
        assert!(!node(&output, "Following").1.is_disabled());
        assert_ne!(
            context
                .memory(|memory| memory.focused())
                .map(|id| id.accesskit_id()),
            Some(target)
        );
        frame(&mut editor, &context, pointer(point, true));
        frame(&mut editor, &context, pointer(point, false));
        for _ in 0..5 {
            frame(&mut editor, &context, key(Key::Tab));
            assert_ne!(
                context
                    .memory(|memory| memory.focused())
                    .map(|id| id.accesskit_id()),
                Some(target)
            );
        }
        assert!(
            commands.try_recv().is_err(),
            "disabled cards dispatch no work"
        );
        assert_eq!(editor.document.project, before);
        editor.busy = false;
        editor.finish_crop(false);
        editor.flat_edit = None;
        frame(&mut editor, &context, vec![]);
    }
    frame(&mut editor, &context, pointer(point, true));
    frame(&mut editor, &context, pointer(point, false));
    assert!(
        matches!(commands.try_recv().unwrap(), Command::Load(_, path, ..) if path.ends_with("photo-0.ARW"))
    );
    assert_eq!(editor.selected, 0);
}

#[test]
fn document_work_still_blocks_navigation_when_a_selected_load_was_pending() {
    let (mut editor, _, commands) = crate::editor::tests::fixture();
    editor.document = super::tests::fixture();
    editor.document.project.order.truncate(2);
    editor.load(
        editor
            .document
            .source_path(editor.document.project.order[0])
            .unwrap(),
    );
    assert!(matches!(commands.try_recv().unwrap(), Command::Load(..)));
    editor.document_pending = Some(999);
    let context = egui::Context::default();
    frame(&mut editor, &context, vec![]);
    let output = frame(&mut editor, &context, vec![]);
    assert!(node(&output, "photo-1.ARW").1.is_disabled());
    let point = point(&output, "photo-1.ARW");
    frame(&mut editor, &context, pointer(point, true));
    frame(&mut editor, &context, pointer(point, false));
    assert!(commands.try_recv().is_err());
}

#[test]
fn disabled_cards_keep_actionable_error_tooltips() {
    let (mut editor, _, _) = crate::editor::tests::fixture();
    editor.document = super::tests::fixture();
    editor.document.project.order.truncate(1);
    editor.current_source = Some(editor.document.project.order[0]);
    editor.busy = true;
    let image_key = super::tests::key(&editor.document, editor.current_source.unwrap());
    editor.thumbnails.cache.insert(
        image_key,
        Err("Source unavailable: actionable detail".into()),
    );
    let context = egui::Context::default();
    context.style_mut_of(egui::Theme::Dark, |style| {
        style.interaction.tooltip_delay = 0.
    });
    frame(&mut editor, &context, vec![]);
    let output = frame(&mut editor, &context, vec![]);
    let point = point(&output, "photo-0.ARW");
    let mut visible = false;
    for _ in 0..4 {
        let output = frame(&mut editor, &context, vec![Event::PointerMoved(point)]);
        visible |= output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.job.text == "Source unavailable: actionable detail"));
    }
    assert!(visible, "disabled selection must not hide error detail");
}

#[test]
fn enabled_reference_cards_keep_ordered_roll_sampling_and_working_photo_return_context() {
    for pick in [Pick::Dmin, Pick::Dmax] {
        let (mut editor, send, commands) = crate::editor::tests::fixture();
        editor.document = super::tests::fixture();
        editor.document.project.order.truncate(2);
        crate::editor::tests::add_ready_fixture(&mut editor);
        editor.selected = 2;
        editor.sync_document();
        let before = editor.document.project.clone();
        let working = editor.ready.as_ref().unwrap().path.clone();
        editor.begin_pick(pick);
        let view = editor.pick_view.unwrap();
        let context = egui::Context::default();
        frame(&mut editor, &context, vec![]);
        let output = frame(&mut editor, &context, vec![]);
        assert!(!node(&output, "photo-0.ARW").1.is_disabled());
        let point = point(&output, "photo-0.ARW");
        frame(&mut editor, &context, pointer(point, true));
        frame(&mut editor, &context, pointer(point, false));
        assert!(
            matches!(commands.try_recv().unwrap(), Command::Load(_, path, ..) if path.ends_with("photo-0.ARW"))
        );
        assert_eq!(
            editor.current_source,
            Some(editor.document.project.order[0])
        );
        assert_eq!(editor.pick, Some(pick));
        assert_eq!(editor.selected, 0);
        assert_eq!(editor.pick_return.as_ref().unwrap().selected, 2);
        assert_eq!(editor.pick_return.as_ref().unwrap().path, working);
        assert_eq!(editor.pick_view.unwrap().viewport, view.viewport);
        assert_eq!(editor.pick_view.unwrap().source, view.source);
        assert_eq!(editor.document.project, before);
        send.send(Message::Ready(
            editor.id,
            Err("sample source unavailable".into()),
        ))
        .unwrap();
        editor.receive();
        editor.finish_pick();
        assert!(
            matches!(commands.try_recv().unwrap(), Command::Load(_, path, ..) if path == working)
        );
        assert_eq!(editor.selected, 2);
        assert_eq!(editor.viewport, view.viewport);
        assert_eq!(editor.source, view.source);
        assert!(editor.pick.is_none());
        assert_eq!(editor.document.project, before);
    }
}

#[test]
fn pending_photo_loading_accepts_mouse_and_keyboard_replacement_and_ignores_old_ready() {
    for keyboard in [false, true] {
        let (mut editor, send, commands) = crate::editor::tests::fixture();
        editor.document = super::tests::fixture();
        editor.document.project.order.truncate(2);
        let first = editor.document.project.order[0];
        let second = editor.document.project.order[1];
        editor.load(editor.document.source_path(first).unwrap());
        let original = editor.id;
        assert!(matches!(commands.try_recv().unwrap(), Command::Load(..)));
        let context = egui::Context::default();
        frame(&mut editor, &context, vec![]);
        let output = frame(&mut editor, &context, vec![]);
        let target = node(&output, "photo-1.ARW").0;
        assert!(!node(&output, "photo-1.ARW").1.is_disabled());
        if keyboard {
            let mut focused = false;
            for _ in 0..12 {
                frame(&mut editor, &context, key(Key::Tab));
                if context.memory(|m| m.focused()).map(|id| id.accesskit_id()) == Some(target) {
                    focused = true;
                    break;
                }
            }
            assert!(focused);
            frame(&mut editor, &context, key(Key::Space));
        } else {
            let point = point(&output, "photo-1.ARW");
            frame(&mut editor, &context, pointer(point, true));
            frame(&mut editor, &context, pointer(point, false));
        }
        assert_eq!(editor.current_source, Some(second));
        assert_eq!(editor.selected, 1);
        assert!(editor.busy && editor.ready.is_none());
        assert!(editor.id > original);
        assert!(matches!(commands.try_recv().unwrap(), Command::Load(..)));
        send.send(Message::Ready(original, Err("obsolete failure".into())))
            .unwrap();
        editor.receive();
        assert!(editor.busy && editor.current_source == Some(second));
        assert_ne!(editor.status, "obsolete failure");
        editor.begin_pick(Pick::Dmin);
        editor.begin_crop();
        assert!(editor.pick.is_none() && editor.crop_edit.is_none());
    }
}
