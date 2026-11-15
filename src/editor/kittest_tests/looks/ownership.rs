use super::*;
use crate::editor::look::testing::Gate;
use crate::platform::menu::Action;

#[test]
fn held_lut_preparation_keeps_native_snapshots_history_browsing_and_tools_available() {
    for size in [egui::vec2(1100., 800.), egui::vec2(800., 600.)] {
        fixture_editor(|mut h, _| {
            h.set_size(size);
            h.try_run_realtime().unwrap();
            let selected = constant_asset(0.7);
            h.enter(|editor, cx| editor.select_look(look::Source::Loaded(selected.clone()), cx));
            paint_until(&mut h, positive_ready);
            choose(&mut h, "None");
            paint_until(&mut h, positive_ready);
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("held-lut.antitone");
            h.enter(|editor, cx| {
                editor.save_project(path.clone(), crate::project::Save::New, None, cx)
            });
            wait_for(&mut h, |editor| {
                !editor.document.write.pending() && !editor.files.pending()
            });
            stage(&mut h, "Photo");
            workflow::enter_numeric(&mut h, "Exposure", "0.5");
            stage(&mut h, "Rendering");
            let gate = h.update(Gate::install);
            choose(&mut h, selected.name());
            let call = gate.next(&mut h.application);
            assert!(call.valid);
            h.read(|editor| {
                assert!(editor.document.lut.pending.is_some());
                assert!(!editor.workspace_input.is_blocked());
                assert!(!editor.editing());
            });
            save_existing_project(&mut h);
            assert!(
                crate::project::Project::load(&path)
                    .unwrap()
                    .output
                    .lut
                    .is_none()
            );
            native_file_action(&mut h, Action::Export);
            h.update(|editor| {
                let form = editor.document.export.form.as_mut().unwrap();
                form.draft.folder = directory.path().to_owned();
                form.draft.suffix = "_pending".into();
            });
            h.run_steps(3);
            let count = h.read(|editor| {
                editor
                    .document
                    .project
                    .order
                    .iter()
                    .filter(|id| editor.document.roll_selection.contains(**id))
                    .count()
            });
            let label = format!(
                "Export {count} {}",
                if count == 1 { "photo" } else { "photos" }
            );
            h.get_by_label(&label).click();
            h.step();
            h.application.run_until(Duration::from_secs(30), |editor| {
                editor.document.export.operation.is_none()
            });
            h.run_steps(3);
            h.read(|editor| {
                assert_eq!(
                    editor.document.export.summary.as_ref().unwrap().exported(),
                    count
                );
                assert!(editor.document.lut.pending.is_some());
                assert!(editor.document.project.output.lut.is_none());
            });
            h.get_by_label("OK").click();
            h.step();
            h.get_by_role_and_label(egui::accesskit::Role::Tab, "Photo")
                .click();
            h.run_steps(3);
            h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure")
                .scroll_to_me();
            h.run_steps(3);
            h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure")
                .click();
            h.step();
            h.event(egui::Event::Text("0.75".into()));
            h.key_press(egui::Key::Enter);
            h.step();
            native_file_action(&mut h, Action::Undo);
            h.read(|editor| {
                assert_eq!(editor.photo_settings().exposure, 0.5);
                assert!(editor.document.lut.pending.is_some());
            });
            native_file_action(&mut h, Action::Redo);
            h.read(|editor| assert_eq!(editor.photo_settings().exposure, 0.75));
            let other = h.read(|editor| {
                editor
                    .document
                    .project
                    .order
                    .iter()
                    .copied()
                    .find(|id| Some(*id) != editor.document.visit.current_source)
                    .unwrap()
            });
            let name = h.read(|editor| {
                editor
                    .document
                    .source_path(other)
                    .unwrap()
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            });
            h.get_by_label(&name).click();
            h.step();
            wait_for(&mut h, |editor| {
                editor.document.visit.current_source == Some(other)
                    && !editor.document.visit.source.pending()
            });
            h.get_by_label("Crop & straighten").scroll_to_me();
            h.run_steps(3);
            h.get_by_label("Crop & straighten").click();
            h.step();
            h.read(|editor| {
                assert!(editor.cropping());
                assert!(editor.document.lut.pending.is_some());
            });
            save_capture(
                &h.render().unwrap(),
                if size.x > 1000. {
                    "held-lut-with-crop-normal"
                } else {
                    "held-lut-with-crop-narrow"
                },
            );
            assert!(!call.cancelled.is_cancelled());
            call.finish();
            wait_for(&mut h, |editor| editor.document.lut.pending.is_none());
            h.read(|editor| {
                assert!(editor.cropping());
                assert_eq!(editor.document.visit.current_source, Some(other));
                assert_eq!(editor.document.project.output.lut.as_ref(), Some(&selected));
                assert!(editor.document.dirty());
                assert!(editor.dialogs.message().is_none());
            });
            save_existing_project(&mut h);
            h.read(|editor| assert!(editor.cropping()));
            assert_eq!(
                crate::project::Project::load(&path)
                    .unwrap()
                    .output
                    .lut
                    .as_ref(),
                Some(&selected)
            );
            h.get_by_label("Cancel").click();
            h.step();
        });
    }
}
