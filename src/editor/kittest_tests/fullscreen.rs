use super::*;

fn native(harness: &mut Harness<'_, Editor>, fullscreen: bool) {
    harness
        .input_mut()
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .fullscreen = Some(fullscreen);
}

fn window_size(harness: &mut Harness<'_, Editor>, size: egui::Vec2) {
    harness.set_size(size);
    harness
        .input_mut()
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .inner_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size));
}

fn reveal(harness: &mut Harness<'_, Editor>) {
    let canvas = harness.ctx.content_rect();
    harness.event(egui::Event::PointerMoved(egui::pos2(
        canvas.center().x,
        canvas.bottom() - 30.,
    )));
    harness.step();
}

fn elapse(harness: &mut Harness<'_, Editor>, seconds: f64) {
    harness.input_mut().time = Some(harness.ctx.input(|input| input.time) + seconds);
    harness.step();
    harness.input_mut().time = None;
    for _ in 0..12 {
        harness.step();
    }
}

fn key(harness: &mut Harness<'_, Editor>, key: egui::Key) {
    harness.event(egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
    harness.event(egui::Event::Key {
        key,
        physical_key: None,
        pressed: false,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
}

#[test]
fn fullscreen_widgets_fill_the_canvas_and_restore_the_workspace_at_both_densities() {
    for (layout, size, width) in [
        ("normal", egui::vec2(1100., 800.), 315.),
        ("narrow", egui::vec2(800., 600.), 270.),
    ] {
        for density in [1., 2.] {
            tools::calibrated_harness(|mut harness| {
                let windowed_size = size - egui::vec2(0., 32.);
                set_layout(&mut harness, windowed_size, width);
                harness
                    .input_mut()
                    .viewports
                    .get_mut(&egui::ViewportId::ROOT)
                    .unwrap()
                    .native_pixels_per_point = Some(density);
                window_size(&mut harness, windowed_size);
                native(&mut harness, false);
                harness.step();
                harness.step();
                let preferences = harness.read(|editor| editor.preferences.clone());
                let project = harness.read(|editor| editor.document.state.project.clone());
                let viewport = harness.read(|editor| editor.document.visit.canvas.viewport);
                let source =
                    harness.read(|editor| (editor.document.source, editor.source_exposure));
                let workspace =
                    harness.read(|editor| editor.active_canvas().last_layout.unwrap().0.canvas);
                harness.get_by_label("Fullscreen viewing").click();
                harness.step();
                harness.read(|editor| assert!(editor.fullscreen.is_some()));
                window_size(&mut harness, size);
                // Acquisition/refinement settle on real workers and wall time, not virtual UI steps.
                harness.try_run_realtime().unwrap();
                reveal(&mut harness);
                for label in ["Export", "Scopes", "Roll", "Photo", "Rendering"] {
                    assert!(
                        harness.query_by_label(label).is_none(),
                        "fullscreen hides {label}"
                    );
                }
                let canvas =
                    harness.read(|editor| editor.active_canvas().last_layout.unwrap().0.canvas);
                assert_eq!(canvas, egui::Rect::from_min_size(egui::Pos2::ZERO, size));
                assert!(canvas.width() > workspace.width() && canvas.height() > workspace.height());
                let exit = harness.get_by_label("Exit fullscreen").rect();
                assert!(canvas.contains_rect(exit));
                harness.get_by_label("View source").click();
                harness.step();
                harness.read(|editor| assert!(editor.document.source));
                harness.get_by_label("Source inspection").click();
                harness.step();
                workflow::enter_numeric(&mut harness, "Exposure", "1.234567");
                harness.read(|editor| assert_eq!(editor.source_exposure, 1.234567));
                key(&mut harness, egui::Key::Escape);
                harness.read(|editor| {
                    assert!(editor.fullscreen.is_some(), "popover owns first Escape")
                });
                harness.get_by_label("100%").click();
                harness.step();
                harness.read(|editor| {
                    assert_eq!(editor.document.visit.canvas.viewport.zoom, Some(1.))
                });
                key(&mut harness, egui::Key::Z);
                harness
                    .read(|editor| assert!(editor.document.visit.canvas.viewport.zoom.is_none()));
                let png = harness.render().unwrap();
                save_capture(&png, &format!("fullscreen-{layout}-{density}x-offscreen"));
                harness.get_by_label("Exit fullscreen").click();
                harness.step();
                harness.read(|editor| {
                    assert!(
                        editor.fullscreen.is_some(),
                        "window restoration is asynchronous"
                    )
                });
                window_size(&mut harness, windowed_size);
                harness.update(|editor| editor.fullscreen_changed(false));
                harness.try_run_realtime().unwrap();
                harness.read(|editor| assert!(editor.fullscreen.is_none()));
                assert!(harness.query_by_label("Scopes").is_some());
                harness.read(|editor| assert_eq!(editor.document.visit.canvas.viewport, viewport));
                harness.read(|editor| assert_eq!(editor.preferences, preferences));
                harness.read(|editor| assert_eq!(editor.document.state.project, project));
                harness.read(|editor| {
                    assert_eq!((editor.document.source, editor.source_exposure), source)
                });
                harness.read(|editor| {
                    assert_eq!(
                        editor.active_canvas().last_layout.unwrap().0.canvas,
                        workspace
                    )
                });
            });
        }
    }
}

#[test]
fn fullscreen_shortcuts_respect_numeric_popup_and_tool_ownership() {
    tools::calibrated_harness(|mut harness| {
        native(&mut harness, false);
        stage(&mut harness, "Photo");
        harness
            .get_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure")
            .scroll_to_me();
        harness.try_run_realtime().unwrap();
        harness
            .get_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure")
            .click();
        harness.step();
        assert!(
            harness
                .get_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure")
                .accesskit_node()
                .is_focused()
        );
        key(&mut harness, egui::Key::F);
        harness.read(|editor| assert!(editor.fullscreen.is_none()));
        key(&mut harness, egui::Key::Escape);
        let draft = tools::crop_draft(&mut harness);
        let project = harness.read(|editor| editor.document.state.project.clone());
        key(&mut harness, egui::Key::F);
        harness.run();
        reveal(&mut harness);
        harness.read(|editor| assert!(editor.fullscreen.is_some()));
        assert!(harness.query_by_label("Apply").is_none());
        assert!(harness.query_by_label("Cancel").is_none());
        key(&mut harness, egui::Key::Enter);
        harness.read(|editor| {
            assert_eq!(
                editor.document.visit.crop_edit.as_ref().unwrap().draft,
                draft
            )
        });
        harness.get_by_label("Viewer help").click();
        harness.step();
        key(&mut harness, egui::Key::F);
        harness.read(|editor| assert!(editor.fullscreen.is_some()));
        key(&mut harness, egui::Key::Escape);
        harness.read(|editor| assert!(editor.fullscreen.is_some()));
        key(&mut harness, egui::Key::Escape);
        native(&mut harness, false);
        harness.update(|editor| editor.fullscreen_changed(false));
        harness.run();
        harness.read(|editor| assert!(editor.fullscreen.is_none()));
        harness.read(|editor| {
            assert_eq!(
                editor.document.visit.crop_edit.as_ref().unwrap().draft,
                draft
            )
        });
        harness.read(|editor| assert_eq!(editor.document.state.project, project));
    });
}

#[test]
fn fullscreen_overlay_autohides_reveals_and_retains_editing_and_keyboard_access() {
    tools::calibrated_harness(|mut harness| {
        native(&mut harness, false);
        harness.step();
        let zoom = harness.get_by_label("Zoom in").rect();
        let fullscreen = harness.get_by_label("Fullscreen viewing").rect();
        let source = harness.get_by_label("View source").rect();
        assert!(zoom.right() <= fullscreen.left() && fullscreen.right() <= source.left());
        let project = harness.read(|editor| editor.document.state.project.clone());
        key(&mut harness, egui::Key::F);
        reveal(&mut harness);
        harness.event(egui::Event::PointerMoved(egui::pos2(400., 250.)));
        harness.step();
        elapse(&mut harness, 3.);
        assert!(harness.query_by_label("Exit fullscreen").is_none());
        harness.read(|editor| assert!(editor.fullscreen.is_some()));
        let png = harness.render().unwrap();
        save_capture(&png, "fullscreen-hidden-offscreen");
        harness.event(egui::Event::PointerMoved(egui::pos2(420., 270.)));
        harness.step();
        assert!(
            harness.query_by_label("Exit fullscreen").is_none(),
            "panning-area movement does not reveal chrome"
        );
        reveal(&mut harness);
        harness.get_by_label("View source").click();
        harness.step();
        harness.get_by_label("Source inspection").click();
        harness.step();
        harness.event(egui::Event::PointerMoved(egui::pos2(400., 250.)));
        harness.step();
        elapse(&mut harness, 3.);
        assert!(
            harness.query_by_label("Exit fullscreen").is_some(),
            "an open inspector keeps its anchor visible"
        );
        workflow::enter_numeric(&mut harness, "Exposure", "2.123456789");
        elapse(&mut harness, 3.);
        assert!(harness.query_by_label("Exit fullscreen").is_some());
        harness.read(|editor| assert_eq!(editor.source_exposure, 2.123456789));
        key(&mut harness, egui::Key::Escape);
        elapse(&mut harness, 3.);
        assert!(harness.query_by_label("Exit fullscreen").is_none());
        key(&mut harness, egui::Key::Tab);
        assert!(
            harness.query_by_label("Exit fullscreen").is_some(),
            "Tab reveals keyboard controls"
        );
        harness.get_by_label("Fit").focus();
        harness.step();
        elapse(&mut harness, 3.);
        assert!(harness.get_by_label("Fit").is_focused());
        key(&mut harness, egui::Key::F);
        harness.update(|editor| editor.fullscreen_changed(false));
        harness.step();
        harness.read(|editor| assert!(editor.fullscreen.is_none()));
        harness.read(|editor| assert_eq!(editor.document.state.project, project));
    });
}
