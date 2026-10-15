use super::*;

fn fixture(index: usize) -> (Editor, mpsc::Sender<Message>, mpsc::Receiver<Command>) {
    let (mut editor, send, commands) = crate::editor::tests::fixture();
    for index in 0..5 {
        editor
            .document
            .register(
                Source {
                    id: SourceId(0),
                    path: format!("photo-{index}.ARW").into(),
                    development: crate::project::SourceDevelopment {
                        dimensions: [900, 600],
                        active_area: [0, 0, 900, 600],
                        camera_model: "test-camera".into(),
                        camera_to_ap1: glam::DMat3::IDENTITY.to_cols_array_2d(),
                        input_floor: crate::processing::CAMERA_INPUT_FLOOR,
                    },
                    reference_only: false,
                    photo: PhotoSettings::default(),
                    local_adjustments: None,
                    orientation: PhotoOrientation::default(),
                    crop: None,
                },
                editor.document.project.development.clone().unwrap(),
            )
            .unwrap();
    }
    editor.current_source = Some(editor.document.project.order[index]);
    editor.selected = index;
    editor.roll_selection.only(editor.current_source.unwrap());
    (editor, send, commands)
}

fn settle(editor: &mut Editor, send: &mpsc::Sender<Message>, commands: &mpsc::Receiver<Command>) {
    if editor.busy {
        assert!(matches!(commands.try_recv().unwrap(), Command::Load(..)));
        send.send(Message::Ready(
            editor.id,
            Err("fixture RAW unavailable".into()),
        ))
        .unwrap();
        editor.receive();
    }
}

#[test]
fn plain_command_and_shift_keep_active_selected_and_last_plain_anchor() {
    let (mut editor, send, commands) = fixture(2);
    editor.sync_document();
    let before = editor.document.project.clone();
    let ids = editor.document.project.order.clone();
    editor.select_roll_photo(4, egui::Modifiers::COMMAND);
    assert_eq!(editor.roll_selection.ids, vec![ids[2], ids[4]]);
    assert_eq!(editor.current_source, Some(ids[2]));
    assert!(commands.try_recv().is_err());
    editor.select_roll_photo(2, egui::Modifiers::COMMAND);
    settle(&mut editor, &send, &commands);
    assert_eq!(editor.current_source, Some(ids[4]));
    assert_eq!(editor.roll_selection.anchor, Some(ids[2]));
    editor.select_roll_photo(4, egui::Modifiers::COMMAND);
    assert_eq!(editor.current_source, Some(ids[4]));
    assert_eq!(editor.roll_selection.ids, vec![ids[4]]);
    assert!(
        commands.try_recv().is_err(),
        "sole selection cannot be deselected"
    );
    editor.select_roll_photo(0, egui::Modifiers::SHIFT);
    settle(&mut editor, &send, &commands);
    assert_eq!(editor.roll_selection.ids, ids[..=2]);
    assert_eq!(
        editor.current_source,
        Some(ids[2]),
        "range restores the last plain anchor as active"
    );
    editor.select_roll_photo(1, egui::Modifiers::NONE);
    settle(&mut editor, &send, &commands);
    assert_eq!(editor.current_source, Some(ids[1]));
    assert_eq!(
        editor.roll_selection.ids,
        ids[..=2],
        "activating a selected photo retains membership"
    );
    assert_eq!(editor.roll_selection.anchor, Some(ids[1]));
    editor.select_roll_photo(4, egui::Modifiers::SHIFT);
    assert_eq!(editor.roll_selection.ids, ids[1..]);
    assert_eq!(editor.current_source, Some(ids[1]));
    let request = editor.id;
    editor.select_active_photo_only();
    assert_eq!(editor.roll_selection.ids, vec![ids[1]]);
    assert_eq!(editor.roll_selection.anchor, Some(ids[1]));
    assert_eq!(editor.current_source, Some(ids[1]));
    assert_eq!(editor.id, request);
    assert!(
        commands.try_recv().is_err(),
        "reducing selection does not reload the photo"
    );
    editor.select_roll_photo(0, egui::Modifiers::NONE);
    settle(&mut editor, &send, &commands);
    assert_eq!(editor.current_source, Some(ids[0]));
    assert_eq!(
        editor.roll_selection.ids,
        vec![ids[0]],
        "an unselected photo starts a new selection"
    );
    editor.sync_document();
    assert_eq!(
        editor.document.project, before,
        "selection is not photographic state"
    );
}

