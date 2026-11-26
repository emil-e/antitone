use super::*;
use egui::accesskit::Role;

#[test]
fn lightbox_compares_live_photo_grades_and_enters_canvas_tools_in_photo() {
    let directory = tempfile::tempdir().unwrap();
    let paths = (1..=15)
        .map(|number| {
            let path = directory.path().join(format!("Frame {number:02}.dng"));
            std::fs::copy(fixture("patches.dng"), &path).unwrap();
            path
        })
        .collect();
    editor_harness_with_thumbnails(true, |mut h, _| {
        h.set_size(egui::vec2(1100., 800.));
        h.update(|e| {
            e.memory_pressure = memory::Pressure::Normal;
            e.set_cache_limits(cache_limits::CacheLimits {
                cpu_bytes: 256 << 20,
                gpu_bytes: 256 << 20,
            });
        });
        h.run_steps(3);
        for name in ["Single photo", "Lightbox"] {
            assert!(
                h.get_by_role_and_label(Role::RadioButton, name)
                    .accesskit_node()
                    .is_disabled()
            );
        }
        h.enter(|e, cx| e.add_raws(paths, cx));
        wait_opened_photo(&mut h);
        use crate::platform::menu::Action;
        h.read(|e| assert!(!e.input().ready()));
        h.update(host::App::native_menu_installed);
        h.step();
        assert!(
            !h.get_by_role_and_label(Role::RadioButton, "Lightbox")
                .accesskit_node()
                .is_disabled()
        );
        h.key_press(egui::Key::G);
        h.step();
        wait_for(&mut h, |e| {
            e.showing_lightbox()
                && e.document
                    .thumbnails
                    .ready_count(&e.document, e.document.target())
                    > 0
        });
        assert!(
            tile_rect(&h, "Frame 01.dng").bottom() < h.get_by_label("Sort photos").rect().top()
        );
        h.read(|e| {
            let menu = e.application_menu_state();
            assert_eq!(menu.checked(Action::Lightbox), Some(true));
            assert_eq!(menu.checked(Action::SinglePhoto), Some(false));
        });
        let width = tile_rect(&h, "Frame 01.dng").width();
        assert!(
            h.get_by_role_and_label(Role::Button, "Larger tiles")
                .accesskit_node()
                .is_disabled()
        );
        for _ in 0..2 {
            h.get_by_role_and_label(Role::Button, "Smaller tiles")
                .click();
            h.run_steps(3);
        }
        assert!(tile_rect(&h, "Frame 01.dng").width() < width);
        let order = h.read(|e| e.document.project.order.clone());
        h.get_all_by_role_and_label(Role::Button, "Frame 01.dng")
            .min_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
            .unwrap()
            .click();
        h.step();
        h.key_press(egui::Key::ArrowDown);
        h.run_steps(3);
        h.read(|e| assert_eq!(e.document.visit.current_source, Some(order[3])));
        h.key_press(egui::Key::ArrowRight);
        h.run_steps(3);
        h.read(|e| assert_eq!(e.document.visit.current_source, Some(order[4])));
        h.key_press(egui::Key::ArrowUp);
        h.run_steps(3);
        h.read(|e| assert_eq!(e.document.visit.current_source, Some(order[1])));
        for index in [4, 7, 10, 13] {
            h.key_press(egui::Key::ArrowDown);
            h.run_steps(3);
            h.read(|e| assert_eq!(e.document.visit.current_source, Some(order[index])));
        }
        let strip_top = h.get_by_label("Sort photos").rect().top();
        assert!(
            h.get_all_by_role_and_label(Role::Button, "Frame 14.dng")
                .any(|tile| tile.rect().top() >= strip_top)
        );

        h.key_press(egui::Key::E);
        h.step();
        wait_for(&mut h, |e| {
            !e.showing_lightbox() && e.canvas_frame.layout.is_some()
        });
        h.read(|e| {
            assert_eq!(
                e.application_menu_state().checked(Action::SinglePhoto),
                Some(true)
            )
        });
        h.update(|e| {
            e.set_dmin(Some(DVec3::splat(3000. / 4031.)));
            e.set_dmax(Some(DVec3::splat(128. / 4031.)));
            e.document.source = false;
        });
        h.enter(|e, cx| e.select_roll_photo(14, egui::Modifiers::NONE, cx));
        wait_opened_photo(&mut h);
        h.get_by_role_and_label(Role::Tab, "Photo").click();
        h.step();
        wait_for(&mut h, thumbnails_ready);
        let (order, project) = h.read(|e| {
            assert!(e.canvas_frame.displayed.is_some());
            (
                e.document.project.order.clone(),
                e.document.state.project.clone(),
            )
        });

        h.get_by_role_and_label(Role::RadioButton, "Lightbox")
            .click();
        h.step();
        wait_for(&mut h, |e| {
            e.showing_lightbox()
                && e.document.thumbnails.visible_order().first() == Some(&order[14])
        });
        let strip = h.get_by_role_and_label(Role::Button, "Sort photos").rect();
        assert!(tile_rect(&h, "Frame 15.dng").bottom() < strip.top());
        assert!(
            h.get_all_by_role_and_label(Role::Button, "Frame 15.dng")
                .any(|node| node.rect().top() >= strip.top())
        );
        h.read(|e| {
            assert!(e.canvas_frame.displayed.is_none());
            assert!(
                !e.document.thumbnails.processed.has_pending(),
                "warm switch queued render work"
            );
            assert_eq!(e.document.state.project, project);
            let demand = e.document.thumbnails.visible_order();
            assert!(demand.contains(&order[14]));
            let unique: std::collections::HashSet<_> = demand.iter().collect();
            assert_eq!(demand.len(), unique.len());
        });

        h.get_all_by_role_and_label(Role::Button, "Frame 14.dng")
            .min_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
            .expect("visible Lightbox photo")
            .click();
        h.step();
        wait_for(&mut h, |e| {
            e.document.visit.current_source == Some(order[13])
                && e.document.visit.canvas.ready.is_some()
                && !e.document.visit.source.pending()
        });
        let active = tile_rect(&h, "Frame 14.dng").center();
        let neighbor = tile_rect(&h, "Frame 15.dng").center();
        let before = render_grid(&mut h);

        workflow::enter_numeric(&mut h, "Exposure", "1.25");
        wait_for(&mut h, |e| {
            e.photo_settings().exposure == 1.25 && thumbnails_ready(e)
        });
        let after = render_grid(&mut h);
        assert!(brightness(&after, active) > brightness(&before, active) + 8.);
        assert_eq!(brightness(&after, neighbor), brightness(&before, neighbor));
        save_capture(&after, "lightbox");
        h.read(|e| {
            assert!(e.showing_lightbox());
            assert!(e.canvas_frame.displayed.is_none());
            assert_eq!(e.document.project.roll, project.roll);
            assert_eq!(
                e.document.thumbnails.visible_order().first(),
                Some(&order[13])
            );
        });

        let exposure = h.get_by_role_and_label(Role::SpinButton, "Exposure");
        exposure.click();
        h.step();
        assert!(
            h.get_by_role_and_label(Role::SpinButton, "Exposure")
                .accesskit_node()
                .is_focused()
        );
        h.key_press(egui::Key::E);
        h.step();
        h.read(|e| assert!(e.showing_lightbox()));
        h.key_press(egui::Key::Escape);
        h.step();

        let crop = h.get_by_role_and_label(Role::Button, "Crop & straighten");
        assert!(!crop.accesskit_node().is_disabled());
        crop.click();
        h.step();
        wait_for(&mut h, |e| e.cropping() && e.canvas_frame.layout.is_some());
        h.key_press(egui::Key::G);
        h.step();
        h.read(|e| assert!(e.cropping() && !e.showing_lightbox()));
        h.get_by_role_and_label(Role::Button, "Cancel").click();
        h.step();
        h.read(|e| {
            assert!(!e.showing_lightbox() && !e.cropping());
            assert_eq!(e.photo_settings().exposure, 1.25);
        });

        for filmstrip in [true, false] {
            h.key_press(egui::Key::G);
            // Keep the two gestures outside egui's triple-click window.
            h.run_steps(40);
            h.read(|e| assert!(e.showing_lightbox()));
            let strip_top = h.get_by_label("Sort photos").rect().top();
            for _ in 0..2 {
                h.get_all_by_role_and_label(Role::Button, "Frame 14.dng")
                    .find(|tile| (tile.rect().top() >= strip_top) == filmstrip)
                    .expect("active photo visible on both surfaces")
                    .click();
                h.step();
            }
            h.run_steps(2);
            h.read(|e| {
                assert!(
                    !e.showing_lightbox(),
                    "double click did not open Single Photo: filmstrip={filmstrip}"
                );
                assert_eq!(e.document.visit.current_source, Some(order[13]));
            });
        }
    });
}

