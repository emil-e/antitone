use super::*;
use crate::{crop::Crop, editor::tests::PickerCase, platform::menu::Action};

#[test]
fn import_failure_success_and_duplicates_preserve_the_active_visit_and_private_tool_or_mask() {
    for kind in 0..3 {
        let directory = tempfile::tempdir().unwrap();
        let first = copy_photo(directory.path(), "first.dng");
        let second = copy_photo(directory.path(), "second.dng");
        let bad = directory.path().join("broken.dng");
        std::fs::write(&bad, b"not a RAW").unwrap();
        let editor = authored_fixture(&[first.to_str().unwrap()]);
        let editor = if kind == 2 {
            crate::editor::tests::accepted_input(editor, crate::processing::InputSettings::Digital)
        } else {
            editor
        };
        with_application(editor, |app| {
            let (view, source, accepted, crop, region) = app.enter(|editor, cx| {
                let area = editor.selected_area().unwrap();
                let crop = Crop::from_region(crate::raw::Rect {
                    x: area.x + 2,
                    y: area.y + 2,
                    width: area.width - 4,
                    height: area.height - 4,
                });
                let region = Coverage::rectangle(crate::raw::Rect {
                    x: area.x + 4,
                    y: area.y + 4,
                    width: 8,
                    height: 8,
                });
                match kind {
                    0 => {
                        editor.begin_crop();
                        editor.document.crop_edit_mut().unwrap().draft = crop;
                    }
                    1 => editor.begin_test_picker(PickerCase::Dmin, cx),
                    _ => {
                        editor.begin_radial_mask(None);
                        assert!(editor.creating_mask());
                    }
                }
                (
                    editor.document.visit.view,
                    editor.document.visit.current_source.unwrap(),
                    editor.document.state.project.clone(),
                    crop,
                    region,
                )
            });
            if kind == 1 {
                app.run_until(Duration::from_secs(5), |editor| {
                    editor.test_pick().is_some()
                });
                app.update(|editor| editor.set_test_pick_region(Some(region)));
            }
            for (paths, count) in [
                (vec![second.clone(), bad.clone()], 1),
                (vec![second.clone()], 2),
                (vec![second.clone()], 2),
            ] {
                let history = app
                    .read(|editor| editor.history_label(crate::editor::history::Direction::Undo));
                let previous_count = app.read(|editor| editor.document.project.order.len());
                app.enter(|editor, cx| {
                    assert!(editor.application_menu_state().enabled(Action::AddScans));
                    editor.add_raws(paths, cx);
                });
                wait(app);
                app.read(|editor| {
                    assert_eq!(editor.document.project.order.len(), count);
                    assert_eq!(editor.document.visit.view, view);
                    assert_eq!(editor.document.visit.current_source, Some(source));
                    assert_eq!(
                        editor.document.project.source(source).unwrap(),
                        accepted.source(source).unwrap()
                    );
                    assert!(!editor.editing());
                    match kind {
                        0 => assert_eq!(editor.document.crop_edit().unwrap().draft, crop),
                        1 => assert_eq!(editor.test_pick_region(), Some(region)),
                        _ => assert!(editor.creating_mask()),
                    }
                    if previous_count == count {
                        assert_eq!(
                            editor.history_label(crate::editor::history::Direction::Undo),
                            history
                        );
                    }
                });
            }
        });
    }
}

#[test]
fn import_that_actually_changes_interpretation_retires_the_dependent_draft() {
    let directory = tempfile::tempdir().unwrap();
    let first = copy_photo(directory.path(), "first.dng");
    let second = directory.path().join("other-camera.dng");
    let mut bytes = std::fs::read(&first).unwrap();
    let locations: Vec<_> = bytes
        .windows(b"Authored Bayer patches".len())
        .enumerate()
        .filter_map(|(i, bytes)| (bytes == b"Authored Bayer patches").then_some(i))
        .collect();
    assert_eq!(locations.len(), 2, "authored Model and UniqueCameraModel");
    for i in locations {
        bytes[i..i + b"Modified Bayer patches".len()].copy_from_slice(b"Modified Bayer patches");
    }
    std::fs::write(&second, bytes).unwrap();
    let mut editor = authored_fixture(&[first.to_str().unwrap()]);
    let selected = editor.document.visit.current_source;
    let view = editor.document.visit.view;
    assert!(editor.document.project.input.is_negative());
    editor.begin_crop();
    assert!(editor.cropping());
    with_application(editor, |app| {
        app.enter(|editor, cx| editor.add_raws(vec![second], cx));
        wait(app);
        app.read(|editor| {
            assert_eq!(editor.document.project.order.len(), 2);
            assert_eq!(
                editor.document.project.input,
                crate::processing::InputSettings::Digital
            );
            assert_eq!(editor.document.visit.current_source, selected);
            assert_ne!(editor.document.visit.view, view);
            assert!(!editor.has_hosted_tool());
            assert_eq!(
                editor.history_label(crate::editor::history::Direction::Undo),
                Some("Import photos")
            );
        });
    });
}
