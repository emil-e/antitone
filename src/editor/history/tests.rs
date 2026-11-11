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
    let mut accepted = exposure(1.);
    history.restore(&mut accepted, Direction::Undo).unwrap();
    assert_eq!(accepted, exposure(0.));
    history.record(exposure(0.), exposure(0.), "No change", None, None);
    assert_eq!(history.redo.len(), 1);
    history.restore(&mut accepted, Direction::Redo).unwrap();
    assert_eq!(accepted, exposure(1.));
    history.group = None;
    history.record(exposure(1.), exposure(2.), "Exposure", None, gesture);
    assert_eq!(history.undo.len(), 2, "a new gesture is a separate edit");
    accepted = exposure(2.);
    history.restore(&mut accepted, Direction::Undo).unwrap();
    assert_eq!(accepted, exposure(1.));
    history.record(exposure(1.), exposure(-1.), "Exposure", None, None);
    assert!(history.redo.is_empty());
    assert_eq!(history.undo.last().unwrap().after.roll.exposure, -1.);
}

fn round_trip(mut editor: Editor, before: &Project, after: &Project) -> Editor {
    for (direction, expected) in [(Direction::Undo, before), (Direction::Redo, after)] {
        editor = crate::editor::tests::complete_document_operation(
            editor,
            Duration::from_secs(60),
            |e, cx| {
                let depths = (e.document.history.undo.len(), e.document.history.redo.len());
                e.restore_history(direction, cx);
                assert_eq!(&e.document.state.project, expected, "{}", e.status);
                assert!(!e.interaction_blocked());
                let after = (e.document.history.undo.len(), e.document.history.redo.len());
                assert_eq!(
                    after,
                    match direction {
                        Direction::Undo => (depths.0 - 1, depths.1 + 1),
                        Direction::Redo => (depths.0 + 1, depths.1 - 1),
                    }
                );
            },
        );
    }
    editor
}

