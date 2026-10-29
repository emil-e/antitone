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

fn round_trip(mut editor: Editor, before: &Project, after: &Project) -> Editor {
    editor = crate::editor::tests::complete_document_operation(
        editor,
        Duration::from_secs(60),
        |e, cx| {
            e.restore_history(Direction::Undo, cx);
            assert!(e.busy && e.document_pending.is_some(), "undo must start");
        },
    );
    assert_eq!(&editor.document.state.project, before, "{}", editor.status);
    editor = crate::editor::tests::complete_document_operation(
        editor,
        Duration::from_secs(60),
        |e, cx| {
            e.restore_history(Direction::Redo, cx);
            assert!(e.busy && e.document_pending.is_some(), "redo must start");
        },
    );
    assert_eq!(&editor.document.state.project, after, "{}", editor.status);
    editor
}

#[test]
fn authored_history_restores_import_crop_dust_local_and_analysis_through_the_real_worker() {
    use crate::{
        adjustment_layers::{AdjustmentLayer, AdjustmentLayerCoverage},
        geometry::SourcePoint,
    };
    let instance = crate::gpu::instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
        .expect("history worker test requires Metal");
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
    assert!(editor.document.visit.ready.is_some(), "{}", editor.status);
    let imported = editor.document.state.project.clone();
    editor = round_trip(editor, &empty, &imported);
    assert!(editor.document.visit.ready.is_some());
    // Calibration is fixture setup; accepted picks have independent controller coverage.
    editor.set_dmin(Some(DVec3::splat(0.2)));
    editor.set_dmax(Some(DVec3::splat(0.005)));
    editor.document.source = false;
    editor.tab = 1;
    let before_crop = editor.history_snapshot();
    editor.begin_crop();
    editor.document.visit.crop_edit.as_mut().unwrap().draft.size = [256, 256];
    editor.finish_crop(false);
    assert_eq!(
        editor.document.state.project, before_crop,
        "cancelled crops are not edits"
    );
    editor.begin_crop();
    editor.document.visit.crop_edit.as_mut().unwrap().draft.size = [256, 256];
    editor.finish_crop(true);
    let cropped = editor.document.state.project.clone();
    editor = round_trip(editor, &before_crop, &cropped);

    editor.begin_dust_cleanup();
    let active = editor.document.visit.ready.as_ref().unwrap().area;
    let geometry = editor.document.visit.viewport.layout_composition(
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
        editor.document.visit.dust.cleanup,
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

fn controller() -> Editor {
    let mut editor = crate::editor::tests::authored_fixture(&["roll-0.dng", "roll-1.dng"]);
    // The authored import is setup, not the edit under test.
    editor.document.history = History::default();
    editor.set_dmin(Some(DVec3::new(0.2, 0.15, 0.1)));
    editor.set_dmax(Some(DVec3::new(0.008, 0.005, 0.002)));
    editor.sync_document();
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
    editor.document.visit.adjustment_layer_settings.exposure = 7.123456789012345;
    editor.record_edit(before.clone(), "Photo exposure", Some(ids[0]));
    let after = editor.document.state.project.clone();
    editor.document.visit.current_source = Some(ids[1]);
    editor.document.visit.adjustment_layer_settings = AdjustmentLayerSettings::default();
    editor.document.visit.viewport.zoom = Some(1.75);
    let viewport = editor.document.visit.viewport;
    editor = accept_history(editor, Direction::Undo);
    assert_eq!(editor.document.state.project, before);
    assert_eq!(editor.document.visit.current_source, Some(ids[0]));
    assert_eq!(editor.document.visit.adjustment_layer_settings.exposure, 0.);
    assert_eq!(editor.document.visit.viewport, viewport);
    editor = accept_history(editor, Direction::Redo);
    assert_eq!(editor.document.state.project, after);
    assert_eq!(
        editor.document.visit.adjustment_layer_settings.exposure,
        7.123456789012345
    );
    assert_eq!(editor.document.visit.viewport, viewport);
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
    editor.document.path = Some(old.clone());
    let before = editor.history_snapshot();
    editor.document.visit.adjustment_layer_settings.exposure = 1.;
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
    let source = editor.document.visit.current_source.unwrap();
    let locator = editor.document.source_path(source).unwrap();
    editor = accept_history(editor, Direction::Undo);
    assert!(editor.document.dirty());
    assert_eq!(editor.document.path, Some(new.clone()));
    assert_eq!(editor.document.source_path(source).unwrap(), locator);
    editor = accept_history(editor, Direction::Redo);
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
            e.document.roll.exposure = 0.375;
            e.record_edit(before, "Roll exposure", None);
        },
    );
    assert_eq!(written.project.roll.exposure, 0.);
    assert!(editor.document.dirty());
    assert_eq!(editor.document.roll.exposure, 0.375);
    editor = accept_history(editor, Direction::Undo);
    assert!(!editor.document.dirty());
    editor = accept_history(editor, Direction::Redo);
    assert!(editor.document.dirty());
    assert_eq!(editor.document.roll.exposure, 0.375);
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
    editor.document.roll.exposure = 0.5;
    editor.record_edit(before, "Roll exposure", None);
    editor = crate::editor::tests::complete_document_operation(
        editor,
        Duration::from_secs(60),
        |editor, cx| {
            editor.start_document(Preparation::Open("missing.antitone".into()), cx);
            assert!(editor.busy && editor.document_pending.is_some());
        },
    );
    assert_eq!(editor.document.history.undo.len(), 1);
    assert_eq!(editor.document.roll.exposure, 0.5);
    editor = crate::editor::tests::complete_document_operation(
        editor,
        Duration::from_secs(60),
        |editor, cx| {
            editor.start_document(Preparation::New, cx);
            assert!(editor.busy && editor.document_pending.is_some());
            assert_eq!(
                editor.document.history.undo.len(),
                1,
                "dispatch is not acceptance"
            );
        },
    );
    assert!(editor.document.history.undo.is_empty());
    assert!(editor.document.history.redo.is_empty());
    assert!(!editor.document.dirty());
}

