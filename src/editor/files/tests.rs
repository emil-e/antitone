use super::*;
use crate::application::test_support::with_application;
use crate::project::Development;

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

#[test]
fn initial_paths_route_by_raw_extension_not_by_make() {
    for path in [
        "/scans/DSC04378.ARW",
        "/scans/IMG_0051.cr2",
        "/scans/Canon_EOS_R_RAW_ISO_100.CR3",
        "/scans/DSC_0977.nef",
        "/scans/P1000513.ORF",
        "/scans/P1010136.RW2",
        "/scans/patches.dng",
    ] {
        assert!(is_raw_path(std::path::Path::new(path)), "{path}");
    }
    for path in [
        "/projects/roll.antitone",
        "/projects/grade.json",
        "/scans/scan.tif",
        "/scans/no-extension",
    ] {
        assert!(!is_raw_path(std::path::Path::new(path)), "{path}");
    }
}

fn ui_frame(
    editor: &mut impl crate::editor::tests::UiFixture,
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
            egui::CentralPanel::default().show(ui, |ui| editor.ui(ui));
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

fn click(
    editor: &mut impl crate::editor::tests::UiFixture,
    context: &egui::Context,
    point: egui::Pos2,
) {
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
    let editor = fixture();
    let before = editor.document.state.project.clone();
    let context = egui::Context::default();
    with_application(editor, |app| {
        ui_frame(app, &context, vec![]);
        app.enter(|e, cx| e.guard_transition(Transition::New, cx));
        ui_frame(app, &context, vec![]);
        let modal = ui_frame(app, &context, vec![]);
        let cancel = modal
            .iter()
            .find(|(label, _)| label == "Cancel")
            .unwrap()
            .1
            .center();
        click(app, &context, cancel);
        app.read(|e| {
            assert!(e.transition.is_none() && e.document_pending.is_none());
            assert_eq!(e.document.state.project, before);
        });
        app.enter(|e, cx| e.guard_transition(Transition::New, cx));
        ui_frame(app, &context, vec![]);
        let modal = ui_frame(app, &context, vec![]);
        let discard = modal
            .iter()
            .find(|(label, _)| label == "Discard changes")
            .unwrap()
            .1
            .center();
        click(app, &context, discard);
        assert!(app.read(|e| e.busy && e.document_pending.is_some()));
        app.run_until(Duration::from_secs(15), |e| e.document_pending.is_none());
        app.read(|e| {
            assert_eq!(e.document.state.project, crate::project::Project::default());
            assert!(e.failure.is_none() && !e.document.dirty());
        });
    });
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
        let mut editor = fixture();
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
    let mut editor = fixture();
    // Keep the numeric controls visible below the layer list in this event fixture.
    editor.scopes.visible = false;
    if !matches!(route, SaveInput::Shortcut) {
        host::App::native_menu_installed(&mut editor);
    }
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("numeric.antitone");
    *editor.document = editor
        .document
        .save(&path, crate::project::Save::New)
        .unwrap();
    let id = editor.document.visit.current_source.unwrap();
    editor.tab = 1;
    for section in ["photo-white-balance", "photo-contrast", "photo-saturation"] {
        editor.preferences.sections.insert(section.into(), false);
    }
    let context = egui::Context::default();
    with_application(editor, |app| {
        for _ in 0..12 {
            ui_frame(app, &context, vec![]);
        }
        let shapes = ui_frame(app, &context, vec![]);
        let exposure = shapes
            .iter()
            .find(|(label, _)| label == "Exposure")
            .unwrap()
            .1;
        let number = shapes
            .iter()
            .filter(|(label, rect)| label.contains("0.00") && rect.min.y >= exposure.min.y)
            .min_by(|(_, a), (_, b)| a.min.y.total_cmp(&b.min.y))
            .unwrap_or_else(|| panic!("exposure value is not visible: {shapes:?}"))
            .1
            .center();
        click(app, &context, number);
        assert!(
            context.text_edit_focused(),
            "numeric field was not focused: {shapes:?}"
        );
        ui_frame(
            app,
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
        assert_eq!(
            app.read(|e| e.document.visit.adjustment_layer_settings.exposure),
            0.
        );
        use crate::platform::menu::Action;
        let events = app.update(|e| match route {
            SaveInput::NativeMenu => {
                host::App::menu_action(e, Action::Save);
                vec![]
            }
            SaveInput::PasteThenSave => {
                for action in [Action::SelectAll, Action::Paste, Action::Save] {
                    host::App::menu_action(e, action);
                }
                host::App::menu_input(e, Some("7.123456789".into()))
            }
            SaveInput::PasteThenSystemQuit => {
                for action in [Action::SelectAll, Action::Paste, Action::Settings] {
                    host::App::menu_action(e, action);
                }
                host::App::request_quit(e);
                host::App::menu_input(e, Some("7.123456789".into()))
            }
            SaveInput::Shortcut => vec![egui::Event::Key {
                key: egui::Key::S,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            }],
        });
        ui_frame(app, &context, events);
        if !matches!(route, SaveInput::NativeMenu) {
            ui_frame(app, &context, vec![]);
        }
        if matches!(route, SaveInput::PasteThenSystemQuit) {
            app.read(|e| {
                assert_eq!(e.transition, Some(Transition::Close));
                assert_eq!(host::App::quit_state(e), Some(host::QuitState::Pending));
                assert!(!e.allow_close);
            });
            let shapes = ui_frame(app, &context, vec![]);
            let save = shapes
                .iter()
                .find(|(label, _)| label == "Save changes")
                .unwrap()
                .1
                .center();
            click(app, &context, save);
        }
        assert!(app.read(|e| e.busy && e.document_pending.is_some()));
        app.run_until(Duration::from_secs(15), |e| e.document_pending.is_none());
        assert_eq!(
            crate::project::Project::load(&path)
                .unwrap()
                .source(id)
                .unwrap()
                .adjustment_layers
                .as_ref()
                .unwrap()
                .layers[0]
                .adjustment
                .photo
                .exposure,
            7.123456789
        );
        app.read(|e| {
            assert!(e.failure.is_none() && !e.document.dirty(), "{}", e.status);
            assert_eq!(
                e.allow_close,
                matches!(route, SaveInput::PasteThenSystemQuit)
            );
        });
    });
}

#[test]
fn viewport_close_is_vetoed_until_the_dirty_document_decision() {
    let mut editor = fixture();
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
        egui::CentralPanel::default().show(ui, |ui| {
            crate::editor::tests::paint_controller(&mut editor, ui)
        });
    });
    output.textures_delta.clear();
    assert!(
        output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .contains(&egui::ViewportCommand::CancelClose)
    );
    assert_eq!(editor.transition, Some(Transition::Close));
    assert!(!editor.allow_close);
    assert!(editor.document_pending.is_none() && editor.document.export.operation.is_none());
    let mut output = context.run_ui(input, |ui| {
        egui::CentralPanel::default().show(ui, |ui| {
            crate::editor::tests::paint_controller(&mut editor, ui)
        });
    });
    output.textures_delta.clear();
    assert!(
        output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .contains(&egui::ViewportCommand::CancelClose)
    );
    assert_eq!(editor.transition, Some(Transition::Close));
    assert!(!editor.allow_close);
    assert!(editor.document_pending.is_none() && editor.document.export.operation.is_none());
}