#[test]
fn authored_history_restores_import_crop_dust_local_and_analysis_before_consumers_prepare() {
    use crate::{
        adjustment_layers::{AdjustmentLayer, AdjustmentLayerCoverage},
        geometry::SourcePoint,
    };
    let instance = crate::gpu::instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
        .expect("history consumer test requires Metal");
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let mut editor = Editor::new(&device, &queue);
    editor.document.thumbnails =
        (Thumbnails::default()).with_sources(editor.document.cache.clone());
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw");
    let empty = editor.history_snapshot();
    editor = crate::editor::tests::complete_document_operation(
        editor,
        Duration::from_secs(60),
        |editor, cx| editor.add_raws(vec![root.join("roll-0.dng"), root.join("roll-1.dng")], cx),
    );
    assert!(
        editor.document.visit.canvas.ready.is_some(),
        "{}",
        editor.status
    );
    let imported = editor.document.state.project.clone();
    editor = round_trip(editor, &empty, &imported);
    assert!(editor.document.visit.canvas.ready.is_some());
    // Calibration is fixture setup; accepted picks have independent controller coverage.
    editor.set_dmin(Some(DVec3::splat(0.2)));
    editor.set_dmax(Some(DVec3::splat(0.005)));
    editor.document.source = false;
    editor.tab = 1;
    let before_crop = editor.history_snapshot();
    editor.begin_crop();
    editor.document.crop_edit_mut().unwrap().draft.size = [256, 256];
    editor.hosted_action(tool::Action::Dismiss);
    assert_eq!(
        editor.document.state.project, before_crop,
        "cancelled crops are not edits"
    );
    editor.begin_crop();
    editor.document.crop_edit_mut().unwrap().draft.size = [256, 256];
    editor.hosted_action(tool::Action::Primary);
    let cropped = editor.document.state.project.clone();
    editor = round_trip(editor, &before_crop, &cropped);

    editor.begin_dust_cleanup();
    let active = editor.document.visit.canvas.ready.as_ref().unwrap().area;
    let geometry = editor.document.visit.canvas.viewport.layout_composition(
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
    assert_eq!(
        editor.document.state.project, cropped,
        "painting is a draft"
    );
    let owner = std::cell::RefCell::new(Some(editor));
    {
        let mut app = crate::application::test_support::TestApplication::new(&owner, || {});
        app.enter(|e, cx| {
            e.commit_dust_mask(cx);
            assert_eq!(
                e.document.state.project, cropped,
                "search is still a proposal"
            );
        });
        app.run_until(Duration::from_secs(30), |e| {
            !e.dust_search_pending() && e.document.visit.dust.pending_request().is_none()
        });
        app.close_execution();
    }
    let mut editor = owner.into_inner().unwrap();
    let repaired = editor.document.state.project.clone();
    let id = editor.document.visit.current_source.unwrap();
    assert_eq!(repaired.source(id).unwrap().repairs.repairs().len(), 1);
    editor = round_trip(editor, &cropped, &repaired);
    assert!(
        editor.dusting(),
        "undo/redo between strokes must keep the tool usable"
    );
    editor.delete_selected_dust_repair();
    let deleted = editor.document.state.project.clone();
    assert!(deleted.source(id).unwrap().repairs.repairs().is_empty());
    editor = round_trip(editor, &repaired, &deleted);
    editor.finish_dust_cleanup();

    editor
        .edit_adjustment_layers(|layers| {
            layers.add(
                "Gradient".into(),
                AdjustmentLayer {
                    mask: AdjustmentLayerCoverage::Linear {
                        start: SourcePoint([128., 128.]),
                        end: SourcePoint([320., 128.]),
                    },
                    photo: AdjustmentLayerSettings::default(),
                    enabled: true,
                    invert: false,
                    strength: 1.,
                },
            )?;
            Ok(())
        })
        .unwrap();
    let local = editor.document.state.project.clone();
    editor = round_trip(editor, &deleted, &local);
    assert_eq!(
        editor
            .document
            .project
            .source(id)
            .unwrap()
            .adjustment_layers
            .as_ref()
            .unwrap()
            .layers
            .len(),
        2
    );

    editor.document.state.project.analysis_area = Some([96, 96, 320, 320]);
    let before_analysis = editor.history_snapshot();
    editor = crate::editor::tests::complete_analysis_operation(
        editor,
        Duration::from_secs(60),
        |editor, cx| editor.begin_analysis(cx),
    );
    assert!(editor.status.starts_with("Analyzed"), "{}", editor.status);
    let analyzed = editor.document.state.project.clone();
    assert_ne!(analyzed, before_analysis);
    drop(round_trip(editor, &before_analysis, &analyzed));
}

#[test]
fn a_gesture_returning_to_its_start_preserves_redo_and_does_not_absorb_an_earlier_action() {
    let mut history = History::default();
    history.record(exposure(0.), exposure(1.), "First", None, None);
    history.record(exposure(1.), exposure(2.), "Second", None, None);
    history.restore(&mut exposure(2.), Direction::Undo).unwrap();
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

fn controller() -> Editor {
    let mut editor = crate::editor::tests::authored_fixture(&["roll-0.dng", "roll-1.dng"]);
    // The authored import is setup, not the edit under test.
    editor.document.history = History::default();
    editor.set_dmin(Some(DVec3::new(0.2, 0.15, 0.1)));
    editor.set_dmax(Some(DVec3::new(0.008, 0.005, 0.002)));

    editor
}

fn accept_history(editor: Editor, direction: Direction) -> Editor {
    crate::editor::tests::complete_document_operation(editor, Duration::from_secs(60), |e, cx| {
        e.restore_history(direction, cx)
    })
}

#[test]
fn accepted_history_restores_exact_settings_and_activates_the_edited_photo_without_rewinding_navigation()
 {
    let mut editor = controller();
    let ids = editor.document.state.project.order.clone();
    let before = editor.history_snapshot();
    editor.photo_settings_mut().exposure = 7.123456789012345;
    editor.record_edit(before.clone(), "Photo exposure", Some(ids[0]));
    let after = editor.document.state.project.clone();
    editor.document.visit.current_source = Some(ids[1]);
    *editor.photo_settings_mut() = AdjustmentLayerSettings::default();
    editor.document.visit.canvas.viewport.zoom = Some(1.75);
    let viewport = editor.document.visit.canvas.viewport;
    editor = accept_history(editor, Direction::Undo);
    assert_eq!(editor.document.state.project, before);
    assert_eq!(editor.document.visit.current_source, Some(ids[0]));
    assert_eq!(editor.photo_settings().exposure, 0.);
    assert_eq!(editor.document.visit.canvas.viewport, viewport);
    editor = accept_history(editor, Direction::Redo);
    assert_eq!(editor.document.state.project, after);
    assert_eq!(editor.photo_settings().exposure, 7.123456789012345);
    assert_eq!(editor.document.visit.canvas.viewport, viewport);
}

#[test]
fn save_as_keeps_removed_source_identity_available_to_the_undo_snapshot() {
    use crate::project::Save;
    let directory = tempfile::tempdir().unwrap();
    let old = directory.path().join("old");
    let new = directory.path().join("new");
    std::fs::create_dir_all(&old).unwrap();
    std::fs::create_dir_all(&new).unwrap();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng");
    for name in ["one.dng", "two.dng"] {
        std::fs::copy(&fixture, old.join(name)).unwrap();
    }
    let mut imported = Document::default();
    imported
        .import(&[old.join("one.dng"), old.join("two.dng")])
        .unwrap();
    for source in &mut imported.project.sources {
        source.path = source.path.file_name().unwrap().into();
    }
    let original_path = old.join("original.antitone");
    imported.project.save(&original_path, Save::New).unwrap();
    let mut document = Document::open(&original_path).unwrap();
    let removed = document.project.order[0];
    let identity = document.file_key(&document.source_path(removed).unwrap());
    assert!(identity.revision.is_some());
    let before = document.project.clone();
    document.project.remove_from_filmstrip(removed).unwrap();
    let mut history = History::default();
    history.record(before, document.project.clone(), "Remove photo", None, None);
    let destination = new.join("moved.antitone");
    let saved = document.save(&destination, Save::New).unwrap();
    history.rebase(document.path.as_deref(), saved.path.as_deref());
    document.accept_save(saved);
    document.project = history.undo.last().unwrap().before.clone();
    let restored = document.file_key(&document.source_path(removed).unwrap());
    assert_eq!(restored, identity);
    assert!(
        document
            .spatial_inputs()
            .iter()
            .any(|input| input.source == removed)
    );
}

#[test]
fn save_as_rebases_both_sides_of_history_and_preserves_the_written_dirty_baseline() {
    let directory = tempfile::tempdir().unwrap();
    let old = directory.path().join("old/grade.antitone");
    let new = directory.path().join("new/grade.antitone");
    let mut editor = controller();
    let scans = old.parent().unwrap().join("scans");
    std::fs::create_dir_all(&scans).unwrap();
    for source in &mut editor.document.project.sources {
        let name = source.path.file_name().unwrap().to_owned();
        std::fs::copy(&source.path, scans.join(&name)).unwrap();
        source.path = PathBuf::from("scans").join(name);
    }
    editor.document.path = Some(old.clone());
    editor.document.capture_source_files().unwrap();
    let source = editor.document.visit.current_source.unwrap();
    let expected_locator = editor
        .document
        .source_path(source)
        .unwrap()
        .canonicalize()
        .unwrap();
    let before = editor.history_snapshot();
    editor.photo_settings_mut().exposure = 1.;
    editor.record_edit(before, "Exposure", editor.document.visit.current_source);
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
    editor = crate::editor::tests::complete_document_operation(
        editor,
        Duration::from_secs(60),
        |e, cx| e.save_project(new.clone(), crate::project::Save::Replace, None, cx),
    );
    assert!(!editor.document.dirty());
    assert_eq!(
        editor
            .document
            .source_path(source)
            .unwrap()
            .canonicalize()
            .unwrap(),
        expected_locator
    );
    editor = accept_history(editor, Direction::Undo);
    assert!(editor.document.dirty());
    assert_eq!(editor.document.path, Some(new.clone()));
    assert_eq!(
        editor
            .document
            .source_path(source)
            .unwrap()
            .canonicalize()
            .unwrap(),
        expected_locator
    );
    editor = accept_history(editor, Direction::Redo);
    assert_eq!(
        editor
            .document
            .source_path(source)
            .unwrap()
            .canonicalize()
            .unwrap(),
        expected_locator
    );
    assert!(!editor.document.dirty());
    assert_eq!(editor.document.state.project, baseline);
    assert_eq!(editor.document.history.undo.len(), 1);
}

#[test]
fn save_completion_baselines_only_the_written_snapshot_and_undo_can_return_to_it() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("grade.antitone");
    let mut editor = controller();
    let written = editor
        .document
        .save(&path, crate::project::Save::New)
        .unwrap();
    editor = crate::editor::tests::complete_document_operation(
        editor,
        Duration::from_secs(60),
        |e, cx| {
            e.save_project(path, crate::project::Save::Replace, None, cx);
            let before = e.history_snapshot();
            e.document.project.roll.exposure = 0.375;
            e.record_edit(before, "Roll exposure", None);
        },
    );
    assert_eq!(written.project.roll.exposure, 0.);
    assert!(editor.document.dirty());
    assert_eq!(editor.document.project.roll.exposure, 0.375);
    editor = accept_history(editor, Direction::Undo);
    assert!(!editor.document.dirty());
    editor = accept_history(editor, Direction::Redo);
    assert!(editor.document.dirty());
    assert_eq!(editor.document.project.roll.exposure, 0.375);
}

#[test]
fn accepted_calibration_is_one_entry_and_an_invalid_pick_preserves_redo() {
    let mut editor = controller();
    let before = editor.history_snapshot();
    (calibration::set_dmin(&mut editor, DVec3::splat(0.25))).unwrap();
    editor.finish_pick(&crate::application::UiContext::render_only());
    let picked = editor.document.state.project.clone();
    assert_ne!(picked, before);
    assert_eq!(editor.document.history.undo.len(), 1);
    editor = accept_history(editor, Direction::Undo);
    assert_eq!(editor.document.state.project, before);
    assert!((calibration::set_dmin(&mut editor, DVec3::splat(0.0001))).is_err());
    editor.finish_pick(&crate::application::UiContext::render_only());
    assert_eq!(editor.document.state.project, before);
    assert!(editor.document.history.undo.is_empty());
    assert_eq!(editor.document.history.redo.len(), 1);
    editor = accept_history(editor, Direction::Redo);
    assert_eq!(editor.document.state.project, picked);
}

#[test]
fn failed_open_keeps_history_and_only_accepted_document_replacement_clears_it() {
    let mut editor = controller();
    let before = editor.history_snapshot();
    editor.document.project.roll.exposure = 0.5;
    editor.record_edit(before, "Roll exposure", None);
    editor = crate::editor::tests::complete_document_operation(
        editor,
        Duration::from_secs(60),
        |editor, cx| {
            editor.open_project("missing.antitone".into(), cx);
            assert!(editor.interaction_blocked() && editor.files.opening());
        },
    );
    assert_eq!(editor.document.history.undo.len(), 1);
    assert_eq!(editor.document.project.roll.exposure, 0.5);
    editor = crate::editor::tests::complete_document_operation(
        editor,
        Duration::from_secs(60),
        |editor, _| {
            editor.new_project();
            assert!(!editor.interaction_blocked());
            assert!(editor.document.history.undo.is_empty());
        },
    );
    assert!(editor.document.history.undo.is_empty());
    assert!(editor.document.history.redo.is_empty());
    assert!(!editor.document.dirty());
}

#[test]
fn draft_preview_preserves_accepted_history_and_metadata_edits_commit_directly() {
    let mut editor = controller();
    let original = editor.history_snapshot();
    let mut edit = editor.edit_project("Rendering settings").unwrap();
    let mut values = edit.values();
    values.roll.exposure = 0.5;
    edit.replace(values);
    assert_eq!(editor.document.project_snapshot().roll.exposure, 0.5);
    assert_eq!(editor.document.state.project, original);
    assert!(editor.document.history.undo.is_empty());
    drop(edit);
    assert_eq!(editor.document.project_snapshot().roll, original.roll);

    editor
        .edit_adjustment_layers(|layers| {
            layers.layers[0].name = "Renamed".into();
            Ok(())
        })
        .unwrap();
    assert_eq!(editor.document.history.undo.len(), 1);
    assert_eq!(editor.document.history.undo[0].before, original);
}

#[test]
fn a_layer_action_rejects_overlap_then_records_its_own_edit_after_completion() {
    let mut editor = controller();
    let original = editor.document.project.clone();
    let source = editor.document.visit.current_source.unwrap();
    let mut previous = editor.edit_photo("Exposure").unwrap();
    previous.update(|project| {
        project
            .source_mut(source)
            .unwrap()
            .adjustment_layers
            .as_mut()
            .unwrap()
            .layers[0]
            .adjustment
            .photo
            .exposure = 0.75;
    });
    let mut first_accepted = original.clone();
    first_accepted
        .source_mut(source)
        .unwrap()
        .adjustment_layers
        .as_mut()
        .unwrap()
        .layers[0]
        .adjustment
        .photo
        .exposure = 0.75;
    assert!(
        editor
            .edit_adjustment_layers(|layers| {
                layers.layers[0].name = "Rejected".into();
                Ok(())
            })
            .is_err()
    );
    assert!(previous.preview().active());
    assert_eq!(editor.document.project, original);
    assert!(editor.document.history.undo.is_empty());
    editor.commit_project(previous).unwrap();
    editor
        .edit_adjustment_layers(|layers| {
            layers.layers[0].name = "Renamed".into();
            Ok(())
        })
        .unwrap();
    assert_eq!(editor.document.history.undo.len(), 2);
    assert_eq!(editor.document.history.undo[0].before, original);
    assert_eq!(editor.document.history.undo[0].after, first_accepted);
    assert_eq!(editor.document.history.undo[1].before, first_accepted);
    assert_eq!(editor.photo_settings().exposure, 0.75);
    editor = accept_history(editor, Direction::Undo);
    assert_eq!(editor.document.project, first_accepted);
    editor = accept_history(editor, Direction::Undo);
    assert_eq!(editor.document.project, original);
}

#[test]
fn photo_draft_keeps_accepted_state_until_one_exact_history_commit() {
    let mut editor = controller();
    let original = editor.history_snapshot();
    let source = editor.document.visit.current_source.unwrap();
    let record = &editor
        .document
        .project
        .source(source)
        .unwrap()
        .adjustment_layers
        .as_ref()
        .unwrap()
        .layers[0];
    assert_eq!(
        editor.adjustment_layer_target(),
        super::super::adjustment_layers::Target::Layer(record.id)
    );
    let mut adjustment = record.adjustment;
    adjustment.photo.exposure = 7.123456789012345;
    editor.photo_control(
        controls::ControlResponse {
            active: true,
            ..controls::ControlResponse::action(egui::Id::new("test-photo-field"), true)
        },
        move |v| *v = adjustment,
    );
    assert_eq!(editor.photo_settings().exposure, 7.123456789012345);
    assert_eq!(editor.document.state.project, original);
    assert!(editor.document.history.undo.is_empty());
    editor.photo_control(
        controls::ControlResponse {
            active: false,
            ..controls::ControlResponse::action(egui::Id::new("test-photo-field"), true)
        },
        move |v| *v = adjustment,
    );
    assert!(!editor.editing());
    assert_eq!(editor.document.history.undo.len(), 1);
    assert_eq!(editor.document.history.undo[0].before, original);
    let source = editor.document.visit.current_source.unwrap();
    assert_eq!(
        editor.document.history.undo[0]
            .after
            .source(source)
            .unwrap()
            .adjustment_layers
            .as_ref()
            .unwrap()
            .layers[0]
            .adjustment
            .photo
            .exposure,
        7.123456789012345
    );
}

#[test]
fn grain_controls_are_saved_dirty_and_undoable_rendering_state() {
    let mut editor = controller();
    let directory = tempfile::tempdir().unwrap();
    let saved = editor
        .document
        .save(
            &directory.path().join("film.antitone"),
            crate::project::Save::New,
        )
        .unwrap();
    editor.document.accept_save(saved);
    let before = editor.history_snapshot();
    let mut edit = editor.edit_project("Rendering settings").unwrap();
    let mut values = edit.values();
    values.output.grain = crate::grain::GrainSettings {
        enabled: true,
        size: 0.025,
        image_sigma: 0.025,
        amount: 0.040,
        ..Default::default()
    };
    let expected_grain = values.output.grain;
    edit.replace(values);
    assert!(!editor.document.dirty());
    assert_eq!(
        editor.document.project_snapshot().output.grain,
        expected_grain
    );
    editor.commit_project(edit).unwrap();
    assert!(editor.document.dirty());
    let entry = &editor.document.history.undo[0];
    assert_eq!(entry.before, before);
    assert_eq!(
        entry.after.output.grain,
        editor.document.project.output.grain
    );
    let written = serde_json::to_string(&entry.after).unwrap();
    let reopened: Project = serde_json::from_str(&written).unwrap();
    assert_eq!(reopened.output.grain, editor.document.project.output.grain);
    editor = accept_history(editor, Direction::Undo);
    assert_eq!(editor.document.project.output.grain, before.output.grain);
    assert!(!editor.document.dirty());
    editor = accept_history(editor, Direction::Redo);
    assert_eq!(
        editor.document.project.output.grain,
        crate::grain::GrainSettings {
            enabled: true,
            size: 0.025,
            image_sigma: 0.025,
            amount: 0.040,
            ..Default::default()
        }
    );
}

#[test]
fn busy_work_and_decisions_cannot_dispatch_document_history() {
    for busy in [false, true] {
        let mut editor = controller();
        let before = editor.history_snapshot();
        editor.document.project.roll.exposure = 1.;
        editor.record_edit(before, "Exposure", None);
        assert!(editor.history_available());
        if busy {
            editor.set_test_busy(true);
        } else {
            editor.block_with_decision();
        }
        editor.restore_history(Direction::Undo, &UiContext::render_only());
        assert_eq!(editor.document.history.undo.len(), 1);
    }
}

#[test]
fn restoration_is_immediate_without_services_and_does_not_change_when_facts_resolve() {
    use crate::application::test_support::with_application;
    use crate::processing::InputSettings;
    for resolved in [false, true] {
        let raw = crate::raw::fixture::bayer_dng_with_as_shot([1800, 2200, 2800], [2, 1, 3]);
        let mut state = Document::default();
        state.import(&[raw.path().into()]).unwrap();
        state.project.input = InputSettings::Digital;
        state.project.scan = None;
        let id = state.project.order[0];
        let mut editor = crate::editor::tests::fixture();
        editor.document = state.into();
        editor.document.visit.current_source = Some(id);
        assert!(editor.source_services.is_none() && editor.developments.is_none());
        let physical = editor.document.source_input(id).unwrap();
        assert!(physical.development().unwrap().resolved().is_err());
        if resolved {
            let loader = crate::source::SourceLoader::new();
            assert!(
                pollster::block_on(physical.camera_samples(&loader, None, &Default::default()))
                    .unwrap()
                    .is_some()
            );
            assert!(physical.development().unwrap().resolved().is_ok());
            loader.close_and_wait().unwrap();
        }
        let before = editor.history_snapshot();
        editor.document.project.roll.exposure = 0.75;
        editor.record_edit(before.clone(), "Exposure", None);
        let after = editor.document.project.clone();
        std::fs::remove_file(raw.path()).unwrap();
        with_application(editor, |app| {
            app.enter(|editor, cx| {
                for (direction, expected, depths) in [
                    (Direction::Undo, &before, (0, 1)),
                    (Direction::Redo, &after, (1, 0)),
                    (Direction::Undo, &before, (0, 1)),
                ] {
                    editor.restore_history(direction, cx);
                    assert_eq!(&editor.document.state.project, expected);
                    assert_eq!(
                        (
                            editor.document.history.undo.len(),
                            editor.document.history.redo.len()
                        ),
                        depths
                    );
                    assert!(!editor.interaction_blocked());

                    assert!(
                        editor
                            .activities
                            .view()
                            .is_none_or(|view| !view.title.contains("Restor"))
                    );
                }
            });
            app.run_until(Duration::from_secs(10), |editor| {
                !editor.document.visit.source.pending()
            });
            app.read(|editor| {
                assert_eq!(editor.document.project, before);
                assert_eq!(editor.history_label(Direction::Redo), Some("Exposure"));
                assert!(editor.document.visit.canvas.ready.is_none());
            });
        });
    }
}

#[test]
fn grading_history_retains_lazy_camera_rasters_without_snapshotting_initialization() {
    use crate::{processing::InputSettings, source::SourceLoader};
    let raw = crate::raw::fixture::bayer_dng_with_as_shot([1800, 2200, 2800], [2, 1, 3]);
    let mut state = Document::default();
    state.import(&[raw.path().into()]).unwrap();
    state.project.input = InputSettings::Digital;
    state.project.scan = None;
    let id = state.project.order[0];
    let mut editor = crate::editor::tests::fixture();
    editor.document = state.into();
    let physical = editor.document.source_input(id).unwrap();
    let loader = SourceLoader::new();
    let camera = pollster::block_on(physical.camera_samples(&loader, None, &Default::default()))
        .unwrap()
        .unwrap();
    loader.close_and_wait().unwrap();
    let pixels = camera.pixels();
    editor.document.sampling_inputs.record(id, &camera);
    assert!(
        editor
            .document
            .state
            .development_metadata
            .entries
            .is_empty(),
        "facts belong to live initialization, not a captured document"
    );
    let before = editor.history_snapshot();
    editor.document.project.roll.exposure = 0.75;
    editor.record_edit(before, "Exposure", None);
    std::fs::remove_file(raw.path()).unwrap();
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.restore_history(Direction::Undo, cx);
            assert_eq!(editor.document.project.roll.exposure, 0.);
            assert!(Arc::ptr_eq(
                &pixels,
                &editor
                    .document
                    .sampling_inputs
                    .get(id, camera.key())
                    .unwrap()
            ));
            assert!(
                editor
                    .document
                    .state
                    .development_metadata
                    .entries
                    .is_empty()
            );
        });
    });
}

