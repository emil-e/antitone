use super::*;

fn saved_editor() -> (Editor, tempfile::TempDir) {
    let mut e = editor();

    let directory = tempfile::tempdir().unwrap();
    *e.document = e
        .document
        .save(
            &directory.path().join("dust.antitone"),
            crate::project::Save::New,
        )
        .unwrap();
    assert!(!e.document.dirty());
    (e, directory)
}

#[test]
fn released_edits_dirty_the_document_but_selection_drafts_and_noops_do_not() {
    let (e, _directory) = saved_editor();
    crate::application::test_support::with_application(e, |app| {
        let source = app.enter(|e, cx| {
            let source = e.document.visit.current_source.unwrap();
            let baseline = e.document.state.project.clone();
            e.document.dust.selected.remove(&source);
            e.select_dust_repair(RepairId(1));
            assert_eq!(e.selected_dust_repair(), Some(RepairId(1)));
            assert_eq!(e.document.state.project, baseline);
            assert!(!e.document.dirty());

            e.begin_dust_mask(Some(RepairId(1)));
            e.document
                .dust_mask_draft_mut()
                .unwrap()
                .pixels
                .insert([54, 50]);
            assert_eq!(e.document.state.project, baseline);
            assert!(!e.document.dirty());

            e.cancel_dust_mask();
            e.begin_dust_mask(Some(RepairId(1)));
            e.commit_dust_mask(cx);
            assert!(!e.document.dirty(), "an unchanged release must not dirty");
            e.begin_dust_mask(Some(RepairId(1)));
            e.document.dust_mask_draft_mut().unwrap().rejected = true;
            e.commit_dust_mask(cx);
            assert!(!e.document.dirty());

            e.begin_dust_mask(Some(RepairId(1)));
            e.document
                .dust_mask_draft_mut()
                .unwrap()
                .pixels
                .insert([54, 50]);
            e.commit_dust_mask(cx);
            assert!(
                !e.document.dirty(),
                "pending search is not an accepted instruction"
            );
            source
        });
        app.run_until(Duration::from_secs(5), |e| !e.dust_search_pending());
        app.update(|e| {
            assert!(e.document.dirty());
            let changed = e.document.state.project.clone();
            *e.document = e
                .document
                .save(
                    e.document.path.as_deref().unwrap(),
                    crate::project::Save::Replace,
                )
                .unwrap();
            e.dust_repairs_mut(source)
                .set_enabled(RepairId(1), false)
                .unwrap();
            assert!(e.document.dirty());
            e.dust_repairs_mut(source)
                .set_enabled(RepairId(1), true)
                .unwrap();
            assert!(!e.document.dirty());

            e.select_dust_repair(RepairId(1));
            e.delete_selected_dust_repair();
            assert!(e.document.dirty());
            assert!(e.dust_repairs(source).is_none());
            assert_ne!(e.document.state.project, changed);
            let set = &e.document.state.project.source(source).unwrap().repairs;
            assert_eq!(serde_json::to_value(set).unwrap()["last_id"], 1);
        });
    });
}

#[test]
fn unchanged_edits_preserve_reopened_mask_margins_and_compatible_evaluations() {
    let (e, directory) = saved_editor();
    crate::application::test_support::with_application(e, |app| {
        app.enter(|e, cx| {
            let source = e.document.visit.current_source.unwrap();
            let mut bits = vec![false; 7 * 7];
            for y in 2..5 {
                for x in 2..5 {
                    bits[y * 7 + x] = true;
                }
            }
            let support = SupportMask::new([48, 48], [7, 7], bits).unwrap();
            e.dust_repairs_mut(source)
                .set_support(RepairId(1), support.clone())
                .unwrap();
            let path = directory.path().join("margins.antitone");
            *e.document = e.document.save(&path, crate::project::Save::New).unwrap();
            e.document.state.project = crate::project::Project::load(&path).unwrap().into();

            e.schedule_dust(e.dust_signature().unwrap(), cx);
            let request = captured_request(e);
            let active = e.document.visit.canvas.ready.as_ref().unwrap().area;
            e.receive_dust(request.request, Ok(Some(response(&request, active))));
            let evaluation = e.document.dust.cached[&(source, RepairId(1))]
                .evaluated
                .clone();
            let reference_revision = e.document.visit.canvas.original.continuation();
            e.begin_dust_mask(Some(RepairId(1)));
            assert_eq!(
                e.effective_dust_repair(source, RepairId(1)).unwrap(),
                request.repair
            );
            e.commit_dust_mask(cx);

            assert!(!e.document.dirty());
            assert_eq!(
                e.document.visit.canvas.original.continuation(),
                reference_revision
            );
            assert_eq!(
                e.dust_repairs(source).unwrap().repairs()[0].support,
                support
            );
            assert!(Arc::ptr_eq(
                &evaluation,
                &e.document.dust.cached[&(source, RepairId(1))].evaluated
            ));
            e.schedule_dust(e.dust_signature().unwrap(), cx);
            assert!(
                e.document.visit.dust.pending_request().is_none(),
                "unchanged masks must reuse compatible evaluations"
            );
        });
    });
}

