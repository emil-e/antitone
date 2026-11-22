use super::*;
use crate::editor::{
    export::{Draft, Scope},
    history::Direction,
};

fn fixture(names: &[&str]) -> (Editor, tempfile::TempDir) {
    let directory = tempfile::tempdir().unwrap();
    let raw = crate::raw::fixture::bayer_dng();
    let paths: Vec<_> = names
        .iter()
        .map(|name| {
            let path = directory.path().join(name);
            std::fs::copy(raw.path(), &path).unwrap();
            path
        })
        .collect();
    let mut editor = crate::editor::tests::fixture();
    editor.document.import(&paths).unwrap();
    (editor, directory)
}

#[test]
fn property_sorts_are_stable_and_keep_undated_photos_last_in_both_directions() {
    let (mut editor, _directory) =
        fixture(&["frame10.dng", "Frame2.dng", "frame3.dng", "frame1.dng"]);
    let [a, b, c, d] = editor.document.project.order[..] else {
        panic!("four photos")
    };
    for (id, timestamp) in [(b, 20250102090000), (d, 20250101090000)] {
        editor.document.project.source_mut(id).unwrap().capture_time =
            Some(timestamp.try_into().unwrap());
    }
    let project = &editor.document.project;
    assert_eq!(Sort::NameAscending.ids(project).as_ref(), [d, b, c, a]);
    assert_eq!(Sort::NameDescending.ids(project).as_ref(), [a, c, b, d]);
    assert_eq!(Sort::CaptureOldest.ids(project).as_ref(), [d, b, a, c]);
    assert_eq!(Sort::CaptureNewest.ids(project).as_ref(), [b, d, a, c]);
    assert_eq!(project.order, [a, b, c, d]);
}

#[test]
fn dropping_a_selected_block_uses_display_order_and_history_restores_the_saved_sequence() {
    let (mut editor, directory) =
        fixture(&["frame10.dng", "frame2.dng", "frame3.dng", "frame1.dng"]);
    let [a, b, c, d] = editor.document.project.order[..] else {
        panic!("four photos")
    };
    editor.document.visit.current_source = Some(a);
    editor.document.roll_order.sort = Sort::NameAscending;
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.select_roll_photo(0, egui::Modifiers::COMMAND, cx);
            let visit = editor.visit_lifetime();
            assert_eq!(editor.export_ids(Scope::All), [d, b, c, a]);
            assert_eq!(editor.export_ids(Scope::Selected), [d, a]);
            let mut draft = Draft::new(OutputTarget::Srgb, directory.path().to_owned());
            draft.scope = Scope::All;
            let export = editor.capture_export(&draft).unwrap();
            editor.document.roll_order.drag = Some(Drag::new(
                editor.document.displayed_order().into_owned(),
                d,
                &editor.document.roll_selection,
                String::new(),
                None,
            ));
            editor.drop_photo_order(2);
            assert_eq!(editor.document.roll_order.sort, Sort::Custom);
            assert_eq!(editor.document.project.order, [b, d, a, c]);
            assert_eq!(editor.export_ids(Scope::Selected), [d, a]);
            assert_eq!(editor.document.visit.current_source, Some(a));
            assert!(visit.upgrade().is_some());
            let names: Vec<_> = export
                .photos
                .iter()
                .map(|photo| photo.path.file_stem().unwrap().to_str().unwrap())
                .collect();
            assert_eq!(names, ["frame1", "frame2", "frame3", "frame10"]);
            assert_eq!(
                editor.history_label(Direction::Undo),
                Some("Reorder photos")
            );

            editor.document.roll_order.sort = Sort::NameDescending;
            editor.restore_history(Direction::Undo, cx);
            assert_eq!(editor.document.project.order, [a, b, c, d]);
            assert_eq!(editor.document.roll_order.sort, Sort::Custom);
            assert_eq!(editor.document.visit.current_source, Some(a));
            assert_eq!(editor.export_ids(Scope::Selected), [a, d]);

            editor.restore_history(Direction::Redo, cx);
            assert_eq!(editor.document.project.order, [b, d, a, c]);
            assert_eq!(editor.document.roll_order.sort, Sort::Custom);
        });
    });
}

#[test]
fn dropping_at_the_same_position_does_not_replace_custom_or_switch_view() {
    let (mut editor, _directory) = fixture(&["frame10.dng", "frame2.dng"]);
    let [a, b] = editor.document.project.order[..] else {
        panic!("two photos")
    };
    editor.document.roll_order.sort = Sort::NameAscending;
    editor.document.roll_order.drag = Some(Drag::new(
        vec![b, a],
        b,
        &editor.document.roll_selection,
        String::new(),
        None,
    ));
    editor.drop_photo_order(0);
    assert_eq!(editor.document.project.order, [a, b]);
    assert_eq!(editor.document.roll_order.sort, Sort::NameAscending);
    assert_eq!(editor.history_label(Direction::Undo), None);
}