#[test]
fn correction_resource_failure_does_not_reverse_restoration_or_advance_history_again() {
    let mut state = Document::default();
    state
        .import(&[PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng")])
        .unwrap();
    let source = &state.project.sources[0];
    let map = Arc::new(
        crate::flat_field::FlatField::from_nodes(
            source.development.active_area,
            [2, 2],
            vec![DVec3::splat(2.); 4],
        )
        .unwrap(),
    );
    let field = crate::project::FlatFieldSettings::new(source.path.clone(), map);
    state.project.set_flat_field(Some(field)).unwrap();
    let mut editor = crate::editor::tests::fixture();
    editor.document = state.into();
    let before = editor.history_snapshot();
    editor.document.project.set_flat_field(None).unwrap();
    editor.record_edit(before.clone(), "Clear correction", None);
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.restore_history(Direction::Undo, cx);
            assert_eq!(editor.document.project, before);
            assert_eq!(
                editor.history_label(Direction::Redo),
                Some("Clear correction")
            );
            assert!(!editor.interaction_blocked());
        });
        app.run_until(Duration::from_secs(10), |editor| {
            !editor.document.correction.pending() && !editor.document.visit.source.pending()
        });
        app.read(|editor| {
            assert_eq!(editor.document.project, before);
            assert!(editor.document.correction.error().is_some());
            assert_eq!(
                (
                    editor.document.history.undo.len(),
                    editor.document.history.redo.len()
                ),
                (0, 1)
            );
            assert!(editor.document.project.active_flat_field().is_some());
        });
    });
}

