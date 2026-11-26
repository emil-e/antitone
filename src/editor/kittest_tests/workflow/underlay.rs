use super::*;

#[test]
fn successive_pans_keep_the_canvas_covered_while_a_composite_is_in_flight() {
    calibrated_workflow(|mut harness, _, _| {
        wait_settled(&mut harness);
        harness.update(|editor| editor.document.visit.canvas.viewport.zoom = Some(4.));
        harness.step();
        wait_for(&mut harness, |editor| {
            let working = &editor.document.visit.working_view;
            working.support_ready() && working.idle()
        });
        let detail = harness.read(|editor| {
            editor
                .document
                .visit
                .working_view
                .regional()
                .unwrap()
                .clone()
        });

        let acquisition = crate::background::testing::pause();
        let pan = |harness: &mut Harness<'_, Editor>, delta| {
            harness.update(|editor| {
                let (layout, density) = editor.active_canvas().last_layout.unwrap();
                editor
                    .document
                    .visit
                    .canvas
                    .viewport
                    .pan(layout, density, egui::vec2(delta, 0.));
            });
            harness.step();
        };
        pan(&mut harness, 120.);
        harness.render().unwrap();
        let first = harness.read(|editor| {
            let view = &editor.canvas_frame.displayed.as_ref().unwrap().view;
            assert_eq!(
                Some(view.spec.frame()),
                editor.document.visit.working_view.current_frame()
            );
            assert!(!Arc::ptr_eq(view, &detail));
            assert!(!editor.document.visit.working_view.support_ready());
            view.clone()
        });
        wait_for(&mut harness, |_| !first.in_flight());

        first.with_completion_deferred(|| {
            pan(&mut harness, -240.);
            harness.read(|editor| {
                let current = editor.document.visit.working_view.current_frame().unwrap();
                assert!(current.max.0[0] > first.spec.frame().max.0[0]);
                assert!(!editor.document.visit.working_view.support_ready());
            });
            harness.render().unwrap();
            wait_for(&mut harness, |editor| {
                let input = editor.scope_request().unwrap();
                let settings = editor.scopes.settings(editor.document.target());
                let Some(counts) = input.status(editor.scopes.gpu.as_ref().unwrap(), settings)
                else {
                    return false;
                };
                let counts = counts.unwrap();
                assert_eq!(counts.invalid, 0);
                assert_eq!(
                    counts.valid,
                    input.frame().output.iter().product::<u32>(),
                    "the panned canvas must not expose gaps beyond the held composite"
                );
                assert!(Arc::ptr_eq(
                    &input.view,
                    &editor.canvas_frame.displayed.as_ref().unwrap().view
                ));
                let fallback = input.view.spec.frame();
                assert_eq!(fallback.min.0, [0., 0.]);
                assert_eq!(fallback.max.0, fallback.composition.size().map(f64::from));
                true
            });

            let (second, underlay) = harness.read(|editor| {
                (
                    editor.document.visit.working_view.current_frame().unwrap(),
                    editor.canvas_frame.displayed.as_ref().unwrap().view.clone(),
                )
            });
            pan(&mut harness, -60.);
            harness.read(|editor| {
                let current = editor.document.visit.working_view.current_frame().unwrap();
                assert!(current.max.0[0] > second.max.0[0]);
                assert!(Arc::ptr_eq(
                    &editor.canvas_frame.displayed.as_ref().unwrap().view,
                    &underlay
                ));
            });
        });

        drop(acquisition);
        wait_for(&mut harness, |editor| {
            editor.document.visit.working_view.idle()
        });
        harness.read(|editor| {
            let working = &editor.document.visit.working_view;
            let painted = &editor.canvas_frame.displayed.as_ref().unwrap().view;
            assert_eq!(Some(painted.spec.frame()), working.current_frame());
            assert!(Arc::ptr_eq(painted, working.regional().unwrap()));
        });
    });
}

