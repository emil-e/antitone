use super::*;
use crate::dust_repair::SearchOutcome;

fn request(e: &mut Editor) -> Request {
    let source = e.document.visit.current_source.unwrap();
    Request {
        request: e.request_id(),
        loaded: e.id,
        source,
        signature: e.dust_signature().unwrap(),
        repair: e.effective_dust_repair(source, RepairId(1)).unwrap(),
        input: e.input(),
        cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    }
}

fn chosen_response(request: &Request, active: SourceRegion, offset: [i32; 2]) -> Response {
    let mut chosen = request.clone();
    chosen.repair.offset = offset;
    response(&chosen, active)
}

fn layout(e: &mut Editor) -> crate::viewport::CanvasLayout {
    e.document.visit.canvas.viewport.layout_composition(
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(864., 576.)),
        Composition::new(
            e.document.visit.canvas.ready.as_ref().unwrap().area,
            Default::default(),
            Default::default(),
        )
        .unwrap(),
        1.,
    )
}

#[test]
fn held_drag_completion_is_preview_only_and_release_records_the_accepted_edit() {
    let mut e = editor();
    let source = e.document.visit.current_source.unwrap();
    e.begin_dust_cleanup();
    let original = e.dust_repairs(source).unwrap().clone();
    let geometry = layout(&mut e);
    e.begin_dust_donor_drag(geometry, screen_source_point(geometry, [81., 51.]));
    e.drag_dust_donor(geometry, screen_source_point(geometry, [91., 56.]));
    let request = request(&mut e);
    assert_eq!(request.repair.offset, [40, 5]);
    e.document.visit.dust.start(&request);
    let active = e.document.visit.canvas.ready.as_ref().unwrap().area;
    e.receive_dust(request.request, Ok(Some(response(&request, active))));
    assert_eq!(e.dust_repairs(source), Some(&original));
    assert!(
        e.document
            .dust
            .preview_cached
            .contains_key(&(source, RepairId(1)))
    );
    e.end_dust_donor_drag();
    assert_eq!(e.dust_repairs(source).unwrap().repairs()[0].offset, [40, 5]);
    assert_eq!(
        e.history_label(history::Direction::Undo),
        Some("Dust donor")
    );
    assert!(e.document.dust.cached.contains_key(&(source, RepairId(1))));
}

#[test]
fn obsolete_success_cannot_modify_new_support_or_offset_or_steal_a_new_request() {
    let mut e = editor();
    let first = request(&mut e);
    let active = e.document.visit.canvas.ready.as_ref().unwrap().area;
    let stale = chosen_response(&first, active, [60, 0]);
    e.document.visit.dust.start(&first);
    e.dust_repairs_mut(first.source)
        .set_support(
            first.repair.id,
            SupportMask::new([60, 60], [3, 3], vec![true; 9]).unwrap(),
        )
        .unwrap();
    e.dust_repairs_mut(first.source)
        .set_offset(first.repair.id, [20, 0])
        .unwrap();
    let expected = e.dust_repairs(first.source).unwrap().clone();
    let second = request(&mut e);
    e.document.visit.dust.start(&second);
    e.receive_dust(first.request, Ok(Some(stale)));
    assert_eq!(e.dust_repairs(first.source), Some(&expected));
    assert_eq!(
        e.document.visit.dust.pending_request(),
        Some(second.request)
    );
    assert!(e.document.dust.cached.is_empty());
}

#[test]
fn ordinary_evaluation_cannot_propose_a_different_offset() {
    let mut e = editor();
    let request = request(&mut e);
    let active = e.document.visit.canvas.ready.as_ref().unwrap().area;
    e.document.visit.dust.start(&request);
    e.receive_dust(
        request.request,
        Ok(Some(chosen_response(&request, active, [60, 0]))),
    );
    assert_eq!(
        e.dust_repairs(request.source).unwrap().repairs()[0],
        request.repair
    );
    assert!(e.document.dust.cached.is_empty());
}

use super::super::worker::testing::Gate;
use crate::application::test_support::{TestApplication, with_application};