#[test]
fn export_cannot_replace_the_open_project_document() {
    let mut editor = fixture();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("project.antitone.json");
    *editor.document = editor
        .document
        .save(&path, crate::project::Save::New)
        .unwrap();
    let before = std::fs::read(&path).unwrap();
    editor.export(
        path.clone(),
        OutputFormat::Tiff(OutputTarget::Srgb),
        Overwrite::Confirmed,
        &UiContext::render_only(),
    );
    assert!(editor.document.export.operation.is_none());
    assert_eq!(std::fs::read(path).unwrap(), before);
    assert!(!editor.busy);
    assert_eq!(editor.failure.as_ref().unwrap().kind, FailureKind::Export);
}

#[test]
fn document_and_export_decisions_ignore_outside_clicks_and_cancel_without_dispatch() {
    use super::super::presentation::tests::key;
    for decision in 0..3 {
        for cancel_key in [egui::Key::Escape, egui::Key::Enter] {
            let mut editor = fixture();
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("existing-file");
            std::fs::write(&path, b"unchanged destination").unwrap();
            let before = editor.document.state.project.clone();
            match decision {
                0 => editor.transition = Some(Transition::Close),
                1 => editor.project_overwrite = Some((path.clone(), Some(Transition::New))),
                _ => {
                    editor.document.export.draft = Some(export::batch::Draft::new(
                        OutputTarget::Srgb,
                        directory.path().to_owned(),
                    ))
                }
            }
            let context = egui::Context::default();
            ui_frame(&mut editor, &context, vec![]);
            ui_frame(&mut editor, &context, vec![]);
            click(&mut editor, &context, egui::pos2(30., 15.));
            assert!(
                editor.transition.is_some()
                    || editor.project_overwrite.is_some()
                    || editor.document.export.draft.is_some()
            );
            assert!(
                editor.document_pending.is_none() && editor.document.export.operation.is_none()
            );
            ui_frame(&mut editor, &context, key(cancel_key));
            assert!(
                editor.transition.is_none()
                    && editor.project_overwrite.is_none()
                    && editor.document.export.draft.is_none()
            );
            assert!(
                editor.document_pending.is_none() && editor.document.export.operation.is_none()
            );
            assert!(!editor.allow_close && !editor.busy);
            assert_eq!(editor.document.state.project, before);
            assert_eq!(std::fs::read(path).unwrap(), b"unchanged destination");
        }
    }
}

