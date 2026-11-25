use super::workflow::enter_numeric;
use super::*;

#[test]
fn gamma_calculation_preserves_independent_control_updates_with_an_unrenderable_layer() {
    tools::calibrated_harness(|mut h| {
        stage(&mut h, "Roll");
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Exposure")
            .scroll_to_me();
        h.try_run_realtime().unwrap();
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Exposure")
            .click();
        h.try_run_realtime().unwrap();
        let unrenderable = h.update(|editor| {
            let source = editor.document.visit.current_source.unwrap();
            let id = editor
                .document
                .project
                .source_mut(source)
                .unwrap()
                .adjustment_layers
                .as_mut()
                .unwrap()
                .add(
                    "Unrenderable".into(),
                    crate::adjustment_layers::AdjustmentLayer {
                        mask: crate::adjustment_layers::AdjustmentLayerCoverage::Radial {
                            center: crate::geometry::SourcePoint([216., 88.]),
                            radii: [f64::MIN_POSITIVE, 1.],
                            angle: 0.,
                            feather: 0.5,
                        },
                        ..crate::adjustment_layers::AdjustmentLayer::global(
                            AdjustmentLayerSettings {
                                exposure: 1.,
                                ..Default::default()
                            },
                        )
                    },
                )
                .unwrap();
            assert!(
                editor
                    .positive_processing(
                        editor.input(),
                        editor.document.project.roll,
                        editor.photo_settings()
                    )
                    .is_err()
            );
            id
        });
        let before = h.read(|editor| editor.document.project.clone());
        let gate = crate::editor::measurement::testing::Gate::default();
        h.update(|editor| editor.document.gamma_pivot.edit_gate = Some(gate.clone()));
        enter_numeric(&mut h, "Gamma", "0.9");
        let solve = gate.next(&mut h.application);
        h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure")
            .scroll_to_me();
        h.try_run_realtime().unwrap();
        assert!(
            !h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure")
                .accesskit_node()
                .is_disabled()
        );
        enter_numeric(&mut h, "Exposure", "0.75");
        let exposure = h.read(|editor| {
            assert!(editor.document.gamma_pivot.calculating());
            assert!(!editor.editing());
            assert_eq!(editor.document.project.input, before.input);
            assert_eq!(editor.document.project.roll.exposure, 0.75);
            editor.document.project.clone()
        });
        solve.run();
        wait_gamma(&mut h);
        h.try_run_realtime().unwrap();
        assert!(
            !h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure")
                .accesskit_node()
                .is_disabled()
        );
        h.read(|editor| {
            assert_eq!(editor.document.project.input.gamma(), 0.9);
            assert_ne!(editor.document.project.roll.wb, before.roll.wb);
            assert_eq!(editor.document.project.roll.exposure, 0.75);
        });
        for expected in [exposure, before] {
            h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
            h.step();
            wait_for(&mut h, |editor| !editor.interaction_blocked());
            h.read(|editor| assert_eq!(editor.document.project, expected));
        }
        h.update(|editor| {
            editor
                .edit_adjustment_layers(|layers| layers.remove(unrenderable))
                .unwrap()
        });
        workflow::wait_settled(&mut h);
    });
}

fn click_pin(h: &mut Harness<'_, Editor>) {
    let role = egui::accesskit::Role::Button;
    scroll_to_control(h, role, "Hold midtone colour");
    let button = h.get_by_role_and_label(role, "Hold midtone colour");
    assert!(!button.accesskit_node().is_disabled());
    button.click();
    h.step();
}

