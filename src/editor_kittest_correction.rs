use super::workflow::{enter_numeric, wait_settled};
use super::*;

fn pick_patch(harness: &mut Harness<'_, Editor>, action: &str, area: Rect) {
    harness.get_by_label(action).scroll_to_me();
    harness.try_run_realtime().unwrap();
    harness.get_by_label(action).click();
    harness.step();
    draw_region(harness, area);
    harness.get_by_label("Apply").click();
    harness.step();
    wait_for(harness, |e| !e.busy);
    harness.read(|editor| assert!(editor.pick.is_none(), "{}", editor.status));
}

#[test]
fn correction_reset_commits_numeric_focus_and_completes_without_another_paint() {
    editor_harness(|mut harness, _| {
        harness.enter(|e, cx| e.add_raws(vec![fixture("illuminated-patches.dng")], cx));
        wait_for(&mut harness, |e| !e.busy);
        harness.update(|e| {
            let mut scan = e.source_context().unwrap().scan().unwrap();
            e.dmin = Some(
                scan.reference(Rect {
                    x: 16,
                    y: 16,
                    width: 32,
                    height: 32,
                })
                .unwrap(),
            );
            e.dmax = Some(
                scan.reference(Rect {
                    x: 80,
                    y: 16,
                    width: 32,
                    height: 32,
                })
                .unwrap(),
            );
            e.scopes.visible = false;
            e.sync_document();
        });
        // Native chooser interaction is not part of this offscreen case.
        harness.enter(|e, cx| e.begin_flat_edit(fixture("bare-light.dng"), None, cx));
        wait_for(&mut harness, |e| !e.busy);
        harness.get_by_label("Apply").click();
        harness.step();
        harness.run_until(Duration::from_secs(20), |e| !e.busy);
        harness.read(|e| assert!(e.correction.is_some(), "{}", e.status));
        harness.set_size(egui::vec2(1100., 1100.));
        harness.update(|e| {
            e.preferences
                .sections
                .insert("roll-flat-field".into(), true);
            e.preferences
                .sections
                .insert("roll-gamma-balance".into(), true);
        });
        harness.step();
        let (old_gamma, selected, viewport, correction) = harness.read(|e| {
            (
                e.roll.gamma,
                e.current_source,
                e.viewport,
                e.document.project.flat_field.clone(),
            )
        });
        let desired = old_gamma * 0.99;
        let role = egui::accesskit::Role::SpinButton;
        harness.get_by_role_and_label(role, "Gamma").click();
        harness.step();
        harness.event(egui::Event::Text(desired.to_string()));
        harness.step();
        assert_eq!(
            harness.read(|e| e.roll.gamma),
            old_gamma,
            "typing is still an uncommitted draft"
        );
        // This action is drawn above Gamma. Starting it immediately while drawing
        // would disable the outgoing input before its focus loss can commit.
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "Reset Flat-field correction")
            .click();
        harness.step();
        harness.read(|e| {
            assert!((e.roll.gamma - desired).abs() < 1e-12, "{}", e.status);
            assert_eq!(e.document.project.roll.gamma, e.roll.gamma);
            assert!(e.busy && e.document_pending.is_some());
        });
        harness.run_until(Duration::from_secs(20), |e| !e.busy);
        harness.read(|e| {
            assert!(e.document.project.flat_field.is_none());
            assert!(e.source_context().unwrap().correction.is_none());
            assert_eq!(e.current_source, selected);
            assert_eq!(e.viewport, viewport);
            assert!((e.document.project.roll.gamma - desired).abs() < 1e-12);
        });
        harness.enter(|e, cx| e.restore_history(crate::editor::history::Direction::Undo, cx));
        harness.run_until(Duration::from_secs(20), |e| !e.busy);
        harness.read(|e| {
            assert_eq!(e.document.project.flat_field, correction);
            assert!((e.document.project.roll.gamma - desired).abs() < 1e-12);
        });
    });
}