#[test]
fn save_decision_queues_the_snapshot_with_its_original_continuation() {
    let mut editor = fixture();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("project.antitone");
    *editor.document = editor
        .document
        .save(&path, crate::project::Save::New)
        .unwrap();
    let id = editor.document.visit.current_source.unwrap();
    editor.document.visit.adjustment_layer_settings.exposure = 1.25;
    let context = egui::Context::default();
    with_application(editor, |app| {
        app.enter(|e, cx| e.guard_transition(Transition::Close, cx));
        ui_frame(app, &context, vec![]);
        let labels = ui_frame(app, &context, vec![]);
        let point = labels
            .iter()
            .find(|(label, _)| label == "Save changes")
            .unwrap()
            .1
            .center();
        click(app, &context, point);
        app.read(|e| {
            assert!(e.transition.is_none() && !e.allow_close);
            assert_eq!(e.after_save, Some(Transition::Close));
            assert!(e.document_pending.is_some());
        });
        app.run_until(Duration::from_secs(15), |e| e.document_pending.is_none());
        assert_eq!(
            crate::project::Project::load(&path)
                .unwrap()
                .source(id)
                .unwrap()
                .adjustment_layers
                .as_ref()
                .unwrap()
                .layers[0]
                .adjustment
                .photo
                .exposure,
            1.25
        );
        app.read(|e| {
            assert!(e.allow_close && !e.document.dirty() && e.failure.is_none());
            assert!(e.after_save.is_none());
        });
    });
}

#[test]
fn project_replace_decision_preserves_destination_snapshot_and_continuation() {
    let mut editor = fixture();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("existing.antitone");
    std::fs::write(&path, b"existing destination").unwrap();
    let id = editor.document.visit.current_source.unwrap();
    editor.document.visit.adjustment_layer_settings.exposure = 1.25;
    editor.project_overwrite = Some((path.clone(), Some(Transition::New)));
    let context = egui::Context::default();
    with_application(editor, |app| {
        ui_frame(app, &context, vec![]);
        let labels = ui_frame(app, &context, vec![]);
        let point = labels
            .iter()
            .find(|(label, _)| label == "Replace project")
            .unwrap()
            .1
            .center();
        click(app, &context, point);
        app.read(|e| {
            assert!(e.project_overwrite.is_none() && e.document_pending.is_some());
            assert_eq!(e.after_save, Some(Transition::New));
        });
        app.run_until(Duration::from_secs(15), |e| e.document_pending.is_none());
        assert_eq!(
            crate::project::Project::load(&path)
                .unwrap()
                .source(id)
                .unwrap()
                .adjustment_layers
                .as_ref()
                .unwrap()
                .layers[0]
                .adjustment
                .photo
                .exposure,
            1.25
        );
        app.read(|e| {
            assert_eq!(e.document.state.project, crate::project::Project::default());
            assert!(e.failure.is_none() && e.after_save.is_none());
        });
    });
}