#[test]
fn extended_roll_wb_is_editable_and_reaches_the_rendered_photo() {
    tools::calibrated_harness(|mut h| {
        stage(&mut h, "Roll");
        let patch = Rect {
            x: 216,
            y: 88,
            width: 16,
            height: 16,
        };
        let before = roll_colour(&h, patch);
        h.get_by_label("White balance wheel").scroll_to_me();
        h.try_run_realtime().unwrap();
        let role = egui::accesskit::Role::SpinButton;
        let number = h
            .get_all_by_role_and_label(role, "Warmth")
            .find(|node| node.accesskit_node().min_numeric_value() == Some(-1.5))
            .expect("expanded Roll Warmth editor");
        number.click();
        h.step();
        h.event(egui::Event::Text("-1.25".into()));
        h.key_press(egui::Key::Enter);
        h.step();
        h.read(|editor| {
            assert_eq!(
                editor.document.state.project.roll.wb.warmth, -1.25,
                "{}",
                editor.status
            );
            assert_eq!(editor.balance(), Balance::default());
        });
        workflow::wait_settled(&mut h);
        let after = roll_colour(&h, patch);
        assert!((after.x / before.x - (-1.25_f64 / 2_f64.sqrt()).exp()).abs() < 1e-5);
        assert!((after.z / before.z - (1.25_f64 / 2_f64.sqrt()).exp()).abs() < 1e-5);
    });
}

#[test]
fn gamma_pivot_is_on_by_default_and_can_be_bypassed_without_a_project_edit() {
    tools::calibrated_harness(|mut h| {
        stage(&mut h, "Roll");
        let before = h.read(|editor| editor.document.state.project.clone());
        h.update(|editor| editor.set_dmax(None));
        h.step();
        assert!(
            h.get_by_role_and_label(egui::accesskit::Role::Button, "Hold midtone colour")
                .accesskit_node()
                .is_disabled()
        );
        h.update(|editor| editor.set_dmax(before.dmax().map(DVec3::from_array)));

        enter_numeric(&mut h, "Gamma", "0.9");
        wait_gamma(&mut h);
        let adjusted = h.read(|editor| {
            assert!(editor.document.gamma_pivot.enabled());
            assert!(editor.document.visit.picker.display().is_none());
            let after = &editor.document.state.project;
            assert_eq!(after.input.gamma(), 0.9);
            assert_ne!(after.roll.wb, before.roll.wb);
            after.clone()
        });
        click_pin(&mut h);
        h.read(|editor| {
            assert!(!editor.document.gamma_pivot.enabled());
            assert_eq!(editor.document.state.project, adjusted);
        });
        enter_numeric(&mut h, "Gamma", "0.8");
        h.read(|editor| {
            assert!(!editor.document.gamma_pivot.calculating());
            assert_eq!(editor.document.project.input.gamma(), 0.8);
            assert_eq!(editor.document.project.roll.wb, adjusted.roll.wb);
        });
    });
}

fn roll_colour(h: &Harness<'_, Editor>, patch: Rect) -> DVec3 {
    h.read(|editor| {
        let mut scan = crate::editor::measurement::capture_scan(editor).unwrap();
        let coverage = crate::coverage::Coverage::rectangle(patch);
        let source = editor.document.find_source(scan.source().path()).unwrap();
        let recipe = editor
            .positive_recipe_for_source(
                editor.input(),
                editor.document.project_snapshot().roll,
                editor.document.project.source(source).unwrap(),
                editor.photo_settings(),
            )
            .unwrap();
        let services = editor.source_services.as_ref().unwrap();
        let samples = crate::editor::measurement::query::Samples::new(
            (&services.evaluation, &services.queue),
            &mut scan,
            editor.correction().gpu().unwrap(),
            coverage,
            &|| false,
        )
        .unwrap();
        let roll = samples.roll(&recipe).unwrap();
        let mean = samples.mean(roll, &|| false).unwrap();
        let cones = crate::color::ap1_to_cones() * mean;
        cones / cones.y
    })
}

fn wait_gamma(h: &mut Harness<'_, Editor>) {
    wait_for(h, |editor| !editor.document.gamma_pivot.calculating());
    h.read(|editor| {
        assert!(
            !editor.status.starts_with("Edit rejected:"),
            "{}",
            editor.status
        )
    });
}