#[test]
fn correction_widgets_measure_exactly_render_thumbnails_toggle_and_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let reference = directory.path().join("bare-light.dng");
    std::fs::copy(fixture("bare-light.dng"), &reference).unwrap();
    editor_harness_with_thumbnails(true, |mut harness, _| {
        harness.enter(|editor, cx| editor.add_raws(vec![fixture("illuminated-patches.dng")], cx));
        wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
        harness.read(|editor| assert!(editor.document.visit.ready.is_some(), "{}", editor.status));
        let before = harness.read(|editor| editor.document.project.clone());
        let selected = harness.read(|editor| editor.current_source);
        harness.get_by_label("Flat-field correction").click();
        harness.step();
        assert!(
            !harness
                .get_by_label("Set up…")
                .accesskit_node()
                .is_disabled()
        );
        assert!(
            harness
                .get_by_label("Reset Flat-field correction")
                .accesskit_node()
                .is_disabled()
        );
        // The reference chooser itself remains native acceptance; use its resulting path.
        harness.enter(|editor, cx| editor.begin_flat_edit(reference.clone(), None, cx));
        wait_for(&mut harness, |e| !e.busy);
        wait_settled(&mut harness);
        let geometry = harness.read(|editor| editor.last_layout.unwrap().0);
        let original = harness.read(|editor| editor.region.unwrap());
        assert!(geometry.canvas.expand(0.1).contains_rect(geometry.image));
        assert!(
            geometry
                .canvas
                .expand(0.1)
                .contains_rect(geometry.screen_coverage(original))
        );
        let center = geometry.screen_coverage(original).center();
        drag(
            &mut harness,
            center,
            center + egui::vec2(geometry.scale * 8., 0.),
        );
        let moved = harness.read(|editor| editor.region.unwrap());
        assert_ne!(moved, original);
        let corner = geometry.screen_coverage(moved).right_bottom();
        drag(
            &mut harness,
            corner,
            corner - egui::vec2(geometry.scale * 4., geometry.scale * 4.),
        );
        let shrunk = harness.read(|editor| {
            editor
                .region
                .unwrap()
                .exact_rectangle(editor.document.visit.ready.as_ref().unwrap().area)
                .unwrap()
        });
        let moved = harness.read(|editor| {
            moved
                .exact_rectangle(editor.document.visit.ready.as_ref().unwrap().area)
                .unwrap()
        });
        assert!(shrunk.width < moved.width && shrunk.height < moved.height);
        harness.get_by_label("Cancel").click();
        harness.step();
        harness.read(|editor| assert_eq!(editor.document.project, before));
        harness.read(|editor| assert_eq!(editor.current_source, selected));
        harness.enter(|editor, cx| editor.begin_flat_edit(reference.clone(), None, cx));
        wait_for(&mut harness, |e| !e.busy);
        let geometry = harness.read(|editor| editor.last_layout.unwrap().0);
        let region = harness.read(|editor| editor.region.unwrap());
        drag(
            &mut harness,
            geometry.screen_coverage(region).right_bottom(),
            geometry.screen_coverage(region).right_bottom()
                - egui::vec2(geometry.scale * 8., geometry.scale * 8.),
        );
        let accepted = harness.read(|editor| {
            editor
                .region
                .unwrap()
                .exact_rectangle(editor.document.visit.ready.as_ref().unwrap().area)
                .unwrap()
        });
        harness.get_by_label("Apply").click();
        harness.step();
        wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
        harness.read(|editor| assert!(editor.correction.is_some(), "{}", editor.status));
        harness.read(|editor| assert_eq!(editor.document.project.order, before.order));
        harness.read(|editor| assert_eq!(editor.document.project.dmin, before.dmin));
        harness.read(|editor| assert_eq!(editor.document.project.dmax, before.dmax));
        let settings = harness.read(|editor| editor.document.project.flat_field.clone().unwrap());
        let configured = harness.read(|editor| editor.document.project.clone());
        harness.get_by_label("Pick Dmin (film base)").scroll_to_me();
        harness.try_run_realtime().unwrap();
        harness.get_by_label("Pick Dmin (film base)").click();
        harness.step();
        harness.read(|editor| assert_eq!(editor.pick, Some(Pick::Dmin)));
        harness.step();
        assert!(
            harness
                .get_by_label("Reset Flat-field correction")
                .accesskit_node()
                .is_disabled()
        );
        harness.get_by_label("Cancel").click();
        harness.step();
        harness.read(|editor| assert_eq!(editor.document.project, configured));
        assert_eq!(
            settings.map.area(),
            [accepted.x, accepted.y, accepted.width, accepted.height]
        );
        harness.read(|editor| {
            assert!(
                editor
                    .document
                    .project
                    .source(settings.reference)
                    .unwrap()
                    .reference_only
            )
        });
        harness
            .read(|editor| assert!(!editor.document.project.order.contains(&settings.reference)));
        let wb = harness.read(|editor| {
            editor
                .document
                .project
                .development
                .as_ref()
                .unwrap()
                .fixed_wb
        });
        let mut scan = Scan::open(&fixture("illuminated-patches.dng"), Some(wb)).unwrap();
        scan.set_flat_field(Some(settings.map.clone())).unwrap();
        let [x, y, w, h] = settings.map.area();
        let mut patches = Vec::new();
        for gy in 0..9 {
            for gx in 0..9 {
                let region = Rect {
                    x: x + 32 + gx * (w - 96) / 8,
                    y: y + 32 + gy * (h - 96) / 8,
                    width: 32,
                    height: 32,
                };
                if let Ok(rgb) = scan.reference(region) {
                    patches.push((region, rgb));
                }
            }
        }
        let dmin = *patches
            .iter()
            .max_by(|a, b| a.1.element_sum().total_cmp(&b.1.element_sum()))
            .expect("usable corrected patches");
        let dmax = *patches
            .iter()
            .filter(|p| (dmin.1 / p.1).min_element() > 1.00001)
            .max_by(|a, b| {
                (dmin.1 / a.1)
                    .min_element()
                    .total_cmp(&(dmin.1 / b.1).min_element())
            })
            .expect("ordered corrected RGB pair");
        pick_patch(&mut harness, "Pick Dmin (film base)", dmin.0);
        harness.read(|editor| assert_eq!(editor.dmin, Some(dmin.1)));
        pick_patch(&mut harness, "Pick Dmax", dmax.0);
        harness.read(|editor| assert_eq!(editor.dmax, Some(dmax.1)));
        let (calibration, roll) =
            harness.read(|editor| (editor.calibration().unwrap(), editor.roll));
        let (region, balance) = patches
            .iter()
            .find_map(|(r, _)| {
                scan.roll_neutral(*r, calibration, roll)
                    .ok()
                    .map(|b| (*r, b))
            })
            .expect("usable corrected roll-neutral patch");
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "White balance")
            .scroll_to_me();
        harness.try_run_realtime().unwrap();
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "White balance")
            .click();
        harness.step();
        harness.try_run_realtime().unwrap();
        // The roll's neutral reference drives gamma balance; roll WB fills per-photo WB.
        pick_patch(&mut harness, "Pick neutral for gamma balance", region);
        harness.read(|editor| assert_eq!(editor.roll.balance, balance));
        let (roll, photo) = harness.read(|editor| (editor.roll, editor.photo));
        let (region, balance) = patches
            .iter()
            .find_map(|(r, _)| {
                scan.photo_neutral(*r, calibration, roll, photo)
                    .ok()
                    .map(|b| (*r, b))
            })
            .expect("usable corrected photo-neutral patch");
        stage(&mut harness, "Photo");
        pick_patch(&mut harness, "Pick neutral for white balance", region);
        harness.read(|editor| assert_eq!(editor.photo.wb, balance));
        harness.update(|editor| {
            adjustments::install_locals(editor);
            editor
                .edit_adjustments(|locals| {
                    locals.adjustments[0].adjustment.photo.exposure = 0.125;
                    Ok(())
                })
                .unwrap();
            editor.sync_document();
            editor
                .validate_document_candidate(&editor.document)
                .unwrap();
        });
        if harness.read(|editor| editor.source) {
            harness.get_by_label("View source").click();
            harness.step();
        }
        stage(&mut harness, "Rendering");
        for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
            select_target(&mut harness, target);
            wait_settled(&mut harness);
            wait_for(&mut harness, |e| {
                e.thumbnails.ready_count(&e.document, target) > 0
            });
            let geometry = harness.read(|editor| editor.last_layout.unwrap().0);
            assert!(geometry.canvas.expand(0.1).contains_rect(geometry.image));
            let timing =
                harness.read(|editor| Arc::new(SelectionTiming::new(editor.id, Instant::now())));
            harness.update(|editor| editor.selection_timing = Some(timing.clone()));
            harness.step();
            let png = harness.render().unwrap();
            assert!(timing.ready_was_painted());
            save_capture(
                &png,
                if target == OutputTarget::Srgb {
                    "correction-positive-srgb-offscreen"
                } else {
                    "correction-positive-p3-offscreen"
                },
            );
        }
        harness.get_by_label("View source").click();
        harness.step();
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "Source inspection")
            .click();
        harness.step();
        harness.step();
        enter_numeric(&mut harness, "Exposure", "2");
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "Source inspection")
            .click();
        harness.step();
        set_layout(&mut harness, egui::vec2(600., 600.), 315.);
        harness.get_by_label("100%").click();
        harness.step();
        harness.read(|editor| assert_eq!(editor.viewport.zoom, Some(1.)));
        stage(&mut harness, "Roll");
        harness.get_by_label("Enabled").scroll_to_me();
        harness.try_run_realtime().unwrap();
        let viewport = harness.read(|editor| editor.viewport);
        let corrected = harness.read(|editor| editor.document.project.clone());
        for enabled in [false, true] {
            harness.get_by_label("Enabled").click();
            harness.step();
            wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
            harness.read(|editor| assert_eq!(editor.correction.is_some(), enabled));
            harness.read(|editor| assert!(editor.source));
            harness.read(|editor| assert_eq!(editor.source_exposure, 2.));
            harness.read(|editor| assert_eq!(editor.viewport, viewport));
            let mut expected = corrected.clone();
            expected.flat_field.as_mut().unwrap().enabled = enabled;
            harness.read(|editor| assert_eq!(editor.document.project, expected));
        }
        wait_settled(&mut harness);
        harness.read(|editor| {
            let rendered = editor.working.displayed.as_ref().unwrap();
            let frame = rendered.spec.frame;
            assert!(
                frame.bounds().unwrap().width
                    < editor.document.visit.ready.as_ref().unwrap().area.width
            );
            // Physical 100% fixes pixel pitch, not the integer bounding ROI: fractional window edges
            // can require an extra original row/column without adding a display pixel.
            let origin = glam::DVec2::from_array(frame.source_point([0., 0.]).0);
            for point in [[1., 0.], [0., 1.]] {
                let step = glam::DVec2::from_array(frame.source_point(point).0) - origin;
                assert!((step.length() - 1.).abs() <= 1e-9);
            }
            assert_eq!(
                [
                    rendered.image.linear.width(),
                    rendered.image.linear.height()
                ],
                frame.output
            );
        });
        let png = harness.render().unwrap();
        save_capture(&png, "correction-source-native-offscreen");
        set_layout(&mut harness, egui::vec2(1100., 800.), 315.);
        stage(&mut harness, "Rendering");
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "Source inspection")
            .click();
        harness.step();
        harness.step();
        enter_numeric(&mut harness, "Exposure", "0");
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "Source inspection")
            .click();
        harness.step();
        harness.get_by_label("View source").click();
        harness.step();
        harness.get_by_label("Fit").click();
        harness.step();
        harness.enter(|editor, cx| {
            editor.save_project(
                directory.path().join("correction.antitone.json"),
                crate::project::Save::New,
                None,
                cx,
            )
        });
        wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
        harness.read(|editor| assert!(!editor.document.dirty(), "{}", editor.status));
        save_existing_project(&mut harness);
        let saved = harness.read(|editor| editor.document.project.clone());
        stage(&mut harness, "Roll");
        harness
            .get_by_label("Reset Flat-field correction")
            .scroll_to_me();
        harness.try_run_realtime().unwrap();
        harness.get_by_label("Reset Flat-field correction").click();
        harness.step();
        wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
        harness.read(|editor| assert!(editor.correction.is_none()));
        let mut cleared = saved.clone();
        cleared.flat_field = None;
        harness.read(|editor| assert_eq!(editor.document.project, cleared));
        open_project(
            &mut harness,
            directory.path().join("correction.antitone.json"),
        );
        harness.get_by_label("Discard changes").click();
        harness.step();
        wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
        harness.read(|editor| assert_eq!(editor.document.project, saved));
        harness.read(|editor| {
            assert_eq!(
                editor.correction.as_ref().unwrap().settings.map,
                settings.map
            )
        });
        wait_settled(&mut harness);
        wait_for(&mut harness, |e| {
            e.thumbnails.ready_count(&e.document, e.target) > 0
        });
        set_layout(&mut harness, egui::vec2(800., 600.), 270.);
        harness
            .get_by_label("Reset Flat-field correction")
            .scroll_to_me();
        harness.try_run_realtime().unwrap();
        harness.get_by_label("Reset Flat-field correction").click();
        harness.step();
        wait_for(&mut harness, |e| !e.busy);
        harness.read(|editor| assert_eq!(editor.document.project, cleared));
        assert!(
            !harness
                .get_by_label("Set up…")
                .accesskit_node()
                .is_disabled()
        );
        assert!(
            harness
                .get_by_label("Reset Flat-field correction")
                .accesskit_node()
                .is_disabled()
        );
        // Supply the native chooser result, then use the ordinary viewer controls.
        harness.enter(|editor, cx| editor.begin_flat_edit(reference.clone(), None, cx));
        wait_for(&mut harness, |e| !e.busy);
        assert!(
            harness
                .get_by_label("Reset Flat-field correction")
                .accesskit_node()
                .is_disabled()
        );
        harness.get_by_label("Fit").click();
        harness.step();
        wait_settled(&mut harness);
        let geometry = harness.read(|editor| editor.last_layout.unwrap().0);
        assert!(geometry.canvas.expand(0.1).contains_rect(geometry.image));
        harness.read(|editor| {
            assert!(
                geometry
                    .canvas
                    .expand(0.1)
                    .contains_rect(geometry.screen_coverage(editor.region.unwrap()))
            )
        });
        let png = harness.render().unwrap();
        save_capture(&png, "correction-setup-narrow-offscreen");
        harness.get_by_label("Cancel").click();
        harness.step();
        harness.read(|editor| assert_eq!(editor.document.project, cleared));
        harness.read(|editor| assert_eq!(editor.current_source, selected));
        harness.enter(|editor, cx| editor.begin_flat_edit(reference.clone(), None, cx));
        wait_for(&mut harness, |e| !e.busy);
        harness.get_by_label("Apply").click();
        harness.step();
        wait_for(&mut harness, |e| !e.busy);
        harness.read(|editor| {
            assert!(editor.correction.is_some(), "{}", editor.status);
            let mut expected = cleared.clone();
            expected.flat_field = editor.document.project.flat_field.clone();
            assert_eq!(editor.document.project, expected);
        });
        std::fs::remove_file(&reference).unwrap();
        open_project(
            &mut harness,
            directory.path().join("correction.antitone.json"),
        );
        harness.get_by_label("Discard changes").click();
        harness.step();
        wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
        harness.read(|editor| assert_eq!(editor.document.project, saved));
        harness.read(|editor| assert!(editor.correction.is_some()));
        harness
            .get_by_label("Reset Flat-field correction")
            .scroll_to_me();
        harness.try_run_realtime().unwrap();
        harness.get_by_label("Reset Flat-field correction").click();
        harness.step();
        wait_for(&mut harness, |e| !e.busy);
        harness.read(|editor| {
            assert_eq!(editor.document.project, cleared);
            assert!(editor.correction.is_none());
            assert!(editor.document.visit.ready.is_some());
            assert!(editor.failure.is_none(), "{}", editor.status);
        });
        harness.read(|editor| assert!(editor.preference_store.is_none()));
    });
}
