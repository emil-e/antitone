use super::*;
use crate::editor::{export::Scope, history::Direction, roll_order::Sort};

fn primary(h: &mut Harness<'_, Editor>, pos: egui::Pos2, pressed: bool) {
    h.event(egui::Event::PointerMoved(pos));
    h.event(egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    });
    h.step();
}

#[test]
fn filmstrip_sort_controls_navigation_and_reordering() {
    let directory = tempfile::tempdir().unwrap();
    let paths: Vec<_> = ["frame10.dng", "frame2.dng", "frame3.dng", "frame1.dng"]
        .map(|name| {
            let path = directory.path().join(name);
            std::fs::copy(fixture("illuminated-patches.dng"), &path).unwrap();
            path
        })
        .into();
    editor_harness_with_thumbnails(true, |mut h, _| {
        h.enter(|editor, cx| editor.add_raws(paths, cx));
        wait_for(&mut h, |e| {
            !e.interaction_blocked() && !e.document.visit.source.pending()
        });
        let (ids, visit) = h.read(|e| (e.document.project.order.clone(), e.visit_lifetime()));
        let [a, b, c, d] = ids[..] else {
            panic!("four imported photos")
        };
        let sort = h.get_by_label("Sort photos").rect();
        assert!(sort.right() < h.get_by_label("frame10.dng").rect().left());
        h.get_by_label("Sort photos").click();
        h.run_steps(3);
        assert!(
            h.get_by_label("Oldest first")
                .accesskit_node()
                .is_disabled()
        );
        h.get_by_label("A to Z").click();
        h.step();
        h.step();
        h.read(|e| {
            assert_eq!(e.document.project.order, ids);
            assert_eq!(e.export_ids(Scope::All), [d, b, c, a]);
            assert_eq!(e.history_label(Direction::Undo), Some("Import photos"));
        });
        assert!(visit.upgrade().is_some());
        assert!(
            h.get_by_label("frame2.dng").rect().left()
                < h.get_by_label("frame10.dng").rect().left()
        );

        h.get_by_label("100%").click();
        h.step();
        h.get_by_label("frame10.dng").click();
        h.step();
        assert!(h.get_by_label("frame10.dng").is_focused());
        h.read(|e| assert!(e.active_canvas().viewport.zoom.is_some()));
        h.key_press(egui::Key::Z);
        h.step();
        h.read(|e| assert!(e.active_canvas().viewport.zoom.is_none()));

        h.key_press(egui::Key::ArrowLeft);
        h.step();
        h.read(|e| assert_eq!(e.document.visit.current_source, Some(c)));
        h.get_by_label("frame2.dng").click();
        h.step();
        h.get_by_label("frame10.dng")
            .click_modifiers(egui::Modifiers::SHIFT);
        h.step();
        h.read(|e| {
            assert_eq!(e.document.visit.current_source, Some(b));
            assert_eq!(e.export_ids(Scope::Selected), [b, c, a]);
        });

        stage(&mut h, "Photo");
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Crop & straighten")
            .scroll_to_me();
        h.try_run_realtime().unwrap();
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Crop & straighten")
            .click();
        h.step();
        h.read(|e| assert!(e.cropping()));

        let start = h.get_by_label("frame1.dng").rect().center();
        let target = h.get_by_label("frame10.dng").rect().left_center() - egui::vec2(4., 0.);
        primary(&mut h, start, true);
        h.event(egui::Event::PointerMoved(target));
        h.step();
        h.read(|e| assert!(e.document.roll_order.drag.is_some()));
        h.key_press(egui::Key::Escape);
        h.step();
        h.read(|e| {
            assert!(e.document.roll_order.drag.is_none());
            assert!(e.cropping(), "Escape belongs to the reorder, not Crop");
        });
        primary(&mut h, target, false);
        h.read(|e| {
            assert_eq!(e.document.project.order, ids);
            assert_eq!(e.document.roll_order.sort, Sort::NameAscending);
        });
        h.key_press(egui::Key::Escape);
        h.step();
        h.read(|e| assert!(!e.cropping(), "the next Escape dismisses Crop"));

        primary(&mut h, start, true);
        h.event(egui::Event::PointerMoved(target));
        h.step();
        primary(&mut h, target, false);
        h.read(|e| {
            assert_eq!(e.document.project.order, [b, c, d, a]);
            assert_eq!(e.document.roll_order.sort, Sort::Custom);
            assert_eq!(e.document.visit.current_source, Some(b));
            assert_eq!(e.export_ids(Scope::Selected), [b, c, a]);
            assert_eq!(e.history_label(Direction::Undo), Some("Reorder photos"));
        });
        let path = directory.path().join("roll.antitone");
        h.read(|e| e.document.save(&path, crate::project::Save::New).unwrap());
        assert_eq!(
            crate::project::Project::load(&path).unwrap().order,
            [b, c, d, a]
        );
        h.enter(|e, cx| e.restore_history(Direction::Undo, cx));
        h.read(|e| {
            assert_eq!(e.document.project.order, ids);
            assert_eq!(e.document.roll_order.sort, Sort::Custom);
            assert_eq!(e.document.visit.current_source, Some(b));
        });
    });
}

