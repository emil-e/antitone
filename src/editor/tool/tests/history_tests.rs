use super::*;

#[test]
fn history_keeps_document_and_photo_tools_installed_without_synthetic_actions() {
    for placement in [Placement::Document, Placement::Photo] {
        let mut editor = crate::editor::tests::fixture();
        let before = editor.document.state.project.clone();
        let mut edit = editor.edit_project("Exposure").unwrap();
        edit.update(|project| project.roll.exposure = 0.75);
        editor.commit_project(edit).unwrap();
        let after = editor.document.state.project.clone();
        let dropped = Rc::new(Cell::new(0));
        let actions = Rc::new(Cell::new(0));
        editor
            .document
            .set_hosted_tool(placement, Box::new(Probe::new(&dropped, &actions)));
        for (direction, expected) in [
            (history::Direction::Undo, &before),
            (history::Direction::Redo, &after),
        ] {
            editor.restore_history(direction, &UiContext::render_only());
            assert_eq!(&editor.document.state.project, expected);
            assert!(editor.current_tool().unwrap().is::<Probe>());
            assert_eq!(dropped.get(), 0);
            assert_eq!(actions.get(), 0);
        }
        editor.end_tool();
        assert_eq!(dropped.get(), 1);
    }
}
