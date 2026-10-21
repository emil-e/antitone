use super::*;

fn saved_editor() -> (Editor, tempfile::TempDir) {
    let mut e = editor();
    e.sync_document();
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
    let (mut e, _directory) = saved_editor();
    let source = e.current_source.unwrap();
    let baseline = e.document.project.clone();
    e.document.dust.selected.remove(&source);
    e.select_dust_repair(RepairId(1));
    assert_eq!(e.selected_dust_repair(), Some(RepairId(1)));
    assert_eq!(e.document.project, baseline);
    assert!(!e.document.dirty());

    e.begin_dust_mask(Some(RepairId(1)));
    e.document
        .dust
        .mask_draft
        .as_mut()
        .unwrap()
        .pixels
        .insert([54, 50]);
    assert_eq!(e.document.project, baseline);
    assert!(!e.document.dirty());

    e.cancel_dust_mask();
    e.begin_dust_mask(Some(RepairId(1)));
    e.commit_dust_mask();
    assert!(!e.document.dirty(), "an unchanged release must not dirty");
    e.begin_dust_mask(Some(RepairId(1)));
    e.document.dust.mask_draft.as_mut().unwrap().rejected = true;
    e.commit_dust_mask();
    assert!(!e.document.dirty());

    e.begin_dust_mask(Some(RepairId(1)));
    e.document
        .dust
        .mask_draft
        .as_mut()
        .unwrap()
        .pixels
        .insert([54, 50]);
    e.commit_dust_mask();
    assert!(e.document.dirty());
    let changed = e.document.project.clone();
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
    assert_ne!(e.document.project, changed);
    let set = &e.document.project.source(source).unwrap().repairs;
    assert_eq!(serde_json::to_value(set).unwrap()["last_id"], 1);
}

#[test]
fn unchanged_edits_preserve_reopened_mask_margins_and_compatible_evaluations() {
    let (mut e, directory) = saved_editor();
    let source = e.current_source.unwrap();
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
    e.document.project = crate::project::Project::load(&path).unwrap();

    e.schedule_dust(e.dust_signature().unwrap());
    let request = captured_request(&e);
    let active = e.document.visit.ready.as_ref().unwrap().area;
    e.receive_dust(request.request, Ok(Some(response(&request, active))));
    let evaluation = e.document.dust.cached[&(source, RepairId(1))]
        .evaluated
        .clone();
    let reference_revision = e.original.revision();
    e.begin_dust_mask(Some(RepairId(1)));
    assert_eq!(
        e.effective_dust_repair(source, RepairId(1)).unwrap(),
        request.repair
    );
    e.commit_dust_mask();

    assert!(!e.document.dirty());
    assert_eq!(e.original.revision(), reference_revision);
    assert_eq!(
        e.dust_repairs(source).unwrap().repairs()[0].support,
        support
    );
    assert!(Arc::ptr_eq(
        &evaluation,
        &e.document.dust.cached[&(source, RepairId(1))].evaluated
    ));
    e.schedule_dust(e.dust_signature().unwrap());
    assert!(
        e.document.dust.pending_request().is_none(),
        "unchanged masks must reuse compatible evaluations"
    );
}

#[test]
fn saves_during_pending_and_failed_evaluation_keep_instructions_and_compatible_caches() {
    let (mut e, directory) = saved_editor();
    let source = e.current_source.unwrap();
    let path = directory.path().join("pending.antitone");
    e.schedule_dust(e.dust_signature().unwrap());
    let request = captured_request(&e);
    let active = e.document.visit.ready.as_ref().unwrap().area;
    let captured = e.document.capture();
    e.start_save(crate::editor::document_services::Write {
        document: captured.clone(),
        path: path.clone(),
        policy: crate::project::Save::New,
    });
    let save_request = e.document_pending.unwrap();
    assert!(e.document.write.request.is_some());
    let saved = captured.save(&path, crate::project::Save::New).unwrap();
    e.save_result(save_request + 1, Ok(Box::new(saved.clone())));
    assert_eq!(e.document_pending, Some(save_request));
    e.save_result(save_request, Ok(Box::new(saved)));
    assert!(!e.document.dirty());
    assert_eq!(e.document.dust.pending_request(), Some(request.request));
    assert_eq!(
        e.document.project.source(source).unwrap().repairs,
        captured.project.source(source).unwrap().repairs
    );

    e.receive_dust(request.request, Ok(Some(response(&request, active))));
    assert!(!e.document.dirty());
    let evaluation = e.document.dust.cached[&(source, RepairId(1))]
        .evaluated
        .clone();
    let saved = e
        .document
        .save(&path, crate::project::Save::Replace)
        .unwrap();
    e.document.accept_save(saved);
    assert!(Arc::ptr_eq(
        &evaluation,
        &e.document.dust.cached[&(source, RepairId(1))].evaluated
    ));

    e.document.dust.cached.clear();
    e.schedule_dust(e.dust_signature().unwrap());
    let failed = captured_request(&e);
    e.receive_dust(failed.request, Err("no eligible donor".into()));
    assert_eq!(e.dust_render_state(), "Repair unavailable");
    assert!(!e.document.dirty());
    let saved = e
        .document
        .save(&path, crate::project::Save::Replace)
        .unwrap();
    let reopened = Document::open(&path);
    // The synthetic source is deliberately not a RAW. Project decoding still restores failed intent.
    assert!(reopened.is_err());
    assert_eq!(crate::project::Project::load(&path).unwrap(), saved.project);
    assert!(saved.project.source(source).unwrap().repairs.repairs()[0].enabled);
}

#[test]
fn save_as_baselines_only_its_snapshot_and_failures_preserve_later_edits_and_existing_bytes() {
    let (mut e, directory) = saved_editor();
    let source = e.current_source.unwrap();
    let path = directory.path().join("save-as.antitone");
    let captured = e.document.capture();
    e.start_save(crate::editor::document_services::Write {
        document: captured.clone(),
        path: path.clone(),
        policy: crate::project::Save::New,
    });
    let request = e.document_pending.unwrap();
    assert!(e.document.write.request.is_some());
    let saved = captured.save(&path, crate::project::Save::New).unwrap();
    let bytes = std::fs::read(&path).unwrap();

    e.dust_repairs_mut(source)
        .set_enabled(RepairId(1), false)
        .unwrap();
    e.save_result(request, Ok(Box::new(saved)));
    assert!(e.document.dirty());
    assert_eq!(e.document.path.as_deref(), Some(path.as_path()));
    assert!(!e.dust_repairs(source).unwrap().repairs()[0].enabled);
    e.dust_repairs_mut(source)
        .set_enabled(RepairId(1), true)
        .unwrap();
    assert!(!e.document.dirty());

    e.reset_dust_repairs(source);
    let before = e.document.project.clone();
    e.start_save(crate::editor::document_services::Write {
        document: e.document.capture(),
        path: path.clone(),
        policy: crate::project::Save::New,
    });
    let request = e.document_pending.unwrap();
    assert!(e.document.write.request.is_some());
    assert!(e.document.save(&path, crate::project::Save::New).is_err());
    e.save_result(request, Err("destination exists".into()));
    assert_eq!(e.document.project, before);
    assert!(e.document.dirty());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
}