#[test]
fn warm_physical_view_underlay_keeps_navigation_covered_during_blocked_acquisition_and_global_edits()
 {
    calibrated_workflow(|mut harness, _, _directory| {
        wait_settled(&mut harness);
        harness.update(|editor| editor.document.visit.canvas.viewport.zoom = Some(4.));
        harness.step();
        wait_for(&mut harness, |e| {
            e.document.visit.working_view.support_ready()
        });
        harness.render().unwrap();
        wait_for(&mut harness, |e| e.document.visit.working_view.idle());
        let retained = harness.read(|editor| {
            editor
                .document
                .visit
                .working_view
                .regional()
                .unwrap()
                .clone()
        });
        harness.read(|editor| {
            assert_eq!(
                Some(retained.spec.frame()),
                editor.document.visit.working_view.current_frame(),
                "retain the completed zoomed view before blocking acquisition"
            );
        });
        harness.update(|editor| {
            // This exercises regrading from resident support, separately from cold production.
            let limits = editor.cache_limits();
            assert!(limits.gpu_bytes > 0);
            editor.set_cache_limits(cache_limits::CacheLimits {
                cpu_bytes: 0,
                ..limits
            });
            let services = editor.source_services.as_ref().unwrap();
            let renderer = services.render_fixture().unwrap();
            assert_eq!(
                renderer.evaluation().cache_policy().generated_bytes,
                limits.gpu_bytes
            );
            assert!(renderer.evaluation().retained_bytes() > 0);
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
            let layout = harness.read(|editor| editor.active_canvas().last_layout.unwrap().0);
            harness.update(|editor| editor.document.visit.canvas.viewport.pan(layout, 1., delta));
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
                e.document
                    .visit
                    .working_view
                    .displayed
                    .as_ref()
                    .is_some_and(|r| {
                        Some(&r.processing()) == e.document.visit.working_view.processing().as_ref()
                            && r.target == e.document.target()
                    })
            });
            harness.render().unwrap();
            harness.read(|editor| assert!(!editor.document.visit.working_view.support_ready()));
            harness.read(|editor| {
                assert!(
                    editor
                        .document
                        .visit
                        .working_view
                        .regional()
                        .unwrap()
                        .same_source_as(&retained)
                )
            });
            harness.read(|editor| {
                assert_eq!(
                    editor
                        .document
                        .visit
                        .working_view
                        .displayed
                        .as_ref()
                        .unwrap()
                        .spec
                        .frame(),
                    editor.document.visit.working_view.current_frame().unwrap()
                )
            });
            harness.read(|editor| {
                assert!(!Arc::ptr_eq(
                    editor
                        .document
                        .visit
                        .working_view
                        .displayed
                        .as_ref()
                        .unwrap(),
                    editor.document.visit.working_view.regional().unwrap()
                ))
            });
            wait_for(&mut harness, |e| {
                let input = e.scope_request().unwrap();
                let scopes::ScopeInput { view, .. } = &input;
                assert!(Arc::ptr_eq(
                    view,
                    e.document.visit.working_view.displayed.as_ref().unwrap()
                ));
                let settings = e.scopes.settings(e.document.target());
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
            harness.read(|editor| assert!(editor.document.visit.working_view.task.is_some()));
        }
        drop(hold);
        wait_for(&mut harness, |e| {
            e.document.visit.working_view.support_ready()
        });
        harness.render().unwrap();
        harness.read(|editor| {
            let acquired = editor
                .document
                .visit
                .working_view
                .displayed
                .as_ref()
                .unwrap();
            assert!(!acquired.same_source_as(&retained));
            assert!(
                Arc::ptr_eq(
                    acquired,
                    editor.document.visit.working_view.regional().unwrap()
                ),
                "matching detail returns to the no-composition path"
            );
        });
    });
}

#[test]
fn zero_retention_keeps_a_coherent_view_during_acquisition_and_publishes_the_latest_grade() {
    calibrated_workflow(|mut harness, _, _directory| {
        wait_settled(&mut harness);
        harness.update(|editor| editor.document.visit.canvas.viewport.zoom = Some(4.));
        harness.step();
        wait_for(&mut harness, |editor| {
            editor.document.visit.working_view.support_ready()
        });
        harness.render().unwrap();
        wait_for(&mut harness, |editor| {
            editor.document.visit.working_view.idle()
        });
        harness.update(|editor| {
            editor.set_cache_limits(cache_limits::CacheLimits {
                cpu_bytes: 0,
                gpu_bytes: 0,
            });
            let services = editor.source_services.as_ref().unwrap();
            let renderer = services.render_fixture().unwrap();
            assert_eq!(renderer.evaluation().cache_policy().generated_bytes, 0);
            assert_eq!(renderer.evaluation().retained_bytes(), 0);
        });
        let hold = crate::background::testing::pause();
        let layout = harness.read(|editor| editor.active_canvas().last_layout.unwrap().0);
        harness.update(|editor| {
            editor
                .document
                .visit
                .canvas
                .viewport
                .pan(layout, 1., egui::vec2(500., 0.))
        });
        harness.step();
        enter_numeric(&mut harness, "Exposure", "1.25");
        harness.render().unwrap();
        harness.read(|editor| {
            assert!(!editor.document.visit.working_view.support_ready());
            assert!(editor.document.visit.working_view.task.is_some());
            assert!(editor.document.visit.working_view.error.is_none());
            let displayed = editor
                .canvas_frame
                .displayed
                .as_ref()
                .expect("live pixels survive zero optional retention");
            assert_eq!(displayed.view.target, editor.document.target());
            assert_eq!(editor.photo_settings().exposure, 1.25);
        });
        drop(hold);
        wait_for(&mut harness, |editor| {
            editor.document.visit.working_view.support_ready()
        });
        harness.render().unwrap();
        harness.read(|editor| {
            let displayed = &editor.canvas_frame.displayed.as_ref().unwrap().view;
            assert_eq!(
                displayed.processing(),
                editor.document.visit.working_view.processing().unwrap()
            );
            assert_eq!(displayed.target, editor.document.target());
            assert_eq!(
                displayed.spec.frame(),
                editor.document.visit.working_view.current_frame().unwrap()
            );
            assert!(editor.document.visit.working_view.error.is_none());
        });
    });
}
