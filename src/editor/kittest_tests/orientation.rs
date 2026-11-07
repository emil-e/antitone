use super::*;

#[test]
fn pending_preview_pixels_follow_turn_and_flip_buttons_before_source_completion() {
    editor_harness_with_gpu(true, |mut h, _, device, queue| {
        h.update(|editor| {
            editor
                .document
                .thumbnails
                .set_limits(cache_limits::CacheLimits {
                    cpu_bytes: 64 << 20,
                    gpu_bytes: 64 << 20,
                });
        });
        h.enter(|editor, cx| editor.add_raws(vec![fixture("patches.dng")], cx));
        wait_for(&mut h, |editor| {
            !editor.interaction_blocked() && editor.document.visit.canvas.ready.is_some()
        });
        stage(&mut h, "Photo");
        let admission = crate::background::testing::pause();
        h.enter(|editor, cx| editor.load(fixture("patches.dng"), cx));
        h.step();
        h.render().unwrap();
        let read_preview = |editor: &Editor| {
            assert!(editor.document.visit.source.pending());
            assert!(editor.document.visit.canvas.ready.is_none());
            let displayed = &editor.canvas_frame.displayed.as_ref().unwrap().view;
            let (_, _, preview) = editor
                .document
                .visit
                .canvas
                .preview_rendered
                .as_ref()
                .unwrap();
            assert!(Arc::ptr_eq(displayed, preview));
            let image = &preview.image;
            (
                [
                    image.linear.width() as usize,
                    image.linear.height() as usize,
                ],
                crate::viewer::readback::image_pixels(&device, &queue, image),
            )
        };
        let (mut size, mut pixels) = h.read(read_preview);
        assert_ne!(size[0], size[1], "turns must change the fixture's aspect");
        assert!(
            pixels
                .iter()
                .any(|pixel| (pixel[0] - pixels[0][0]).abs() > 0.1),
            "the fixture must distinguish transformed pixels from a no-op"
        );
        for label in [
            "Turn right",
            "Turn left",
            "Flip horizontal",
            "Flip vertical",
        ] {
            let role = egui::accesskit::Role::Button;
            h.get_by_role_and_label(role, label).scroll_to_me();
            h.step();
            let button = h.get_by_role_and_label(role, label);
            assert!(!button.accesskit_node().is_disabled());
            button.click();
            h.step();
            h.step();
            h.render().unwrap();
            let (next_size, next_pixels) = h.read(read_preview);
            let [width, height] = size;
            let expected_size = if label.starts_with("Turn") {
                [height, width]
            } else {
                size
            };
            assert_eq!(next_size, expected_size, "{label}");
            for y in 0..height {
                for x in 0..width {
                    // Exact displayed-axis permutations, independent of production geometry.
                    let [next_x, next_y] = match label {
                        "Turn right" => [height - 1 - y, x],
                        "Turn left" => [y, width - 1 - x],
                        "Flip horizontal" => [width - 1 - x, y],
                        "Flip vertical" => [x, height - 1 - y],
                        _ => unreachable!(),
                    };
                    let expected = pixels[y * width + x];
                    let actual = next_pixels[next_y * next_size[0] + next_x];
                    for channel in 0..4 {
                        assert!(
                            (actual[channel] - expected[channel]).abs() < 3e-5,
                            "{label} at {x},{y}: {actual:?} vs {expected:?}"
                        );
                    }
                }
            }
            size = next_size;
            pixels = next_pixels;
        }
        drop(admission);
        wait_for(&mut h, |editor| !editor.document.visit.source.pending());
        h.read(|editor| {
            let orientation = editor.photo_orientation();
            assert!(orientation.flips.horizontal && orientation.flips.vertical);
            assert_eq!(
                editor
                    .document
                    .project
                    .source(editor.document.visit.current_source.unwrap())
                    .unwrap()
                    .orientation,
                orientation
            );
        });
    });
}