#[test]
fn export_replace_decision_dispatches_the_staged_format_and_current_accepted_grade_once() {
    for format in [
        OutputFormat::Tiff(OutputTarget::Srgb),
        OutputFormat::Tiff(OutputTarget::DisplayP3),
        OutputFormat::Exr(ExrPrecision::Half),
        OutputFormat::Exr(ExrPrecision::Full),
    ] {
        let mut editor = super::super::tests::authored_fixture(&["patches.dng"]);
        editor.set_dmin(Some(DVec3::splat(0.8)));
        editor.set_dmax(Some(DVec3::splat(0.02)));
        editor.document.visit.adjustment_layer_settings.exposure = 1.25;
        editor.document.roll.exposure = -0.25;
        editor.document.visit.crop = Some(Crop::from_region(Rect {
            x: 20,
            y: 20,
            width: 100,
            height: 80,
        }));
        editor.sync_document();
        let directory = tempfile::tempdir().unwrap();
        let path = directory
            .path()
            .join(if matches!(format, OutputFormat::Tiff(_)) {
                "patches.tif"
            } else {
                "patches.exr"
            });
        let reference = directory.path().join("reference-output");
        std::fs::write(&path, b"existing destination").unwrap();
        let expected = editor
            .export_input(reference.clone(), format, Overwrite::Refuse)
            .unwrap()
            .unwrap();
        assert!(Arc::ptr_eq(
            &expected.input.source,
            &editor
                .document
                .visit
                .canvas
                .ready
                .as_ref()
                .unwrap()
                .developed
        ));
        assert_eq!(
            expected.recipe.neutral.input.calibration().unwrap(),
            editor.calibration().unwrap()
        );
        assert_eq!(expected.recipe.roll, editor.document.roll);
        assert_eq!(
            expected.recipe.grade.layers()[0].photo,
            editor.document.visit.adjustment_layer_settings
        );
        assert_eq!(
            expected.recipe.composition.crop(),
            editor.document.visit.crop.unwrap()
        );
        let services = editor.export_services.as_ref().unwrap().clone();
        let mut draft = export::batch::Draft::new(OutputTarget::Srgb, directory.path().to_owned());
        match format {
            OutputFormat::Tiff(target) => draft.target = target,
            OutputFormat::Exr(precision) => {
                draft.kind = export::batch::Kind::Exr;
                draft.precision = precision;
            }
        }
        draft.collision = export::batch::Collision::Replace;
        draft.confirming_replace = true;
        editor.document.export.draft = Some(draft);
        let before = editor.document.state.project.clone();
        let context = egui::Context::default();
        with_application(editor, |app| {
            ui_frame(app, &context, vec![]);
            let labels = ui_frame(app, &context, vec![]);
            let point = labels
                .iter()
                .find(|(label, _)| label == "Replace and export")
                .unwrap()
                .1
                .center();
            click(app, &context, point);
            app.read(|e| {
                assert!(e.document.export.draft.is_none() && e.document.export.operation.is_some())
            });
            app.run_until(Duration::from_secs(30), |e| {
                e.document.export.operation.is_none()
            });
            app.read(|e| {
                assert!(e.failure.is_none() && !e.busy, "{}", e.status);
                assert_eq!(e.document.state.project, before);
            });
            pollster::block_on(services.write(
                expected,
                Arc::new(std::sync::atomic::AtomicBool::new(false)),
            ))
            .unwrap();
            match format {
                OutputFormat::Tiff(target) => {
                    use tiff::decoder::{Decoder, DecodingResult};
                    let mut actual = Decoder::new(std::fs::File::open(&path).unwrap()).unwrap();
                    let mut expected =
                        Decoder::new(std::fs::File::open(&reference).unwrap()).unwrap();
                    assert_eq!(actual.dimensions().unwrap(), (100, 80));
                    assert_eq!(
                        actual
                            .get_tag_u8_vec(tiff::tags::Tag::Unknown(34675))
                            .unwrap(),
                        crate::profiles::icc(target)
                    );
                    let (DecodingResult::U16(actual), DecodingResult::U16(expected)) =
                        (actual.read_image().unwrap(), expected.read_image().unwrap())
                    else {
                        panic!("RGB16 TIFF expected")
                    };
                    assert_eq!(actual, expected);
                }
                OutputFormat::Exr(precision) => {
                    let actual = exr::prelude::read_first_flat_layer_from_file(&path).unwrap();
                    let expected =
                        exr::prelude::read_first_flat_layer_from_file(&reference).unwrap();
                    assert_eq!(actual.layer_data.size, exr::math::Vec2(100, 80));
                    assert_eq!(
                        actual.layer_data.channel_data,
                        expected.layer_data.channel_data
                    );
                    for channel in &actual.layer_data.channel_data.list {
                        assert!(matches!(
                            (&channel.sample_data, precision),
                            (exr::image::FlatSamples::F16(_), ExrPrecision::Half)
                                | (exr::image::FlatSamples::F32(_), ExrPrecision::Full)
                        ));
                    }
                }
            }
            assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 2);
        });
    }
}

