use super::*;

fn exposure(value: f64) -> Project {
    let mut project = Project::default();
    project.roll.exposure = value;
    project
}

#[test]
fn chronological_snapshots_group_gestures_and_branch_only_on_changed_acceptance() {
    let mut history = History::default();
    let gesture = Some(egui::Id::new("exposure-drag"));
    history.record(exposure(0.), exposure(0.5), "Exposure", None, gesture);
    history.record(exposure(0.5), exposure(1.), "Exposure", None, gesture);
    assert_eq!(history.undo.len(), 1);
    assert_eq!(history.undo[0].before.roll.exposure, 0.);
    assert_eq!(history.undo[0].after.roll.exposure, 1.);
    history.complete(Direction::Undo);
    history.record(exposure(0.), exposure(0.), "No change", None, None);
    assert_eq!(history.redo.len(), 1);
    history.complete(Direction::Redo);
    history.group = None;
    history.record(exposure(1.), exposure(2.), "Exposure", None, gesture);
    assert_eq!(history.undo.len(), 2, "a new gesture is a separate edit");
    history.complete(Direction::Undo);
    history.record(exposure(1.), exposure(-1.), "Exposure", None, None);
    assert!(history.redo.is_empty());
    assert_eq!(history.undo.last().unwrap().after.roll.exposure, -1.);
}

fn wait(editor: &mut Editor) {
    let started = std::time::Instant::now();
    while editor.busy {
        editor.receive();
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "{}",
            editor.status
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn round_trip(editor: &mut Editor, before: &Project, after: &Project) {
    editor.restore_history(Direction::Undo);
    assert!(editor.busy, "undo must dispatch");
    wait(editor);
    assert_eq!(&editor.document.project, before, "{}", editor.status);
    editor.restore_history(Direction::Redo);
    assert!(editor.busy, "redo must dispatch");
    wait(editor);
    assert_eq!(&editor.document.project, after, "{}", editor.status);
}

#[test]
fn authored_history_restores_import_crop_dust_local_and_analysis_through_the_real_worker() {
    use crate::{
        geometry::SourcePoint,
        local_adjustments::{GradientMask, LocalAdjustment},
    };
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::METAL,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
        .expect("history worker test requires Metal");
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let mut editor = Editor::new(&device, &queue, None);
    editor.thumbnails = Thumbnails::default();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw");
    let empty = editor.history_snapshot();
    editor.add_raws(vec![root.join("roll-0.dng"), root.join("roll-1.dng")]);
    wait(&mut editor);
    assert!(editor.ready.is_some(), "{}", editor.status);
    let imported = editor.document.project.clone();
    round_trip(&mut editor, &empty, &imported);
    assert!(editor.ready.is_some());
    // Calibration is fixture setup; accepted picks have independent controller coverage.
    editor.dmin = Some(DVec3::splat(0.2));
    editor.dmax = Some(DVec3::splat(0.005));
    editor.source = false;
    editor.tab = 1;
    let before_crop = editor.history_snapshot();
    editor.begin_crop();
    editor.crop_edit.as_mut().unwrap().draft.size = [256, 256];
    editor.finish_crop(false);
    assert_eq!(
        editor.document.project, before_crop,
        "cancelled crops are not edits"
    );
    editor.begin_crop();
    editor.crop_edit.as_mut().unwrap().draft.size = [256, 256];
    editor.finish_crop(true);
    let cropped = editor.document.project.clone();
    round_trip(&mut editor, &before_crop, &cropped);

    editor.begin_dust_cleanup();
    let active = editor.ready.as_ref().unwrap().area;
    let geometry = editor.viewport.layout_composition(
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800., 600.)),
        Composition::new(active, Default::default(), Default::default()).unwrap(),
        1.,
    );
    editor.begin_dust_mask(None);
    editor.begin_dust_stroke();
    editor.paint_dust_stroke(
        geometry,
        geometry.screen_point(egui::pos2(200., 200.)),
        false,
    );
    assert_eq!(editor.document.project, cropped, "painting is a draft");
    editor.commit_dust_mask();
    let repaired = editor.document.project.clone();
    let id = editor.current_source.unwrap();
    assert_eq!(repaired.source(id).unwrap().repairs.repairs().len(), 1);
    round_trip(&mut editor, &cropped, &repaired);
    assert!(
        editor.dust.cleanup,
        "undo/redo between strokes must keep the tool usable"
    );
    editor.delete_selected_dust_repair();
    let deleted = editor.document.project.clone();
    assert!(deleted.source(id).unwrap().repairs.repairs().is_empty());
    round_trip(&mut editor, &repaired, &deleted);
    editor.finish_dust_cleanup();

    editor
        .edit_adjustments(|locals| {
            locals.add(
                "Gradient".into(),
                LocalAdjustment {
                    mask: GradientMask::Linear {
                        start: SourcePoint([128., 128.]),
                        end: SourcePoint([320., 128.]),
                    },
                    photo: PhotoSettings::default(),
                    enabled: true,
                    invert: false,
                    strength: 1.,
                },
            )?;
            Ok(())
        })
        .unwrap();
    let local = editor.document.project.clone();
    round_trip(&mut editor, &deleted, &local);
    assert_eq!(
        editor
            .document
            .project
            .source(id)
            .unwrap()
            .local_adjustments
            .as_ref()
            .unwrap()
            .adjustments
            .len(),
        1
    );

    editor.document.project.analysis_area = Some([96, 96, 320, 320]);
    let before_analysis = editor.history_snapshot();
    editor.begin_analysis();
    wait(&mut editor);
    assert!(editor.status.starts_with("Analyzed"), "{}", editor.status);
    let analyzed = editor.document.project.clone();
    assert_ne!(analyzed, before_analysis);
    round_trip(&mut editor, &before_analysis, &analyzed);
}

