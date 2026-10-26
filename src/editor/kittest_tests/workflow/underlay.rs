use super::*;

#[test]
fn physical_view_underlay_keeps_navigation_covered_during_blocked_acquisition_and_global_edits() {
    calibrated_workflow(|mut harness, _, _directory| {
        wait_settled(&mut harness);
        harness.update(|editor| editor.viewport.zoom = Some(4.));
        harness.step();
        wait_for(&mut harness, |e| e.working.support_ready());
        harness.render().unwrap();
        let retained = harness.read(|editor| editor.working.regional().unwrap().clone());
        harness.update(|editor| {
            editor.thumbnails.set_limits(cache_limits::CacheLimits {
                cpu_bytes: 0,
                gpu_bytes: 0,
            })
        });
        // Hold physical acquisition, not the UI executor or frame-bound colour work.
        let hold = crate::background::testing::pause();
        for (index, delta) in [
            egui::vec2(500., 0.),
            egui::vec2(-1000., 300.),
            egui::vec2(700., -600.),
        ]
        .into_iter()
        .enumerate()
        {
            let layout = harness.read(|editor| editor.last_layout.unwrap().0);
            harness.update(|editor| editor.viewport.pan(layout, 1., delta));
            harness.step();
            if index == 1 {
                stage(&mut harness, "Rendering");
                select_target(&mut harness, OutputTarget::Srgb);
                stage(&mut harness, "Photo");
            }
            // Exercise the actual exposure widget without waiting for acquisition to become idle.
            harness
                .get_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure")
                .scroll_to_me();
            harness.try_run_realtime().unwrap();
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
            harness.read(|editor| assert!(!editor.working.support_ready()));
            harness.read(|editor| assert!(!editor.working.reference_ready()));
            harness.read(|editor| {
                assert!(editor.working.regional().unwrap().same_source_as(&retained))
            });
            harness.read(|editor| {
                assert_eq!(
                    editor.working.displayed.as_ref().unwrap().spec.frame,
                    editor.working.current_frame().unwrap()
                )
            });
            harness.read(|editor| {
                assert!(!Arc::ptr_eq(
                    editor.working.displayed.as_ref().unwrap(),
                    editor.working.regional().unwrap()
                ))
            });
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
            harness.read(|editor| assert!(editor.document.visit.working.task.is_some()));
        }
        drop(hold);
        wait_for(&mut harness, |e| e.working.support_ready());
        harness.render().unwrap();
        harness.read(|editor| {
            let acquired = editor.working.displayed.as_ref().unwrap();
            assert!(!acquired.same_source_as(&retained));
            assert!(
                Arc::ptr_eq(acquired, editor.working.regional().unwrap()),
                "matching detail returns to the no-composition path"
            );
        });
    });
}
