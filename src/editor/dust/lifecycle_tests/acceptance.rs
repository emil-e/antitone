use super::*;
use crate::dust_repair::SearchOutcome;

fn request(e: &mut Editor) -> Request {
    let source = e.current_source.unwrap();
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
    e.viewport.layout_composition(
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(864., 576.)),
        Composition::new(
            e.document.visit.ready.as_ref().unwrap().area,
            Default::default(),
            Default::default(),
        )
        .unwrap(),
        1.,
    )
}

fn queue(e: &mut Editor, origin: [u32; 2]) -> SearchRequest {
    e.begin_dust_mask(None);
    e.document.dust.mask_draft.as_mut().unwrap().pixels = (origin[1]..origin[1] + 3)
        .flat_map(|y| (origin[0]..origin[0] + 3).map(move |x| [x, y]))
        .collect();
    e.commit_dust_mask();
    e.dust_search_requests().last().unwrap().clone()
}

#[test]
fn held_drag_completion_is_preview_only_and_release_records_the_accepted_edit() {
    let mut e = editor();
    let source = e.current_source.unwrap();
    e.begin_dust_cleanup();
    let original = e.dust_repairs(source).unwrap().clone();
    let geometry = layout(&mut e);
    e.begin_dust_donor_drag(geometry, screen_source_point(geometry, [81., 51.]));
    e.drag_dust_donor(geometry, screen_source_point(geometry, [91., 56.]));
    let request = request(&mut e);
    assert_eq!(request.repair.offset, [40, 5]);
    e.document.dust.start(&request);
    let active = e.document.visit.ready.as_ref().unwrap().area;
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
    let active = e.document.visit.ready.as_ref().unwrap().area;
    let stale = chosen_response(&first, active, [60, 0]);
    e.document.dust.start(&first);
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
    e.document.dust.start(&second);
    e.receive_dust(first.request, Ok(Some(stale)));
    assert_eq!(e.dust_repairs(first.source), Some(&expected));
    assert_eq!(e.document.dust.pending_request(), Some(second.request));
    assert!(e.document.dust.cached.is_empty());
}

