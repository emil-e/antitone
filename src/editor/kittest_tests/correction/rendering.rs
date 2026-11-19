use super::*;

#[test]
fn reference_rendering_progresses_while_modal_input_is_reserved() {
    editor_harness(|mut h, _| {
        h.enter(|editor, cx| editor.begin_flat_edit(fixture("bare-light.dng"), None, cx));
        wait_for(&mut h, |editor| !editor.interaction_blocked());
        wait_for(&mut h, |editor| {
            editor.document.flat_edit().unwrap().view.ready.is_some()
        });
        h.update(|editor| {
            editor
                .document
                .flat_edit_mut()
                .unwrap()
                .view
                .test_original_mut()
                .invalidate();
            editor.set_test_busy(true);
        });
        h.step();
        wait_for(&mut h, |editor| {
            let state = editor.document.flat_edit().unwrap();
            let original = state.view.test_original();
            original.request().is_some() || original.task.is_some()
        });
        h.step();
        h.read(|editor| {
            assert!(editor.interaction_blocked());
            let state = editor.document.flat_edit().unwrap();
            let original = state.view.test_original();
            assert!(
                original.task.is_some() || original.settled().is_some(),
                "input reservation does not prevent reference rendering"
            );
        });
        wait_for(&mut h, |editor| {
            editor
                .document
                .flat_edit()
                .unwrap()
                .view
                .test_original()
                .settled()
                .is_some()
        });
        h.update(|editor| {
            assert!(editor.interaction_blocked());
            let rendered = editor
                .document
                .flat_edit_mut()
                .unwrap()
                .view
                .test_original_mut()
                .rendered()
                .unwrap()
                .unwrap();
            assert!(Arc::ptr_eq(
                &rendered,
                &editor.canvas_frame.displayed.as_ref().unwrap().view
            ));
            editor.set_test_busy(false);
        });
    });
}