#[test]
fn pending_photo_navigation_keeps_multiselection_but_removal_waits_for_completion() {
    let (mut editor, send, commands) = fixture(2);
    let ids = editor.document.project.order.clone();
    editor.select_roll_photo(0, egui::Modifiers::NONE);
    let first = editor.id;
    assert!(matches!(commands.try_recv().unwrap(), Command::Load(..)));
    assert!(editor.busy && editor.selection_timing.is_some());
    editor.select_roll_photo(4, egui::Modifiers::COMMAND);
    assert_eq!(editor.roll_selection.ids, vec![ids[0], ids[4]]);
    assert_eq!(editor.current_source, Some(ids[0]));
    assert!(commands.try_recv().is_err());
    editor.request_removal();
    assert!(editor.removal_pending.is_none());
    editor.select_roll_photo(4, egui::Modifiers::NONE);
    assert!(matches!(commands.try_recv().unwrap(), Command::Load(..)));
    assert_ne!(editor.id, first);
    assert_eq!(
        editor
            .source_epoch
            .load(std::sync::atomic::Ordering::Relaxed),
        editor.id
    );
    assert_eq!(editor.roll_selection.ids, vec![ids[0], ids[4]]);
    assert_eq!(editor.current_source, Some(ids[4]));
    send.send(Message::Ready(first, Err("obsolete source".into())))
        .unwrap();
    editor.receive();
    assert!(editor.busy && editor.current_source == Some(ids[4]));
    send.send(Message::Ready(editor.id, Err("fixture unavailable".into())))
        .unwrap();
    editor.receive();
    editor.request_removal();
    assert_eq!(editor.removal_pending, Some(vec![ids[0], ids[4]]));
}

#[test]
fn deselecting_active_prefers_selected_successor_then_predecessor() {
    let (mut editor, send, commands) = fixture(2);
    let ids = editor.document.project.order.clone();
    editor.select_roll_photo(0, egui::Modifiers::COMMAND);
    editor.select_roll_photo(4, egui::Modifiers::COMMAND);
    editor.select_roll_photo(2, egui::Modifiers::COMMAND);
    settle(&mut editor, &send, &commands);
    assert_eq!(editor.current_source, Some(ids[4]));
    editor.select_roll_photo(4, egui::Modifiers::COMMAND);
    settle(&mut editor, &send, &commands);
    assert_eq!(editor.current_source, Some(ids[0]));
    assert_eq!(editor.roll_selection.ids, vec![ids[0]]);
}

#[test]
fn removal_is_staged_cancellable_and_uses_old_index_in_the_updated_order() {
    for (index, removed, expected) in [(2, vec![0, 2], 4), (4, vec![0, 4], 3), (0, vec![0], 1)] {
        let (mut editor, _, commands) = fixture(index);
        let ids = editor.document.project.order.clone();
        editor.roll_selection.ids = removed.iter().map(|index| ids[*index]).collect();
        editor.sync_document();
        let before = editor.document.project.clone();
        editor.request_removal();
        assert_eq!(editor.document.project, before);
        let selection = editor.roll_selection.ids.clone();
        editor.removal_pending = None;
        assert_eq!(editor.roll_selection.ids, selection);
        assert!(commands.try_recv().is_err());
        editor.request_removal();
        editor.remove_selected_photos();
        assert_eq!(editor.current_source, Some(ids[expected]));
        assert_eq!(
            editor.selected,
            index.min(editor.document.project.order.len() - 1)
        );
        assert_eq!(editor.roll_selection.ids, vec![ids[expected]]);
        assert_eq!(editor.document.project.sources, before.sources);
        let mut expected_project = before;
        expected_project.order.retain(|id| !selection.contains(id));
        assert_eq!(editor.document.project, expected_project);
        assert!(editor.removal_pending.is_none());
        assert!(matches!(commands.try_recv().unwrap(), Command::Load(..)));
    }
}

