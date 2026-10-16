use super::*;
#[path = "editor_analysis_private_review_tests.rs"]
mod private;
use crate::{project::SourceAnalysis, scan_regions::Areas};

fn reply(
    _editor: &mut Editor,
    receiver: &mpsc::Receiver<Command>,
) -> (u64, analysis_review::Approved) {
    let command = receiver
        .try_iter()
        .find(|c| matches!(c, Command::AnalysisAreas(..)))
        .expect("area preparation command");
    let Command::AnalysisAreas(id, request) = command else {
        unreachable!()
    };
    let entries = request
        .document
        .project
        .order
        .iter()
        .map(|&id| {
            let active: Rect = request
                .document
                .project
                .source(id)
                .unwrap()
                .development
                .active_area
                .into();
            let opening = Coverage::rectangle(active).corners().map(|p| p.0);
            let picture = Coverage::rectangle(request.picture.unwrap_or_else(|| suggest(active)))
                .corners()
                .map(|p| p.0);
            SourceAnalysis {
                source: id,
                areas: Areas {
                    opening: Some(opening),
                    picture: Some(picture),
                    opening_observed: [true; 4],
                    picture_observed: [true; 4],
                    ..Areas::default()
                },
            }
        })
        .collect();
    let files = request
        .document
        .project
        .order
        .iter()
        .map(|&id| thumbnails::FileKey::new(&request.document.source_path(id).unwrap()))
        .collect();
    (id, analysis_review::Approved { entries, files })
}
fn prepared(editor: &mut Editor, receiver: &mpsc::Receiver<Command>) {
    let (id, approved) = reply(editor, receiver);
    editor.analysis_areas_result(id, Ok(Some(approved)));
    assert!(editor.analysis_review.is_some(), "{}", editor.status);
}

