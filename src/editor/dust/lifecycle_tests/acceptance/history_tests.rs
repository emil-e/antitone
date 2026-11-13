use super::*;

#[test]
fn held_mask_erase_and_donor_gestures_cannot_rebind_to_a_reused_repair_id() {
    for (donor, erase) in [(false, false), (false, true), (true, false)] {
        with_search(editor(), |app, gate| {
            let (source, target) = app.enter(|e, cx| {
                let source = e.document.visit.current_source.unwrap();
                let mut edit = e.edit_photo("Repair A").unwrap();
                let target = edit.update(|project| {
                    project
                        .source_mut(source)
                        .unwrap()
                        .repairs
                        .insert(
                            SupportMask::new([80, 80], [3, 3], vec![true; 9]).unwrap(),
                            RepairOrigin::Manual,
                            [20, 0],
                        )
                        .unwrap()
                });
                e.commit_project(edit).unwrap();
                e.begin_dust_cleanup();
                queue(e, [110, 80], cx);
                e.select_dust_repair(target);
                if donor {
                    let geometry = layout(e);
                    e.begin_dust_donor_drag(geometry, screen_source_point(geometry, [101., 81.]));
                    e.drag_dust_donor(geometry, screen_source_point(geometry, [111., 86.]));
                    assert!(
                        e.dust_session()
                            .unwrap()
                            .edit
                            .borrow()
                            .donor_draft
                            .is_some()
                    );
                } else {
                    e.begin_dust_mask(Some(target));
                    let mut draft = e.document.dust_mask_draft_mut().unwrap();
                    if erase {
                        draft.pixels.clear();
                    } else {
                        draft.pixels.insert([83, 80]);
                    }
                }
                (source, target)
            });
            let call = gate.next(app);
            app.enter(|e, cx| e.restore_history(history::Direction::Undo, cx));
            call.complete(Ok(SearchOutcome::Candidate([20, 0])));
            settled(app);
            let replacement = app.read(|e| {
                let repair = e
                    .dust_repairs(source)
                    .unwrap()
                    .repairs()
                    .iter()
                    .find(|repair| repair.id == target)
                    .unwrap();
                assert_eq!(
                    repair.support.origin(),
                    [110, 80],
                    "the allocator reused A's ID for B"
                );
                assert_eq!(
                    e.effective_dust_repairs(source),
                    e.dust_repairs(source).unwrap().repairs()
                );
                e.document.state.project.clone()
            });
            app.enter(|e, cx| {
                if donor {
                    e.end_dust_donor_drag();
                } else {
                    e.commit_dust_mask(cx);
                }
                assert!(
                    !e.dust_search_pending(),
                    "a stale gesture must not start a new search"
                );
                assert_eq!(e.document.state.project, replacement);
                assert!(e.dusting());
                assert!(e.status.contains("changed during gesture"), "{}", e.status);
            });
        });
    }
}

#[test]
fn completed_repair_undo_redo_preserves_the_dust_owner_and_can_continue_drawing() {
    with_search(editor(), |app, gate| {
        let before = app.read(|e| e.document.state.project.clone());
        let interaction = app.enter(|e, cx| {
            e.begin_dust_cleanup();
            e.document.visit.dust.brush_size = 17;
            queue(e, [80, 80], cx);
            Rc::downgrade(&e.dust_session().unwrap())
        });
        gate.next(app)
            .complete(Ok(SearchOutcome::Candidate([20, 0])));
        settled(app);
        app.update(|e| e.dust_session().unwrap().edit.borrow_mut().show_masks = false);
        let after = app.read(|e| e.document.state.project.clone());
        assert_ne!(before, after);
        for (direction, expected) in [
            (history::Direction::Undo, &before),
            (history::Direction::Redo, &after),
        ] {
            app.enter(|e, cx| e.restore_history(direction, cx));
            app.read(|e| {
                assert_eq!(&e.document.state.project, expected);
                assert!(Weak::ptr_eq(
                    &interaction,
                    &Rc::downgrade(&e.dust_session().unwrap())
                ));
                assert_eq!(e.document.visit.dust.brush_size, 17);
                assert!(!e.dust_outlines());
            });
        }
        app.enter(|e, cx| {
            queue(e, [110, 80], cx);
        });
        gate.next(app)
            .complete(Ok(SearchOutcome::Candidate([20, 0])));
        settled(app);
        app.read(|e| {
            assert_eq!(
                e.dust_repairs(e.document.visit.current_source.unwrap())
                    .unwrap()
                    .repairs()
                    .len(),
                3
            );
            assert!(Weak::ptr_eq(
                &interaction,
                &Rc::downgrade(&e.dust_session().unwrap())
            ));
        });
    });
}