#[test]
fn save_captures_accepted_values_during_gamma_calculation_and_source_preparation() {
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    };
    let directory = tempfile::tempdir().unwrap();
    let second = directory.path().join("cold-source.dng");
    std::fs::copy(fixture("illuminated-patches.dng"), &second).unwrap();
    let destination = directory.path().join("deferred.antitone");
    tools::calibrated_harness(|mut h| {
        h.enter(|editor, cx| editor.add_raws(vec![second.clone()], cx));
        wait_for(&mut h, |editor| !editor.interaction_blocked());
        h.get_by_label("patches.dng").click();
        h.step();
        wait_for(&mut h, |editor| {
            editor
                .document
                .visit
                .canvas
                .ready
                .as_ref()
                .is_some_and(|ready| ready.path.file_name().unwrap() == "patches.dng")
        });
        stage(&mut h, "Roll");

        let entered = Arc::new(AtomicBool::new(false));
        let observed = entered.clone();
        let (release, resume) = mpsc::channel();
        struct Release(mpsc::Sender<()>);
        impl Drop for Release {
            fn drop(&mut self) {
                let _ = self.0.send(());
            }
        }
        let release = Release(release);
        let resume = Mutex::new(resume);
        let path = second.clone();
        let loader = crate::source::SourceLoader::new_observed(move |source, phase, _| {
            if source == path && matches!(phase, crate::source::NativePhase::UnpackBegin) {
                observed.store(true, Ordering::Release);
                resume
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(15))
                    .unwrap();
            }
            Ok(())
        });
        let gamma = crate::editor::measurement::testing::Gate::default();
        h.update(|editor| {
            editor.document.path = Some(destination.clone());
            editor.document.cache = Arc::new(crate::source::SourceCache::new(0));
            editor.source_services.as_mut().unwrap().developments = loader.clone();
            editor.developments = Some(loader);
            editor.document.gamma_pivot.edit_gate = Some(gamma.clone());
        });
        let before = h.read(|editor| (editor.document.project.input, editor.document.project.roll));
        enter_numeric(&mut h, "Gamma", "0.9");
        let solve = gamma.next(&mut h.application);
        native_file_action(&mut h, crate::platform::menu::Action::Save);
        wait_for(&mut h, |editor| {
            destination.exists()
                && !editor.document.write.pending()
                && !editor.document.files.pending()
        });
        let first_saved = crate::project::Project::load(&destination).unwrap();
        assert_eq!(
            (first_saved.input, first_saved.roll),
            before,
            "a private calculation does not alter the snapshot"
        );
        h.read(|editor| {
            assert!(editor.document.gamma_pivot.calculating());
            assert!(!editor.editing());
            assert!(
                editor
                    .application_menu_state()
                    .enabled(crate::platform::menu::Action::Save)
            );
        });
        h.get_by_label("cold-source.dng").click();
        h.step();
        wait_for(&mut h, |_| entered.load(Ordering::Acquire));
        h.read(|editor| {
            assert!(!editor.interaction_blocked() && editor.document.visit.canvas.ready.is_none())
        });
        solve.run();
        wait_gamma(&mut h);
        h.run_steps(3);
        let accepted = h.read(|editor| {
            assert!(
                editor.document.visit.source.pending(),
                "source preparation is still parked after gamma accepts"
            );
            assert_eq!(editor.document.project.input.gamma(), 0.9);
            assert_ne!(editor.document.project.roll.wb, Balance::default());

            (editor.document.project.input, editor.document.project.roll)
        });
        native_file_action(&mut h, crate::platform::menu::Action::Save);
        wait_for(&mut h, |editor| {
            destination.exists() && !editor.document.write.pending()
        });
        h.read(|editor| {
            assert!(
                editor.document.visit.source.pending(),
                "Save completes while independent source preparation stays parked"
            )
        });
        release.0.send(()).unwrap();
        wait_for(&mut h, |editor| {
            destination.exists() && !editor.interaction_blocked() && !editor.document.dirty()
        });
        let saved = crate::project::Project::load(&destination).unwrap();
        assert_eq!((saved.input, saved.roll), accepted);
        h.read(|editor| assert!(!editor.editing()));
    });
}

