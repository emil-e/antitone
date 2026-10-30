use super::*;

mod freshness;
mod sampling;
mod save;
use crate::{
    crop::Crop,
    project::{Project, Save},
    spatial_metadata::{Boundary, CommonSize, Group, Region, SourceRegions, VERSION},
};

/// Declared physical hypotheses isolate consumer transactions from blind discovery. The
/// authored roll's coordinate basis is real, but these bounds are not detector output.
pub(super) fn declared(editor: &Editor) -> Metadata {
    let sources: Vec<_> = editor
        .document
        .spatial_inputs()
        .into_iter()
        .map(|input| {
            let region = Region {
                angle: 0.,
                skew: 0.,
                boundaries: [
                    [1., 0., 96.],
                    [-1., 0., -416.],
                    [0., 1., 96.],
                    [0., -1., -416.],
                ]
                .map(|plane| Boundary::Measured { plane }),
            };
            SourceRegions {
                input,
                observation: Some(region.clone()),
                picture: Some(region),
                film: None,
                holder_opening: None,
                material: None,
            }
        })
        .collect();
    let metadata = Metadata {
        version: VERSION,
        groups: vec![Group {
            development: editor.document.state.project.sources[0]
                .development
                .decode
                .clone(),
            code_scale: editor.document.spatial_code_scale(),
            common_size: CommonSize::fit(&sources),
            sources,
        }],
    };
    metadata.validate().unwrap();
    metadata
}

fn supplied(editor: &mut Editor) -> Metadata {
    let metadata = declared(editor);
    let inputs = editor.document.spatial_inputs();
    let scope = &mut editor.document;
    let request = scope.spatial.begin().unwrap();
    assert!(
        scope
            .spatial
            .publish(request, metadata.clone(), &scope.state.project, &inputs)
            .unwrap()
    );
    editor.auto_crop_margin = 0.;
    metadata
}

