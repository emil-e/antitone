use super::*;
mod private;
use crate::{project::SourceAnalysis, scan_regions::Areas};

// Review-only cases install prepared areas; operation cases exercise the preceding workers.
fn approved(request: &Request) -> analysis_review::Approved {
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
    analysis_review::Approved { entries, files }
}

pub(super) fn prepared(editor: &mut Editor, cx: &UiContext<'_, Editor>) {
    let request = editor.analysis_request().expect("eligible review");
    let approved = approved(&request);
    editor.prepared_analysis_review(request, approved, cx);
    assert!(
        editor.document.analysis_review().is_some(),
        "{}",
        editor.status
    );
}

#[test]
fn both_sampling_frames_select_directly_and_resize_without_changing_accepted_areas() {
    let editor = ready_state();
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            prepared(editor, cx);
            let active = editor.document.visit.canvas.ready.as_ref().unwrap().area;
            let opening = Coverage::rectangle(active);
            let picture = Coverage::rectangle(Rect {
                x: active.x + active.width / 4,
                y: active.y + active.height / 4,
                width: active.width / 2,
                height: active.height / 2,
            });
            let areas = &mut editor
                .document
                .analysis_review_mut()
                .unwrap()
                .approved
                .entries[0]
                .areas;
            areas.opening = Some(opening.corners().map(|p| p.0));
            areas.picture = Some(picture.corners().map(|p| p.0));
            editor.refresh_review_region();
            let before = editor.document.state.project.clone();
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
            assert!(editor.document.analysis_review().unwrap().picture);
            let drag = PickDrag::begin(geometry, point(picture, 0), editor.pick_region());
            editor.set_pick_region(Some(
                drag.update(geometry, point(picture, 0) + egui::vec2(5., 5.)),
            ));
            editor.stash_review_region();
            let edited = editor
                .review_areas(editor.document.visit.current_source.unwrap())
                .unwrap();
            assert!(edited.picture_edited);
            assert_eq!(edited.opening, Some(opening.corners().map(|p| p.0)));
            editor.select_review_area(geometry, point(opening, 0));
            assert!(!editor.document.analysis_review().unwrap().picture);
            assert_eq!(editor.pick_region(), Some(opening));
            assert_eq!(editor.document.state.project, before);
            let areas = &mut editor
                .document
                .analysis_review_mut()
                .unwrap()
                .approved
                .entries[0]
                .areas;
            areas.picture = areas.opening;
            assert_eq!(
                editor.review_area_at(geometry, point(opening, 0)),
                Some(false),
                "coincident handles retain the selected area"
            );
            editor.select_review_area(geometry, geometry.image.center());
            assert!(editor.document.analysis_review().unwrap().picture);
            editor.select_review_area(geometry, point(opening, 0) + egui::vec2(8., 8.));
            assert!(
                !editor.document.analysis_review().unwrap().picture,
                "labels disambiguate coincident frames"
            );
        });
    });
}

#[test]
fn drawing_missing_opening_does_not_invent_picture_support_and_last_picture_edit_enables_analyze() {
    let editor = ready_state();
    let ctx = egui::Context::default();
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|e, cx| {
            let request = e.analysis_request().unwrap();
            let mut approved = approved(&request);
            approved.entries[0].areas = Areas::default();
            e.prepared_analysis_review(request, approved, cx);
            e.set_pick_region(Some(Coverage::rectangle(
                e.document.visit.canvas.ready.as_ref().unwrap().area,
            )));
        });
        super::super::super::tests::app_frame(app, &ctx, vec![]);
        app.update(|e| {
            let areas = e
                .review_areas(e.document.visit.current_source.unwrap())
                .unwrap();
            assert!(areas.opening.is_some() && areas.picture.is_none());
            let geometry = e.document.visit.canvas.last_layout.unwrap().0;
            e.select_review_area(geometry, geometry.image.center());
            e.set_pick_region(Some(Coverage::rectangle(
                e.document.visit.canvas.ready.as_ref().unwrap().area,
            )));
        });
        super::super::super::tests::app_frame(app, &ctx, vec![]);
        app.read(|e| {
            assert!(
                e.review_areas(e.document.visit.current_source.unwrap())
                    .unwrap()
                    .ready()
            )
        });
        click_label(app, &ctx, "Analyze roll");
        assert!(app.read(|e| e.analysis_fitting()));
    });
}