#[test]
fn a_committed_gamma_solve_survives_browsing_tabs_and_hidden_controls() {
    let directory = tempfile::tempdir().unwrap();
    let second = directory.path().join("second.dng");
    std::fs::copy(fixture("illuminated-patches.dng"), &second).unwrap();
    tools::calibrated_harness(|mut h| {
        h.enter(|editor, cx| editor.add_raws(vec![second.clone()], cx));
        wait_for(&mut h, |editor| !editor.interaction_blocked());
        h.get_by_label("patches.dng").click();
        h.step();
        wait_for(&mut h, |editor| {
            editor
                .document
                .visit
                .canvas
                .ready
                .as_ref()
                .is_some_and(|ready| ready.path.file_name().unwrap() == "patches.dng")
        });
        h.update(|editor| {
            let id = editor.document.project.order[1];
            let source = editor.document.project.source_mut(id).unwrap();
            source.adjustment_layers.as_mut().unwrap().layers[0]
                .adjustment
                .photo
                .exposure = -1.75;
        });
        let photos = h.read(|editor| editor.document.project.sources.clone());
        assert_ne!(photos[0].adjustment_layers, photos[1].adjustment_layers);
        stage(&mut h, "Roll");
        let gate = crate::editor::measurement::testing::Gate::default();
        h.update(|editor| editor.document.gamma_pivot.edit_gate = Some(gate.clone()));
        enter_numeric(&mut h, "Gamma", "0.9");
        let solve = gate.next(&mut h.application);
        h.get_by_label("second.dng").click();
        h.step();
        wait_for(&mut h, |editor| {
            editor
                .document
                .visit
                .canvas
                .ready
                .as_ref()
                .is_some_and(|ready| ready.path == second)
        });
        h.read(|editor| {
            assert!(editor.document.gamma_pivot.calculating());
            assert_eq!(editor.document.project.input.gamma(), 0.75);
            assert_eq!(editor.document.project.roll.wb, Balance::default());
            assert_eq!(editor.document.project.sources, photos);
            assert_eq!(editor.photo_settings().exposure, -1.75);
        });
        for tab in ["Photo", "Rendering"] {
            stage(&mut h, tab);
            h.read(|editor| assert!(editor.document.gamma_pivot.calculating()));
        }
        h.key_press(egui::Key::F);
        h.step();
        h.read(|editor| {
            assert!(editor.fullscreen.is_some());
            assert!(editor.document.gamma_pivot.calculating());
        });
        solve.run();
        wait_gamma(&mut h);
        h.read(|editor| {
            assert_eq!(editor.document.project.input.gamma(), 0.9);
            assert_ne!(editor.document.project.roll.wb, Balance::default());
            assert_eq!(editor.document.project.sources, photos);
            assert!(editor.document.gamma_pivot.enabled());
        });
    });
}