#[test]
fn prepared_regions_apply_atomically_and_undo_redo_only_the_accepted_crops() {
    let (scheduler, _) = observed();
    let mut editor = fixture(scheduler.clone());
    let regions = supplied(&mut editor);
    assert!(
        editor.document.state.project.analysis_regions.is_empty(),
        "this must not rely on accepted Analyze evidence"
    );
    with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.review_crop_proposals(editor.document.project.order.clone(), "Whole roll", cx)
        });
        app.run_until(LIMIT, |editor| editor.document.spatial_preparing.is_none());
        app.update(|editor| {
            assert!(editor.auto_crop_valid(), "{}", editor.status);
            assert!(
                editor
                    .document
                    .project
                    .sources
                    .iter()
                    .all(|s| s.crop.is_none())
            );
            editor.finish_auto_crop(true);
        });
        app.run_until(LIMIT, |editor| editor.document.auto_crop_edit().is_none());
        app.read(|editor| {
            assert!(
                editor.document.auto_crop_edit().is_none(),
                "{}",
                editor.status
            );
            for source in &editor.document.state.project.sources {
                assert_eq!(
                    source.crop,
                    Some(Crop::from_region([96, 96, 320, 320].into()))
                );
            }
            assert_eq!(
                editor.history_label(history::Direction::Undo),
                Some("Auto-crop")
            );
            assert_eq!(*editor.document.spatial.ready(), regions);
        });
        app.enter(|editor, cx| editor.restore_history(history::Direction::Undo, cx));
        app.run_until(LIMIT, |editor| editor.document_pending.is_none());
        app.read(|editor| {
            assert!(
                editor
                    .document
                    .project
                    .sources
                    .iter()
                    .all(|s| s.crop.is_none()),
                "{}",
                editor.status
            );
            assert_eq!(*editor.document.spatial.ready(), regions);
            assert!(editor.document.state.project.analysis_regions.is_empty());
        });
        app.enter(|editor, cx| editor.restore_history(history::Direction::Redo, cx));
        app.run_until(LIMIT, |editor| editor.document_pending.is_none());
        app.read(|editor| {
            assert!(
                editor
                    .document
                    .project
                    .sources
                    .iter()
                    .all(|s| s.crop == Some(Crop::from_region([96, 96, 320, 320].into()))),
                "{}",
                editor.status
            );
            assert_eq!(*editor.document.spatial.ready(), regions);
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn changed_supporting_geometry_prevents_any_selected_crop_from_being_applied() {
    let (scheduler, _) = observed();
    let mut editor = fixture(scheduler.clone());
    let dir = tempfile::tempdir().unwrap();
    let peer = editor.document.state.project.order[1];
    let copy = dir.path().join("supporter.dng");
    std::fs::copy(editor.document.source_path(peer).unwrap(), &copy).unwrap();
    editor.document.state.project.source_mut(peer).unwrap().path = copy.clone();
    editor.document.capture_source_files().unwrap();
    supplied(&mut editor);
    editor.document.roll_selection = Default::default();
    editor.document.roll_selection.reconcile(
        &editor.document.state.project.order,
        editor.document.visit.current_source,
    );
    let undo = editor.history_label(history::Direction::Undo);
    with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.review_crop_proposals(
                editor
                    .document
                    .project
                    .order
                    .iter()
                    .copied()
                    .filter(|id| editor.document.roll_selection.contains(*id))
                    .collect(),
                "Entry selection",
                cx,
            )
        });
        app.run_until(LIMIT, |editor| editor.document.spatial_preparing.is_none());
        let accepted = app.update(|editor| {
            assert!(editor.auto_crop_valid());
            editor
                .document
                .project
                .source_mut(peer)
                .unwrap()
                .development
                .active_area[2] -= 1;
            let accepted = editor.document.state.project.clone();
            editor.finish_auto_crop(true);
            accepted
        });
        app.run_until(LIMIT, |editor| {
            editor
                .document
                .auto_crop_operation()
                .is_some_and(|op| op.apply.is_some())
        });
        app.read(|editor| {
            assert!(editor.document.auto_crop_edit().is_some());
            assert_eq!(editor.document.state.project, accepted);
            assert_eq!(editor.history_label(history::Direction::Undo), undo);
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn selected_targets_are_captured_at_request_time_and_cancel_does_not_accept_margins() {
    let (scheduler, starts) = observed();
    let mut editor = fixture(scheduler.clone());
    let regions = supplied(&mut editor);
    let first = editor.document.state.project.order[0];
    let second = editor.document.state.project.order[1];
    editor.document.roll_selection = Default::default();
    editor
        .document
        .roll_selection
        .reconcile(&editor.document.state.project.order, Some(first));
    let admission = crate::background::testing::pause();
    with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.review_crop_proposals(
                editor
                    .document
                    .project
                    .order
                    .iter()
                    .copied()
                    .filter(|id| editor.document.roll_selection.contains(*id))
                    .collect(),
                "Entry selection",
                cx,
            )
        });
        app.update(|editor| {
            editor.document.roll_selection = Default::default();
            editor
                .document
                .roll_selection
                .reconcile(&editor.document.state.project.order, Some(second));
        });
        drop(admission);
        app.run_until(LIMIT, |editor| editor.document.spatial_preparing.is_none());
        app.update(|editor| {
            assert!(editor.auto_crop_preview(first).is_some());
            assert!(editor.auto_crop_preview(second).is_none());
            editor.adjust_auto_crop_margin(1.);
            assert_eq!(*editor.document.spatial.ready(), regions);
            editor.finish_auto_crop(false);
            assert!(
                editor
                    .document
                    .project
                    .sources
                    .iter()
                    .all(|s| s.crop.is_none())
            );
        });
        assert_eq!(starts.load(Ordering::SeqCst), 0);
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn camera_limited_picture_metadata_is_a_usable_visible_crop_without_four_measured_sides() {
    let (scheduler, _) = observed();
    let mut editor = fixture(scheduler.clone());
    let mut metadata = supplied(&mut editor);
    let first = metadata.groups[0].sources[0].input.source;
    let row = &mut metadata.groups[0].sources[0];
    row.observation.as_mut().unwrap().boundaries[2] = Boundary::CameraClipped;
    row.picture = row.observation.clone();
    metadata.groups[0].common_size = CommonSize::fit(&metadata.groups[0].sources);
    metadata.validate().unwrap();
    let inputs = editor.document.spatial_inputs();
    let scope = &mut editor.document;
    let request = scope.spatial.begin().unwrap();
    assert!(
        scope
            .spatial
            .publish(request, metadata.clone(), &scope.state.project, &inputs)
            .unwrap()
    );
    with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.review_crop_proposals(editor.document.project.order.clone(), "Whole roll", cx)
        });
        app.run_until(LIMIT, |editor| editor.document.spatial_preparing.is_none());
        app.update(|editor| {
            assert!(editor.auto_crop_valid(), "{}", editor.status);
            let crop = editor.auto_crop_preview(first).unwrap().unwrap();
            let active = editor
                .document
                .project
                .source(first)
                .unwrap()
                .development
                .active_area;
            assert_eq!(
                crop.region(active.into()).unwrap(),
                [96, active[1], 320, 416 - active[1]].into()
            );
            assert_eq!(*editor.document.spatial.ready(), metadata);
            editor.finish_auto_crop(false);
            assert!(
                editor
                    .document
                    .project
                    .sources
                    .iter()
                    .all(|s| s.crop.is_none())
            );
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn a_real_save_captures_only_ready_regions_and_does_not_retire_in_flight_preparation() {
    let (scheduler, starts, release) = paused();
    let mut editor = fixture(scheduler.clone());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("captured.antitone");
    let written = editor.document.capture().save(&path, Save::New).unwrap();
    editor.document.accept_save(written);
    with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.review_crop_proposals(editor.document.project.order.clone(), "Whole roll", cx)
        });
        await_native(app, &starts);
        let request = app.read(|editor| editor.document.spatial_preparing.as_ref().unwrap().id);
        app.read(|editor| {
            use crate::platform::menu::Action;
            let state = editor.application_menu_state();
            assert!(state.enabled(Action::Save) && state.enabled(Action::SaveAs));
            for action in [
                Action::New,
                Action::Open,
                Action::AddScans,
                Action::Close,
                Action::Quit,
            ] {
                assert!(
                    !state.enabled(action),
                    "only captured writes may overlap shared preparation"
                );
            }
        });
        app.update(|editor| {
            crate::platform::host::App::menu_action(editor, crate::platform::menu::Action::Save)
        });
        let context = egui::Context::default();
        crate::editor::tests::app_frame(app, &context, vec![]);
        crate::editor::tests::app_frame(app, &context, vec![]);
        assert!(
            app.read(|editor| editor.document_pending.is_some()),
            "normal Save command must dispatch while regions are preparing"
        );
        assert!(app.read(|editor| {
            editor
                .document
                .spatial_preparing
                .as_ref()
                .is_some_and(|p| p.id == request)
        }));
        release.send(()).unwrap();
        app.run_until(LIMIT, |editor| {
            editor.document_pending.is_none()
                && editor.document.spatial_preparing.is_none()
                && editor.document.auto_crop_edit().is_some()
        });
        app.read(|editor| {
            assert!(
                editor.document.auto_crop_edit().is_some(),
                "Save must not discard the original consumer continuation: {}",
                editor.status
            );
            assert_eq!(
                editor
                    .document
                    .spatial
                    .ready()
                    .groups
                    .iter()
                    .map(|g| g.sources.len())
                    .sum::<usize>(),
                2
            );
            assert!(
                !editor.document.dirty(),
                "automatic later results do not alter the written accepted baseline"
            );
            assert_eq!(editor.document.path.as_deref(), Some(path.as_path()));
        });
        let (_, saved) = Project::load_with_spatial_metadata(&path).unwrap();
        assert!(
            saved.groups.is_empty(),
            "the captured write cannot gain results that arrived later"
        );
    });
    scheduler.close_and_wait().unwrap();
}
