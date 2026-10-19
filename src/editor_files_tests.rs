use super::*;

#[test]
fn project_save_names_use_antitone_without_duplicating_legacy_suffixes() {
    assert_eq!(project_save_name(None), "Untitled.antitone");
    for (path, expected) in [
        ("/projects/grade.antitone", "grade.antitone"),
        ("/projects/grade.ANTITONE", "grade.ANTITONE"),
        ("/projects/grade.json", "grade.antitone"),
        ("/projects/grade.antitone.json", "grade.antitone"),
        ("/projects/grade.ANTITONE.JSON", "grade.ANTITONE"),
        ("/projects/照片.v2.json", "照片.v2.antitone"),
    ] {
        assert_eq!(
            project_save_name(Some(std::path::Path::new(path))),
            expected
        );
    }
}

#[test]
fn raw_picker_offers_common_camera_formats_not_only_arw() {
    for extension in [
        "arw", "cr2", "cr3", "dng", "nef", "orf", "pef", "raf", "rw2", "srw",
    ] {
        assert!(RAW_EXTENSIONS.contains(&extension), "{extension}");
    }
    assert!(RAW_EXTENSIONS.windows(2).all(|pair| pair[0] < pair[1]));
    for extension in ["gpr", "jpg", "png", "tif", "tiff", "x3f"] {
        assert!(!RAW_EXTENSIONS.contains(&extension), "{extension}");
    }
}

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
fn cancel_preserves_the_document_and_discard_dispatches_the_guarded_transition() {
    let (mut editor, _send, receiver) = fixture();
    let context = egui::Context::default();
    ui_frame(&mut editor, &context, vec![]);
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
        .find(|(label, _)| label == "Discard changes")
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
fn save_transition_prompts_are_neutral_and_make_edit_loss_conditional_on_not_saving() {
    for (transition, explanation) in [
        (
            Transition::New,
            "Creating a new project without saving will discard unsaved edits in this project.",
        ),
        (
            Transition::Open(PathBuf::from("another.antitone.json")),
            "Opening another project without saving will discard unsaved edits in this project.",
        ),
        (
            Transition::Close,
            "Closing Antitone without saving will discard unsaved edits in this project.",
        ),
    ] {
        let (mut editor, _send, _receiver) = fixture();
        let context = egui::Context::default();
        editor.transition = Some(transition);
        ui_frame(&mut editor, &context, vec![]);
        let labels = ui_frame(&mut editor, &context, vec![]);
        assert!(
            labels
                .iter()
                .any(|(label, _)| label == "Save changes to this project?")
        );
        assert!(labels.iter().any(|(label, _)| label == explanation));
        assert!(
            !labels
                .iter()
                .any(|(label, _)| matches!(label.as_str(), "Warning" | "Information" | "Error"))
        );
    }
}

#[test]
fn command_save_commits_pending_numeric_text_before_snapshot() {
    numeric_save(SaveInput::Shortcut);
}

#[test]
fn native_menu_save_commits_pending_numeric_text_before_snapshot() {
    numeric_save(SaveInput::NativeMenu);
}

#[test]
fn native_paste_precedes_a_queued_save_before_focus_is_committed() {
    numeric_save(SaveInput::PasteThenSave);
}

#[test]
fn system_quit_preserves_leading_paste_before_the_save_decision() {
    numeric_save(SaveInput::PasteThenSystemQuit);
}

#[derive(Clone, Copy)]
enum SaveInput {
    Shortcut,
    NativeMenu,
    PasteThenSave,
    PasteThenSystemQuit,
}

fn numeric_save(route: SaveInput) {
    let (mut editor, _send, receiver) = fixture();
    if !matches!(route, SaveInput::Shortcut) {
        host::App::native_menu_installed(&mut editor);
    }
    let directory = tempfile::tempdir().unwrap();
    editor.document = editor
        .document
        .save(
            &directory.path().join("numeric.antitone"),
            crate::project::Save::New,
        )
        .unwrap();
    editor.tab = 1;
    // Keep the pending Exposure edit visible; scrolling is unrelated to this contract.
    for section in ["photo-white-balance", "photo-contrast", "photo-saturation"] {
        editor.preferences.sections.insert(section.into(), false);
    }
    let context = egui::Context::default();
    for _ in 0..12 {
        ui_frame(&mut editor, &context, Vec::new());
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
    assert!(
        context.text_edit_focused(),
        "numeric field was not focused: {shapes:?}"
    );
    ui_frame(
        &mut editor,
        &context,
        vec![egui::Event::Text(
            if matches!(
                route,
                SaveInput::PasteThenSave | SaveInput::PasteThenSystemQuit
            ) {
                "1.25"
            } else {
                "7.123456789"
            }
            .into(),
        )],
    );
    assert_eq!(editor.photo.exposure, 0.);
    use crate::platform::menu::Action;
    let events = match route {
        SaveInput::NativeMenu => {
            host::App::menu_action(&mut editor, Action::Save);
            vec![]
        }
        SaveInput::PasteThenSave => {
            for action in [Action::SelectAll, Action::Paste, Action::Save] {
                host::App::menu_action(&mut editor, action);
            }
            host::App::menu_input(&mut editor, Some("7.123456789".into()))
        }
        SaveInput::PasteThenSystemQuit => {
            for action in [Action::SelectAll, Action::Paste, Action::Settings] {
                host::App::menu_action(&mut editor, action);
            }
            host::App::request_quit(&mut editor);
            host::App::menu_input(&mut editor, Some("7.123456789".into()))
        }
        SaveInput::Shortcut => vec![egui::Event::Key {
            key: egui::Key::S,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        }],
    };
    ui_frame(&mut editor, &context, events);
    if !matches!(route, SaveInput::NativeMenu) {
        assert!(
            receiver.try_recv().is_err(),
            "input must be processed before committing focus"
        );
        ui_frame(&mut editor, &context, vec![]);
    }
    if matches!(route, SaveInput::PasteThenSystemQuit) {
        assert_eq!(editor.transition, Some(Transition::Close));
        assert_eq!(
            host::App::quit_state(&editor),
            Some(host::QuitState::Pending)
        );
        assert!(!editor.allow_close);
        let shapes = ui_frame(&mut editor, &context, vec![]);
        let save = shapes
            .iter()
            .find(|(label, _)| label == "Save changes")
            .unwrap()
            .1
            .center();
        click(&mut editor, &context, save);
    }
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
    ui_frame(&mut editor, &context, vec![]);
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
    let mut output = context.run_ui(input.clone(), |ui| {
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
    assert_eq!(editor.failure.as_ref().unwrap().kind, FailureKind::Export);
}

#[test]
fn document_and_export_decisions_ignore_outside_clicks_and_cancel_without_dispatch() {
    use super::super::presentation::tests::key;
    for decision in 0..3 {
        for cancel_key in [egui::Key::Escape, egui::Key::Enter] {
            let (mut editor, _send, receiver) = fixture();
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("existing-file");
            std::fs::write(&path, b"unchanged destination").unwrap();
            let before = editor.document.project.clone();
            match decision {
                0 => editor.transition = Some(Transition::Close),
                1 => editor.project_overwrite = Some((path.clone(), Some(Transition::New))),
                _ => editor.overwrite = Some((path.clone(), OutputFormat::Exr(ExrPrecision::Half))),
            }
            let context = egui::Context::default();
            ui_frame(&mut editor, &context, vec![]);
            ui_frame(&mut editor, &context, vec![]);
            click(&mut editor, &context, egui::pos2(30., 15.));
            assert!(
                editor.transition.is_some()
                    || editor.project_overwrite.is_some()
                    || editor.overwrite.is_some()
            );
            assert!(receiver.try_recv().is_err());
            ui_frame(&mut editor, &context, key(cancel_key));
            assert!(
                editor.transition.is_none()
                    && editor.project_overwrite.is_none()
                    && editor.overwrite.is_none()
            );
            assert!(receiver.try_recv().is_err());
            assert!(!editor.allow_close && !editor.busy);
            assert_eq!(editor.document.project, before);
            assert_eq!(std::fs::read(path).unwrap(), b"unchanged destination");
        }
    }
}

#[test]
fn save_decision_queues_the_snapshot_with_its_original_continuation() {
    let (mut editor, _send, receiver) = fixture();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("project.antitone");
    editor.document = editor
        .document
        .save(&path, crate::project::Save::New)
        .unwrap();
    editor.photo.exposure = 1.25;
    let context = egui::Context::default();
    editor.guard_transition(Transition::Close, &context);
    ui_frame(&mut editor, &context, vec![]);
    let labels = ui_frame(&mut editor, &context, vec![]);
    let point = labels
        .iter()
        .find(|(label, _)| label == "Save changes")
        .unwrap()
        .1
        .center();
    click(&mut editor, &context, point);
    assert!(editor.transition.is_none());
    assert_eq!(editor.after_save, Some(Transition::Close));
    assert!(!editor.allow_close);
    let Command::Document(_, work) = receiver.try_recv().unwrap() else {
        panic!()
    };
    let DocumentWork::Save(snapshot, destination, policy) = *work else {
        panic!()
    };
    assert_eq!(destination, path);
    assert!(matches!(policy, crate::project::Save::Replace));
    assert_eq!(snapshot.project, editor.document.project);
    assert_eq!(
        snapshot
            .project
            .source(editor.current_source.unwrap())
            .unwrap()
            .photo
            .exposure,
        1.25
    );
    assert!(receiver.try_recv().is_err());
}

#[test]
fn project_replace_decision_preserves_destination_snapshot_and_continuation() {
    let (mut editor, _send, receiver) = fixture();
    let path = PathBuf::from("existing.antitone.json");
    editor.project_overwrite = Some((path.clone(), Some(Transition::New)));
    let context = egui::Context::default();
    ui_frame(&mut editor, &context, vec![]);
    let labels = ui_frame(&mut editor, &context, vec![]);
    let point = labels
        .iter()
        .find(|(label, _)| label == "Replace project")
        .unwrap()
        .1
        .center();
    click(&mut editor, &context, point);
    assert!(editor.project_overwrite.is_none());
    assert_eq!(editor.after_save, Some(Transition::New));
    let Command::Document(_, work) = receiver.try_recv().unwrap() else {
        panic!()
    };
    let DocumentWork::Save(snapshot, destination, policy) = *work else {
        panic!()
    };
    assert_eq!(destination, path);
    assert_eq!(snapshot.project, editor.document.project);
    assert!(matches!(policy, crate::project::Save::Replace));
    assert!(receiver.try_recv().is_err());
}

#[test]
fn export_replace_decision_dispatches_the_staged_format_and_current_accepted_grade_once() {
    for format in [
        OutputFormat::Tiff(OutputTarget::Srgb),
        OutputFormat::Tiff(OutputTarget::DisplayP3),
        OutputFormat::Exr(ExrPrecision::Half),
        OutputFormat::Exr(ExrPrecision::Full),
    ] {
        let (mut editor, _send, receiver) = super::super::tests::fixture();
        super::super::tests::add_ready_fixture(&mut editor);
        editor.crop = Some(Crop::from_region(Rect {
            x: 20,
            y: 20,
            width: 100,
            height: 80,
        }));
        editor.sync_document();
        let path = PathBuf::from("existing-output");
        editor.overwrite = Some((path.clone(), format));
        let before = editor.document.project.clone();
        let context = egui::Context::default();
        ui_frame(&mut editor, &context, vec![]);
        let labels = ui_frame(&mut editor, &context, vec![]);
        let point = labels
            .iter()
            .find(|(label, _)| label == "Replace file")
            .unwrap()
            .1
            .center();
        click(&mut editor, &context, point);
        assert!(editor.overwrite.is_none());
        let Command::Export(id, destination, dispatched, recipe, policy) =
            receiver.try_recv().unwrap()
        else {
            panic!()
        };
        assert_eq!(id.source, editor.id);
        assert_eq!(Some(id.request), editor.export_pending);
        assert_eq!(destination, path);
        match (dispatched, format) {
            (OutputFormat::Tiff(a), OutputFormat::Tiff(b)) => assert_eq!(a, b),
            (OutputFormat::Exr(ExrPrecision::Half), OutputFormat::Exr(ExrPrecision::Half))
            | (OutputFormat::Exr(ExrPrecision::Full), OutputFormat::Exr(ExrPrecision::Full)) => {}
            _ => panic!("staged export format changed"),
        }
        assert_eq!(recipe.neutral.calibration, editor.calibration().unwrap());
        assert_eq!(recipe.roll, editor.roll);
        assert_eq!(recipe.grade.global(), editor.photo);
        assert_eq!(recipe.composition.crop(), editor.crop.unwrap());
        assert!(matches!(policy, Overwrite::Confirmed));
        assert_eq!(editor.document.project, before);
        assert!(receiver.try_recv().is_err());
    }
}

fn fixture() -> (Editor, mpsc::Sender<Message>, mpsc::Receiver<Command>) {
    let (commands, receiver) = mpsc::channel();
    let (send, messages) = mpsc::channel();
    let mut editor = Editor::state(
        commands,
        messages,
        Arc::new(std::sync::atomic::AtomicU64::new(0)),
    );
    editor
        .document
        .register(
            Source {
                id: SourceId(0),
                path: "fixture.ARW".into(),
                development: crate::project::SourceDevelopment {
                    lens_correction: None,
                    dimensions: [900, 600],
                    active_area: [0, 0, 900, 600],
                    camera_model: "test-camera".into(),
                    camera_to_ap1: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
                    input_floor: crate::processing::CAMERA_INPUT_FLOOR,
                },
                reference_only: false,
                photo: PhotoSettings::default(),
                local_adjustments: None,
                repairs: Default::default(),
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
    editor.photo = PhotoSettings {
        contrast: 0.4,
        pivot: 0.25,
        contrast_colour_mix: 0.75,
        saturation: 1.3,
        saturation_density: 0.25,
        ..PhotoSettings::default()
    };
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
    let mut other = editor.document.project.source(id).unwrap().clone();
    other.path = "second.ARW".into();
    other.orientation = PhotoOrientation::default();
    other.crop = None;
    other.photo = PhotoSettings {
        contrast: 1.6,
        saturation: 0.7,
        contrast_colour_mix: 0.25,
        saturation_density: 0.75,
        ..PhotoSettings::default()
    };
    let other_photo = other.photo;
    let other_id = editor.document.project.add_source(other).unwrap();
    editor.load("second.ARW".into());
    assert_eq!(editor.current_source, Some(other_id));
    assert_eq!(editor.photo, other_photo);
    editor.load("fixture.ARW".into());
    assert_eq!(editor.current_source, Some(id));
    assert_eq!((editor.roll, editor.photo, editor.dmin, editor.dmax), grade);
    editor.load("temporary-reference.ARW".into());
    assert_eq!(editor.photo, PhotoSettings::default());
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
    assert_eq!(
        editor
            .source_epoch
            .load(std::sync::atomic::Ordering::Relaxed),
        id
    );
    assert!(editor.busy);
    assert_eq!(editor.document.project, before);
    send.send(Message::Prepared(id, Err("changed RAW".into())))
        .unwrap();
    editor.receive();
    assert!(!editor.busy);
    assert_eq!(editor.document.project, before);
    assert_eq!(editor.id, active);
    assert_eq!(
        editor
            .source_epoch
            .load(std::sync::atomic::Ordering::Relaxed),
        active
    );
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
    assert_eq!(
        editor
            .source_epoch
            .load(std::sync::atomic::Ordering::Relaxed),
        editor.id
    );
    assert!(receiver.try_recv().is_err());
}

#[test]
fn save_snapshot_flushes_current_photo_and_retains_other_source_settings() {
    let (mut editor, _send, receiver) = fixture();
    let one = editor.current_source.unwrap();
    let mut other = editor.document.project.source(one).unwrap().clone();
    other.path = "unvisited.ARW".into();
    let two = editor.document.project.add_source(other).unwrap();
    editor.photo = PhotoSettings {
        exposure: 7.123456789012345,
        contrast: 1.6,
        pivot: 0.25,
        contrast_colour_mix: 0.75,
        saturation: 1.3,
        saturation_density: 0.25,
        ..PhotoSettings::default()
    };
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
    let Command::Load(_, _, wb, expected, _, _, _) = receiver.recv().unwrap() else {
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
    let context = egui::Context::default();
    ui_frame(&mut editor, &context, vec![]);
    editor.photo.exposure = 7.123456789012345;
    assert_eq!(
        editor
            .document
            .project
            .source(editor.current_source.unwrap())
            .unwrap()
            .photo
            .exposure,
        0.,
    );
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
