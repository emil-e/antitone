use super::workflow::enter_numeric;
use super::*;

fn pin(h: &mut Harness<'_, Editor>) {
    let role = egui::accesskit::Role::Button;
    h.get_by_role_and_label(role, "Gamma colour pivot")
        .scroll_to_me();
    h.try_run_realtime().unwrap();
    let button = h.get_by_role_and_label(role, "Gamma colour pivot");
    assert!(!button.accesskit_node().is_disabled());
    button.click();
    h.step();
}

fn selected_patch(h: &mut Harness<'_, Editor>) {
    wait_for(h, |editor| {
        editor.document.visit.picker.display() == Some(crate::editor::gamma_pivot::DISPLAY)
    });
    draw_region(
        h,
        Rect {
            x: 216,
            y: 88,
            width: 16,
            height: 16,
        },
    );
}

fn held_roll_colour(h: &Harness<'_, Editor>) -> DVec3 {
    let mut scan = h.read(|editor| crate::editor::measurement::capture_scan(editor).unwrap());
    scan_roll_colour(h, &mut scan)
}

fn scan_roll_colour(h: &Harness<'_, Editor>, scan: &mut Scan) -> DVec3 {
    h.read(|editor| {
        let coverage = crate::coverage::Coverage::rectangle(Rect {
            x: 216,
            y: 88,
            width: 16,
            height: 16,
        });
        let samples = crate::editor::measurement::query::Samples::new(
            editor.source_services.as_ref().unwrap(),
            scan,
            coverage,
            &|| false,
        )
        .unwrap();
        let photograph = samples
            .photograph(
                editor.document.input,
                editor.document.roll,
                AdjustmentLayerSnapshot::new(vec![]).unwrap(),
            )
            .unwrap();
        let mean = samples.mean(photograph.roll, &|| false).unwrap();
        let cones = crate::color::ap1_to_cones() * mean;
        cones / cones.y
    })
}

fn wait_gamma(h: &mut Harness<'_, Editor>) {
    wait_for(h, |editor| !editor.document.gamma_pivot.pending());
    h.read(|editor| {
        assert!(
            !editor.status.starts_with("Edit rejected:"),
            "{}",
            editor.status
        )
    });
}

#[test]
fn admitted_save_waits_for_gamma_but_not_independent_cold_source_preparation() {
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
        pin(&mut h);
        selected_patch(&mut h);
        h.get_by_label("Apply").click();
        h.step();
        wait_for(&mut h, |editor| {
            editor.document.gamma_pivot.locked() && !editor.interaction_blocked()
        });
        h.read(|editor| assert!(editor.document.gamma_pivot.locked()));

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
        enter_numeric(&mut h, "Gamma", "0.9");
        let solve = gamma.next(&mut h.application);
        native_file_action(&mut h, crate::platform::menu::Action::Save);
        h.read(|editor| {
            assert_eq!(
                editor.document.deferred_menu.front(),
                Some(&crate::platform::menu::Action::Save)
            )
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
            assert_eq!(editor.document.input.gamma(), 0.9);
            assert_ne!(editor.document.roll.wb, Balance::default());

            (editor.document.input, editor.document.roll)
        });
        wait_for(&mut h, |editor| {
            destination.exists() && editor.document_pending.is_none()
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
        h.read(|editor| assert!(editor.document.deferred_menu.is_empty()));
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
            editor.sync_document();
        });
        let photos = h.read(|editor| editor.document.project.sources.clone());
        assert_ne!(photos[0].adjustment_layers, photos[1].adjustment_layers);
        stage(&mut h, "Roll");
        pin(&mut h);
        selected_patch(&mut h);
        h.get_by_label("Apply").click();
        h.step();
        wait_for(&mut h, |editor| {
            editor.document.gamma_pivot.locked() && !editor.interaction_blocked()
        });
        h.read(|editor| assert!(editor.document.gamma_pivot.locked()));
        let mut captured =
            h.read(|editor| crate::editor::measurement::capture_scan(editor).unwrap());
        let held = scan_roll_colour(&h, &mut captured);
        let gate = crate::editor::measurement::testing::Gate::default();
        h.update(|editor| editor.document.gamma_pivot.edit_gate = Some(gate.clone()));
        enter_numeric(&mut h, "Gamma", "0.9");
        let solve = gate.next(&mut h.application);
        // A delayed photo action retains its selected source rather than retargeting.
        h.enter(|editor, cx| {
            editor.control_command(
                crate::editor::editing::Command::Photo(
                    crate::editor::editing::PhotoAction::TurnLeft,
                ),
                cx,
            )
        });
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
            assert!(editor.document.gamma_pivot.pending());
            assert_eq!(editor.document.project.sources, photos);
            assert_eq!(
                editor.document.visit.adjustment_layer_settings.exposure,
                -1.75
            );
        });
        assert!(
            (held_roll_colour(&h) - held).abs().max_element() > 0.05,
            "the destination source must have contrasting sampled colour"
        );
        for tab in ["Photo", "Rendering"] {
            stage(&mut h, tab);
            h.read(|editor| assert!(editor.document.gamma_pivot.pending()));
        }
        h.key_press(egui::Key::F);
        h.step();
        h.read(|editor| {
            assert!(editor.fullscreen.is_some());
            assert!(editor.document.gamma_pivot.pending());
        });
        solve.run();
        wait_gamma(&mut h);
        assert!(
            (scan_roll_colour(&h, &mut captured) - held)
                .abs()
                .max_element()
                < 1e-4,
            "compensation must hold the captured source, not the newly selected one"
        );
        h.read(|editor| {
            assert_eq!(editor.document.input.gamma(), 0.9);
            assert_ne!(editor.document.roll.wb, Balance::default());
            assert_eq!(editor.document.project.sources, photos);
            assert!(editor.document.gamma_pivot.locked());
        });
    });
}