#[test]
fn opening_exception_updates_automatic_picture_coverage_but_not_authored_picture_or_peer() {
    let editor = ready_state();
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            let mut other = editor.document.state.project.sources[0].clone();
            other.id = SourceId(0);
            other.path = "other.ARW".into();
            editor.document.state.project.add_source(other).unwrap();
            prepared(editor, cx);
            let active = editor.document.visit.canvas.ready.as_ref().unwrap().area;
            let review = editor.document.analysis_review_mut().unwrap();
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
            let before = editor.document.state.project.clone();
            editor.set_pick_region(Some(Coverage::rectangle(rect([90, 70, 600, 400]))));
            editor.stash_review_region();
            let review = editor.document.analysis_review_mut().unwrap();
            assert_ne!(review.approved.entries[0].areas.picture, original);
            assert_eq!(review.approved.entries[1], peer);
            let authored = Coverage::rectangle(rect([100, 100, 500, 300]));
            review.approved.entries[0].areas.edit_picture(authored);
            editor.set_pick_region(Some(Coverage::rectangle(rect([120, 90, 500, 300]))));
            editor.stash_review_region();
            assert_eq!(
                editor.document.analysis_review().unwrap().approved.entries[0]
                    .areas
                    .picture,
                Some(authored.corners().map(|p| p.0))
            );
            assert_eq!(editor.document.state.project, before);
        });
    });
}

#[test]
fn all_ordered_photo_fields_control_the_warning_in_one_review_not_a_second_dialog() {
    for field in 0..7 {
        let editor = ready_state();
        crate::application::test_support::with_application(editor, |app| {
            app.enter(|editor, cx| {
                let mut other = editor.document.state.project.sources[0].clone();
                other.id = SourceId(0);
                other.path = "other.ARW".into();
                match field {
                    0 => {
                        other.adjustment_layers.as_mut().unwrap().layers[0]
                            .adjustment
                            .photo
                            .exposure = 0.1
                    }
                    1 => {
                        other.adjustment_layers.as_mut().unwrap().layers[0]
                            .adjustment
                            .photo
                            .wb
                            .warmth = 0.1
                    }
                    2 => {
                        other.adjustment_layers.as_mut().unwrap().layers[0]
                            .adjustment
                            .photo
                            .contrast = 1.1
                    }
                    3 => {
                        other.adjustment_layers.as_mut().unwrap().layers[0]
                            .adjustment
                            .photo
                            .pivot = 0.3
                    }
                    4 => {
                        other.adjustment_layers.as_mut().unwrap().layers[0]
                            .adjustment
                            .photo
                            .contrast_colour_mix = 0.4
                    }
                    5 => {
                        other.adjustment_layers.as_mut().unwrap().layers[0]
                            .adjustment
                            .photo
                            .saturation = 1.1
                    }
                    _ => {
                        other.adjustment_layers.as_mut().unwrap().layers[0]
                            .adjustment
                            .photo
                            .saturation_density = 0.4
                    }
                }
                editor.document.state.project.add_source(other).unwrap();
                editor.sync_document();
                let before = editor.document.state.project.clone();
                prepared(editor, cx);
                assert!(
                    editor.document.analysis_review().unwrap().replaces_photos,
                    "field {field}"
                );
                editor.finish_pick(cx);
                assert_eq!(editor.document.state.project, before);
                assert!(!editor.analysis_fitting() && editor.document.analysis_review().is_none());
                assert!(!editor.analysis_preparing());
            });
        });
    }
}