#[test]
fn ordinary_evaluation_cannot_propose_a_different_offset() {
    let mut e = editor();
    let request = request(&mut e);
    let active = e.document.visit.ready.as_ref().unwrap().area;
    e.document.dust.start(&request);
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

#[test]
fn pending_stroke_has_no_donor_or_saved_instruction_then_accepts_only_the_search_result() {
    let mut e = editor();
    e.begin_dust_cleanup();
    let before = e.history_snapshot();
    let dirty = e.document.dirty();
    let search = queue(&mut e, [80, 80]);
    assert_eq!(e.document.project, before);
    assert_eq!(e.document.dirty(), dirty);
    assert!(e.dust_search_pending());
    assert!(e.dust_donor_regions().is_empty());
    assert!(!e.file_available());
    assert!(!e.history_available());
    assert_eq!(e.history_label(history::Direction::Undo), None);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("pending.antitone");
    e.save_project(
        path.clone(),
        crate::project::Save::New,
        None,
        &UiContext::render_only(),
    );
    assert!(!path.exists());
    assert!(e.document_pending.is_none());
    e.open_export_dialog();
    assert!(e.export_dialog.is_none());
    e.receive_dust_search(search.id, Ok(SearchOutcome::Candidate([20, 0])));
    assert!(!e.dust_search_pending());
    let stored = e
        .dust_repairs(search.source)
        .unwrap()
        .repairs()
        .last()
        .unwrap();
    assert_eq!(stored.support, search.support);
    assert_eq!(stored.offset, [20, 0]);
    assert_eq!(
        e.dust_donor_regions()[0].donor.x as i32 - e.dust_donor_regions()[0].target.x as i32,
        20
    );
    assert!(
        e.document.dust.cached.is_empty(),
        "donor publication must not wait for healing"
    );
    assert_eq!(e.history_label(history::Direction::Undo), Some("Dust mask"));
}

#[test]
fn failed_heal_keeps_the_completed_search_donor_draggable() {
    let mut e = editor();
    let search = queue(&mut e, [80, 80]);
    e.receive_dust_search(search.id, Ok(SearchOutcome::Candidate([30, 0])));
    let stored = e
        .dust_repairs(search.source)
        .unwrap()
        .repairs()
        .last()
        .unwrap()
        .clone();
    let mut evaluation = request(&mut e);
    evaluation.repair = stored.clone();
    e.document.dust.start(&evaluation);
    e.receive_dust(evaluation.request, Err("authored healing failure".into()));
    let geometry = layout(&mut e);
    let point = screen_source_point(geometry, [111., 81.]);
    assert!(e.dust_donor_at(geometry, point));
    e.begin_dust_donor_drag(geometry, point);
    e.drag_dust_donor(geometry, screen_source_point(geometry, [121., 86.]));
    e.end_dust_donor_drag();
    assert_eq!(
        e.dust_repairs(search.source)
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
            .contains_key(&(search.source, stored.id))
    );
}

#[test]
fn changed_support_keeps_the_previous_instruction_until_search_finishes() {
    let mut e = editor();
    e.begin_dust_cleanup();
    let source = e.current_source.unwrap();
    let previous = e.dust_repairs(source).unwrap().repairs()[0].clone();
    e.begin_dust_mask(Some(previous.id));
    e.document
        .dust
        .mask_draft
        .as_mut()
        .unwrap()
        .pixels
        .insert([53, 50]);
    e.commit_dust_mask();
    let search = e.dust_search_requests()[0].clone();
    assert_eq!(e.dust_repairs(source).unwrap().repairs()[0], previous);
    assert!(e.dust_donor_regions().is_empty());
    e.receive_dust_search(search.id, Ok(SearchOutcome::Candidate([40, 0])));
    let changed = &e.dust_repairs(source).unwrap().repairs()[0];
    assert!(changed.support.contains([53, 50]));
    assert_eq!(changed.offset, [40, 0]);
}

#[test]
fn starting_another_edit_cannot_discard_a_released_pending_mask() {
    let mut e = editor();
    let source = e.current_source.unwrap();
    e.begin_dust_mask(Some(RepairId(1)));
    e.document
        .dust
        .mask_draft
        .as_mut()
        .unwrap()
        .pixels
        .insert([53, 50]);
    e.commit_dust_mask();
    let search = e.dust_search_requests()[0].clone();
    e.begin_dust_mask(Some(RepairId(1)));
    assert!(e.document.dust.mask_draft.is_none());
    e.cancel_dust_mask();
    assert!(!search.cancelled.is_cancelled());
    assert!(e.dust_search_pending());
    e.receive_dust_search(search.id, Ok(SearchOutcome::Candidate([30, 0])));
    assert!(
        e.dust_repairs(source).unwrap().repairs()[0]
            .support
            .contains([53, 50])
    );
}

#[test]
fn stale_search_cannot_overwrite_a_newer_accepted_instruction() {
    let mut e = editor();
    let source = e.current_source.unwrap();
    e.begin_dust_mask(Some(RepairId(1)));
    e.document
        .dust
        .mask_draft
        .as_mut()
        .unwrap()
        .pixels
        .insert([53, 50]);
    e.commit_dust_mask();
    let search = e.dust_search_requests()[0].clone();
    e.dust_repairs_mut(source)
        .set_offset(RepairId(1), [40, 0])
        .unwrap();
    let before = e.history_snapshot();
    e.receive_dust_search(search.id, Ok(SearchOutcome::Candidate([60, 0])));
    assert_eq!(e.document.project, before);
    assert!(!e.dust_search_pending());
    assert_eq!(e.history_label(history::Direction::Undo), None);
}

#[test]
fn reset_cancels_pending_search_and_ignores_its_late_success() {
    let mut e = editor();
    let search = queue(&mut e, [80, 80]);
    e.reset_dust_repairs(search.source);
    assert!(search.cancelled.is_cancelled());
    assert!(!e.dust_search_pending());
    let before = e.history_snapshot();
    e.receive_dust_search(search.id, Ok(SearchOutcome::Candidate([20, 0])));
    assert_eq!(e.document.project, before);
    assert!(e.dust_repairs(search.source).is_none());
}

#[test]
fn impossible_stroke_is_rejected_whole_without_changing_existing_repairs() {
    let mut e = editor();
    let source = e.current_source.unwrap();
    let active = SourceRegion {
        x: 12,
        y: 7,
        width: 96,
        height: 96,
    };
    e.document.visit.ready.as_mut().unwrap().area = active;
    e.document
        .project
        .source_mut(source)
        .unwrap()
        .development
        .active_area = [12, 7, 96, 96];
    let before = e.history_snapshot();
    e.begin_dust_mask(None);
    e.document.dust.mask_draft.as_mut().unwrap().pixels = (active.y..active.y + active.height)
        .flat_map(|y| (active.x..active.x + active.width).map(move |x| [x, y]))
        .collect();
    e.commit_dust_mask();
    let search = e.dust_search_requests()[0].clone();
    assert_eq!(e.document.project, before);
    e.receive_dust_search(search.id, Ok(SearchOutcome::Impossible));
    assert_eq!(e.document.project, before);
    assert!(!e.dust_search_pending());
    assert!(e.status.contains("no clean donor"), "{}", e.status);
    assert_eq!(e.history_label(history::Direction::Undo), None);
}

#[test]
fn pending_strokes_complete_independently_without_overriding_manual_selection() {
    let mut e = editor();
    let a = queue(&mut e, [80, 80]);
    let b = queue(&mut e, [100, 100]);
    e.select_dust_repair(RepairId(1));
    e.receive_dust_search(b.id, Ok(SearchOutcome::Candidate([20, 0])));
    e.receive_dust_search(a.id, Ok(SearchOutcome::Candidate([20, 0])));
    assert_eq!(e.selected_dust_repair(), Some(RepairId(1)));
    let repairs = e.dust_repairs(a.source).unwrap().repairs();
    assert_eq!(repairs.len(), 3);
    assert!(repairs.iter().any(|r| r.support == a.support));
    assert!(repairs.iter().any(|r| r.support == b.support));
}

#[test]
fn interpretation_change_rescores_the_pending_support_instead_of_publishing_a_stale_choice() {
    let mut e = editor();
    let original = queue(&mut e, [80, 80]);
    e.set_gamma(0.8);
    e.receive_dust_search(original.id, Ok(SearchOutcome::Candidate([20, 0])));
    let retry = e.dust_search_requests()[0].clone();
    assert_ne!(original.id, retry.id);
    assert_eq!(retry.support, original.support);
    assert_eq!(retry.input, e.input());
    assert_eq!(e.dust_repairs(original.source).unwrap().repairs().len(), 1);
    e.receive_dust_search(retry.id, Ok(SearchOutcome::Candidate([30, 0])));
    assert_eq!(
        e.dust_repairs(original.source)
            .unwrap()
            .repairs()
            .last()
            .unwrap()
            .offset,
        [30, 0]
    );
}

fn coherent_editor() -> Editor {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let e = Editor::new(&device, &queue);
    let mut e =
        crate::editor::tests::complete_document_operation(e, Duration::from_secs(30), |e, cx| {
            e.add_raws(
                vec![
                    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("tests/fixtures/raw/patches.dng"),
                ],
                cx,
            );
        });
    e.set_dmin(Some(DVec3::splat(0.8)));
    e.set_dmax(Some(DVec3::splat(0.02)));
    e.source = false;
    e
}

#[test]
fn real_evaluation_workflow_continues_after_a_failed_repair_without_another_paint() {
    let mut e = coherent_editor();
    let source = e.current_source.unwrap();
    let active = e.document.visit.ready.as_ref().unwrap().area;
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
    crate::application::test_support::with_application(e, |app| {
        app.enter(|e, cx| e.schedule_dust(e.dust_signature().unwrap(), cx));
        app.run_until(Duration::from_secs(10), |e| {
            e.document.dust.has_evaluation(source, good)
        });
        app.read(|e| {
            assert!(e.document.dust.failed.contains_key(&(source, RepairId(1))));
            assert!(e.document.dust.pending.is_none());
        });
    });
}

#[test]
fn pending_search_blocks_undo_then_one_completed_stroke_round_trips_without_a_provisional_entry() {
    let mut e = coherent_editor();
    let before = e.history_snapshot();
    let search = queue(&mut e, [50, 50]);
    crate::application::test_support::with_application(e, |app| {
        app.enter(|e, cx| {
            e.restore_history(history::Direction::Undo, cx);
            assert!(e.document_pending.is_none());
            assert_eq!(e.document.project, before);
            e.receive_dust_search(search.id, Ok(SearchOutcome::Candidate([30, 0])));
        });
        let after = app.read(|e| e.document.project.clone());
        app.enter(|e, cx| e.restore_history(history::Direction::Undo, cx));
        app.run_until(Duration::from_secs(30), |e| {
            !e.busy && e.document_pending.is_none()
        });
        app.read(|e| assert_eq!(e.document.project, before));
        app.enter(|e, cx| e.restore_history(history::Direction::Redo, cx));
        app.run_until(Duration::from_secs(30), |e| {
            !e.busy && e.document_pending.is_none()
        });
        app.read(|e| assert_eq!(e.document.project, after));
    });
}

#[test]
fn search_retains_its_source_and_publishes_after_navigation_without_retargeting() {
    let e = coherent_editor();
    let mut e =
        crate::editor::tests::complete_document_operation(e, Duration::from_secs(30), |e, cx| {
            e.add_raws(
                vec![
                    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("tests/fixtures/raw/roll-0.dng"),
                ],
                cx,
            );
        });
    let old = e.current_source.unwrap();
    let other = e
        .document
        .project
        .order
        .iter()
        .copied()
        .find(|id| *id != old)
        .unwrap();
    let index = e
        .document
        .project
        .order
        .iter()
        .position(|id| *id == other)
        .unwrap();
    let search = queue(&mut e, [50, 50]);
    crate::application::test_support::with_application(e, |app| {
        let admission = crate::background::testing::pause();
        app.enter(|e, cx| {
            e.start_dust_searches(cx);
            e.select_roll_photo(index, egui::Modifiers::NONE, cx);
            assert_eq!(e.current_source, Some(other));
            assert!(e.dust_search_pending());
        });
        drop(admission);
        app.run_until(Duration::from_secs(30), |e| {
            !e.dust_search_pending() && !e.busy
        });
        app.read(|e| {
            assert_eq!(e.current_source, Some(other));
            assert!(e.dust_repairs(other).is_none());
            assert_eq!(
                e.dust_repairs(old).unwrap().repairs()[0].support,
                search.support
            );
        });
    });
}

#[test]
fn real_search_publishes_a_donor_before_a_failed_heal_without_another_paint() {
    let mut e = coherent_editor();
    let source = e.current_source.unwrap();
    let active = e.document.visit.ready.as_ref().unwrap().area;
    // Source-edge support has possible donors but cannot satisfy the healer's exterior.
    let search = queue(&mut e, [active.x, active.y]);
    crate::application::test_support::with_application(e, |app| {
        app.enter(|e, cx| e.start_dust_searches(cx));
        app.run_until(Duration::from_secs(10), |e| !e.dust_search_pending());
        app.read(|e| {
            assert_eq!(
                e.dust_repairs(source).unwrap().repairs().len(),
                1,
                "{}",
                e.status
            );
            assert_eq!(
                e.dust_repairs(source).unwrap().repairs()[0].support,
                search.support
            );
            assert!(!e.dust_donor_regions().is_empty());
        });
        app.run_until(Duration::from_secs(10), |e| {
            e.document.dust.failed.contains_key(&(source, RepairId(1)))
        });
        app.read(|e| assert!(!e.dust_donor_regions().is_empty()));
    });
}