#[test]
fn both_sampling_frames_select_directly_and_resize_without_changing_accepted_areas() {
    let (mut editor, _send, receiver) = ready_state();
    editor.request_analysis();
    prepared(&mut editor, &receiver);
    let active = editor.ready.as_ref().unwrap().area;
    let opening = Coverage::rectangle(active);
    let picture = Coverage::rectangle(Rect {
        x: active.x + active.width / 4,
        y: active.y + active.height / 4,
        width: active.width / 2,
        height: active.height / 2,
    });
    let areas = &mut editor.analysis_review.as_mut().unwrap().approved.entries[0].areas;
    areas.opening = Some(opening.corners().map(|p| p.0));
    areas.picture = Some(picture.corners().map(|p| p.0));
    editor.refresh_review_region();
    let before = editor.document.project.clone();
    let mut viewport = Viewport::default();
    viewport.zoom = Some(0.5);
    let geometry = viewport.layout_composition(
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000., 700.)),
        Composition::new(active, Default::default(), Default::default()).unwrap(),
        1.,
    );
    let point = |c: Coverage, i: usize| {
        geometry.screen_frame_point(geometry.composition.source_to_frame(c.corners()[i]))
    };
    assert_eq!(
        editor.review_area_at(geometry, point(picture, 0)),
        Some(true)
    );
    editor.select_review_area(geometry, point(picture, 0));
    assert!(editor.analysis_review.as_ref().unwrap().picture);
    let drag = PickDrag::begin(geometry, point(picture, 0), editor.region);
    editor.region = Some(drag.update(geometry, point(picture, 0) + egui::vec2(5., 5.)));
    editor.stash_review_region();
    let edited = editor.review_areas(editor.current_source.unwrap()).unwrap();
    assert!(edited.picture_edited);
    assert_eq!(edited.opening, Some(opening.corners().map(|p| p.0)));
    editor.select_review_area(geometry, point(opening, 0));
    assert!(!editor.analysis_review.as_ref().unwrap().picture);
    assert_eq!(editor.region, Some(opening));
    assert_eq!(editor.document.project, before);
    let areas = &mut editor.analysis_review.as_mut().unwrap().approved.entries[0].areas;
    areas.picture = areas.opening;
    assert_eq!(
        editor.review_area_at(geometry, point(opening, 0)),
        Some(false),
        "coincident handles retain the selected area"
    );
    editor.select_review_area(geometry, geometry.image.center());
    assert!(editor.analysis_review.as_ref().unwrap().picture);
    editor.select_review_area(geometry, point(opening, 0) + egui::vec2(8., 8.));
    assert!(
        !editor.analysis_review.as_ref().unwrap().picture,
        "labels disambiguate coincident frames"
    );
}
#[test]
fn drawing_missing_opening_does_not_invent_picture_support_and_last_picture_edit_enables_analyze() {
    let (mut editor, _send, receiver) = ready_state();
    editor.request_analysis();
    let (id, mut approved) = reply(&mut editor, &receiver);
    approved.entries[0].areas = Areas::default();
    editor.analysis_areas_result(id, Ok(Some(approved)));
    let area = editor.ready.as_ref().unwrap().area;
    let ctx = egui::Context::default();
    editor.region = Some(Coverage::rectangle(area));
    super::super::super::tests::app_frame(&mut editor, &ctx, vec![]);
    assert!(
        editor
            .review_areas(editor.current_source.unwrap())
            .unwrap()
            .opening
            .is_some()
    );
    assert!(
        editor
            .review_areas(editor.current_source.unwrap())
            .unwrap()
            .picture
            .is_none()
    );
    let geometry = editor.last_layout.unwrap().0;
    editor.select_review_area(geometry, geometry.image.center());
    editor.region = Some(Coverage::rectangle(area));
    super::super::super::tests::app_frame(&mut editor, &ctx, vec![]);
    assert!(
        editor
            .review_areas(editor.current_source.unwrap())
            .unwrap()
            .ready()
    );
    click_label(&mut editor, &ctx, "Analyze roll");
    assert!(
        receiver
            .try_iter()
            .any(|c| matches!(c, Command::Analyze(..)))
    );
}
#[test]
fn opening_exception_updates_automatic_picture_coverage_but_not_authored_picture_or_peer() {
    let (mut editor, _send, receiver) = ready_state();
    let mut other = editor.document.project.sources[0].clone();
    other.id = SourceId(0);
    other.path = "other.ARW".into();
    editor.document.project.add_source(other).unwrap();
    editor.request_analysis();
    prepared(&mut editor, &receiver);
    let active = editor.ready.as_ref().unwrap().area;
    let review = editor.analysis_review.as_mut().unwrap();
    let a = &mut review.approved.entries[0].areas;
    a.picture_observed = [false; 4];
    a.picture_geometry = Some(crate::scan_regions::PictureGeometry {
        size: [768, 512],
        angle: 0.,
        sides: [None; 4],
    });
    a.refit_picture(active);
    let original = a.picture;
    let peer = review.approved.entries[1].clone();
    let before = editor.document.project.clone();
    editor.region = Some(Coverage::rectangle(rect([90, 70, 600, 400])));
    editor.stash_review_region();
    let review = editor.analysis_review.as_mut().unwrap();
    assert_ne!(review.approved.entries[0].areas.picture, original);
    assert_eq!(review.approved.entries[1], peer);
    let authored = Coverage::rectangle(rect([100, 100, 500, 300]));
    review.approved.entries[0].areas.edit_picture(authored);
    editor.region = Some(Coverage::rectangle(rect([120, 90, 500, 300])));
    editor.stash_review_region();
    assert_eq!(
        editor.analysis_review.as_ref().unwrap().approved.entries[0]
            .areas
            .picture,
        Some(authored.corners().map(|p| p.0))
    );
    assert_eq!(editor.document.project, before);
}
#[test]
fn all_ordered_photo_fields_control_the_warning_in_one_review_not_a_second_dialog() {
    for field in 0..7 {
        let (mut editor, _send, receiver) = ready_state();
        let mut other = editor.document.project.sources[0].clone();
        other.id = SourceId(0);
        other.path = "other.ARW".into();
        match field {
            0 => other.photo.exposure = 0.1,
            1 => other.photo.wb.warmth = 0.1,
            2 => other.photo.contrast = 1.1,
            3 => other.photo.pivot = 0.3,
            4 => other.photo.contrast_colour_mix = 0.4,
            5 => other.photo.saturation = 1.1,
            _ => other.photo.saturation_density = 0.4,
        }
        editor.document.project.add_source(other).unwrap();
        editor.sync_document();
        let before = editor.document.project.clone();
        editor.request_analysis();
        assert!(
            editor.busy && editor.analysis_preparing.is_some() && editor.analysis_pending.is_none()
        );
        prepared(&mut editor, &receiver);
        assert!(
            editor.analysis_review.as_ref().unwrap().replaces_photos,
            "field {field}"
        );
        editor.request_analysis();
        assert!(receiver.try_recv().is_err());
        editor.finish_pick();
        assert_eq!(editor.document.project, before);
        assert!(editor.analysis_pending.is_none() && editor.analysis_review.is_none());
        assert!(receiver.try_recv().is_err());
    }
}
#[test]
fn failed_analysis_dispatch_tears_down_review_and_restores_working_photo() {
    let (mut editor, _send, receiver) = ready_state();
    let first = editor.current_source;
    let mut other = editor.document.project.sources[0].clone();
    other.id = SourceId(0);
    other.path = "other.ARW".into();
    other.photo.exposure = 1.7;
    let other = editor.document.project.add_source(other).unwrap();
    editor.select_roll_photo(1, egui::Modifiers::COMMAND);
    editor.request_analysis();
    prepared(&mut editor, &receiver);
    let before = editor.document.project.clone();
    let proxy = editor.ready.as_ref().unwrap().proxy.clone();
    let development = editor.ready.as_ref().unwrap().development.clone();
    editor.browse_analysis_review(other);
    let record = editor.document.project.source(other).unwrap().clone();
    editor.ready = Some(Ready {
        file: thumbnails::FileKey::new(&record.path),
        thumbnail: None,
        path: record.path.clone(),
        area: record.development.active_area.into(),
        analysis_support: None,
        floor: editor.floor,
        proxy,
        record,
        development,
    });
    editor.busy = false;
    drop(receiver);
    editor.confirm_analysis_review();
    assert!(
        editor.analysis_pending.is_none()
            && editor.analysis_review.is_none()
            && editor.pick.is_none()
    );
    assert_eq!(editor.current_source, first);
    assert!(
        editor.roll_selection.contains(first.unwrap()) && editor.roll_selection.contains(other)
    );
    assert_eq!(editor.document.project, before);
}
#[test]
fn default_photos_still_require_area_review_but_retired_grades_do_not_trigger_reset_warning() {
    let (mut editor, _send, receiver) = ready_state();
    let mut retired = editor.document.project.sources[0].clone();
    retired.id = SourceId(0);
    retired.path = "retired.ARW".into();
    retired.photo.contrast = 1.3;
    let retired = editor.document.project.add_source(retired).unwrap();
    editor.document.project.order.retain(|id| *id != retired);
    editor.roll.exposure = 1.;
    editor.roll.wb.tint = 0.2;
    editor.request_analysis();
    prepared(&mut editor, &receiver);
    assert!(!editor.analysis_review.as_ref().unwrap().replaces_photos);
    assert!(editor.analysis_pending.is_none());
    editor.confirm_analysis_review();
    let Command::Analyze(_, request) = receiver.try_recv().unwrap() else {
        panic!("confirmed analysis command")
    };
    assert_eq!(request.areas.unwrap().entries.len(), 1);
    assert_eq!(
        editor
            .document
            .project
            .source(retired)
            .unwrap()
            .photo
            .contrast,
        1.3
    );
}
fn change(editor: &mut Editor, kind: usize) {
    match kind {
        0 => editor.id += 1,
        1 => editor.current_source = None,
        2 => editor.document.path = Some("other.antitone.json".into()),
        3 => editor.document.project.order.clear(),
        4 => editor.photo.pivot = 0.4,
        5 => editor.roll.gamma = 0.8,
        6 => {
            editor.analysis_generation.fetch_add(1, Ordering::Relaxed);
        }
        7 => editor.document.project.analysis_area = Some([52, 42, 720, 540]),
        _ => editor.pick = Some(Pick::Wb),
    }
    editor.sync_document();
}
#[test]
fn stale_preparation_cannot_open_review_or_change_accepted_state() {
    for changed in 0..9 {
        let (mut editor, _send, receiver) = ready_state();
        editor.request_analysis();
        let (id, approved) = reply(&mut editor, &receiver);
        change(&mut editor, changed);
        let before = editor.document.project.clone();
        editor.analysis_areas_result(id, Ok(Some(approved)));
        assert!(
            editor.analysis_review.is_none() && editor.analysis_pending.is_none(),
            "change {changed}"
        );
        assert_eq!(editor.document.project, before);
        assert!(receiver.try_recv().is_err());
    }
}
#[test]
fn stale_review_cannot_authorize_analyze() {
    for changed in 0..9 {
        let (mut editor, _send, receiver) = ready_state();
        editor.request_analysis();
        prepared(&mut editor, &receiver);
        change(&mut editor, changed);
        let before = editor.document.project.clone();
        editor.confirm_analysis_review();
        assert!(editor.analysis_pending.is_none(), "change {changed}");
        assert_eq!(editor.document.project, before);
        assert!(
            !receiver
                .try_iter()
                .any(|c| matches!(c, Command::Analyze(..)))
        );
    }
}
#[test]
fn preparation_cancellation_and_late_or_failed_replies_preserve_grade_and_areas() {
    for fail in [false, true] {
        let (mut editor, _send, receiver) = ready_state();
        editor.sync_document();
        let before = editor.document.project.clone();
        editor.request_analysis();
        let (id, approved) = reply(&mut editor, &receiver);
        editor.analysis_areas_result(id + 1, Ok(Some(approved.clone())));
        assert!(editor.analysis_preparing.is_some());
        if fail {
            editor.analysis_areas_result(id, Err("bad capture".into()));
        } else {
            editor.cancel_analysis();
            assert!(editor.busy);
            editor.analysis_areas_result(id, Ok(Some(approved)));
        }
        assert!(editor.analysis_review.is_none() && editor.analysis_preparing.is_none());
        assert_eq!(editor.document.project, before);
        assert!(!editor.busy);
    }
}
#[test]
fn per_photo_edits_are_drafts_until_analysis_success_and_do_not_copy_to_peers() {
    let (mut editor, _send, receiver) = ready_state();
    let mut other = editor.document.project.sources[0].clone();
    other.id = SourceId(0);
    other.path = "other.ARW".into();
    let other = editor.document.project.add_source(other).unwrap();
    editor.request_analysis();
    prepared(&mut editor, &receiver);
    let before = editor.document.project.clone();
    let original = editor.analysis_review.as_ref().unwrap().approved.entries[1]
        .areas
        .clone();
    editor.region = Some(Coverage::rectangle(rect([40, 30, 700, 500])));
    editor.stash_review_region();
    assert!(
        editor
            .review_areas(editor.current_source.unwrap())
            .unwrap()
            .opening_edited
    );
    assert_eq!(editor.review_areas(other).unwrap(), &original);
    editor.confirm_analysis_review();
    let Command::Analyze(id, request) = receiver.try_recv().unwrap() else {
        panic!()
    };
    assert!(request.areas.unwrap().entries[0].areas.opening_edited);
    assert_eq!(editor.document.project, before);
    editor.analysis_result(id, Err("rejected by unchanged numerical gates".into()));
    assert_eq!(editor.document.project, before);
    assert!(editor.analysis_review.is_none() && editor.pick.is_none());
}
#[test]
fn changed_file_while_a_completed_native_reply_waits_cannot_promote_its_candidate() {
    let (mut editor, _send, receiver) = ready_state();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.ARW");
    std::fs::write(&path, b"first snapshot").unwrap();
    editor.document.project.sources[0].path = path.clone();
    editor.request_analysis();
    prepared(&mut editor, &receiver);
    let before = editor.document.project.clone();
    editor.confirm_analysis_review();
    let id = editor.analysis_pending.as_ref().unwrap().id;
    let candidate = outcome(&editor);
    std::fs::write(path, b"different-length revision while reply waits").unwrap();
    editor.analysis_result(id, Ok(Some(Box::new(candidate))));
    assert_eq!(editor.document.project, before);
    assert!(editor.analysis_review.is_none());
    assert!(editor.status.contains("source changed"));
}
#[test]
fn missing_or_revised_sources_cannot_start_analyze_even_after_area_confirmation() {
    let (mut editor, _send, receiver) = ready_state();
    editor.request_analysis();
    prepared(&mut editor, &receiver);
    editor.analysis_review.as_mut().unwrap().approved.entries[0]
        .areas
        .picture = None;
    editor.confirm_analysis_review();
    assert!(editor.analysis_pending.is_none() && receiver.try_recv().is_err());
    assert!(editor.status.contains("missing"));
    editor.finish_pick();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.ARW");
    std::fs::write(&path, b"first snapshot").unwrap();
    editor.document.project.sources[0].path = path.clone();
    editor.request_analysis();
    prepared(&mut editor, &receiver);
    std::fs::write(path, b"different-length snapshot").unwrap();
    editor.confirm_analysis_review();
    assert!(editor.analysis_pending.is_none() && receiver.try_recv().is_err());
    assert!(editor.status.contains("source changed"));
}
#[test]
fn review_reveals_roll_controls_without_persisting_temporary_scope_visibility_and_restores_context()
{
    for scopes_visible in [false, true] {
        for exit in 0..5 {
            let (mut editor, _send, receiver) = ready_state();
            editor.tab = 1;
            editor.scopes.visible = scopes_visible;
            editor.preferences.scopes_visible = scopes_visible;
            editor.request_analysis();
            prepared(&mut editor, &receiver);
            assert_eq!(editor.tab, 0);
            assert!(!editor.scopes.visible);
            let ctx = egui::Context::default();
            super::super::super::tests::app_frame(&mut editor, &ctx, vec![]);
            assert_eq!(editor.preferences.scopes_visible, scopes_visible);
            if exit == 0 {
                editor.finish_pick();
            } else {
                editor.confirm_analysis_review();
                let id = editor.analysis_pending.as_ref().unwrap().id;
                match exit {
                    1 => editor.analysis_result(id, Err("numerical rejection".into())),
                    2 => editor.analysis_result(id, Ok(None)),
                    3 => {
                        let candidate = outcome(&editor);
                        editor.analysis_result(id, Ok(Some(Box::new(candidate))));
                        assert!(editor.status.starts_with("Analyzed"), "{}", editor.status);
                    }
                    _ => {
                        editor.analysis_generation.fetch_add(1, Ordering::Relaxed);
                        editor.analysis_result(id, Ok(None));
                    }
                }
            }
            assert!(editor.analysis_review.is_none());
            assert_eq!(editor.tab, 1);
            assert_eq!(editor.scopes.visible, scopes_visible);
            assert_eq!(editor.preferences.scopes_visible, scopes_visible);
        }
    }
}
#[test]
fn valid_sampling_exceptions_have_no_inspect_badge_or_extra_photo_warning() {
    let (mut editor, _send, receiver) = ready_state();
    editor.request_analysis();
    prepared(&mut editor, &receiver);
    let areas = &mut editor.analysis_review.as_mut().unwrap().approved.entries[0].areas;
    areas.opening_shared = true;
    areas.opening_exception = true;
    assert!(areas.ready() && areas.needs_inspection());
    let id = editor.current_source.unwrap();
    assert!(editor.source_tool_warning(id).is_none());
    let context = egui::Context::default();
    let labels = super::super::super::tests::app_frame(&mut editor, &context, vec![]);
    assert!(
        !labels
            .iter()
            .any(|(label, _)| label == "Inspect" || label == "Need area")
    );
}
#[test]
fn composed_review_shows_reset_consequence_and_has_one_analyze_or_cancel_action() {
    for accept in [false, true] {
        let (mut editor, _send, receiver) = ready_state();
        editor.photo.pivot = 0.3;
        let ctx = egui::Context::default();
        click_label(&mut editor, &ctx, "Analyze");
        prepared(&mut editor, &receiver);
        let before = editor.document.project.clone();
        let labels = super::super::super::tests::app_frame(&mut editor, &ctx, vec![]);
        assert!(labels.iter().any(|(s, _)| s == "Tool warning"));
        assert!(
            !labels
                .iter()
                .any(|(s, _)| s == "Photo adjustments will be replaced.")
        );
        assert!(!labels.iter().any(|(s, _)| s == "Warning"));
        assert!(
            labels.iter().any(|(s, _)| s == "Opening")
                && labels.iter().any(|(s, _)| s == "Picture")
        );
        assert!(
            !labels
                .iter()
                .any(|(s, _)| s == "Picture statistics" || s == "Roll analysis")
        );
        assert!(
            receiver
                .try_iter()
                .all(|c| !matches!(c, Command::Analyze(..)))
        );
        click_label(
            &mut editor,
            &ctx,
            if accept { "Analyze roll" } else { "Cancel" },
        );
        assert_eq!(editor.document.project, before);
        assert_eq!(
            receiver
                .try_iter()
                .filter(|c| matches!(c, Command::Analyze(..)))
                .count(),
            usize::from(accept)
        );
        if accept {
            assert!(editor.analysis_pending.is_some());
        } else {
            assert!(editor.analysis_review.is_none());
        }
    }
}
#[test]
fn review_browsing_and_cancel_restore_working_photo_without_changing_multi_selection() {
    let (mut editor, _send, receiver) = ready_state();
    let first = editor.current_source.unwrap();
    let mut other = editor.document.project.sources[0].clone();
    other.id = SourceId(0);
    other.path = "other.ARW".into();
    let other = editor.document.project.add_source(other).unwrap();
    editor.select_roll_photo(1, egui::Modifiers::COMMAND);
    assert!(editor.roll_selection.contains(first) && editor.roll_selection.contains(other));
    editor.request_analysis();
    prepared(&mut editor, &receiver);
    editor.browse_analysis_review(other);
    let pending = editor.id;
    assert_eq!(editor.current_source, Some(other));
    assert!(editor.roll_selection.contains(first) && editor.roll_selection.contains(other));
    editor.finish_pick();
    assert_eq!(editor.current_source, Some(first));
    assert_eq!(editor.selected, 0);
    assert!(editor.roll_selection.contains(first) && editor.roll_selection.contains(other));
    assert_ne!(editor.id, pending);
    assert!(editor.analysis_review.is_none() && editor.region.is_none());
}