#[test]
fn evaluation_completion_during_save_preserves_instructions_and_compatible_caches() {
    let (e, directory) = saved_editor();
    let path = directory.path().join("pending.antitone");
    let hold = crate::background::testing::pause();
    crate::application::test_support::with_application(e, |app| {
        let (source, captured, evaluation) = app.enter(|e, cx| {
            let source = e.document.visit.current_source.unwrap();
            e.schedule_dust(e.dust_signature().unwrap(), cx);
            let request = captured_request(e);
            let active = e.document.visit.canvas.ready.as_ref().unwrap().area;
            let captured = e.document.capture();
            drop(e.start_save(
                crate::editor::save::Write {
                    document: captured.clone(),
                    path: path.clone(),
                    policy: crate::project::Save::New,
                },
                cx,
            ));
            assert!(e.interaction_blocked() && e.document.write.pending());
            assert_eq!(
                e.document.visit.dust.pending_request(),
                Some(request.request)
            );
            e.receive_dust(request.request, Ok(Some(response(&request, active))));
            let evaluation = e.document.dust.cached[&(source, RepairId(1))]
                .evaluated
                .clone();
            assert!(e.document.write.pending());
            (source, captured, evaluation)
        });
        app.pump();
        drop(hold);
        app.run_until(Duration::from_secs(10), |e| !e.document.write.pending());
        app.enter(|e, cx| {
            assert!(!e.document.dirty());
            assert_eq!(
                e.document.project.source(source).unwrap().repairs,
                captured.project.source(source).unwrap().repairs
            );
            assert!(Arc::ptr_eq(
                &evaluation,
                &e.document.dust.cached[&(source, RepairId(1))].evaluated
            ));
            e.document.dust.cached.clear();
            e.schedule_dust(e.dust_signature().unwrap(), cx);
            let failed = captured_request(e);
            e.receive_dust(failed.request, Err("evaluation failed".into()));
            assert_eq!(e.dust_render_state(), "Repair unavailable");
            assert!(!e.document.dirty());
            let saved = e
                .document
                .save(&path, crate::project::Save::Replace)
                .unwrap();
            let reopened = Document::open(&path).unwrap();
            assert_eq!(reopened.project, saved.project);
            assert!(!reopened.dirty());
            assert_eq!(crate::project::Project::load(&path).unwrap(), saved.project);
            assert!(saved.project.source(source).unwrap().repairs.repairs()[0].enabled);
        });
    });
}

#[test]
fn save_as_baselines_only_its_snapshot_and_failures_preserve_later_edits_and_existing_bytes() {
    let (e, directory) = saved_editor();
    let path = directory.path().join("save-as.antitone");
    crate::application::test_support::with_application(e, |app| {
        let source = app.enter(|e, cx| {
            let source = e.document.visit.current_source.unwrap();
            drop(e.start_save(
                crate::editor::save::Write {
                    document: e.document.capture(),
                    path: path.clone(),
                    policy: crate::project::Save::New,
                },
                cx,
            ));
            assert!(e.interaction_blocked() && e.document.write.pending());
            e.dust_repairs_mut(source)
                .set_enabled(RepairId(1), false)
                .unwrap();
            source
        });
        app.run_until(Duration::from_secs(10), |e| !e.document.write.pending());
        let bytes = std::fs::read(&path).unwrap();
        let before = app.enter(|e, cx| {
            assert!(e.document.dirty());
            assert_eq!(e.document.path.as_deref(), Some(path.as_path()));
            assert!(!e.dust_repairs(source).unwrap().repairs()[0].enabled);
            e.dust_repairs_mut(source)
                .set_enabled(RepairId(1), true)
                .unwrap();
            assert!(!e.document.dirty());
            e.reset_dust_repairs(source);
            let before = e.document.project.clone();
            drop(e.start_save(
                crate::editor::save::Write {
                    document: e.document.capture(),
                    path: path.clone(),
                    policy: crate::project::Save::New,
                },
                cx,
            ));
            assert!(e.document.write.pending());
            before
        });
        app.run_until(Duration::from_secs(10), |e| !e.document.write.pending());
        app.read(|e| {
            assert_eq!(e.document.project, before);
            assert!(e.document.dirty());
            assert!(e.dialogs.is_open());
        });
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    });
}