#[test]
fn gamma_pivot_pin_apply_cancel_numeric_wheel_reset_history_and_reopen() {
    let directory = tempfile::tempdir().unwrap();
    tools::calibrated_harness(|mut h| {
        stage(&mut h, "Roll");
        let before = h.read(|editor| editor.document.state.project.clone());
        save_capture(&h.render().unwrap(), "gamma-pivot-inactive");
        pin(&mut h);
        wait_for(&mut h, |editor| {
            editor.document.visit.picker.display().is_some()
        });
        save_capture(&h.render().unwrap(), "gamma-pivot-choosing");
        pin(&mut h);
        wait_for(&mut h, |editor| {
            editor.document.visit.picker.display().is_none()
        });
        h.read(|editor| assert_eq!(editor.document.state.project, before));
        pin(&mut h);
        selected_patch(&mut h);
        h.read(|editor| {
            assert!(!editor.document.gamma_pivot.locked());
            assert_eq!(editor.document.state.project, before);
        });
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Cancel")
            .click();
        h.step();
        wait_for(&mut h, |editor| {
            editor.document.visit.picker.display().is_none()
        });
        h.read(|editor| assert_eq!(editor.document.state.project, before));

        pin(&mut h);
        selected_patch(&mut h);
        assert!(!h.get_by_label("Apply").accesskit_node().is_disabled());
        h.get_by_label("Apply").click();
        h.step();
        wait_for(&mut h, |editor| {
            editor.document.gamma_pivot.locked() && !editor.interaction_blocked()
        });
        h.read(|editor| {
            assert!(editor.document.gamma_pivot.locked(), "{}", editor.status);
            assert_eq!(editor.document.state.project, before);
        });
        h.try_run_realtime().unwrap();
        save_capture(&h.render().unwrap(), "gamma-pivot-locked");
        let held = held_roll_colour(&h);
        assert!(
            (held - DVec3::ONE).abs().max_element() > 0.1,
            "the selected patch must be coloured"
        );

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
            assert!(editor.document.gamma_pivot.pending());
            assert_eq!(editor.document.input, before.input);
        });
        assert!(
            !path.exists(),
            "Save must wait for complete gamma/WB admission"
        );
        solve.run();
        wait_gamma(&mut h);
        wait_for(&mut h, |editor| {
            path.exists() && !editor.interaction_blocked() && !editor.document.dirty()
        });
        let saved = crate::project::Project::load(&path).unwrap();
        h.read(|editor| {
            assert_eq!(saved.input, editor.document.input);
            assert_eq!(saved.roll, editor.document.roll);
        });
        h.update(|editor| editor.document.gamma_pivot.edit_gate = None);
        let numeric = h.read(|editor| {
            assert_eq!(editor.document.input.gamma(), 0.9);
            assert_ne!(editor.document.roll.wb, before.roll.wb);
            editor.document.state.project.clone()
        });
        assert!((held_roll_colour(&h) - held).abs().max_element() < 1e-4);
        // The actual production wheel must change both accepted curve balance and Roll WB.
        h.get_by_label("Gamma balance wheel").scroll_to_me();
        h.try_run_realtime().unwrap();
        let center = h.get_by_label("Gamma balance wheel").rect().center();
        drag(&mut h, center, center + egui::vec2(9., -4.));
        wait_gamma(&mut h);
        let wheel = h.read(|editor| {
            assert_ne!(editor.document.input, numeric.input);
            assert_ne!(editor.document.roll.wb, numeric.roll.wb);
            editor.document.state.project.clone()
        });
        assert!((held_roll_colour(&h) - held).abs().max_element() < 1e-4);
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
        let warmth = h.read(|editor| editor.document.roll.wb.warmth);
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
        h.read(|editor| assert_eq!(editor.document.roll.wb.warmth, 0.25));
        let previous_colour = held;
        let held = held_roll_colour(&h);
        assert!((held - previous_colour).abs().max_element() > 0.01);
        enter_numeric(&mut h, "Gamma", "0.8");
        wait_gamma(&mut h);
        assert!((held_roll_colour(&h) - held).abs().max_element() < 1e-4);

        h.get_by_label("Reset Gamma & balance").scroll_to_me();
        h.try_run_realtime().unwrap();
        h.get_by_label("Reset Gamma & balance").click();
        h.step();
        wait_gamma(&mut h);
        let reset = h.read(|editor| {
            assert_eq!(
                editor.document.input.gamma(),
                crate::processing::DEFAULT_GAMMA
            );
            assert_eq!(editor.balance(), Balance::default());
            assert_ne!(editor.document.roll.wb, wheel.roll.wb);
            editor.document.state.project.clone()
        });
        assert!((held_roll_colour(&h) - held).abs().max_element() < 1e-4);
        h.update(|editor| editor.document.path = Some(directory.path().join("pivot.antitone")));
        save_existing_project(&mut h);
        let path = h.read(|editor| editor.document.path.clone().unwrap());
        let saved = crate::project::Project::load(&path).unwrap();
        assert_eq!(saved.input, reset.input);
        assert_eq!(saved.roll, reset.roll);
        open_project(&mut h, path);
        wait_for(&mut h, |editor| {
            !editor.interaction_blocked() && editor.document_pending.is_none()
        });
        h.read(|editor| {
            assert!(!editor.document.gamma_pivot.locked());
            assert_eq!(editor.document.input, reset.input);
            assert_eq!(editor.document.roll, reset.roll);
        });
    });
}