#[test]
fn a_gesture_returning_to_its_start_preserves_redo_and_does_not_absorb_an_earlier_action() {
    let mut history = History::default();
    history.record(exposure(0.), exposure(1.), "First", None, None);
    history.record(exposure(1.), exposure(2.), "Second", None, None);
    history.complete(Direction::Undo);
    let gesture = Some(egui::Id::new("gesture"));
    history.record(exposure(1.), exposure(1.5), "Gesture", None, gesture);
    history.record(exposure(1.5), exposure(1.), "Gesture", None, gesture);
    history.finish_group();
    assert_eq!(history.undo.len(), 1);
    assert_eq!(history.redo.len(), 1);
    assert_eq!(history.undo[0].after.roll.exposure, 1.);
    history.record(exposure(1.), exposure(1.5), "Gesture", None, gesture);
    history.record(exposure(1.5), exposure(1.), "Gesture", None, gesture);
    history.record(exposure(1.), exposure(1.75), "Gesture", None, gesture);
    history.finish_group();
    assert_eq!(history.undo.len(), 2);
    assert_eq!(history.undo[0].after.roll.exposure, 1.);
    assert_eq!(history.undo[1].before.roll.exposure, 1.);
    assert_eq!(history.undo[1].after.roll.exposure, 1.75);
    assert!(history.redo.is_empty());
}

#[test]
fn snapshot_capacity_drops_oldest_complete_actions() {
    let mut history = History::default();
    for index in 0..HISTORY_LIMIT + 3 {
        history.record(
            exposure(index as f64),
            exposure(index as f64 + 1.),
            "Edit",
            None,
            None,
        );
    }
    assert_eq!(history.undo.len(), HISTORY_LIMIT);
    assert_eq!(history.undo[0].before.roll.exposure, 3.);
    assert_eq!(
        history.undo.last().unwrap().after.roll.exposure,
        (HISTORY_LIMIT + 3) as f64
    );
}