#[test]
fn capture_sort_and_edge_scroll_use_the_same_displayed_ids() {
    let directory = tempfile::tempdir().unwrap();
    let older = crate::raw::fixture::bayer_dng_with_exif_date(0x9003, b"2024:02:29 15:07:18\0");
    let newer = crate::raw::fixture::bayer_dng_with_exif_date(0x9003, b"2025:03:01 13:50:39\0");
    let paths: Vec<_> = (0..20)
        .map(|index| {
            let path = directory.path().join(format!("frame-{index}.dng"));
            let raw = match index {
                0 => older.path().to_owned(),
                10 => newer.path().to_owned(),
                _ => fixture("illuminated-patches.dng"),
            };
            std::fs::copy(raw, &path).unwrap();
            path
        })
        .collect();
    editor_harness_with_thumbnails(true, |mut h, _| {
        h.set_size(egui::vec2(820., 650.));
        h.enter(|editor, cx| editor.add_raws(paths, cx));
        wait_for(&mut h, |e| {
            !e.interaction_blocked() && !e.document.visit.source.pending()
        });
        let custom = h.read(|e| e.document.project.order.clone());
        h.get_by_label("Sort photos").click();
        h.run_steps(3);
        h.get_by_label("Newest first").click();
        h.run_steps(3);
        h.read(|e| {
            let shown = e.export_ids(Scope::All);
            assert_eq!(&shown[..2], [custom[10], custom[0]]);
            assert_eq!(&shown[2..11], &custom[1..10]);
            assert_eq!(&shown[11..], &custom[11..]);
        });
        let start = h.get_by_label("frame-10.dng").rect().center();
        let before = h.get_by_label("frame-3.dng").rect().left();
        assert!(start.x < before);
        let edge = egui::pos2(h.ctx.content_rect().right() - 24., start.y);
        primary(&mut h, start, true);
        h.event(egui::Event::PointerMoved(edge));
        h.run_steps(24);
        h.read(|e| assert!(e.document.roll_order.drag.is_some()));
        assert!(
            h.get_by_label("frame-3.dng").rect().left() < before,
            "the strip did not scroll"
        );
        primary(&mut h, edge - egui::vec2(0., 180.), false);
        h.read(|e| {
            assert!(e.document.roll_order.drag.is_none());
            assert_eq!(e.document.project.order, custom);
            assert_eq!(e.document.roll_order.sort, Sort::CaptureNewest);
        });

        let start = h.get_by_label("frame-3.dng").rect().center();
        primary(&mut h, start, true);
        h.hover_at(start - egui::vec2(30., 0.));
        h.step();
        h.read(|e| assert!(e.document.roll_order.drag.is_some()));
        native_file_action(&mut h, crate::platform::menu::Action::PhotoInspection);
        h.step();
        h.read(|e| {
            assert!(e.fullscreen_requested());
            assert!(e.document.roll_order.drag.is_none());
            assert_eq!(e.document.project.order, custom);
        });
        primary(&mut h, start, false);
        h.read(|e| assert_eq!(e.document.project.order, custom));
    });
}
