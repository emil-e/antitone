use super::*;

#[test]
fn physical_view_uncalibrated_source_keeps_detail_and_underlay_through_inspection_and_navigation() {
    let (mut harness, _) = editor_harness();
    harness
        .state_mut()
        .thumbnails
        .set_limits(cache_limits::CacheLimits {
            cpu_bytes: 0,
            gpu_bytes: 0,
        });
    harness.state_mut().add_raws(vec![fixture("patches.dng")]);
    wait_for(&mut harness, |e| {
        !e.busy && e.document_pending.is_none() && e.working.support_ready()
    });
    assert!(harness.state().source);
    assert!(harness.state().calibration().is_none());
    stage(&mut harness, "Rendering");
    harness.state_mut().viewport.zoom = Some(4.);
    harness.step();
    wait_for(&mut harness, |e| e.working.support_ready());
    harness.render().unwrap();
    let retained = harness.state().working.regional().unwrap().clone();
    assert!(retained.spec.is_inspection());
    let mut project = harness.state().document.project.clone();
    // A remembered positive comparison must neither replace source working detail on idle nor
    // force an optional reference past the zero budget while its control is hidden.
    harness.state_mut().working.compare = true;
    let (sender, commands) = mpsc::channel();
    let real_sender = std::mem::replace(&mut harness.state_mut().commands, sender);
    let mut wake = None;
    for (index, delta) in [egui::vec2(400., 0.), egui::vec2(-800., 100.)]
        .into_iter()
        .enumerate()
    {
        let layout = harness.state().last_layout.unwrap().0;
        harness.state_mut().viewport.pan(layout, 1., delta);
        harness.step();
        harness.step();
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "Source inspection")
            .click();
        harness.step();
        enter_numeric(
            &mut harness,
            "Exposure",
            if index == 0 { "0.37" } else { "1.2" },
        );
        assert_eq!(
            harness.state().source_exposure,
            if index == 0 { 0.37 } else { 1.2 }
        );
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "Source inspection")
            .click();
        harness.step();
        if index == 1 {
            select_target(&mut harness, OutputTarget::Srgb);
            project.output.tiff = crate::project::TiffTarget::Srgb;
        }
        wait_for(&mut harness, |e| {
            e.working.displayed.as_ref().is_some_and(|v| {
                v.spec.frame == e.working.current_frame().unwrap()
                    && Some(&v.processing) == e.working.processing()
                    && v.target == e.target
            })
        });
        harness.render().unwrap();
        assert!(!harness.state().working.support_ready());
        assert!(
            harness
                .state()
                .working
                .regional()
                .unwrap()
                .same_source_as(&retained)
        );
        assert!(
            harness
                .state()
                .working
                .displayed
                .as_ref()
                .unwrap()
                .spec
                .is_inspection()
        );
        assert!(
            harness.state().scope_request().is_err(),
            "source inspection still suspends photographic scopes"
        );
        assert!(!harness.state().working.reference_ready());
        assert!(harness.state().original.refining.is_none());
        assert_eq!(
            harness.state().document.project,
            project,
            "inspection EV and navigation do not alter the document"
        );
        assert!(
            !Arc::ptr_eq(
                harness.state().working.displayed.as_ref().unwrap(),
                harness.state().working.regional().unwrap()
            ),
            "the physical-frame underlay composite, not only a transformed regional patch, is painted"
        );
        let held = harness.state().working.displayed.as_ref().unwrap().clone();
        for _ in 0..5 {
            harness.step();
        }
        harness.render().unwrap();
        assert!(
            Arc::ptr_eq(&held, harness.state().working.displayed.as_ref().unwrap()),
            "idle does not replace source detail"
        );
        while let Ok(command) = commands.try_recv() {
            if matches!(command, Command::ViewSource(..)) {
                assert!(wake.replace(command).is_none());
            }
        }
    }
    harness.state_mut().commands = real_sender;
    harness.state().commands.send(wake.unwrap()).unwrap();
    wait_for(&mut harness, |e| e.working.support_ready());
    harness.render().unwrap();
    assert!(
        !harness
            .state()
            .working
            .regional()
            .unwrap()
            .same_source_as(&retained)
    );
    assert!(Arc::ptr_eq(
        harness.state().working.displayed.as_ref().unwrap(),
        harness.state().working.regional().unwrap()
    ));
}