fn controller() -> (Editor, mpsc::Sender<Message>, mpsc::Receiver<Command>) {
    let (mut editor, send, commands) = crate::editor::tests::fixture();
    for index in 0..2 {
        editor
            .document
            .register(
                Source {
                    id: SourceId(0),
                    path: format!("photo-{index}.dng").into(),
                    development: crate::project::SourceDevelopment {
                        lens_correction: None,
                        dimensions: [64, 64],
                        active_area: [0, 0, 64, 64],
                        camera_model: "test-camera".into(),
                        camera_to_ap1: glam::DMat3::IDENTITY.to_cols_array_2d(),
                        input_floor: editor.floor,
                    },
                    reference_only: false,
                    photo: PhotoSettings::default(),
                    local_adjustments: None,
                    repairs: Default::default(),
                    orientation: Default::default(),
                    crop: None,
                },
                editor.document.project.development.clone().unwrap(),
            )
            .unwrap();
    }
    editor.current_source = Some(editor.document.project.order[0]);
    editor.sync_document();
    (editor, send, commands)
}

// Only preparation is supplied here. Dispatch, result freshness, mirror restoration and
// history promotion use the real controller; authored worker tests cover RAW preparation.
fn accept_history(
    editor: &mut Editor,
    send: &mpsc::Sender<Message>,
    commands: &mpsc::Receiver<Command>,
    direction: Direction,
) {
    editor.restore_history(direction);
    let Command::Document(request, work) = commands.try_recv().unwrap() else {
        panic!("history dispatch")
    };
    let DocumentWork::History(document, selected, actual) = *work else {
        panic!("history candidate")
    };
    assert_eq!(actual, direction);
    assert!(editor.busy);
    send.send(Message::Prepared(
        request,
        Ok(Box::new(Prepared {
            document,
            ready: None,
            selected,
            reuse_ready: false,
            correction: None,
            preserve_view: true,
        })),
    ))
    .unwrap();
    editor.receive();
    assert!(!editor.busy, "{}", editor.status);
}

#[test]
fn accepted_history_restores_exact_settings_and_activates_the_edited_photo_without_rewinding_navigation()
 {
    let (mut editor, send, commands) = controller();
    let ids = editor.document.project.order.clone();
    let before = editor.history_snapshot();
    editor.photo.exposure = 7.123456789012345;
    editor.record_edit(before.clone(), "Photo exposure", Some(ids[0]));
    let after = editor.document.project.clone();
    editor.current_source = Some(ids[1]);
    editor.photo = PhotoSettings::default();
    editor.viewport.zoom = Some(1.75);
    let viewport = editor.viewport;
    accept_history(&mut editor, &send, &commands, Direction::Undo);
    assert_eq!(editor.document.project, before);
    assert_eq!(editor.current_source, Some(ids[0]));
    assert_eq!(editor.photo.exposure, 0.);
    assert_eq!(editor.viewport, viewport);
    accept_history(&mut editor, &send, &commands, Direction::Redo);
    assert_eq!(editor.document.project, after);
    assert_eq!(editor.photo.exposure, 7.123456789012345);
    assert_eq!(editor.viewport, viewport);
}

#[test]
fn save_as_rebases_both_sides_of_history_and_preserves_the_written_dirty_baseline() {
    let directory = tempfile::tempdir().unwrap();
    let old = directory.path().join("old/grade.antitone");
    let new = directory.path().join("new/grade.antitone");
    let (mut editor, send, commands) = controller();
    editor.document.path = Some(old.clone());
    let before = editor.history_snapshot();
    editor.photo.exposure = 1.;
    editor.record_edit(before, "Exposure", editor.current_source);
    let saved = editor.document.save(&new, crate::project::Save::New);
    // The destination directory is intentionally absent: ordinary failed writes do not
    // rebase either history or the live document.
    assert!(saved.is_err());
    assert_eq!(editor.document.path, Some(old.clone()));
    std::fs::create_dir_all(new.parent().unwrap()).unwrap();
    let saved = editor
        .document
        .save(&new, crate::project::Save::New)
        .unwrap();
    let baseline = saved.project.clone();
    editor.start_document(DocumentWork::Save(
        editor.document.clone(),
        new.clone(),
        crate::project::Save::Replace,
    ));
    let Command::Document(request, _) = commands.try_recv().unwrap() else {
        panic!()
    };
    send.send(Message::Saved(request, Ok(Box::new(saved))))
        .unwrap();
    editor.receive();
    assert!(!editor.document.dirty());
    let source = editor.current_source.unwrap();
    let locator = editor.document.source_path(source).unwrap();
    accept_history(&mut editor, &send, &commands, Direction::Undo);
    assert!(editor.document.dirty());
    assert_eq!(editor.document.path, Some(new.clone()));
    assert_eq!(editor.document.source_path(source).unwrap(), locator);
    accept_history(&mut editor, &send, &commands, Direction::Redo);
    assert!(!editor.document.dirty());
    assert_eq!(editor.document.project, baseline);
    assert_eq!(editor.history.undo.len(), 1);
}