fn finish_controls(editor: &mut Editor) {
    let context = egui::Context::default();
    let photo = editor.document.visit.current_source;
    let mut output = context.run_ui(Default::default(), |ui| editor.record_controls(photo, ui));
    output.textures_delta.clear();
}

#[test]
fn unchanged_control_passes_do_not_clone_projects_and_local_edits_capture_their_before_state_lazily()
 {
    let mut editor = controller();
    let original = editor.history_snapshot();
    editor.history_snapshot_clones = 0;
    editor.begin_control_history("Photo settings");
    finish_controls(&mut editor);
    assert_eq!(editor.history_snapshot_clones, 0);
    assert!(editor.document.history.undo.is_empty());

    editor.begin_control_history("Photo settings");
    // Renaming changes saved metadata without changing photographic controls.
    editor
        .edit_adjustment_layers(|layers| {
            layers.layers[0].name = "Renamed".into();
            Ok(())
        })
        .unwrap();
    assert_eq!(editor.history_snapshot_clones, 1);
    assert!(
        editor.document.history.undo.is_empty(),
        "the control pass still owns the edit"
    );
    finish_controls(&mut editor);
    assert_eq!(editor.history_snapshot_clones, 2);
    assert_eq!(editor.document.history.undo[0].before, original);
    assert!(
        editor.document.history.undo[0]
            .after
            .source(editor.document.visit.current_source.unwrap())
            .unwrap()
            .adjustment_layers
            .is_some()
    );

    editor.history_snapshot_clones = 0;
    editor.begin_control_history("Photo settings");
    finish_controls(&mut editor);
    assert_eq!(editor.history_snapshot_clones, 0);
    assert_eq!(editor.document.history.undo.len(), 1);
}

#[test]
fn scalar_control_edits_materialize_exact_before_and_after_projects_only_after_a_change() {
    let mut editor = controller();
    let original = editor.history_snapshot();
    editor.history_snapshot_clones = 0;
    editor.begin_control_history("Photo settings");
    editor.document.visit.adjustment_layer_settings.exposure = 7.123456789012345;
    assert_eq!(
        editor.history_snapshot_clones, 0,
        "capture contains only small control values"
    );
    finish_controls(&mut editor);
    assert_eq!(editor.history_snapshot_clones, 2);
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
fn film_emulation_controls_are_saved_dirty_and_undoable_rendering_state() {
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
    editor.begin_control_history("Rendering settings");
    editor.document.film_emulation = FilmEmulationPreset::Fine.settings();
    finish_controls(&mut editor);
    assert!(editor.document.dirty());
    let entry = &editor.document.history.undo[0];
    assert_eq!(entry.before, before);
    assert_eq!(
        entry.after.output.film_emulation,
        editor.document.film_emulation
    );
    let written = serde_json::to_string(&entry.after).unwrap();
    let reopened: Project = serde_json::from_str(&written).unwrap();
    assert_eq!(
        reopened.output.film_emulation,
        editor.document.film_emulation
    );
    editor = accept_history(editor, Direction::Undo);
    assert_eq!(editor.document.film_emulation, before.output.film_emulation);
    assert!(!editor.document.dirty());
    editor = accept_history(editor, Direction::Redo);
    assert_eq!(
        editor.document.film_emulation,
        FilmEmulationPreset::Fine.settings()
    );
}

#[test]
fn busy_decisions_and_drafts_cannot_dispatch_document_history() {
    let mut editor = controller();
    let before = editor.history_snapshot();
    editor.document.roll.exposure = 1.;
    editor.record_edit(before, "Exposure", None);
    for blocked in 0..4 {
        editor.busy = blocked == 0;
        editor.document.visit.picking.display = (blocked == 1).then_some(Pick::Dmin.display());
        editor.document.removal_pending =
            (blocked == 2).then_some(vec![editor.document.visit.current_source.unwrap()]);
        editor.transition = (blocked == 3).then_some(Transition::New);
        editor.restore_history(Direction::Undo, &UiContext::render_only());
        assert!(editor.document_pending.is_none());
        assert_eq!(editor.document.history.undo.len(), 1);
    }
}

#[test]
fn failed_and_stale_restoration_preserve_state_and_history_until_acceptance() {
    let mut editor = crate::editor::tests::fixture();
    let before = editor.history_snapshot();
    editor.document.roll.exposure = 1.;
    editor.record_edit(before, "Exposure", None);
    let accepted = editor.document.state.project.clone();
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.restore_history(Direction::Undo, cx);
            let request = editor.document_pending.unwrap();
            assert_eq!(editor.document.state.project, accepted);
            editor.document_prepared(request + 1, Err("stale".into()));
            assert_eq!(editor.document_pending, Some(request));
            editor.document_prepared(request, Err("missing RAW".into()));
            assert_eq!(editor.document.state.project, accepted);
            assert_eq!(editor.document.history.undo.len(), 1);
            assert!(editor.document.history.redo.is_empty());
            assert!(!editor.busy);
        });
        app.pump();
    });
}
