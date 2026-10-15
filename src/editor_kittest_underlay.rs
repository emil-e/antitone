use super::*;

#[test]
fn physical_view_underlay_keeps_navigation_covered_during_blocked_acquisition_and_global_edits() {
    let (mut harness, _, _directory) = calibrated_workflow();
    wait_settled(&mut harness);
    harness.state_mut().viewport.zoom = Some(4.);
    harness.step();
    wait_for(&mut harness, |e| e.working.support_ready());
    harness.render().unwrap();
    let retained = harness.state().working.regional().unwrap().clone();
    harness
        .state_mut()
        .thumbnails
        .set_limits(cache_limits::CacheLimits {
            cpu_bytes: 0,
            gpu_bytes: 0,
        });
    // Keep the real worker/source alive but hold subsequent commands at the test boundary. The
    // editor must cover navigation without a newly acquired regional slice or optional reference.
    let (sender, commands) = mpsc::channel();
    let real_sender = std::mem::replace(&mut harness.state_mut().commands, sender);
    let mut wake = None;
    for (index, delta) in [
        egui::vec2(500., 0.),
        egui::vec2(-1000., 300.),
        egui::vec2(700., -600.),
    ]
    .into_iter()
    .enumerate()
    {
        let layout = harness.state().last_layout.unwrap().0;
        harness.state_mut().viewport.pan(layout, 1., delta);
        harness.step();
        if index == 1 {
            stage(&mut harness, "Rendering");
            select_target(&mut harness, OutputTarget::Srgb);
            stage(&mut harness, "Photo");
        }
        // Exercise the actual exposure widget without waiting for acquisition to become idle.
        harness
            .get_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure")
            .click();
        harness.step();
        harness.event(egui::Event::Text(format!("{}", 0.6 + index as f64 * 0.2)));
        harness.event(egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        harness.step();
        wait_for(&mut harness, |e| {
            e.working.displayed.as_ref().is_some_and(|r| {
                Some(&r.processing) == e.working.processing() && r.target == e.target
            })
        });
        harness.render().unwrap();
        assert!(!harness.state().working.support_ready());
        assert!(!harness.state().working.reference_ready());
        assert!(
            harness
                .state()
                .working
                .regional()
                .unwrap()
                .same_source_as(&retained)
        );
        assert_eq!(
            harness
                .state()
                .working
                .displayed
                .as_ref()
                .unwrap()
                .spec
                .frame,
            harness.state().working.current_frame().unwrap()
        );
        assert!(!Arc::ptr_eq(
            harness.state().working.displayed.as_ref().unwrap(),
            harness.state().working.regional().unwrap()
        ));
        wait_for(&mut harness, |e| {
            let input = e.scope_request().unwrap();
            match &input {
                scopes::ScopeInput::Rendered(view, _) => {
                    assert!(Arc::ptr_eq(view, e.working.displayed.as_ref().unwrap()))
                }
                _ => panic!("scopes must share the accepted coverage composite"),
            }
            let settings = crate::scopes::Settings {
                target: e.target,
                plot: if e.scopes.vector {
                    crate::scopes::Plot::Vector
                } else {
                    e.scopes.waveform
                },
                amplification: e.scopes.amplification,
            };
            let Some(counts) = input.status(e.scopes.gpu.as_ref().unwrap(), settings) else {
                return false;
            };
            let counts = counts.unwrap();
            assert_eq!(counts.invalid, 0);
            assert_eq!(
                counts.valid,
                input.frame().output[0] * input.frame().output[1],
                "no uncovered photograph pixels while regional acquisition is blocked"
            );
            true
        });
        while let Ok(command) = commands.try_recv() {
            if matches!(command, Command::ViewSource(..)) {
                assert!(
                    wake.replace(command).is_none(),
                    "one acquisition wake coalesces all navigation"
                );
            }
        }
    }
    harness.state_mut().commands = real_sender;
    harness
        .state()
        .commands
        .send(wake.expect("pending latest-view acquisition"))
        .unwrap();
    wait_for(&mut harness, |e| e.working.support_ready());
    harness.render().unwrap();
    let acquired = harness.state().working.displayed.as_ref().unwrap();
    assert!(!acquired.same_source_as(&retained));
    assert!(
        Arc::ptr_eq(acquired, harness.state().working.regional().unwrap()),
        "matching detail returns to the no-composition path"
    );
}