#[test]
fn removing_every_photo_clears_working_images_and_stale_replies_but_keeps_roll_basis() {
    let (mut editor, send, commands) = fixture(2);
    crate::editor::tests::add_ready_fixture(&mut editor);
    editor.roll_selection.ids = editor.document.project.order.clone();
    editor.sync_document();
    let mut expected = editor.document.project.clone();
    expected.order.clear();
    let old_id = editor.id;
    let ready = editor.ready.take().unwrap();
    editor.request_removal();
    editor.remove_selected_photos();
    assert_eq!(editor.document.project, expected);
    assert!(editor.current_source.is_none());
    assert!(editor.ready.is_none() && editor.pending_preview.is_none());
    assert!(
        editor.original.settled.is_none()
            && editor.original.refining.is_none()
            && editor.original.signature.is_none()
    );
    assert!(editor.roll_selection.ids.is_empty());
    assert_eq!(
        editor
            .source_epoch
            .load(std::sync::atomic::Ordering::Relaxed),
        editor.id
    );
    assert!(editor.analysis_area_pending.is_none());
    assert!(matches!(commands.try_recv().unwrap(), Command::Unload));
    send.send(Message::Ready(old_id, Ok(Box::new(ready))))
        .unwrap();
    editor.receive();
    assert!(editor.ready.is_none() && editor.current_source.is_none());
    editor.sync_document();
    assert_eq!(editor.document.project, expected);
    editor.document.validate().unwrap();
    let path = std::env::temp_dir().join(format!(
        "antitone-empty-removal-{}.json",
        std::process::id()
    ));
    let saved = editor
        .document
        .save(&path, crate::project::Save::New)
        .unwrap();
    assert!(!saved.dirty());
    let reopened = Document::open(&path).unwrap();
    assert_eq!(reopened.project, saved.project);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn picker_browsing_preserves_multiselection_and_anchor() {
    let (mut editor, send, commands) = fixture(2);
    crate::editor::tests::add_ready_fixture(&mut editor);
    editor.selected = editor.document.project.order.len() - 1;
    let active = editor.current_source.unwrap();
    editor.roll_selection.only(active);
    editor.select_roll_photo(0, egui::Modifiers::COMMAND);
    let selection = editor.roll_selection.ids.clone();
    let anchor = editor.roll_selection.anchor;
    editor.begin_pick(Pick::Dmin);
    editor.pick_roll_source(editor.document.project.order[0]);
    settle(&mut editor, &send, &commands);
    assert_eq!(editor.roll_selection.ids, selection);
    assert_eq!(editor.roll_selection.anchor, anchor);
    editor.request_removal();
    assert!(editor.removal_pending.is_none());
    editor.finish_pick();
    settle(&mut editor, &send, &commands);
    assert_eq!(editor.current_source, Some(active));
    assert_eq!(editor.roll_selection.ids, selection);
    assert_eq!(editor.roll_selection.anchor, anchor);
}

#[test]
fn drafts_foreground_work_and_decisions_reject_selection_and_removal() {
    for blocker in [
        "busy",
        "pick",
        "failure",
        "transition",
        "analysis",
        "removal",
    ] {
        let (mut editor, _, commands) = fixture(2);
        editor
            .roll_selection
            .ids
            .push(editor.document.project.order[4]);
        match blocker {
            "busy" => editor.busy = true,
            "pick" => editor.pick = Some(Pick::Dmax),
            "failure" => editor.report_failure(FailureKind::Save, "fixture".into()),
            "transition" => editor.transition = Some(Transition::Close),
            "analysis" => editor.document_pending = Some(12),
            "removal" => editor.removal_pending = Some(vec![editor.current_source.unwrap()]),
            _ => unreachable!(),
        }
        let before = editor.document.project.clone();
        let selection = editor.roll_selection.ids.clone();
        let pending = editor.removal_pending.clone();
        editor.select_roll_photo(0, egui::Modifiers::NONE);
        editor.select_active_photo_only();
        editor.request_removal();
        assert_eq!(editor.document.project, before);
        assert_eq!(editor.roll_selection.ids, selection);
        assert_eq!(editor.removal_pending, pending);
        assert!(commands.try_recv().is_err());
    }
}
