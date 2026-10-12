use super::*;

fn ui_frame(
    editor: &mut Editor,
    context: &egui::Context,
    events: Vec<egui::Event>,
) -> Vec<(String, egui::Rect)> {
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
            egui::CentralPanel::default().show(ui, |ui| host::App::ui(editor, ui));
        },
    );
    output.textures_delta.clear();
    output
        .shapes
        .into_iter()
        .filter_map(|shape| {
            if let egui::Shape::Text(text) = shape.shape {
                Some((
                    text.galley.job.text.clone(),
                    egui::Rect::from_min_size(text.pos, text.galley.size()),
                ))
            } else {
                None
            }
        })
        .collect()
}
fn click(editor: &mut Editor, context: &egui::Context, point: egui::Pos2) {
    for pressed in [true, false] {
        ui_frame(
            editor,
            context,
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
}
#[test]
fn menu_row_and_cancel_discard_dialog_dispatch_use_the_existing_header() {
    let (mut editor, _send, receiver) = fixture();
    let context = egui::Context::default();
    let initial = ui_frame(&mut editor, &context, vec![]);
    for menu in ["File", "Export"] {
        assert!(initial.iter().any(|(label, _)| label == menu));
    }
    assert!(
        !initial
            .iter()
            .any(|(label, _)| label == "Add RAWs…" || label == "Export TIFF…")
    );
    editor.guard_transition(Transition::New, &context);
    ui_frame(&mut editor, &context, vec![]);
    let modal = ui_frame(&mut editor, &context, vec![]);
    let cancel = modal
        .iter()
        .find(|(label, _)| label == "Cancel")
        .unwrap()
        .1
        .center();
    click(&mut editor, &context, cancel);
    assert!(editor.transition.is_none());
    assert!(receiver.try_recv().is_err());
    editor.guard_transition(Transition::New, &context);
    ui_frame(&mut editor, &context, vec![]);
    let modal = ui_frame(&mut editor, &context, vec![]);
    let discard = modal
        .iter()
        .find(|(label, _)| label == "Discard")
        .unwrap()
        .1
        .center();
    click(&mut editor, &context, discard);
    let Command::Document(_, work) = receiver.try_recv().unwrap() else {
        panic!()
    };
    assert!(matches!(*work, DocumentWork::New));
}
#[test]
fn command_save_commits_pending_numeric_text_before_snapshot() {
    let (mut editor, _send, receiver) = fixture();
    let directory = tempfile::tempdir().unwrap();
    editor.document = editor
        .document
        .save(
            &directory.path().join("numeric.antitone.json"),
            crate::project::Save::New,
        )
        .unwrap();
    editor.tab = 1;
    let context = egui::Context::default();
    ui_frame(&mut editor, &context, vec![]);
    for frame in 0..60 {
        let events = if frame == 0 {
            vec![
                egui::Event::PointerMoved(egui::pos2(950., 550.)),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    phase: egui::TouchPhase::Move,
                    delta: egui::vec2(0., -600.),
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        } else {
            vec![]
        };
        ui_frame(&mut editor, &context, events);
    }
    let shapes = ui_frame(&mut editor, &context, vec![]);
    let exposure = shapes
        .iter()
        .find(|(label, _)| label == "Exposure")
        .unwrap()
        .1;
    let number = shapes
        .iter()
        .filter(|(label, rect)| label.contains("0.00") && rect.min.y >= exposure.min.y)
        .min_by(|(_, a), (_, b)| a.min.y.total_cmp(&b.min.y))
        .unwrap()
        .1
        .center();
    click(&mut editor, &context, number);
    ui_frame(
        &mut editor,
        &context,
        vec![egui::Event::Text("7.123456789".into())],
    );
    assert_eq!(editor.photo.exposure, 0.);
    ui_frame(
        &mut editor,
        &context,
        vec![egui::Event::Key {
            key: egui::Key::S,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        }],
    );
    let Command::Document(_, work) = receiver.try_recv().unwrap() else {
        panic!()
    };
    let DocumentWork::Save(snapshot, _, _) = *work else {
        panic!()
    };
    assert_eq!(
        snapshot
            .project
            .source(editor.current_source.unwrap())
            .unwrap()
            .photo
            .exposure,
        7.123456789
    );
}
#[test]
fn viewport_close_is_vetoed_until_the_dirty_document_decision() {
    let (mut editor, _send, receiver) = fixture();
    let context = egui::Context::default();
    let mut input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1100., 800.),
        )),
        ..Default::default()
    };
    input
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .events
        .push(egui::ViewportEvent::Close);
    let mut output = context.run_ui(input, |ui| {
        egui::CentralPanel::default().show(ui, |ui| host::App::ui(&mut editor, ui));
    });
    output.textures_delta.clear();
    assert!(
        output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .contains(&egui::ViewportCommand::CancelClose)
    );
    assert_eq!(editor.transition, Some(Transition::Close));
    assert!(!editor.allow_close);
    assert!(receiver.try_recv().is_err());
}
#[test]
fn export_cannot_replace_the_open_project_document() {
    let (mut editor, _send, receiver) = fixture();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("project.antitone.json");
    editor.document = editor
        .document
        .save(&path, crate::project::Save::New)
        .unwrap();
    let before = std::fs::read(&path).unwrap();
    editor.export(
        path.clone(),
        OutputFormat::Tiff(OutputTarget::Srgb),
        Overwrite::Confirmed,
    );
    assert!(receiver.try_recv().is_err());
    assert_eq!(std::fs::read(path).unwrap(), before);
    assert!(!editor.busy);
}
fn fixture() -> (Editor, mpsc::Sender<Message>, mpsc::Receiver<Command>) {
    let (commands, receiver) = mpsc::channel();
    let (send, messages) = mpsc::channel();
    let mut editor = Editor::state(
        commands,
        messages,
        Arc::new(std::sync::atomic::AtomicU64::new(0)),
    );
    editor.started = true;
    editor
        .document
        .register(
            Source {
                id: SourceId(0),
                path: "fixture.ARW".into(),
                fingerprint: crate::project::Fingerprint {
                    bytes: 1,
                    blake3: "a".repeat(64),
                },
                development: crate::project::SourceDevelopment {
                    dimensions: [900, 600],
                    active_area: [0, 0, 900, 600],
                    camera_model: "test-camera".into(),
                    camera_to_ap1: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
                    input_floor: crate::processing::CAMERA_INPUT_FLOOR,
                },
                reference_only: false,
                photo: PhotoSettings::default(),
                orientation: Default::default(),
                crop: None,
            },
            Development::new(crate::raw::version(), [2., 1., 1.5, 1.]),
        )
        .unwrap();
    editor.current_source = Some(editor.document.project.order[0]);
    editor.floor = crate::processing::CAMERA_INPUT_FLOOR;
    editor.dmin = Some(DVec3::new(0.2, 0.15, 0.1));
    editor.dmax = Some(DVec3::new(0.008, 0.005, 0.002));
    editor.sync_document();
    (editor, send, receiver)
}
#[test]
fn orientation_is_source_owned_and_restored_without_changing_grade_on_load() {
    let (mut editor, _send, receiver) = fixture();
    let id = editor.current_source.unwrap();
    editor.roll_orientation.horizontal = true;
    editor.roll_orientation.turn_clockwise();
    editor.photo_orientation.turn_clockwise();
    editor.photo_orientation.flips.vertical = true;
    editor.crop = Some(Crop::from_region(Rect {
        x: 30,
        y: 20,
        width: 401,
        height: 203,
    }));
    let crop = editor.crop;
    let orientation = editor.photo_orientation;
    let grade = (editor.roll, editor.photo, editor.dmin, editor.dmax);
    editor.sync_document();
    assert_eq!(editor.document.project.orientation, editor.roll_orientation);
    assert_eq!(
        editor.document.project.source(id).unwrap().orientation,
        orientation
    );
    editor.load("temporary-reference.ARW".into());
    assert_eq!(editor.photo_orientation, PhotoOrientation::default());
    assert!(editor.crop.is_none());
    editor.load("fixture.ARW".into());
    assert_eq!(editor.photo_orientation, orientation);
    assert_eq!(editor.crop, crop);
    assert_eq!((editor.roll, editor.photo, editor.dmin, editor.dmax), grade);
    assert_eq!(editor.roll_orientation, editor.document.project.orientation);
    assert!(matches!(receiver.try_recv().unwrap(), Command::Load(..)));
}
#[test]
fn failed_and_stale_document_results_preserve_state_and_active_request() {
    let (mut editor, send, receiver) = fixture();
    editor.photo.exposure = 7.123456789012345;
    editor.sync_document();
    let before = editor.document.project.clone();
    let active = editor.id;
    editor.start_document(DocumentWork::Open("candidate.json".into()));
    let id = editor.document_pending.unwrap();
    assert!(matches!(receiver.recv().unwrap(), Command::Document(_, _)));
    send.send(Message::Prepared(id + 1, Err("stale candidate".into())))
        .unwrap();
    send.send(Message::Picked(
        active,
        Pick::Dmin,
        Ok(Measurement::Reference(DVec3::ONE)),
    ))
    .unwrap();
    send.send(Message::Ready(active, Err("old load".into())))
        .unwrap();
    editor.receive();
    assert_eq!(editor.document_pending, Some(id));
    assert!(editor.busy);
    assert_eq!(editor.document.project, before);
    send.send(Message::Prepared(id, Err("changed RAW".into())))
        .unwrap();
    editor.receive();
    assert!(!editor.busy);
    assert_eq!(editor.document.project, before);
    assert_eq!(editor.id, active);
    assert_eq!(editor.photo.exposure, 7.123456789012345);
    assert!(editor.document.dirty());
}
#[test]
fn save_failure_preserves_dirty_state_and_does_not_continue_transition() {
    let (mut editor, send, receiver) = fixture();
    let before = editor.document.project.clone();
    editor.queue_save(
        "candidate.json".into(),
        crate::project::Save::New,
        Some(Transition::New),
    );
    let id = editor.document_pending.unwrap();
    assert!(matches!(receiver.recv().unwrap(), Command::Document(_, _)));
    send.send(Message::Saved(id, Err("write failed".into())))
        .unwrap();
    editor.receive();
    assert_eq!(editor.document.project, before);
    assert!(editor.document.dirty());
    assert_eq!(editor.transition, Some(Transition::New));
    assert!(editor.resume_transition.is_none());
    assert!(receiver.try_recv().is_err());
}
#[test]
fn save_snapshot_flushes_current_photo_and_retains_other_source_settings() {
    let (mut editor, _send, receiver) = fixture();
    let one = editor.current_source.unwrap();
    let mut other = editor.document.project.source(one).unwrap().clone();
    other.path = "unvisited.ARW".into();
    let two = editor.document.project.add_source(other).unwrap();
    editor.photo.exposure = 7.123456789012345;
    editor.roll.exposure = -0.23456789012345678;
    editor.queue_save("candidate.json".into(), crate::project::Save::New, None);
    let Command::Document(_, work) = receiver.recv().unwrap() else {
        panic!()
    };
    let DocumentWork::Save(snapshot, _, _) = *work else {
        panic!()
    };
    assert_eq!(snapshot.project.source(one).unwrap().photo, editor.photo);
    assert_eq!(
        snapshot.project.source(two).unwrap().photo,
        PhotoSettings::default()
    );
    assert_eq!(snapshot.project.roll, editor.roll);
    assert_eq!(
        snapshot.project.processing_model,
        editor.roll.response.processing_model()
    );
}
#[test]
fn new_resets_development_and_rejects_previous_document_messages() {
    let (mut editor, send, _receiver) = fixture();
    let previous = editor.id;
    editor.start_document(DocumentWork::New);
    let id = editor.document_pending.unwrap();
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
    editor.receive();
    assert_eq!(editor.document.project, crate::project::Project::default());
    assert!(editor.document.project.development.is_none());
    assert!(editor.current_source.is_none());
    assert_eq!(editor.floor, 0.);
    assert!(!editor.document.dirty());
    send.send(Message::Picked(
        previous,
        Pick::Dmin,
        Ok(Measurement::Reference(DVec3::ONE)),
    ))
    .unwrap();
    editor.receive();
    assert!(editor.dmin.is_none());
    assert!(editor.id > previous);
}
#[test]
fn navigation_and_view_state_do_not_dirty_a_saved_document() {
    let (mut editor, _send, receiver) = fixture();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("project.antitone.json");
    editor.document = editor
        .document
        .save(&path, crate::project::Save::New)
        .unwrap();
    let before = editor.document.project.clone();
    editor.source = false;
    editor.viewport.zoom = Some(2.);
    editor.region = Some(Coverage::rectangle(Rect {
        x: 12,
        y: 12,
        width: 32,
        height: 32,
    }));
    let source_path = editor
        .document
        .source_path(editor.current_source.unwrap())
        .unwrap();
    editor.load(source_path);
    let Command::Load(_, _, wb, expected, _, _) = receiver.recv().unwrap() else {
        panic!()
    };
    assert_eq!(
        wb,
        editor
            .document
            .project
            .development
            .as_ref()
            .map(|value| value.fixed_wb)
    );
    assert!(expected.is_some());
    editor.sync_document();
    assert_eq!(editor.document.project, before);
    assert!(!editor.document.dirty());
}
#[test]
fn dirty_open_new_and_close_are_guarded_without_dispatching_or_allowing_close() {
    let (mut editor, _send, receiver) = fixture();
    let context = egui::Context::default();
    for transition in [
        Transition::New,
        Transition::Open("candidate.json".into()),
        Transition::Close,
    ] {
        editor.guard_transition(transition.clone(), &context);
        assert_eq!(editor.transition, Some(transition));
        assert!(receiver.try_recv().is_err());
        assert!(!editor.allow_close);
        editor.transition = None;
    }
}
#[test]
fn command_save_uses_the_current_document_after_control_processing() {
    let (mut editor, _send, receiver) = fixture();
    let directory = tempfile::tempdir().unwrap();
    editor.document = editor
        .document
        .save(
            &directory.path().join("project.antitone.json"),
            crate::project::Save::New,
        )
        .unwrap();
    editor.photo.exposure = 7.123456789012345;
    let context = egui::Context::default();
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1100., 800.),
            )),
            events: vec![egui::Event::Key {
                key: egui::Key::S,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            }],
            ..Default::default()
        },
        |ui| {
            egui::CentralPanel::default().show(ui, |ui| host::App::ui(&mut editor, ui));
        },
    );
    output.textures_delta.clear();
    let Command::Document(_, work) = receiver.try_recv().unwrap() else {
        panic!()
    };
    let DocumentWork::Save(snapshot, _, _) = *work else {
        panic!()
    };
    assert_eq!(
        snapshot
            .project
            .source(editor.current_source.unwrap())
            .unwrap()
            .photo,
        editor.photo
    );
}