#[test]
fn unavailable_analysis_services_tear_down_review_and_restore_working_photo() {
    crate::application::test_support::with_application(ready_state(), |app| {
        let (before, first, other) = app.enter(|editor, cx| {
            let first = editor.document.visit.current_source;
            let mut other = editor.document.state.project.sources[0].clone();
            other.id = SourceId(0);
            other.path = "other.ARW".into();
            other.adjustment_layers.as_mut().unwrap().layers[0]
                .adjustment
                .photo
                .exposure = 1.7;
            let other = editor.document.state.project.add_source(other).unwrap();
            editor.select_roll_photo(1, egui::Modifiers::COMMAND, cx);
            prepared(editor, cx);
            let before = editor.document.state.project.clone();
            let proxy = editor
                .document
                .visit
                .canvas
                .ready
                .as_ref()
                .unwrap()
                .proxy
                .clone();
            editor.browse_analysis_review(other, cx);
            let record = editor.document.state.project.source(other).unwrap().clone();
            editor.document.visit.canvas.ready = Some(Ready {
                reference: None,
                developed: crate::editor::tests::developed_fixture(),
                file: thumbnails::FileKey::new(&record.path),
                thumbnail: None,
                path: record.path.clone(),
                area: record.development.active_area.into(),
                analysis_support: None,
                proxy,
                record,
            });
            editor.busy = false;
            editor.confirm_analysis_review(cx);
            assert!(editor.analysis_fitting());
            (before, first, other)
        });
        app.run_until(Duration::from_secs(5), |e| !e.analysis_fitting());
        app.read(|editor| {
            assert!(
                !editor.analysis_fitting()
                    && editor.document.analysis_review().is_none()
                    && editor.active_pick().is_none()
            );
            assert_eq!(editor.document.visit.current_source, Some(other));
            assert!(
                editor.document.roll_selection.contains(first.unwrap())
                    && editor.document.roll_selection.contains(other)
            );
            assert_eq!(editor.document.state.project, before);
        });
    });
}

#[test]
fn default_photos_still_require_area_review_but_retired_grades_do_not_trigger_reset_warning() {
    let editor = ready_state();
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            let mut retired = editor.document.state.project.sources[0].clone();
            retired.id = SourceId(0);
            retired.path = "retired.ARW".into();
            retired.adjustment_layers.as_mut().unwrap().layers[0]
                .adjustment
                .photo
                .contrast = 1.3;
            let retired = editor.document.state.project.add_source(retired).unwrap();
            editor
                .document
                .state
                .project
                .order
                .retain(|id| *id != retired);
            editor.document.roll.exposure = 1.;
            editor.document.roll.wb.tint = 0.2;
            prepared(editor, cx);
            assert!(!editor.document.analysis_review().unwrap().replaces_photos);
            assert!(!editor.analysis_fitting());
            editor.confirm_analysis_review(cx);
            assert_eq!(
                editor
                    .document
                    .analysis_review()
                    .unwrap()
                    .approved
                    .entries
                    .len(),
                1
            );
            assert_eq!(
                editor
                    .document
                    .project
                    .source(retired)
                    .unwrap()
                    .adjustment_layers
                    .as_ref()
                    .unwrap()
                    .layers[0]
                    .adjustment
                    .photo
                    .contrast,
                1.3
            );
        });
    });
}

#[test]
fn preparation_cancellation_and_failed_work_preserve_grade_and_areas() {
    for fail in [false, true] {
        crate::application::test_support::with_application(ready_state(), |app| {
            let before = app.enter(|editor, cx| {
                let request = editor.analysis_request().unwrap();
                let approved = approved(&request);
                let reply = analysis_workflow::testing::preparation(editor);
                let before = editor.document.state.project.clone();
                editor.prepare_analysis_review(request, cx);
                if !fail {
                    editor.cancel_analysis();
                    assert!(editor.busy);
                }
                reply
                    .result
                    .send(if fail {
                        Err("bad capture".into())
                    } else {
                        Ok(Some(approved))
                    })
                    .ok()
                    .unwrap();
                before
            });
            app.run_until(Duration::from_secs(10), |editor| {
                editor.document.analysis_operation().is_none()
            });
            app.read(|editor| {
                assert!(editor.document.analysis_review().is_none() && !editor.busy);
                assert_eq!(editor.document.state.project, before);
                assert!(
                    editor
                        .history_label(super::super::super::history::Direction::Undo)
                        .is_none()
                );
            });
        });
    }
}