fn fixture() -> Editor {
    let mut editor = super::super::tests::with_document_services(Editor::state());
    editor
        .document
        .register_with_development(
            Source {
                id: SourceId(0),
                path: "fixture.ARW".into(),
                development: crate::project::SourceDevelopment {
                    decode: Development::new("test-backend".into(), [1.; 4]),
                    lens_correction: None,
                    dng_development: None,
                    dimensions: [900, 600],
                    active_area: [0, 0, 900, 600],
                    camera_model: "test-camera".into(),
                    camera_to_ap1: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
                    input_floor: crate::processing::CAMERA_INPUT_FLOOR,
                },
                reference_only: false,
                adjustment_layers: Some(crate::project::AdjustmentLayers::with_global(
                    AdjustmentLayerSettings::default(),
                )),
                repairs: Default::default(),
                orientation: Default::default(),
                crop: None,
            },
            Development::new(crate::raw::version(), [1.; 4]),
        )
        .unwrap();
    editor.document.visit.current_source = Some(editor.document.state.project.order[0]);
    editor.set_floor(crate::processing::CAMERA_INPUT_FLOOR);
    editor.set_dmin(Some(DVec3::new(0.2, 0.15, 0.1)));
    editor.set_dmax(Some(DVec3::new(0.008, 0.005, 0.002)));
    editor.sync_document();
    editor
}

#[test]
fn orientation_is_source_owned_and_restored_without_changing_grade_on_load() {
    let editor = fixture();
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            let id = editor.document.visit.current_source.unwrap();
            editor.document.visit.adjustment_layer_settings = AdjustmentLayerSettings {
                contrast: 0.4,
                pivot: 0.25,
                contrast_colour_mix: 0.75,
                saturation: 1.3,
                saturation_density: 0.25,
                ..AdjustmentLayerSettings::default()
            };
            editor.document.roll_orientation.horizontal = true;
            editor.document.roll_orientation.turn_clockwise();
            editor.document.visit.photo_orientation.turn_clockwise();
            editor.document.visit.photo_orientation.flips.vertical = true;
            editor.document.visit.crop = Some(Crop::from_region(Rect {
                x: 30,
                y: 20,
                width: 401,
                height: 203,
            }));
            let crop = editor.document.visit.crop;
            let orientation = editor.document.visit.photo_orientation;
            let grade = (
                editor.document.roll,
                editor.document.visit.adjustment_layer_settings,
                editor.dmin(),
                editor.dmax(),
            );
            editor.sync_document();
            assert_eq!(
                editor.document.state.project.orientation,
                editor.document.roll_orientation
            );
            assert_eq!(
                editor
                    .document
                    .state
                    .project
                    .source(id)
                    .unwrap()
                    .orientation,
                orientation
            );
            let mut other = editor.document.state.project.source(id).unwrap().clone();
            other.path = "second.ARW".into();
            other.orientation = PhotoOrientation::default();
            other.crop = None;
            other.adjustment_layers.as_mut().unwrap().layers[0]
                .adjustment
                .photo = AdjustmentLayerSettings {
                contrast: 1.6,
                saturation: 0.7,
                contrast_colour_mix: 0.25,
                saturation_density: 0.75,
                ..AdjustmentLayerSettings::default()
            };
            let other_photo = other.adjustment_layers.as_ref().unwrap().layers[0]
                .adjustment
                .photo;
            let other_id = editor.document.state.project.add_source(other).unwrap();
            editor.load("second.ARW".into(), cx);
            assert_eq!(editor.document.visit.current_source, Some(other_id));
            assert_eq!(editor.document.visit.adjustment_layer_settings, other_photo);
            editor.load("fixture.ARW".into(), cx);
            assert_eq!(editor.document.visit.current_source, Some(id));
            assert_eq!(
                (
                    editor.document.roll,
                    editor.document.visit.adjustment_layer_settings,
                    editor.dmin(),
                    editor.dmax()
                ),
                grade
            );
            editor.load("temporary-reference.ARW".into(), cx);
            assert_eq!(
                editor.document.visit.adjustment_layer_settings,
                AdjustmentLayerSettings::default()
            );
            assert_eq!(
                editor.document.visit.photo_orientation,
                PhotoOrientation::default()
            );
            assert!(editor.document.visit.crop.is_none());
            editor.load("fixture.ARW".into(), cx);
            assert_eq!(editor.document.visit.photo_orientation, orientation);
            assert_eq!(editor.document.visit.crop, crop);
            assert_eq!(
                (
                    editor.document.roll,
                    editor.document.visit.adjustment_layer_settings,
                    editor.dmin(),
                    editor.dmax()
                ),
                grade
            );
            assert_eq!(
                editor.document.roll_orientation,
                editor.document.state.project.orientation
            );
            assert!(editor.document.visit.source.pending());
        });
    });
}

