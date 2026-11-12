use super::*;
use futures_lite::future::{block_on, poll_once};

#[test]
fn overlapping_owners_release_only_their_own_blocks() {
    let input = WorkspaceInput::default();
    let observer = input.clone();
    assert!(!observer.is_blocked());

    let first = input.block();
    assert!(observer.is_blocked());
    assert!(!first.is_covered());

    let second = observer.block();
    assert!(first.is_covered());
    assert!(second.is_covered());
    drop(first);
    assert!(input.is_blocked());
    assert!(!second.is_covered());
    drop(second);
    assert!(!observer.is_blocked());
}

#[test]
fn unpolled_and_suspended_scopes_release_on_destruction() {
    block_on(async {
        let input = WorkspaceInput::default();
        for polled in [false, true] {
            let block = input.block();
            let mut scope = Box::pin(async move {
                let _workspace = block;
                std::future::pending::<()>().await;
            });
            assert!(input.is_blocked());
            if polled {
                assert!(poll_once(&mut scope).await.is_none());
            }
            drop(scope);
            assert!(!input.is_blocked());
        }
    });
}

#[test]
fn block_changes_request_repaint_without_borrowing_the_editor() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    let context = egui::Context::default();
    let repaints = Arc::new(AtomicUsize::new(0));
    let wake = repaints.clone();
    context.set_request_repaint_callback(move |_| {
        wake.fetch_add(1, Ordering::Relaxed);
    });
    let owner = RefCell::new(WorkspaceInput::default());
    owner.borrow().set_context(&context);
    let input = owner.borrow().clone();
    let _borrow = owner.borrow_mut();
    let drain_repaints = || {
        for _ in 0..4 {
            let mut output = context.run_ui(Default::default(), |_| {});
            output.textures_delta.clear();
        }
    };

    drain_repaints();
    let before = repaints.load(Ordering::Relaxed);
    let block = input.block();
    assert!(repaints.load(Ordering::Relaxed) > before);

    drain_repaints();
    let before = repaints.load(Ordering::Relaxed);
    drop(block);
    assert!(repaints.load(Ordering::Relaxed) > before);
    assert!(!input.is_blocked());
}

#[test]
fn activities_and_queued_dialogs_share_input_but_reporters_do_not_own_it() {
    use crate::editor::{ActivityOptions, Editor, activity};

    fn assert_send<T: Send>() {}
    assert_send::<activity::Reporter>();

    let mut editor = Editor::state();
    let local = editor
        .activities
        .start(ActivityOptions::new("Local feedback"));
    assert!(!editor.workspace_input.is_blocked());

    let modal = editor
        .activities
        .start(ActivityOptions::new("Modal feedback").modal());
    let reporter = modal.reporter();
    let snapshot = editor.activities.view().unwrap();
    assert!(editor.workspace_input.is_blocked());
    drop(editor.dialogs.show(|_, _, _| Some(())));
    drop(editor.dialogs.show(|_, _, _| Some(())));
    drop(modal);
    assert!(reporter.view().is_none());
    assert!(snapshot.modal);
    assert!(editor.workspace_input.is_blocked());

    let context = egui::Context::default();
    editor.dialogs.paint(&context, false);
    assert!(editor.workspace_input.is_blocked());
    editor.dialogs.paint(&context, false);
    assert!(!editor.workspace_input.is_blocked());
    assert_eq!(editor.activities.view().unwrap().title, "Local feedback");
    drop(local);
}

#[test]
fn an_unidentified_modal_owner_blocks_all_workspace_entry_routes_and_then_releases_them() {
    use crate::{editor::tests::authored_fixture, platform::menu::Action};

    let mut editor = authored_fixture(&["patches.dng"]);
    editor.set_dmin(Some(crate::editor::DVec3::splat(0.8)));
    editor.set_dmax(Some(crate::editor::DVec3::splat(0.02)));
    assert!(editor.document.project.input.ready());
    let available = |editor: &crate::editor::Editor, expected| {
        assert_eq!(editor.tool_start_available(), expected);
        assert_eq!(editor.history_available(), expected);
        assert_eq!(editor.roll_navigation_available(), expected);
        assert_eq!(editor.fullscreen_available(), expected);
        for action in [Action::Save, Action::Settings, Action::Export] {
            assert_eq!(
                editor.application_menu_state().enabled(action),
                expected,
                "{action:?}"
            );
        }
    };
    available(&editor, true);
    let accepted = editor.document.state.project.clone();
    let block = editor.workspace_input.block();
    available(&editor, false);
    editor.begin_crop();
    assert!(!editor.cropping());
    assert_eq!(editor.document.state.project, accepted);

    drop(block);
    available(&editor, true);
    assert!(!editor.cropping(), "refused actions are not replayed");
    editor.begin_crop();
    assert!(editor.cropping(), "a fresh invocation is admitted");
    editor.end_tool();
}

#[test]
fn moving_the_export_form_out_of_its_slot_does_not_unblock_input() {
    let mut editor = crate::editor::Editor::state();
    assert!(editor.tool_start_available());
    editor.open_export_dialog();
    let form = editor.document.export.form.take().unwrap();
    assert!(editor.workspace_input.is_blocked());
    assert!(!editor.tool_start_available());
    drop(form);
    assert!(!editor.workspace_input.is_blocked());
    assert!(editor.tool_start_available());
}