#[test]
fn pending_search_survives_compatible_history_and_publishes_into_the_restored_grade() {
    with_search(editor(), |app, gate| {
        let before = app.read(|e| e.document.state.project.clone());
        let interaction = app.enter(|e, cx| {
            let mut edit = e.edit_project("Exposure").unwrap();
            edit.update(|project| project.roll.exposure = 0.75);
            e.commit_project(edit).unwrap();
            e.begin_dust_cleanup();
            queue(e, [80, 80], cx);
            Rc::downgrade(&e.dust_session().unwrap())
        });
        let call = gate.next(app);
        app.enter(|e, cx| e.restore_history(history::Direction::Undo, cx));
        app.read(|e| {
            assert_eq!(e.document.state.project, before);
            assert!(e.dust_search_pending());
            assert!(Weak::ptr_eq(
                &interaction,
                &Rc::downgrade(&e.dust_session().unwrap())
            ));
            assert_eq!(e.history_label(history::Direction::Redo), Some("Exposure"));
        });
        assert!(!call.cancelled.is_cancelled());
        call.complete(Ok(SearchOutcome::Candidate([20, 0])));
        settled(app);
        app.read(|e| {
            assert_eq!(e.document.project.roll.exposure, before.roll.exposure);
            assert_eq!(
                e.dust_repairs(e.document.visit.current_source.unwrap())
                    .unwrap()
                    .repairs()
                    .len(),
                2
            );
            assert_eq!(e.history_label(history::Direction::Undo), Some("Dust mask"));
            assert_eq!(e.history_label(history::Direction::Redo), None);
        });
        app.enter(|e, cx| e.restore_history(history::Direction::Undo, cx));
        app.read(|e| assert_eq!(e.document.state.project, before));
    });
}

#[test]
fn pending_search_cannot_publish_after_history_changes_its_neutral_input_or_repair_target() {
    for change_input in [false, true] {
        with_search(editor(), |app, gate| {
            let before = app.read(|e| e.document.state.project.clone());
            let interaction = app.enter(|e, cx| {
                let source = e.document.visit.current_source.unwrap();
                let mut edit = e.edit_project("Search basis").unwrap();
                edit.update(|project| {
                    if change_input {
                        project.set_gamma(0.7);
                    } else {
                        project
                            .source_mut(source)
                            .unwrap()
                            .repairs
                            .set_offset(RepairId(1), [40, 0])
                            .unwrap();
                    }
                });
                e.commit_project(edit).unwrap();
                e.begin_dust_cleanup();
                if change_input {
                    queue(e, [80, 80], cx);
                } else {
                    change_support(e, cx);
                }
                Rc::downgrade(&e.dust_session().unwrap())
            });
            let call = gate.next(app);
            app.enter(|e, cx| e.restore_history(history::Direction::Undo, cx));
            app.read(|e| assert_eq!(e.document.state.project, before));
            assert!(!call.cancelled.is_cancelled());
            call.complete(Ok(SearchOutcome::Candidate([20, 0])));
            settled(app);
            app.read(|e| {
                assert_eq!(e.document.state.project, before);
                assert_eq!(e.history_label(history::Direction::Undo), None);
                assert_eq!(
                    e.history_label(history::Direction::Redo),
                    Some("Search basis")
                );
                assert!(Weak::ptr_eq(
                    &interaction,
                    &Rc::downgrade(&e.dust_session().unwrap())
                ));
                assert!(
                    e.status.contains("changed during donor search"),
                    "{}",
                    e.status
                );
            });
        });
    }
}

#[test]
fn redo_removal_prevents_a_pending_support_change_from_resurrecting_the_repair() {
    with_search(editor(), |app, gate| {
        app.enter(|e, cx| {
            let source = e.document.visit.current_source.unwrap();
            let mut edit = e.edit_photo("Remove repair").unwrap();
            edit.update(|project| {
                project
                    .source_mut(source)
                    .unwrap()
                    .repairs
                    .remove(RepairId(1))
            })
            .unwrap();
            e.commit_project(edit).unwrap();
            e.restore_history(history::Direction::Undo, cx);
            e.begin_dust_cleanup();
            change_support(e, cx);
        });
        let call = gate.next(app);
        app.enter(|e, cx| e.restore_history(history::Direction::Redo, cx));
        let removed = app.read(|e| e.document.state.project.clone());
        call.complete(Ok(SearchOutcome::Candidate([20, 0])));
        settled(app);
        app.read(|e| {
            assert_eq!(e.document.state.project, removed);
            assert_eq!(
                e.history_label(history::Direction::Undo),
                Some("Remove repair")
            );
            assert!(e.dusting());
        });
    });
}
