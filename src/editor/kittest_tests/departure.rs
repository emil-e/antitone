use super::*;
use crate::platform::menu::Action;

fn save_baseline(h: &mut Harness<'_, Editor>, directory: &std::path::Path) -> PathBuf {
    let path = directory.join("departure.antitone");
    h.enter(|editor, cx| editor.save_project(path.clone(), crate::project::Save::New, None, cx));
    wait_for(h, |editor| {
        !editor.document.files.pending() && !editor.document.write.pending()
    });
    h.read(|editor| assert!(!editor.document.dirty(), "{}", editor.status));
    path
}

#[test]
fn native_new_keeps_a_changed_crop_on_cancel_then_saves_only_accepted_changes_before_departure() {
    for size in [egui::vec2(1100., 800.), egui::vec2(800., 600.)] {
        workflow::calibrated_workflow(|mut h, _, directory| {
            h.set_size(size);
            h.try_run_realtime().unwrap();
            let path = save_baseline(&mut h, directory.path());
            workflow::enter_numeric(&mut h, "Exposure", "0.6");
            let accepted = h.read(|editor| editor.document.state.project.clone());
            let draft = tools::crop_draft(&mut h);
            h.read(|editor| {
                assert!(editor.document.dirty());
                assert!(!editor.editing());
                assert!(editor.application_menu_state().enabled(Action::New));
            });
            native_file_action(&mut h, Action::New);
            wait_for(&mut h, |editor| editor.dialogs.is_open());
            h.try_run_realtime().unwrap();
            save_capture(
                &h.render().unwrap(),
                if size.x > 1000. {
                    "crop-dirty-new-normal"
                } else {
                    "crop-dirty-new-narrow"
                },
            );
            {
                let cancel: Vec<_> = h
                    .query_all_by_role_and_label(egui::accesskit::Role::Button, "Cancel")
                    .filter(|node| !node.accesskit_node().is_disabled())
                    .collect();
                assert_eq!(cancel.len(), 1, "only the decision's Cancel is enabled");
                cancel[0].click();
            }
            h.step();
            wait_for(&mut h, |editor| !editor.document.files.pending());
            h.read(|editor| {
                assert_eq!(editor.document.state.project, accepted);
                assert_eq!(editor.document.crop_edit().unwrap().draft, draft);
                assert!(!editor.allow_close);
            });
            native_file_action(&mut h, Action::New);
            wait_for(&mut h, |editor| editor.dialogs.is_open());
            h.get_by_label("Save changes").click();
            h.step();
            wait_for(&mut h, |editor| !editor.document.files.pending());
            h.read(|editor| {
                assert!(editor.document.project.sources.is_empty());
                assert!(!editor.has_hosted_tool());
                assert!(!editor.document.dirty() && !editor.dialogs.is_open());
                assert_eq!(
                    editor.history_label(crate::editor::history::Direction::Undo),
                    None
                );
            });
            assert_eq!(crate::project::Project::load(&path).unwrap(), accepted);
        });
    }
}

#[test]
fn system_quit_with_a_clean_project_and_armed_mask_needs_no_extra_draft_prompt() {
    for size in [egui::vec2(1100., 800.), egui::vec2(800., 600.)] {
        workflow::calibrated_workflow(|mut h, _, directory| {
            h.set_size(size);
            h.try_run_realtime().unwrap();
            let path = save_baseline(&mut h, directory.path());
            let accepted = h.read(|editor| editor.document.state.project.clone());
            h.get_by_label("Add radial").scroll_to_me();
            h.try_run_realtime().unwrap();
            h.get_by_label("Add radial").click();
            h.step();
            h.read(|editor| {
                assert!(editor.creating_mask());
                assert!(!editor.document.dirty() && !editor.editing());
            });
            h.update(host::App::request_quit);
            h.step();
            h.read(|editor| {
                assert_eq!(
                    host::App::quit_state(editor),
                    Some(host::QuitState::Approved)
                );
                assert!(editor.allow_close);
                assert!(!editor.dialogs.is_open() && !editor.document.files.pending());
                assert!(!editor.document.write.pending());
                assert_eq!(editor.document.state.project, accepted);
            });
            h.application.close_execution();
            assert_eq!(crate::project::Project::load(&path).unwrap(), accepted);
        });
    }
}