fn queue(
    e: &mut Editor,
    origin: [u32; 2],
    cx: &UiContext<'_, Editor>,
) -> super::super::search::PendingStroke {
    e.begin_dust_mask(None);
    e.document.dust_mask_draft_mut().unwrap().pixels = (origin[1]..origin[1] + 3)
        .flat_map(|y| (origin[0]..origin[0] + 3).map(move |x| [x, y]))
        .collect();
    e.commit_dust_mask(cx);
    e.pending_dust_strokes().last().unwrap().clone()
}

fn with_search(e: Editor, test: impl FnOnce(&mut TestApplication<'_, Editor>, &Gate)) {
    let mut e = e;
    let gate = Gate::install(&mut e);
    with_application(e, |app| test(app, &gate));
}

fn settled(app: &mut TestApplication<'_, Editor>) {
    app.run_until(Duration::from_secs(5), |e| !e.dust_search_pending());
}

#[test]
fn pending_stroke_has_no_donor_or_saved_instruction_then_accepts_only_the_search_result() {
    with_search(editor(), |app, gate| {
        let before = app.read(|e| (e.document.state.project.clone(), e.document.dirty()));
        let stroke = app.enter(|e, cx| {
            e.begin_dust_cleanup();
            let stroke = queue(e, [80, 80], cx);
            assert_eq!(e.document.state.project, before.0);
            assert_eq!(e.document.dirty(), before.1);
            assert!(e.dust_search_pending());
            assert!(e.dust_donor_regions().is_empty());
            assert!(!e.file_available());
            assert!(!e.history_available());
            assert_eq!(e.history_label(history::Direction::Undo), None);
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("pending.antitone");
            e.save_project(path.clone(), crate::project::Save::New, None, cx);
            assert!(!path.exists());
            assert!(e.document_pending.is_none());
            e.open_export_dialog();
            assert!(e.document.export.draft.is_none());
            stroke
        });
        let call = gate.next(app);
        assert_eq!(call.input.support, stroke.support);
        call.complete(Ok(SearchOutcome::Candidate([20, 0])));
        settled(app);
        app.read(|e| {
            let stored = e
                .dust_repairs(stroke.source)
                .unwrap()
                .repairs()
                .last()
                .unwrap();
            assert_eq!(stored.support, stroke.support);
            assert_eq!(stored.offset, [20, 0]);
            let region = &e.dust_donor_regions()[0];
            assert_eq!(region.donor.x as i32 - region.target.x as i32, 20);
            assert!(
                e.document.dust.cached.is_empty(),
                "publication must not wait for healing"
            );
            assert_eq!(e.history_label(history::Direction::Undo), Some("Dust mask"));
        });
    });
}

#[test]
fn failed_heal_keeps_the_completed_search_donor_draggable() {
    with_search(editor(), |app, gate| {
        let stroke = app.enter(|e, cx| queue(e, [80, 80], cx));
        gate.next(app)
            .complete(Ok(SearchOutcome::Candidate([30, 0])));
        settled(app);
        app.update(|e| {
            let stored = e
                .dust_repairs(stroke.source)
                .unwrap()
                .repairs()
                .last()
                .unwrap()
                .clone();
            let mut evaluation = request(e);
            evaluation.repair = stored.clone();
            e.document.visit.dust.start(&evaluation);
            e.receive_dust(evaluation.request, Err("authored healing failure".into()));
            let geometry = layout(e);
            assert!(e.dust_donor_at(geometry, screen_source_point(geometry, [111., 81.])));
            e.begin_dust_donor_drag(geometry, screen_source_point(geometry, [111., 81.]));
            e.drag_dust_donor(geometry, screen_source_point(geometry, [121., 86.]));
            e.end_dust_donor_drag();
            assert_eq!(
                e.dust_repairs(stroke.source)
                    .unwrap()
                    .repairs()
                    .last()
                    .unwrap()
                    .offset,
                [40, 5]
            );
            assert!(
                !e.document
                    .dust
                    .failed
                    .contains_key(&(stroke.source, stored.id))
            );
        });
    });
}

fn change_support(e: &mut Editor, cx: &UiContext<'_, Editor>) {
    e.begin_dust_mask(Some(RepairId(1)));
    e.document
        .dust_mask_draft_mut()
        .unwrap()
        .pixels
        .insert([53, 50]);
    e.commit_dust_mask(cx);
}

#[test]
fn changed_support_keeps_the_previous_instruction_until_search_finishes() {
    with_search(editor(), |app, gate| {
        let (source, previous) = app.enter(|e, cx| {
            e.begin_dust_cleanup();
            let source = e.document.visit.current_source.unwrap();
            let previous = e.dust_repairs(source).unwrap().repairs()[0].clone();
            change_support(e, cx);
            assert_eq!(e.dust_repairs(source).unwrap().repairs()[0], previous);
            assert!(e.dust_donor_regions().is_empty());
            (source, previous)
        });
        let call = gate.next(app);
        assert_eq!(call.input.previous_offset, Some(previous.offset));
        call.complete(Ok(SearchOutcome::Candidate([40, 0])));
        settled(app);
        app.read(|e| {
            let changed = &e.dust_repairs(source).unwrap().repairs()[0];
            assert!(changed.support.contains([53, 50]));
            assert_eq!(changed.offset, [40, 0]);
            assert_eq!(changed.id, previous.id);
        });
    });
}

#[test]
fn starting_another_edit_cannot_discard_a_released_pending_mask() {
    with_search(editor(), |app, gate| {
        let source = app.enter(|e, cx| {
            change_support(e, cx);
            e.begin_dust_mask(Some(RepairId(1)));
            assert!(e.document.dust_mask_draft().is_none());
            e.cancel_dust_mask();
            assert!(e.dust_search_pending());
            e.document.visit.current_source.unwrap()
        });
        let call = gate.next(app);
        assert!(!call.cancelled.is_cancelled());
        call.complete(Ok(SearchOutcome::Candidate([30, 0])));
        settled(app);
        app.read(|e| {
            assert!(
                e.dust_repairs(source).unwrap().repairs()[0]
                    .support
                    .contains([53, 50])
            )
        });
    });
}

#[test]
fn deleting_a_repair_retires_its_unfinished_mask_change() {
    with_search(editor(), |app, gate| {
        let source = app.enter(|e, cx| {
            change_support(e, cx);
            e.document.visit.current_source.unwrap()
        });
        let call = gate.next(app);
        let before = app.update(|e| {
            e.delete_selected_dust_repair();
            assert!(!e.dust_search_pending());
            assert!(e.dust_repairs(source).is_none());
            e.history_snapshot()
        });
        app.run_until(Duration::from_secs(5), |_| call.cancelled.is_cancelled());
        call.complete(Ok(SearchOutcome::Candidate([30, 0])));
        app.pump();
        app.read(|e| assert_eq!(e.document.state.project, before));
    });
}

#[test]
fn reset_cancels_pending_search_and_ignores_its_late_success() {
    with_search(editor(), |app, gate| {
        let stroke = app.enter(|e, cx| queue(e, [80, 80], cx));
        let call = gate.next(app);
        let before = app.update(|e| {
            e.reset_dust_repairs(stroke.source);
            assert!(!e.dust_search_pending());
            e.history_snapshot()
        });
        app.run_until(Duration::from_secs(5), |_| call.cancelled.is_cancelled());
        call.complete(Ok(SearchOutcome::Candidate([20, 0])));
        app.pump();
        app.read(|e| {
            assert_eq!(e.document.state.project, before);
            assert!(e.dust_repairs(stroke.source).is_none());
        });
    });
}

#[test]
fn retiring_the_document_cancels_its_search_without_publishing_into_the_replacement() {
    with_search(editor(), |app, gate| {
        app.enter(|e, cx| {
            queue(e, [80, 80], cx);
        });
        let call = gate.next(app);
        app.update(|e| e.document = e.document.replacement(Document::default()));
        app.run_until(Duration::from_secs(5), |_| call.cancelled.is_cancelled());
        call.complete(Ok(SearchOutcome::Candidate([20, 0])));
        app.pump();
        app.read(|e| {
            assert!(!e.dust_search_pending());
            assert!(e.document.project.sources.is_empty());
            assert_eq!(e.history_label(history::Direction::Undo), None);
        });
    });
}

#[test]
fn retirement_before_the_first_poll_never_dispatches_a_search() {
    with_search(editor(), |app, gate| {
        app.enter(|e, cx| {
            let stroke = queue(e, [80, 80], cx);
            e.reset_dust_repairs(stroke.source);
        });
        app.pump();
        assert!(gate.is_empty());
        app.read(|e| assert!(!e.dust_search_pending()));
    });
}

#[test]
fn stopped_execution_leaves_no_pending_stroke_or_accepted_edit() {
    with_search(editor(), |app, gate| {
        let before = app.read(|e| e.document.state.project.clone());
        app.close_execution();
        app.enter(|e, cx| {
            e.begin_dust_mask(None);
            e.document
                .dust_mask_draft_mut()
                .unwrap()
                .pixels
                .insert([80, 80]);
            e.commit_dust_mask(cx);
            assert!(!e.dust_search_pending());
            assert_eq!(e.document.state.project, before);
            assert!(e.status.contains("stopped"));
        });
        assert!(gate.is_empty());
    });
}

#[test]
fn failed_cancelled_impossible_and_invalid_choices_leave_no_provisional_repair_or_history() {
    let outcomes = [
        Err("authored search failure".into()),
        Ok(SearchOutcome::Cancelled),
        Ok(SearchOutcome::Impossible),
        Ok(SearchOutcome::Candidate([0, 0])),
        Ok(SearchOutcome::Candidate([i32::MAX, 0])),
    ];
    for outcome in outcomes {
        with_search(editor(), |app, gate| {
            let before = app.read(|e| e.document.state.project.clone());
            app.enter(|e, cx| {
                queue(e, [80, 80], cx);
            });
            gate.next(app).complete(outcome);
            settled(app);
            app.read(|e| {
                assert_eq!(e.document.state.project, before);
                assert_eq!(e.history_label(history::Direction::Undo), None);
            });
        });
    }
}

#[test]
fn impossible_stroke_is_rejected_whole_without_changing_existing_repairs() {
    with_search(editor(), |app, gate| {
        let before = app.enter(|e, cx| {
            let source = e.document.visit.current_source.unwrap();
            let active = SourceRegion {
                x: 12,
                y: 7,
                width: 96,
                height: 96,
            };
            e.document.visit.canvas.ready.as_mut().unwrap().area = active;
            e.document
                .project
                .source_mut(source)
                .unwrap()
                .development
                .active_area = [12, 7, 96, 96];
            // Settle the existing repair so unrelated healing does not replace rejection feedback.
            let evaluation = request(e);
            e.document.visit.dust.start(&evaluation);
            e.receive_dust(evaluation.request, Ok(Some(response(&evaluation, active))));
            let before = e.history_snapshot();
            e.begin_dust_mask(None);
            e.document.dust_mask_draft_mut().unwrap().pixels = (active.y..active.y + active.height)
                .flat_map(|y| (active.x..active.x + active.width).map(move |x| [x, y]))
                .collect();
            e.commit_dust_mask(cx);
            assert_eq!(e.document.state.project, before);
            before
        });
        gate.next(app).complete(Ok(SearchOutcome::Impossible));
        settled(app);
        app.read(|e| {
            assert_eq!(e.document.state.project, before);
            assert!(e.status.contains("no clean donor"), "{}", e.status);
            assert_eq!(e.history_label(history::Direction::Undo), None);
        });
    });
}

#[test]
fn pending_strokes_complete_independently_without_overriding_manual_selection() {
    with_search(editor(), |app, gate| {
        let a = app.enter(|e, cx| queue(e, [80, 80], cx));
        let first = gate.next(app);
        let b = app.enter(|e, cx| queue(e, [100, 100], cx));
        let second = gate.next(app);
        app.update(|e| e.select_dust_repair(RepairId(1)));
        second.complete(Ok(SearchOutcome::Candidate([20, 0])));
        app.run_until(Duration::from_secs(5), |e| {
            e.pending_dust_strokes().len() == 1
        });
        first.complete(Ok(SearchOutcome::Candidate([20, 0])));
        settled(app);
        app.read(|e| {
            assert_eq!(e.selected_dust_repair(), Some(RepairId(1)));
            let repairs = e.dust_repairs(a.source).unwrap().repairs();
            assert_eq!(repairs.len(), 3);
            assert!(repairs.iter().any(|r| r.support == a.support));
            assert!(repairs.iter().any(|r| r.support == b.support));
        });
    });
}

#[test]
fn older_search_can_accept_its_repair_but_cannot_steal_selection_from_a_newer_stroke() {
    with_search(editor(), |app, gate| {
        let a = app.enter(|e, cx| queue(e, [80, 80], cx));
        let first = gate.next(app);
        let b = app.enter(|e, cx| queue(e, [100, 100], cx));
        let second = gate.next(app);
        second.complete(Ok(SearchOutcome::Candidate([20, 0])));
        app.run_until(Duration::from_secs(5), |e| {
            e.pending_dust_strokes().len() == 1
        });
        let selected = app.read(Editor::selected_dust_repair);
        first.complete(Ok(SearchOutcome::Candidate([20, 0])));
        settled(app);
        app.read(|e| {
            let repairs = e.dust_repairs(a.source).unwrap().repairs();
            assert!(repairs.iter().any(|r| r.support == a.support));
            assert_eq!(e.selected_dust_repair(), selected);
            assert_eq!(
                repairs
                    .iter()
                    .find(|r| Some(r.id) == selected)
                    .unwrap()
                    .support,
                b.support
            );
        });
    });
}

#[test]
fn ending_dust_discards_unfinished_strokes_without_changing_accepted_repairs() {
    with_search(editor(), |app, gate| {
        let before = app.read(|e| e.document.state.project.clone());
        app.enter(|e, cx| {
            queue(e, [80, 80], cx);
        });
        let call = gate.next(app);
        app.update(|e| {
            e.finish_dust_cleanup();
            assert!(!e.dust_search_pending());
            assert!(e.pending_dust_strokes().is_empty());
            assert!(!e.document.dust().is_some());
        });
        app.run_until(Duration::from_secs(5), |_| call.cancelled.is_cancelled());
        call.complete(Ok(SearchOutcome::Candidate([30, 0])));
        app.pump();
        app.read(|e| {
            assert_eq!(e.document.state.project, before);
            assert_eq!(e.history_label(history::Direction::Undo), None);
        });
    });
}

fn coherent_editor() -> Editor {
    let mut e = super::super::super::tests::authored_fixture(&["patches.dng"]);
    e.set_dmin(Some(DVec3::splat(0.8)));
    e.set_dmax(Some(DVec3::splat(0.02)));
    e.document.source = false;
    e
}

#[test]
fn real_evaluation_workflow_continues_after_a_failed_repair_without_another_paint() {
    let mut e = coherent_editor();
    let source = e.document.visit.current_source.unwrap();
    let active = e.document.visit.canvas.ready.as_ref().unwrap().area;
    e.dust_repairs_mut(source)
        .insert(
            SupportMask::new([active.x, active.y], [3, 3], vec![true; 9]).unwrap(),
            RepairOrigin::Manual,
            [30, 0],
        )
        .unwrap();
    let good = e
        .dust_repairs_mut(source)
        .insert(
            SupportMask::new([80, 80], [3, 3], vec![true; 9]).unwrap(),
            RepairOrigin::Manual,
            [30, 0],
        )
        .unwrap();
    with_application(e, |app| {
        app.enter(|e, cx| e.schedule_dust(e.dust_signature().unwrap(), cx));
        app.run_until(Duration::from_secs(10), |e| {
            e.document.dust.has_evaluation(source, good)
        });
        app.read(|e| {
            assert!(e.document.dust.failed.contains_key(&(source, RepairId(1))));
            assert!(e.document.visit.dust.pending.is_none());
        });
    });
}

#[test]
fn repair_evaluation_chain_keeps_photo_pixels_and_correction_while_reference_view_is_active() {
    let mut e = coherent_editor();
    let source = e.document.visit.current_source.unwrap();
    let record = e.document.project.source(source).unwrap();
    let area = record.development.active_area;
    let field = Arc::new(
        crate::flat_field::FlatField::from_nodes(area, [2, 2], vec![DVec3::splat(1.2); 4]).unwrap(),
    );
    let settings = crate::project::FlatFieldSettings::new(record, field);
    e.document.project.set_flat_field(Some(settings));
    e.document.correction = crate::editor::Correction::prepare(
        &e.source_services.as_ref().unwrap().device,
        &e.document.capture(),
        None,
    )
    .unwrap();
    let photo = e.source_context().unwrap();
    assert!(photo.correction.is_some());
    let signature = e.dust_signature().unwrap();
    let mut reference_editor = super::super::super::tests::authored_fixture(&["bare-light.dng"]);
    let reference = reference_editor.document.visit.canvas.ready.take().unwrap();
    let reference_scan =
        Scan::with_reference(reference.developed.clone(), reference.reference.clone());
    let repairs = [[50, 50], [100, 80]].map(|origin| {
        let id = e
            .dust_repairs_mut(source)
            .insert(
                SupportMask::new(origin, [3, 3], vec![true; 9]).unwrap(),
                RepairOrigin::Manual,
                [30, 0],
            )
            .unwrap();
        let repair = e
            .dust_repairs(source)
            .unwrap()
            .repairs()
            .iter()
            .find(|r| r.id == id)
            .unwrap()
            .clone();
        let request = Request {
            request: 1,
            loaded: e.id,
            source,
            signature,
            repair,
            input: e.input(),
            cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };
        let expected = evaluate(&photo.scan().unwrap(), &request)
            .unwrap()
            .evaluated;
        let wrong_source = evaluate(&reference_scan, &request).unwrap().evaluated;
        let uncorrected = Scan::with_reference(photo.source.clone(), photo.reference.clone());
        let wrong_correction = evaluate(&uncorrected, &request).unwrap().evaluated;
        let values = |r: &EvaluatedRepair| r.pixels().collect::<Vec<_>>();
        assert_ne!(values(&expected), values(&wrong_source));
        assert_ne!(values(&expected), values(&wrong_correction));
        (id, expected)
    });
    let hold = crate::background::testing::pause();
    with_application(e, |app| {
        app.enter(|e, cx| e.schedule_dust(signature, cx));
        app.pump();
        app.read(|e| assert!(e.document.visit.dust.pending.is_some()));
        // Only install prepared reference UI; both repair evaluations use the real continuation.
        app.update(|e| {
            e.finish_dust_cleanup();
            super::super::super::tests::paused_reference_fixture(e, reference, None);
        });
        let (working_revision, reference_revision) = app.read(|e| {
            assert!(e.canvas_source_context().unwrap().correction.is_none());
            (
                e.document.visit.canvas.grade.original.revision(),
                e.active_canvas().grade.original.revision(),
            )
        });
        drop(hold);
        app.run_until(Duration::from_secs(15), |e| {
            repairs
                .iter()
                .all(|(id, _)| e.document.dust.has_evaluation(source, *id))
        });
        app.read(|e| {
            assert_eq!(
                e.active_canvas().grade.original.revision(),
                reference_revision
            );
            assert!(e.document.visit.canvas.grade.original.revision() > working_revision);
        });
        app.update(Editor::finish_flat_edit);
        app.read(|e| {
            assert!(e.document.visit.dust.pending.is_none());
            for (id, expected) in repairs {
                let cached = e.document.dust.cached.get(&(source, id)).unwrap();
                assert_eq!(
                    cached.evaluated.pixels().collect::<Vec<_>>(),
                    expected.pixels().collect::<Vec<_>>()
                );
            }
        });
    });
}

#[test]
fn pending_search_blocks_undo_then_one_completed_stroke_round_trips_without_a_provisional_entry() {
    let e = coherent_editor();
    with_search(e, |app, gate| {
        let before = app.read(|e| e.document.state.project.clone());
        app.enter(|e, cx| {
            queue(e, [50, 50], cx);
            e.restore_history(history::Direction::Undo, cx);
            assert!(e.document_pending.is_none());
            assert_eq!(e.document.state.project, before);
        });
        gate.next(app)
            .complete(Ok(SearchOutcome::Candidate([30, 0])));
        settled(app);
        let after = app.read(|e| e.document.state.project.clone());
        assert_ne!(before, after);
        app.enter(|e, cx| e.restore_history(history::Direction::Undo, cx));
        app.run_until(Duration::from_secs(30), |e| {
            !e.busy && e.document_pending.is_none()
        });
        app.read(|e| assert_eq!(e.document.state.project, before));
        app.enter(|e, cx| e.restore_history(history::Direction::Redo, cx));
        app.run_until(Duration::from_secs(30), |e| {
            !e.busy && e.document_pending.is_none()
        });
        app.read(|e| assert_eq!(e.document.state.project, after));
    });
}

#[test]
fn navigation_discards_the_unfinished_stroke_and_returning_can_start_a_new_one() {
    let mut e = super::super::super::tests::authored_fixture(&["patches.dng", "roll-0.dng"]);
    e.set_dmin(Some(DVec3::splat(0.8)));
    e.set_dmax(Some(DVec3::splat(0.02)));
    e.document.source = false;
    e.tab = 1;
    with_search(e, |app, gate| {
        let old = app.read(|e| e.document.visit.current_source.unwrap());
        let (other, index) = app.read(|e| {
            e.document
                .project
                .order
                .iter()
                .enumerate()
                .find(|(_, id)| **id != old)
                .map(|(index, id)| (*id, index))
                .unwrap()
        });
        app.enter(|e, cx| {
            queue(e, [50, 50], cx);
        });
        let call = gate.next(app);
        let captured = call.input.context.source.clone();
        app.enter(|e, cx| {
            e.select_roll_photo(index, egui::Modifiers::NONE, cx);
            assert_eq!(e.document.visit.current_source, Some(other));
            assert!(!e.dust_search_pending());
            assert!(e.pending_dust_strokes().is_empty());
        });
        app.run_until(Duration::from_secs(30), |e| {
            !e.busy && call.cancelled.is_cancelled()
        });
        call.complete(Ok(SearchOutcome::Candidate([30, 0])));
        app.pump();
        app.read(|e| {
            assert!(e.dust_repairs(old).is_none());
            assert!(e.dust_repairs(other).is_none());
            assert!(!Arc::ptr_eq(
                &captured,
                &e.document.visit.canvas.ready.as_ref().unwrap().developed
            ));
        });
        let index = app.read(|e| {
            e.document
                .project
                .order
                .iter()
                .position(|id| *id == old)
                .unwrap()
        });
        app.enter(|e, cx| e.select_roll_photo(index, egui::Modifiers::NONE, cx));
        app.run_until(Duration::from_secs(30), |e| !e.busy);
        let stroke = app.enter(|e, cx| queue(e, [50, 50], cx));
        gate.next(app)
            .complete(Ok(SearchOutcome::Candidate([30, 0])));
        settled(app);
        app.read(|e| {
            let repairs = e.dust_repairs(old).unwrap().repairs();
            assert_eq!(repairs.len(), 1);
            assert_eq!(repairs[0].support, stroke.support);
            assert!(e.dust_repairs(other).is_none());
        });
    });
}

#[test]
fn real_search_publishes_a_donor_before_a_failed_heal_without_another_paint() {
    let e = coherent_editor();
    with_application(e, |app| {
        let stroke = app.enter(|e, cx| {
            let active = e.document.visit.canvas.ready.as_ref().unwrap().area;
            queue(e, [active.x, active.y], cx)
        });
        app.run_until(Duration::from_secs(10), |e| !e.dust_search_pending());
        app.read(|e| {
            assert_eq!(
                e.dust_repairs(stroke.source).unwrap().repairs().len(),
                1,
                "{}",
                e.status
            );
            assert_eq!(
                e.dust_repairs(stroke.source).unwrap().repairs()[0].support,
                stroke.support
            );
            assert!(!e.dust_donor_regions().is_empty());
        });
        app.run_until(Duration::from_secs(10), |e| {
            e.document
                .dust
                .failed
                .contains_key(&(stroke.source, RepairId(1)))
        });
        app.read(|e| assert!(!e.dust_donor_regions().is_empty()));
    });
}
