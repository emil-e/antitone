use super::*;

#[test]
fn indeterminate_highlight_never_paints_outside_the_pill_at_either_end() {
    editor_harness(|editor, _| {
        let (device, queue, submissions) = editor.read(|editor| {
            let services = editor.source_services.as_ref().unwrap();
            (
                services.device.clone(),
                services.queue.clone(),
                host::App::submission_gate(editor).clone(),
            )
        });
        for width in [280., 800.] {
            let renderer = FloatRenderer::new(
                &device,
                &queue,
                submissions.clone(),
                Rc::new(RefCell::new(FloatCapture::default())),
            );
            let mut harness = EguiHarness::builder()
                .with_size(egui::vec2(width, 160.))
                .renderer(renderer)
                .build_ui_state(
                    |ui, (time, highlight): &mut (f64, bool)| {
                        ui.input_mut(|input| input.time = *time);
                        ui.ctx().global_style_mut(|style| {
                            style.visuals.extreme_bg_color = egui::Color32::BLACK;
                            style.visuals.selection.bg_fill = if *highlight {
                                egui::Color32::from_rgb(240, 120, 30)
                            } else {
                                egui::Color32::TRANSPARENT
                            };
                        });
                        presentation::ProgressContent {
                            title: "Working…",
                            progress: presentation::Progress::Indeterminate,
                            cancel: None,
                        }
                        .modal(ui.ctx(), egui::Id::new("pill-coverage"));
                    },
                    (20., false),
                );
            harness.run_steps(3);
            // Finish the area's entrance fade before taking a track-only control.
            harness.state_mut().0 = 25.;
            harness.step();
            let control = harness.render().unwrap();
            let track = harness
                .get_by_role_and_label(egui::accesskit::Role::ProgressIndicator, "Working…")
                .rect();
            assert!(
                egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 160.))
                    .contains_rect(track)
            );
            let radius = track.height() / 2.;
            let mut changed_inside = false;
            for phase in [0., 0.05, 0.1, 0.19, 0.25, 0.5, 0.75, 0.81, 0.9, 0.95, 1.] {
                *harness.state_mut() = (25. + phase / 0.8, true);
                harness.step();
                let image = harness.render().unwrap();
                for y in track.top().floor() as u32..track.bottom().ceil() as u32 {
                    for x in track.left().floor() as u32..track.right().ceil() as u32 {
                        let point = egui::pos2(x as f32 + 0.5, y as f32 + 0.5);
                        let nearest = egui::pos2(
                            point.x.clamp(track.left() + radius, track.right() - radius),
                            track.center().y,
                        );
                        let distance = point.distance(nearest) - radius;
                        let before = control.get_pixel(x, y);
                        let after = image.get_pixel(x, y);
                        let changed = (0..3).any(|c| before[c].abs_diff(after[c]) > 1);
                        if distance > 1. {
                            // Allow the usual one-pixel AA fringe, not rectangular
                            // scissor corners filled by the animated highlight.
                            assert!(
                                !changed,
                                "highlight escaped the pill: width={width} phase={phase} pixel=({x},{y}) distance={distance} control={before:?} actual={after:?}"
                            );
                        } else if distance < 0. && changed {
                            changed_inside = true;
                        }
                    }
                }
                if phase == 0. || phase == 0.5 {
                    save_capture(
                        &image,
                        &format!(
                            "progress-pill-{}-{}-offscreen",
                            width as u32,
                            if phase == 0. { "left" } else { "right" }
                        ),
                    );
                }
            }
            assert!(
                changed_inside,
                "control must distinguish visible highlight from the track-only frame"
            );
        }
    });
}