#[test]
fn failed_and_stale_document_results_preserve_state_and_active_request() {
    let mut editor = fixture();
    editor.document.visit.adjustment_layer_settings.exposure = 7.123456789012345;
    editor.sync_document();
    let before = editor.document.state.project.clone();
    let active = editor.id;
    with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.start_document(Preparation::Open("candidate.json".into()), cx);
            let id = editor.document_pending.unwrap();
            editor.document_prepared(id + 1, Err("stale candidate".into()));
            editor.source_result(active, Err("old load".into()));
            assert_eq!(editor.document_pending, Some(id));
            assert_eq!(
                editor
                    .source_epoch
                    .load(std::sync::atomic::Ordering::Relaxed),
                id
            );
            assert!(editor.busy);
            assert_eq!(editor.document.state.project, before);
            editor.document_prepared(id, Err("changed RAW".into()));
            assert!(!editor.busy && editor.document_pending.is_none());
            assert_eq!(editor.document.state.project, before);
            assert_eq!(editor.id, active);
            assert_eq!(
                editor
                    .source_epoch
                    .load(std::sync::atomic::Ordering::Relaxed),
                active
            );
            assert_eq!(
                editor.document.visit.adjustment_layer_settings.exposure,
                7.123456789012345
            );
            assert!(editor.document.dirty());
        });
        app.pump();
    });
}

#[test]
fn save_failure_preserves_dirty_state_and_does_not_continue_transition() {
    let editor = fixture();
    let before = editor.document.state.project.clone();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("occupied.antitone");
    std::fs::write(&path, b"existing destination").unwrap();
    with_application(editor, |app| {
        app.enter(|e, cx| {
            e.save_project(
                path.clone(),
                crate::project::Save::New,
                Some(Transition::New),
                cx,
            )
        });
        app.run_until(Duration::from_secs(15), |e| e.document_pending.is_none());
        app.read(|e| {
            assert_eq!(e.document.state.project, before);
            assert!(e.document.dirty() && !e.busy);
            assert_eq!(e.transition, Some(Transition::New));
            assert_eq!(
                e.source_epoch.load(std::sync::atomic::Ordering::Relaxed),
                e.id
            );
            assert_eq!(e.failure.as_ref().unwrap().kind, FailureKind::Save);
        });
        assert_eq!(std::fs::read(&path).unwrap(), b"existing destination");
    });
}

#[test]
fn save_snapshot_flushes_current_photo_and_retains_other_source_settings() {
    let mut editor = fixture();
    let one = editor.document.visit.current_source.unwrap();
    let mut other = editor.document.state.project.source(one).unwrap().clone();
    other.path = "unvisited.ARW".into();
    let two = editor.document.state.project.add_source(other).unwrap();
    editor.document.visit.adjustment_layer_settings = AdjustmentLayerSettings {
        exposure: 7.123456789012345,
        contrast: 1.6,
        pivot: 0.25,
        contrast_colour_mix: 0.75,
        saturation: 1.3,
        saturation_density: 0.25,
        ..AdjustmentLayerSettings::default()
    };
    editor.document.roll.exposure = -0.23456789012345678;
    let photo = editor.document.visit.adjustment_layer_settings;
    let roll = editor.document.roll;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("snapshot.antitone");
    with_application(editor, |app| {
        app.enter(|e, cx| e.save_project(path.clone(), crate::project::Save::New, None, cx));
        app.run_until(Duration::from_secs(15), |e| e.document_pending.is_none());
        let snapshot = crate::project::Project::load(&path).unwrap();
        assert_eq!(
            snapshot
                .source(one)
                .unwrap()
                .adjustment_layers
                .as_ref()
                .unwrap()
                .layers[0]
                .adjustment
                .photo,
            photo
        );
        assert_eq!(
            snapshot
                .source(two)
                .unwrap()
                .adjustment_layers
                .as_ref()
                .unwrap()
                .layers[0]
                .adjustment
                .photo,
            AdjustmentLayerSettings::default()
        );
        assert_eq!(snapshot.roll, roll);
        assert_eq!(snapshot.processing_model, crate::PROCESSING_MODEL_VERSION);
    });
}