#[test]
fn a_live_project_or_document_edit_rejects_history_without_retiring_the_owner() {
    for document in [false, true] {
        let mut editor = crate::editor::tests::fixture();
        let before = editor.history_snapshot();
        editor.document.project.roll.exposure = 1.;
        editor.record_edit(before, "Exposure", None);
        let accepted = editor.document.project.clone();
        if document {
            let edit = editor.edit_document("Held document").unwrap();
            editor.restore_history(Direction::Undo, &UiContext::render_only());
            assert!(editor.owns_document_edit(&edit));
            assert_eq!(editor.document.project, accepted);
            assert_eq!(
                (
                    editor.document.history.undo.len(),
                    editor.document.history.redo.len()
                ),
                (1, 0)
            );
            drop(edit);
        } else {
            let edit = editor.edit_project("Held project").unwrap();
            let preview = edit.preview();
            editor.restore_history(Direction::Undo, &UiContext::render_only());
            assert!(preview.active());
            assert_eq!(editor.document.project, accepted);
            assert_eq!(
                (
                    editor.document.history.undo.len(),
                    editor.document.history.redo.len()
                ),
                (1, 0)
            );
            drop(edit);
        }
        editor.restore_history(Direction::Undo, &UiContext::render_only());
        assert_eq!(editor.document.project.roll.exposure, 0.);
        assert_eq!(
            (
                editor.document.history.undo.len(),
                editor.document.history.redo.len()
            ),
            (0, 1)
        );
    }
}