#[test]
fn save_completion_baselines_only_the_written_snapshot_and_undo_can_return_to_it() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("grade.antitone");
    let (mut editor, send, commands) = controller();
    let written = editor
        .document
        .save(&path, crate::project::Save::New)
        .unwrap();
    editor.start_document(DocumentWork::Save(
        editor.document.clone(),
        path,
        crate::project::Save::Replace,
    ));
    let Command::Document(request, _) = commands.try_recv().unwrap() else {
        panic!()
    };
    let before = editor.history_snapshot();
    editor.roll.exposure = 0.375;
    editor.record_edit(before, "Roll exposure", None);
    send.send(Message::Saved(request, Ok(Box::new(written))))
        .unwrap();
    editor.receive();
    assert!(editor.document.dirty());
    assert_eq!(editor.roll.exposure, 0.375);
    accept_history(&mut editor, &send, &commands, Direction::Undo);
    assert!(!editor.document.dirty());
    accept_history(&mut editor, &send, &commands, Direction::Redo);
    assert!(editor.document.dirty());
    assert_eq!(editor.roll.exposure, 0.375);
}

#[test]
fn accepted_calibration_is_one_entry_and_an_invalid_pick_preserves_redo() {
    let (mut editor, send, commands) = controller();
    let before = editor.history_snapshot();
    send.send(Message::Picked(
        editor.id,
        Pick::Dmin,
        Ok(Measurement::Reference(DVec3::splat(0.25))),
    ))
    .unwrap();
    editor.receive();
    let picked = editor.document.project.clone();
    assert_ne!(picked, before);
    assert_eq!(editor.history.undo.len(), 1);
    accept_history(&mut editor, &send, &commands, Direction::Undo);
    assert_eq!(editor.document.project, before);
    send.send(Message::Picked(
        editor.id,
        Pick::Dmin,
        Ok(Measurement::Reference(DVec3::splat(0.0001))),
    ))
    .unwrap();
    editor.receive();
    assert_eq!(editor.document.project, before);
    assert!(editor.history.undo.is_empty());
    assert_eq!(editor.history.redo.len(), 1);
    accept_history(&mut editor, &send, &commands, Direction::Redo);
    assert_eq!(editor.document.project, picked);
}

