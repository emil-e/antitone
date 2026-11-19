use super::*;

#[test]
fn precision_rendering_progresses_while_modal_input_is_reserved() {
    editor_harness(|mut h, _| {
        h.update(|editor| editor.document.project.input = InputSettings::Digital);
        h.enter(|editor, cx| editor.add_raws(vec![fixture("patches.dng")], cx));
        wait_for(&mut h, |editor| {
            !editor.interaction_blocked() && editor.active_canvas().ready.is_some()
        });
        h.enter(|editor, cx| {
            editor.begin_test_picker(super::super::super::tests::PickerCase::Wb, cx)
        });
        wait_for(&mut h, |editor| {
            editor.active_canvas().original.wanted.is_some()
        });
        h.update(|editor| {
            editor.active_canvas_mut().original.invalidate();
            editor.set_test_busy(true);
        });
        h.step();
        wait_for(&mut h, |editor| {
            let original = &editor.active_canvas().original;
            original.request().is_some() || original.task.is_some()
        });
        h.step();
        h.read(|editor| {
            assert!(editor.interaction_blocked());
            let original = &editor.active_canvas().original;
            assert!(
                original.task.is_some() || original.settled().is_some(),
                "input reservation does not prevent precision rendering"
            );
        });
        wait_for(&mut h, |editor| {
            editor.active_canvas().original.settled().is_some()
        });
        h.update(|editor| {
            assert!(editor.interaction_blocked());
            let rendered = editor.original_rendered().unwrap().unwrap();
            assert!(Arc::ptr_eq(
                &rendered,
                &editor.canvas_frame.displayed.as_ref().unwrap().view
            ));
            editor.set_test_busy(false);
        });
    });
}