#[test]
fn new_resets_development_and_replaces_the_previous_document() {
    let editor = fixture();
    let previous = editor.id;
    with_application(editor, |app| {
        app.enter(|e, cx| e.start_document(Preparation::New, cx));
        app.run_until(Duration::from_secs(15), |e| e.document_pending.is_none());
        app.update(|e| {
            assert_eq!(e.document.state.project, crate::project::Project::default());
            assert!(
                e.document.state.project.development().is_none()
                    && e.document.visit.current_source.is_none()
            );
            assert_eq!(e.floor(), crate::processing::CAMERA_INPUT_FLOOR);
            assert!(!e.document.dirty());
            assert!(e.dmin().is_none());
            assert!(e.id > previous);
        });
    });
}

#[test]
fn navigation_and_view_state_do_not_dirty_a_saved_document() {
    let editor = fixture();
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("project.antitone.json");
            *editor.document = editor
                .document
                .save(&path, crate::project::Save::New)
                .unwrap();
            let before = editor.document.state.project.clone();
            editor.document.source = false;
            editor.document.visit.canvas.viewport.zoom = Some(2.);
            editor.set_pick_region(Some(Coverage::rectangle(Rect {
                x: 12,
                y: 12,
                width: 32,
                height: 32,
            })));
            let source_path = editor
                .document
                .source_path(editor.document.visit.current_source.unwrap())
                .unwrap();
            let request = editor.source_input(source_path.clone());
            editor.load(source_path, cx);
            let wb = request.wb;
            let expected = &request.expected;
            assert_eq!(
                wb,
                editor
                    .document
                    .project
                    .development()
                    .map(|value| value.fixed_wb)
            );
            assert!(expected.is_some());
            editor.sync_document();
            assert_eq!(editor.document.state.project, before);
            assert!(!editor.document.dirty());
        });
    });
}

#[test]
fn dirty_open_new_and_close_are_guarded_without_dispatching_or_allowing_close() {
    let mut editor = fixture();
    for transition in [
        Transition::New,
        Transition::Open("candidate.json".into()),
        Transition::Close,
    ] {
        editor.guard_transition(transition.clone(), &UiContext::render_only());
        assert_eq!(editor.transition, Some(transition));
        assert!(editor.document_pending.is_none() && editor.document.export.operation.is_none());
        assert!(!editor.allow_close);
        editor.transition = None;
    }
}

#[test]
fn command_save_uses_the_current_document_after_control_processing() {
    let mut editor = fixture();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("project.antitone");
    *editor.document = editor
        .document
        .save(&path, crate::project::Save::New)
        .unwrap();
    let id = editor.document.visit.current_source.unwrap();
    let context = egui::Context::default();
    with_application(editor, |app| {
        ui_frame(app, &context, vec![]);
        app.update(|e| {
            e.document.visit.adjustment_layer_settings.exposure = 7.123456789012345;
            assert_eq!(
                e.document
                    .state
                    .project
                    .source(id)
                    .unwrap()
                    .adjustment_layers
                    .as_ref()
                    .unwrap()
                    .layers[0]
                    .adjustment
                    .photo
                    .exposure,
                0.
            );
        });
        ui_frame(
            app,
            &context,
            vec![egui::Event::Key {
                key: egui::Key::S,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            }],
        );
        app.run_until(Duration::from_secs(15), |e| e.document_pending.is_none());
        assert_eq!(
            crate::project::Project::load(&path)
                .unwrap()
                .source(id)
                .unwrap()
                .adjustment_layers
                .as_ref()
                .unwrap()
                .layers[0]
                .adjustment
                .photo,
            app.read(|e| e.document.visit.adjustment_layer_settings)
        );
    });
}