#[test]
fn failed_open_keeps_history_and_only_accepted_document_replacement_clears_it() {
    let (mut editor, send, commands) = controller();
    let before = editor.history_snapshot();
    editor.roll.exposure = 0.5;
    editor.record_edit(before, "Roll exposure", None);
    editor.start_document(DocumentWork::Open("missing.antitone".into()));
    let Command::Document(request, _) = commands.try_recv().unwrap() else {
        panic!()
    };
    send.send(Message::Prepared(request, Err("missing project".into())))
        .unwrap();
    editor.receive();
    assert_eq!(editor.history.undo.len(), 1);
    assert_eq!(editor.roll.exposure, 0.5);
    editor.start_document(DocumentWork::New);
    let Command::Document(request, _) = commands.try_recv().unwrap() else {
        panic!()
    };
    assert_eq!(editor.history.undo.len(), 1, "dispatch is not acceptance");
    send.send(Message::Prepared(
        request,
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
    assert!(editor.history.undo.is_empty());
    assert!(editor.history.redo.is_empty());
    assert!(!editor.document.dirty());
}

fn finish_controls(editor: &mut Editor) {
    let context = egui::Context::default();
    let photo = editor.current_source;
    let mut output = context.run_ui(Default::default(), |ui| editor.record_controls(photo, ui));
    output.textures_delta.clear();
}

#[test]
fn unchanged_control_passes_do_not_clone_projects_and_local_edits_capture_their_before_state_lazily()
 {
    let (mut editor, _, _) = controller();
    let original = editor.history_snapshot();
    editor.history_snapshot_clones = 0;
    editor.begin_control_history("Photo settings");
    finish_controls(&mut editor);
    assert_eq!(editor.history_snapshot_clones, 0);
    assert!(editor.history.undo.is_empty());

    editor.begin_control_history("Photo settings");
    // A saved empty local collection is a metadata edit even though its processing
    // and all scalar controls are unchanged. It must retain the pre-edit project.
    editor.edit_adjustments(|_| Ok(())).unwrap();
    assert_eq!(editor.history_snapshot_clones, 1);
    assert!(
        editor.history.undo.is_empty(),
        "the control pass still owns the edit"
    );
    finish_controls(&mut editor);
    assert_eq!(editor.history_snapshot_clones, 2);
    assert_eq!(editor.history.undo[0].before, original);
    assert!(
        editor.history.undo[0]
            .after
            .source(editor.current_source.unwrap())
            .unwrap()
            .local_adjustments
            .is_some()
    );

    editor.history_snapshot_clones = 0;
    editor.begin_control_history("Photo settings");
    finish_controls(&mut editor);
    assert_eq!(editor.history_snapshot_clones, 0);
    assert_eq!(editor.history.undo.len(), 1);
}

#[test]
fn scalar_control_edits_materialize_exact_before_and_after_projects_only_after_a_change() {
    let (mut editor, _, _) = controller();
    let original = editor.history_snapshot();
    editor.history_snapshot_clones = 0;
    editor.begin_control_history("Photo settings");
    editor.photo.exposure = 7.123456789012345;
    assert_eq!(
        editor.history_snapshot_clones, 0,
        "capture contains only small control values"
    );
    finish_controls(&mut editor);
    assert_eq!(editor.history_snapshot_clones, 2);
    assert_eq!(editor.history.undo[0].before, original);
    let source = editor.current_source.unwrap();
    assert_eq!(
        editor.history.undo[0]
            .after
            .source(source)
            .unwrap()
            .photo
            .exposure,
        7.123456789012345
    );
}

#[test]
fn busy_decisions_and_drafts_cannot_dispatch_document_history() {
    let (mut editor, _, commands) = controller();
    let before = editor.history_snapshot();
    editor.roll.exposure = 1.;
    editor.record_edit(before, "Exposure", None);
    for blocked in 0..4 {
        editor.busy = blocked == 0;
        editor.pick = (blocked == 1).then_some(Pick::Dmin);
        editor.removal_pending = (blocked == 2).then_some(vec![editor.current_source.unwrap()]);
        editor.transition = (blocked == 3).then_some(Transition::New);
        editor.restore_history(Direction::Undo);
        assert!(commands.try_recv().is_err());
        assert_eq!(editor.history.undo.len(), 1);
    }
}

#[test]
fn failed_and_stale_restoration_preserve_state_and_history_until_acceptance() {
    let (mut editor, send, commands) = crate::editor::tests::fixture();
    let before = editor.history_snapshot();
    editor.roll.exposure = 1.;
    editor.record_edit(before, "Exposure", None);
    let accepted = editor.document.project.clone();
    editor.restore_history(Direction::Undo);
    let Command::Document(request, work) = commands.try_recv().unwrap() else {
        panic!()
    };
    let DocumentWork::History(candidate, _, Direction::Undo) = *work else {
        panic!()
    };
    assert_eq!(candidate.project.roll.exposure, 0.);
    assert_eq!(editor.document.project, accepted);
    send.send(Message::Prepared(request + 1, Err("stale".into())))
        .unwrap();
    editor.receive();
    assert_eq!(editor.document_pending, Some(request));
    send.send(Message::Prepared(request, Err("missing RAW".into())))
        .unwrap();
    editor.receive();
    assert_eq!(editor.document.project, accepted);
    assert_eq!(editor.history.undo.len(), 1);
    assert!(editor.history.redo.is_empty());
    assert!(!editor.busy);
}