#[test]
fn switching_views_uses_the_selected_toolbar_and_only_photo_handles_zoom() {
    editor_harness(|mut h, _| {
        h.enter(|editor, cx| editor.add_raws(vec![fixture("patches.dng")], cx));
        wait_opened_photo(&mut h);
        wait_for(&mut h, |editor| editor.canvas_frame.layout.is_some());

        for fullscreen in [false, true] {
            if fullscreen {
                h.get_by_role_and_label(Role::Button, "Fullscreen viewing")
                    .click();
                h.step();
                h.read(|editor| assert!(editor.fullscreen.is_some()));
                let canvas = h.ctx.content_rect();
                h.hover_at(egui::pos2(canvas.center().x, canvas.bottom() - 30.));
                h.step();
            }
            let fullscreen_label = if fullscreen {
                "Exit fullscreen"
            } else {
                "Fullscreen viewing"
            };
            h.get_by_role_and_label(Role::Button, fullscreen_label);
            h.get_by_role_and_label(Role::Button, "Fit");
            assert!(h.query_by_label("Smaller tiles").is_none());

            h.key_press(egui::Key::Z);
            h.step();
            let zoomed = h.read(|editor| {
                let viewport = editor.active_canvas().viewport;
                assert_eq!(viewport.zoom, Some(1.));
                viewport
            });
            h.get_by_role_and_label(Role::RadioButton, "Lightbox")
                .click();
            h.step();
            assert!(h.query_by_label("Fit").is_none());
            assert!(h.query_by_label("Viewer help").is_none());
            h.get_by_role_and_label(Role::Button, "Smaller tiles");
            h.get_by_role_and_label(Role::Button, fullscreen_label);

            h.key_press(egui::Key::Z);
            h.step();
            h.read(|editor| {
                assert!(editor.showing_lightbox());
                assert!(editor.canvas_frame.layout.is_none());
                assert_eq!(editor.active_canvas().viewport, zoomed);
            });

            h.get_by_role_and_label(Role::RadioButton, "Single photo")
                .click();
            h.step();
            assert!(h.query_by_label("Smaller tiles").is_none());
            h.get_by_role_and_label(Role::Button, "Fit");
            h.key_press(egui::Key::Z);
            h.step();
            h.read(|editor| {
                assert!(!editor.showing_lightbox());
                assert!(editor.active_canvas().viewport.zoom.is_none());
                assert_eq!(editor.fullscreen.is_some(), fullscreen);
            });
        }
    });
}

fn thumbnails_ready(editor: &Editor) -> bool {
    editor
        .document
        .thumbnails
        .ready_count(&editor.document, editor.document.target())
        == editor.document.project.order.len()
}

fn tile_rect(h: &Harness<'_, Editor>, name: &str) -> egui::Rect {
    h.get_all_by_role_and_label(Role::Button, name)
        .map(|tile| tile.rect())
        .min_by(|a, b| a.top().total_cmp(&b.top()))
        .expect("visible tile")
}

fn render_grid(h: &mut Harness<'_, Editor>) -> image::RgbaImage {
    h.event(egui::Event::PointerGone);
    h.step();
    h.render().unwrap()
}

fn brightness(png: &image::RgbaImage, at: egui::Pos2) -> f32 {
    let (x, y) = (at.x as u32, at.y as u32);
    let rows = (y - 8)..(y + 8);
    let columns = (x - 8)..(x + 8);
    rows.flat_map(|y| {
        columns
            .clone()
            .map(move |x| f32::from(png.get_pixel(x, y)[1]))
    })
    .sum::<f32>()
        / 256.
}