#[test]
fn per_photo_edits_are_drafts_until_analysis_success_and_do_not_copy_to_peers() {
    crate::application::test_support::with_application(ready_state(), |app| {
        let (before, mut reply) = app.enter(|editor, cx| {
            let mut other = editor.document.state.project.sources[0].clone();
            other.id = SourceId(0);
            other.path = "other.ARW".into();
            let other = editor.document.state.project.add_source(other).unwrap();
            prepared(editor, cx);
            let before = editor.document.state.project.clone();
            let original = editor.document.analysis_review().unwrap().approved.entries[1]
                .areas
                .clone();
            editor.set_pick_region(Some(Coverage::rectangle(rect([40, 30, 700, 500]))));
            editor.stash_review_region();
            assert!(
                editor
                    .review_areas(editor.document.visit.current_source.unwrap())
                    .unwrap()
                    .opening_edited
            );
            assert_eq!(editor.review_areas(other).unwrap(), &original);
            let reply = analysis_workflow::testing::fitting(editor);
            editor.confirm_analysis_review(cx);
            assert_eq!(editor.document.state.project, before);
            (before, reply)
        });
        let request = analysis_workflow::testing::request(app, &mut reply);
        assert!(
            request.areas.as_ref().unwrap().entries[0]
                .areas
                .opening_edited
        );
        reply
            .result
            .send(Err("rejected by unchanged numerical gates".into()))
            .ok()
            .unwrap();
        app.run_until(Duration::from_secs(10), |editor| {
            editor.document.analysis_operation().is_none()
        });
        app.read(|editor| {
            assert_eq!(editor.document.state.project, before);
            assert!(editor.document.analysis_review().is_none() && editor.active_pick().is_none());
        });
    });
}

#[test]
fn missing_reviewed_picture_cannot_start_fitting() {
    let editor = ready_state();
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            prepared(editor, cx);
            editor
                .document
                .analysis_review_mut()
                .unwrap()
                .approved
                .entries[0]
                .areas
                .picture = None;
            editor.confirm_analysis_review(cx);
            assert!(!editor.analysis_fitting());
            assert!(editor.status.contains("missing"));
            editor.finish_pick(cx);
        });
    });
}

#[test]
fn roll_review_controls_restore_scope_visibility_and_context() {
    for scopes_visible in [false, true] {
        for exit in 0..5 {
            crate::application::test_support::with_application(ready_state(), |app| {
                let reply = app.enter(|editor, cx| {
                    editor.tab = 1;
                    editor.scopes.visible = scopes_visible;
                    editor.preferences.scopes_visible = scopes_visible;
                    prepared(editor, cx);
                    assert_eq!(editor.tab, 0);
                    assert!(!editor.scopes.visible);
                    let ctx = egui::Context::default();
                    super::super::super::tests::app_frame(editor, &ctx, vec![]);
                    assert_eq!(editor.preferences.scopes_visible, scopes_visible);
                    if exit == 0 {
                        editor.finish_pick(cx);
                        None
                    } else {
                        let reply = analysis_workflow::testing::fitting(editor);
                        editor.confirm_analysis_review(cx);
                        if exit == 4 {
                            editor.cancel_analysis();
                        }
                        Some(reply)
                    }
                });
                if let Some(mut reply) = reply {
                    let request = analysis_workflow::testing::request(app, &mut reply);
                    let result = match exit {
                        1 => Err("numerical rejection".into()),
                        3 => Ok(Some(Box::new(outcome(&request)))),
                        _ => Ok(None),
                    };
                    reply.result.send(result).ok().unwrap();
                    app.run_until(Duration::from_secs(10), |editor| {
                        editor.document.analysis_operation().is_none()
                    });
                    if exit == 3 {
                        app.read(|editor| {
                            assert!(editor.status.starts_with("Analyzed"), "{}", editor.status)
                        });
                    }
                }
                app.read(|editor| {
                    assert!(editor.document.analysis_review().is_none());
                    assert_eq!(editor.tab, 0);
                    assert_eq!(editor.scopes.visible, scopes_visible);
                    assert_eq!(editor.preferences.scopes_visible, scopes_visible);
                });
            });
        }
    }
}