#[test]
fn a_gamma_balance_wheel_can_continue_its_own_drag_while_compensation_is_pending() {
    tools::calibrated_harness(|mut h| {
        stage(&mut h, "Roll");
        h.get_by_label("Gamma balance wheel").scroll_to_me();
        h.try_run_realtime().unwrap();
        let before = h.read(|editor| editor.document.project.clone());
        let start = h.get_by_label("Gamma balance wheel").rect().center();
        let gate = crate::editor::measurement::testing::Gate::default();
        h.update(|editor| editor.document.gamma_pivot.edit_gate = Some(gate.clone()));
        let middle = start + egui::vec2(7., -3.);
        h.input_mut().events.extend([
            egui::Event::PointerMoved(start),
            egui::Event::PointerButton {
                pos: start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::PointerMoved(middle),
        ]);
        h.step();
        h.step();
        let first = gate.next(&mut h.application);
        let first_request = h.read(|editor| {
            assert_eq!(editor.document.project, before);
            assert_eq!(editor.document.project_snapshot().input, before.input);
            assert_eq!(editor.document.project_snapshot().roll, before.roll);
            editor.document.gamma_pivot.requested_input().unwrap()
        });
        assert!(
            !h.get_by_label("Gamma balance wheel")
                .accesskit_node()
                .is_disabled(),
            "the owning wheel stays usable"
        );
        let end = start + egui::vec2(6., -2.);
        h.event(egui::Event::PointerMoved(end));
        h.step();
        let second = gate.next(&mut h.application);
        h.read(|editor| {
            assert_ne!(
                editor.document.gamma_pivot.requested_input().unwrap(),
                first_request
            );
            assert_eq!(editor.document.project, before);
        });
        wait_for(&mut h, |_| first.cancelled.is_cancelled());
        let completed = first.completed.clone();
        first.run();
        completed.wait(&mut h.application);
        h.event(egui::Event::PointerButton {
            pos: end,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        h.step();
        h.read(|editor| assert_eq!(editor.document.project, before));
        second.run();
        wait_gamma(&mut h);
        h.read(|editor| {
            assert_ne!(editor.document.project.input, before.input);
            assert_ne!(editor.document.project.roll.wb, before.roll.wb);
            assert!(!editor.editing());
        });
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
        h.step();
        wait_for(&mut h, |editor| !editor.interaction_blocked());
        h.read(|editor| assert_eq!(editor.document.project, before));
    });
}

#[test]
fn gamma_pivot_numeric_wheel_reset_history_and_reopen() {
    let directory = tempfile::tempdir().unwrap();
    tools::calibrated_harness(|mut h| {
        stage(&mut h, "Roll");
        // The crop lies wholly inside one authored, flat-colour tile. Midtone selection
        // cannot change the tile's colour, so the independent Roll prefix is an oracle.
        let patch = Rect {
            x: 216,
            y: 88,
            width: 16,
            height: 16,
        };
        h.update(|editor| *editor.photo_crop_mut() = Some(crate::crop::Crop::from_region(patch)));
        h.step();
        let mut held = roll_colour(&h, patch);
        assert!((held - DVec3::ONE).abs().max_element() > 0.1);
        let before = h.read(|editor| editor.document.state.project.clone());
        save_capture(&h.render().unwrap(), "gamma-midtone-lock-on");

        let path = directory.path().join("pivot.antitone");
        let gate = crate::editor::measurement::testing::Gate::default();
        h.update(|editor| {
            editor.document.path = Some(path.clone());
            editor.document.gamma_pivot.edit_gate = Some(gate.clone());
        });
        enter_numeric(&mut h, "Gamma", "0.9");
        let solve = gate.next(&mut h.application);
        native_file_action(&mut h, crate::platform::menu::Action::Save);
        h.read(|editor| {
            assert!(editor.document.gamma_pivot.calculating());
            assert_eq!(editor.document.project.input, before.input);
        });
        wait_for(&mut h, |editor| {
            path.exists() && !editor.document.write.pending() && !editor.document.files.pending()
        });
        let frozen = crate::project::Project::load(&path).unwrap();
        assert_eq!(
            (frozen.input, frozen.roll),
            (before.input, before.roll),
            "Save captures the accepted pair, not the pending proposal"
        );
        solve.run();
        wait_gamma(&mut h);
        native_file_action(&mut h, crate::platform::menu::Action::Save);
        wait_for(&mut h, |editor| {
            path.exists() && !editor.interaction_blocked() && !editor.document.dirty()
        });
        let saved = crate::project::Project::load(&path).unwrap();
        h.read(|editor| {
            assert_eq!(saved.input, editor.document.project.input);
            assert_eq!(saved.roll, editor.document.project.roll);
        });
        h.update(|editor| editor.document.gamma_pivot.edit_gate = None);
        let numeric = h.read(|editor| {
            assert_eq!(editor.document.project.input.gamma(), 0.9);
            assert_ne!(editor.document.project.roll.wb, before.roll.wb);
            editor.document.state.project.clone()
        });
        assert!((roll_colour(&h, patch) - held).abs().max_element() < 1e-4);
        // The actual production wheel must change both accepted curve balance and Roll WB.
        h.get_by_label("Gamma balance wheel").scroll_to_me();
        h.try_run_realtime().unwrap();
        let center = h.get_by_label("Gamma balance wheel").rect().center();
        drag(&mut h, center, center + egui::vec2(9., -4.));
        wait_gamma(&mut h);
        let wheel = h.read(|editor| {
            assert_ne!(editor.document.project.input, numeric.input);
            assert_ne!(editor.document.project.roll.wb, numeric.roll.wb);
            editor.document.state.project.clone()
        });
        assert!((roll_colour(&h, patch) - held).abs().max_element() < 1e-4);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
        h.step();
        wait_for(&mut h, |editor| !editor.interaction_blocked());
        h.read(|editor| assert_eq!(editor.document.state.project, numeric));
        h.key_press_modifiers(
            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            egui::Key::Z,
        );
        h.step();
        wait_for(&mut h, |editor| !editor.interaction_blocked());
        h.read(|editor| assert_eq!(editor.document.state.project, wheel));

        h.get_by_label("White balance wheel").scroll_to_me();
        h.try_run_realtime().unwrap();
        let warmth = h.read(|editor| editor.document.project.roll.wb.warmth);
        let mut rows = h
            .get_all_by_role_and_label(egui::accesskit::Role::SpinButton, "Warmth")
            .filter(|node| node.accesskit_node().numeric_value() == Some(warmth));
        let row = rows.next().expect("visible Roll WB warmth");
        assert!(
            rows.next().is_none(),
            "fixture must distinguish WB from gamma balance"
        );
        row.click();
        drop(rows);
        h.step();
        h.event(egui::Event::Text("0.25".into()));
        h.key_press(egui::Key::Enter);
        h.step();
        h.read(|editor| assert_eq!(editor.document.project.roll.wb.warmth, 0.25));
        let shifted = roll_colour(&h, patch);
        assert!((shifted - held).abs().max_element() > 0.01);
        held = shifted;
        let manually_adjusted = h.read(|editor| editor.document.project.roll.wb);
        enter_numeric(&mut h, "Gamma", "0.8");
        wait_gamma(&mut h);
        h.read(|editor| assert_ne!(editor.document.project.roll.wb, manually_adjusted));

        assert!((roll_colour(&h, patch) - held).abs().max_element() < 1e-4);
        h.get_by_label("Reset Gamma & balance").scroll_to_me();
        h.try_run_realtime().unwrap();
        h.get_by_label("Reset Gamma & balance").click();
        h.step();
        wait_gamma(&mut h);
        let reset = h.read(|editor| {
            assert_eq!(
                editor.document.project.input.gamma(),
                crate::processing::DEFAULT_GAMMA
            );
            assert_eq!(editor.balance(), Balance::default());
            assert_ne!(editor.document.project.roll.wb, wheel.roll.wb);
            editor.document.state.project.clone()
        });
        assert!((roll_colour(&h, patch) - held).abs().max_element() < 1e-4);
        h.update(|editor| editor.document.path = Some(directory.path().join("pivot.antitone")));
        save_existing_project(&mut h);
        let path = h.read(|editor| editor.document.path.clone().unwrap());
        let saved = crate::project::Project::load(&path).unwrap();
        assert_eq!(saved.input, reset.input);
        assert_eq!(saved.roll, reset.roll);
        click_pin(&mut h);
        h.read(|editor| assert!(!editor.document.gamma_pivot.enabled()));
        open_project(&mut h, path);
        wait_for(&mut h, |editor| !editor.interaction_blocked());
        h.read(|editor| {
            assert!(editor.document.gamma_pivot.enabled());
            assert_eq!(editor.document.project.input, reset.input);
            assert_eq!(editor.document.project.roll, reset.roll);
        });
    });
}

#[test]
fn orientation_during_gamma_preparation_is_preserved_by_narrow_publication() {
    tools::calibrated_harness(|mut h| {
        stage(&mut h, "Roll");
        let before = h.read(|editor| editor.document.project.clone());
        let gate = crate::editor::measurement::testing::Gate::default();
        h.update(|editor| editor.document.gamma_pivot.edit_gate = Some(gate.clone()));
        enter_numeric(&mut h, "Gamma", "0.9");
        let solve = gate.next(&mut h.application);
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Turn right")
            .click();
        h.step();
        let oriented = h.read(|editor| {
            assert_ne!(editor.document.project.orientation, before.orientation);
            assert_eq!(editor.document.project.input, before.input);
            assert!(editor.document.gamma_pivot.calculating());
            assert!(!editor.editing());
            editor.document.project.clone()
        });
        assert!(!solve.cancelled.is_cancelled());
        let completed = solve.completed.clone();
        solve.run();
        completed.wait(&mut h.application);
        wait_for(&mut h, |editor| !editor.document.gamma_pivot.calculating());
        for _ in 0..3 {
            h.step();
        }
        h.read(|editor| {
            assert_eq!(editor.document.project.input.gamma(), 0.9);
            assert_eq!(editor.document.project.orientation, oriented.orientation);
        });
        for expected in [oriented, before] {
            h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
            h.step();
            wait_for(&mut h, |editor| !editor.interaction_blocked());
            h.read(|editor| assert_eq!(editor.document.project, expected));
        }
    });
}

#[test]
fn hiding_gamma_controls_preserves_their_independent_calculation() {
    tools::calibrated_harness(|mut h| {
        stage(&mut h, "Roll");
        let before = h.read(|editor| editor.document.project.clone());
        h.get_by_role_and_label(egui::accesskit::Role::Slider, "Gamma")
            .scroll_to_me();
        h.try_run_realtime().unwrap();
        let track = h
            .get_by_role_and_label(egui::accesskit::Role::Slider, "Gamma")
            .rect();
        let point = egui::pos2(track.left() + track.width() * 0.75, track.center().y);
        let gate = crate::editor::measurement::testing::Gate::default();
        h.update(|editor| editor.document.gamma_pivot.edit_gate = Some(gate.clone()));
        h.event(egui::Event::PointerMoved(point));
        h.event(egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
        h.step();
        let solve = gate.next(&mut h.application);
        h.read(|editor| {
            assert!(
                !editor.editing(),
                "held input retains no mutation authority"
            );
            assert!(editor.document.gamma_pivot.calculating());
            assert_ne!(
                editor.document.gamma_pivot.requested_input().unwrap(),
                before.input
            );
            assert_eq!(editor.document.project_snapshot().input, before.input);
            assert_eq!(editor.document.project_snapshot().roll, before.roll);
            assert_eq!(editor.document.project, before);
        });
        // Queued kittest events each paint a frame; this boundary needs one input pass.
        h.input_mut().events.extend([
            egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::Key {
                key: egui::Key::F,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::Key {
                key: egui::Key::F,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        h.step();
        h.read(|editor| {
            assert!(
                editor.fullscreen.is_some(),
                "the real hide-controls action must run"
            );
            assert!(!editor.editing());
            assert!(editor.document.gamma_pivot.calculating());
            assert!(
                editor
                    .application_menu_state()
                    .enabled(crate::platform::menu::Action::Save)
            );
            assert_eq!(editor.document.project, before);
        });
        assert!(!solve.cancelled.is_cancelled());
        solve.run();
        wait_gamma(&mut h);
        let accepted = h.read(|editor| {
            assert_ne!(editor.document.project.input, before.input);
            assert_ne!(editor.document.project.roll.wb, before.roll.wb);
            editor.document.project.clone()
        });
        h.key_press(egui::Key::F);
        h.step();
        h.read(|editor| assert!(!editor.fullscreen_requested()));
        h.update(|editor| editor.fullscreen_changed(false));
        h.try_run_realtime().unwrap();
        h.read(|editor| {
            assert!(editor.fullscreen.is_none());
            assert!(!editor.editing());
            assert_eq!(
                editor.document.project, accepted,
                "showing the controls preserves the completed result"
            );
        });
    });
}
