use super::*;
use crate::platform::menu::Action;

fn ask_remove(h: &mut Harness<'_, Editor>, name: &str) {
    h.get_by_label(name).click_secondary();
    h.step();
    h.step();
    let remove = h.get_by_label("Remove selected photos…");
    assert!(!remove.accesskit_node().is_disabled());
    remove.click();
    h.step();
    wait_for(h, |editor| editor.dialogs.is_open());
}

#[test]
fn add_preserves_a_changed_crop_and_context_removal_cancels_then_retires_only_on_confirmation() {
    for size in [egui::vec2(1100., 800.), egui::vec2(800., 600.)] {
        workflow::calibrated_workflow(|mut h, _, directory| {
            h.set_size(size);
            h.try_run_realtime().unwrap();
            let draft = tools::crop_draft(&mut h);
            let (source, view, original) = h.read(|editor| {
                (
                    editor.document.visit.current_source.unwrap(),
                    editor.visit_lifetime(),
                    editor.document.state.project.clone(),
                )
            });
            let added = directory.path().join("added.dng");
            std::fs::copy(fixture("roll-1.dng"), &added).unwrap();
            h.enter(|editor, cx| {
                assert!(editor.application_menu_state().enabled(Action::AddScans));
                // Supply the native chooser result; the production importer still runs.
                editor.add_raws(vec![added], cx);
            });
            wait_for(&mut h, |editor| editor.document.importing.is_none());
            h.try_run_realtime().unwrap();
            h.read(|editor| {
                assert_eq!(editor.document.visit.current_source, Some(source));
                assert!(view.upgrade().is_some());
                assert_eq!(editor.document.crop_edit().unwrap().draft, draft);
                assert_eq!(
                    editor.document.project.order.len(),
                    original.order.len() + 1
                );
                for id in &original.order {
                    assert_eq!(
                        editor.document.project.source(*id).unwrap(),
                        original.source(*id).unwrap()
                    );
                }
            });
            let imported = h.read(|editor| editor.document.state.project.clone());
            ask_remove(&mut h, "working-photo.dng");
            h.try_run_realtime().unwrap();
            save_capture(
                &h.render().unwrap(),
                if size.x > 1000. {
                    "crop-remove-normal"
                } else {
                    "crop-remove-narrow"
                },
            );
            {
                let cancel: Vec<_> = h
                    .query_all_by_role_and_label(egui::accesskit::Role::Button, "Cancel")
                    .filter(|node| !node.accesskit_node().is_disabled())
                    .collect();
                assert_eq!(cancel.len(), 1, "the removal decision owns Cancel");
                cancel[0].click();
            }
            h.step();
            wait_for(&mut h, |editor| !editor.dialogs.is_open());
            h.read(|editor| {
                assert_eq!(editor.document.state.project, imported);
                assert_eq!(editor.document.crop_edit().unwrap().draft, draft);
                assert!(view.upgrade().is_some());
            });
            ask_remove(&mut h, "working-photo.dng");
            h.get_by_label("Remove photos").click();
            h.step();
            wait_for(&mut h, |editor| {
                editor.document.project.source(source).is_err()
                    && !editor.document.visit.source.pending()
            });
            h.read(|editor| {
                assert!(!editor.has_hosted_tool());
                assert_ne!(editor.document.visit.current_source, Some(source));
                assert_eq!(
                    editor.document.project.order.len(),
                    imported.order.len() - 1
                );
                for id in imported.order.iter().copied().filter(|id| *id != source) {
                    assert_eq!(
                        editor.document.project.source(id).unwrap(),
                        imported.source(id).unwrap()
                    );
                }
                assert_eq!(editor.document.project.output, imported.output);
                assert_eq!(editor.document.project.roll, imported.roll);
                assert_eq!(
                    editor.history_label(crate::editor::history::Direction::Undo),
                    Some("Remove photos")
                );
            });
        });
    }
}