#[test]
fn valid_sampling_exceptions_have_no_inspect_badge_or_extra_photo_warning() {
    let editor = ready_state();
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            prepared(editor, cx);
            let areas = &mut editor
                .document
                .analysis_review_mut()
                .unwrap()
                .approved
                .entries[0]
                .areas;
            areas.opening_shared = true;
            areas.opening_exception = true;
            assert!(areas.ready() && areas.needs_inspection());
            let id = editor.document.visit.current_source.unwrap();
            assert!(editor.source_tool_warning(id).is_none());
            let context = egui::Context::default();
            let labels = super::super::super::tests::app_frame(editor, &context, vec![]);
            assert!(
                !labels
                    .iter()
                    .any(|(label, _)| label == "Inspect" || label == "Need area")
            );
        });
    });
}

#[test]
fn composed_review_shows_reset_consequence_and_has_one_analyze_or_cancel_action() {
    for accept in [false, true] {
        let mut editor = ready_state();
        editor.document.visit.adjustment_layer_settings.pivot = 0.3;
        let ctx = egui::Context::default();
        crate::application::test_support::with_application(editor, |app| {
            app.enter(prepared);
            let before = app.read(|e| e.document.state.project.clone());
            let labels = super::super::super::tests::app_frame(app, &ctx, vec![]);
            assert!(labels.iter().any(|(s, _)| s == "Tool warning"));
            assert!(
                !labels
                    .iter()
                    .any(|(s, _)| s == "Photo layers will be replaced.")
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
            assert!(app.read(|e| !e.analysis_fitting()));
            click_label(app, &ctx, if accept { "Analyze roll" } else { "Cancel" });
            app.read(|e| {
                assert_eq!(e.document.state.project, before);
                assert_eq!(e.analysis_fitting(), accept);
                if !accept {
                    assert!(e.document.analysis_review().is_none());
                }
            });
        });
    }
}

#[test]
fn review_browsing_and_cancel_keep_the_displayed_photo_without_changing_multi_selection() {
    let editor = ready_state();
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            let first = editor.document.visit.current_source.unwrap();
            let mut other = editor.document.state.project.sources[0].clone();
            other.id = SourceId(0);
            other.path = "other.ARW".into();
            let other = editor.document.state.project.add_source(other).unwrap();
            editor.select_roll_photo(1, egui::Modifiers::COMMAND, cx);
            assert!(
                editor.document.roll_selection.contains(first)
                    && editor.document.roll_selection.contains(other)
            );
            prepared(editor, cx);
            editor.browse_analysis_review(other, cx);
            let pending = editor.id;
            assert_eq!(editor.document.visit.current_source, Some(other));
            assert!(
                editor.document.roll_selection.contains(first)
                    && editor.document.roll_selection.contains(other)
            );
            editor.finish_pick(cx);
            assert_eq!(editor.document.visit.current_source, Some(other));
            assert_eq!(editor.document.selected, 1);
            assert!(
                editor.document.roll_selection.contains(first)
                    && editor.document.roll_selection.contains(other)
            );
            assert_eq!(editor.id, pending);
            assert!(editor.document.analysis_review().is_none() && editor.pick_region().is_none());
        });
    });
}
