use super::*;

fn exact_values(repair: &crate::dust_repair::EvaluatedRepair) -> Vec<([u32; 2], [u32; 3])> {
    repair
        .pixels()
        .map(|(point, value)| (point, value.map(f32::to_bits)))
        .collect()
}

#[test]
fn authored_dust_save_new_open_restores_intent_exact_results_and_shared_view_scopes() {
    let mut h = tools::calibrated_harness();
    stage(&mut h, "Photo");
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("dust.antitone");
    h.state_mut().sync_document();
    h.state_mut().document = h
        .state()
        .document
        .save(&path, crate::project::Save::New)
        .unwrap();

    h.get_by_role_and_label(egui::accesskit::Role::Button, "Dust repair")
        .click();
    h.step();
    h.state_mut().dust.brush_size = 32;
    click(&mut h, egui::pos2(96., 32.), egui::Modifiers::NONE);
    ready(&mut h, 1);
    click(&mut h, egui::pos2(32., 96.), egui::Modifiers::NONE);
    ready(&mut h, 2);

    let region = h.state().dust_donor_regions().remove(0);
    let point = region.support.selected_points().next().unwrap();
    let offset = [
        region.donor.x as i32 - region.target.x as i32,
        region.donor.y as i32 - region.target.y as i32,
    ];
    let start = egui::pos2(
        point[0] as f32 + offset[0] as f32 + 0.5,
        point[1] as f32 + offset[1] as f32 + 0.5,
    );
    let layout = h.state().last_layout.unwrap().0;
    drag(
        &mut h,
        layout.screen_point(start),
        layout.screen_point(start + egui::vec2((50 - offset[0]) as f32, (50 - offset[1]) as f32)),
    );
    ready(&mut h, 2);
    let source = h.state().current_source.unwrap();
    assert!(h.state().document.dirty());
    assert_eq!(
        h.state().dust_repairs(source).unwrap().repairs()[0].donor,
        DonorChoice::Automatic
    );
    assert_eq!(
        h.state().dust_repairs(source).unwrap().repairs()[1].donor,
        DonorChoice::Offset([50, 50])
    );

    let signature = h.state().dust_signature().unwrap();
    let evaluated = h.state().dust_evaluations(signature);
    let expected = h.state().document.project.clone();
    file_menu(&mut h, "New");
    h.run();
    assert!(
        h.state().transition.is_some(),
        "repair-only edits must guard New"
    );
    h.get_by_label("Cancel").click();
    h.run();
    assert!(h.state().transition.is_none());
    assert_eq!(h.state().document.project, expected);
    save_existing_project(&mut h);
    assert!(Arc::ptr_eq(
        &evaluated[0],
        &h.state().dust_evaluations(signature)[0]
    ));

    file_menu(&mut h, "New");
    wait_for(&mut h, |e| !e.busy && e.document_pending.is_none());
    assert!(h.state().document.project.sources.is_empty());
    open_project(&mut h, path.clone());
    wait_for(&mut h, |e| !e.busy && e.document_pending.is_none());
    assert_eq!(h.state().document.project, expected);
    assert!(!h.state().document.dirty());
    assert_eq!(h.state().selected_dust_repair(), None);
    ready(&mut h, 2);
    let restored = h.state().dust_evaluations(signature);
    for (before, after) in evaluated.iter().zip(&restored) {
        assert!(!Arc::ptr_eq(before, after));
        assert_eq!(before.donors, after.donors);
        assert_eq!(exact_values(before), exact_values(after));
    }

    let view = h.state().working.displayed.as_ref().unwrap().clone();
    let scopes::ScopeInput::Rendered(scope, _) = h.state().scope_request().unwrap() else {
        panic!("scopes did not follow restored repairs")
    };
    assert!(Arc::ptr_eq(&scope, &view));
    h.get_by_label("Original-resolution reference").click();
    h.step();
    // The reference must match the complete processing intent, including admitted repairs.
    wait_for(&mut h, |e| {
        assert!(e.original.refinement_error.is_none(), "{}", e.status);
        !e.working.updating
            && e.working.reference_ready()
            && e.working.displayed.as_ref().is_some_and(|view| {
                Some(&view.processing) == e.working.processing() && view.target == e.target
            })
    });
    h.render().unwrap();
    let reference = h.state().working.displayed.as_ref().unwrap();
    assert!(Arc::ptr_eq(
        &reference.image,
        &h.state().original.settled.as_ref().unwrap().2
    ));
    let scopes::ScopeInput::Rendered(scope, _) = h.state().scope_request().unwrap() else {
        panic!("scopes did not follow the restored original reference")
    };
    assert!(Arc::ptr_eq(&scope, reference));
    assert_eq!(h.state().document.project, expected);
    h.get_by_label("Original-resolution reference").click();
    h.step();
    ready(&mut h, 2);

    let scan = Scan::open(
        &h.state().document.source_path(source).unwrap(),
        Some(expected.development.as_ref().unwrap().fixed_wb),
    )
    .unwrap();
    for repair in expected.source(source).unwrap().repairs.repairs() {
        let request = crate::editor::dust::Request {
            request: 1,
            loaded: h.state().id,
            source,
            signature,
            repair: repair.clone(),
            calibration: h.state().calibration().unwrap(),
            roll: h.state().roll.inversion_only(),
            cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            wake: h.ctx.clone(),
        };
        let cpu = crate::editor::dust::evaluate(&scan, &request).unwrap();
        let gpu = restored
            .iter()
            .find(|result| result.id == repair.id)
            .unwrap();
        assert_eq!(cpu.evaluated.donors, gpu.donors);
        assert_eq!(exact_values(&cpu.evaluated), exact_values(gpu));
    }

    let (_hold, inbox) = mpsc::channel();
    let replies = std::mem::replace(&mut h.state_mut().messages, inbox);
    open_project(&mut h, directory.path().join("missing.antitone"));
    for _ in 0..5 {
        h.step();
        assert!(h.state().document_pending.is_some());
        assert!(Arc::ptr_eq(
            &view,
            h.state().working.displayed.as_ref().unwrap()
        ));
    }
    h.state_mut().messages = replies;
    wait_for(&mut h, |e| !e.busy && e.document_pending.is_none());
    assert_eq!(h.state().document.project, expected);
    assert!(Arc::ptr_eq(
        &restored[0],
        &h.state().dust_evaluations(signature)[0]
    ));
    assert!(Arc::ptr_eq(
        &view,
        h.state().working.displayed.as_ref().unwrap()
    ));
    stage(&mut h, "Photo");
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Dust repair")
        .click();
    h.step();

    for (name, size) in [
        ("normal", egui::vec2(1100., 800.)),
        ("narrow", egui::vec2(800., 600.)),
    ] {
        h.set_size(size);
        h.get_by_label("TIFF/EXR and thumbnails do not include repairs.")
            .scroll_to_me();
        h.try_run_realtime().unwrap();
        let warning = h
            .get_by_label("TIFF/EXR and thumbnails do not include repairs.")
            .rect();
        assert!(h.ctx.content_rect().contains_rect(warning));
        save_capture(
            &h.render().unwrap(),
            &format!("dust-persistence-{name}-offscreen"),
        );
    }

    h.get_by_label("Done").click();
    h.step();
    file_menu(&mut h, "New");
    wait_for(&mut h, |e| !e.busy && e.document_pending.is_none());
    open_project(&mut h, path);
    wait_for(&mut h, |e| !e.busy && e.document_pending.is_none());
    ready(&mut h, 2);
    for (before, after) in evaluated.iter().zip(h.state().dust_evaluations(signature)) {
        assert_eq!(before.donors, after.donors);
        assert_eq!(exact_values(before), exact_values(&after));
    }
}
