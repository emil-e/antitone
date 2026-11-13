use super::*;
use crate::platform::menu::Action;

fn save_initial(h: &mut Harness<'_, Editor>, directory: &std::path::Path) {
    let path = directory.join("snapshots.antitone");
    h.enter(|editor, cx| editor.save_project(path.clone(), crate::project::Save::New, None, cx));
    wait_for(h, |editor| {
        !editor.document.write.pending() && !editor.files.pending()
    });
    assert!(path.exists());
    h.read(|editor| assert!(!editor.document.dirty(), "{}", editor.status));
}

#[test]
fn native_save_and_export_capture_accepted_crop_without_closing_or_applying_the_draft() {
    for size in [egui::vec2(1100., 800.), egui::vec2(800., 600.)] {
        workflow::calibrated_workflow(|mut h, _, directory| {
            h.set_size(size);
            h.try_run_realtime().unwrap();
            save_initial(&mut h, directory.path());
            let accepted = h.read(|editor| editor.document.state.project.clone());
            let source = h.read(|editor| editor.document.visit.current_source.unwrap());
            let area = h.read(|editor| editor.selected_area().unwrap());
            let draft = tools::crop_draft(&mut h);
            assert_ne!(draft.size, [area.width, area.height]);
            save_existing_project(&mut h);
            h.read(|editor| {
                assert_eq!(editor.document.crop_edit().unwrap().draft, draft);
                assert_eq!(editor.document.state.project, accepted);
            });
            native_file_action(&mut h, Action::Export);
            h.update(|editor| {
                let form = editor
                    .document
                    .export
                    .form
                    .as_mut()
                    .expect("Export while Crop owns a private draft");
                form.draft.folder = directory.path().to_owned();
                form.draft.kind = crate::editor::export::Kind::Exr;
                form.draft.precision = ExrPrecision::Full;
            });
            h.try_run_realtime().unwrap();
            save_capture(
                &h.render().unwrap(),
                if size.x > 1000. {
                    "crop-snapshot-export-normal"
                } else {
                    "crop-snapshot-export-narrow"
                },
            );
            h.get_by_label("Export 1 photo").click();
            h.step();
            wait_for(&mut h, |editor| editor.document.export.operation.is_none());
            h.read(|editor| {
                assert_eq!(editor.document.crop_edit().unwrap().draft, draft);
                assert_eq!(editor.document.state.project, accepted);
                assert_eq!(
                    editor.document.export.summary.as_ref().unwrap().exported(),
                    1,
                    "{}",
                    editor.status
                );
            });
            let image = exr::prelude::read_first_flat_layer_from_file(
                directory.path().join("working-photo.exr"),
            )
            .unwrap();
            assert_eq!(
                [image.layer_data.size.0, image.layer_data.size.1],
                [area.width as usize, area.height as usize]
            );
            h.get_by_label("OK").click();
            h.step();
            h.get_by_label("Apply").click();
            h.step();
            h.read(|editor| {
                assert!(editor.document.crop_edit().is_none());
                assert_eq!(
                    editor.document.project.source(source).unwrap().crop,
                    Some(draft)
                );
                assert!(editor.document.dirty());
                let written =
                    crate::project::Project::load(editor.document.path.as_ref().unwrap()).unwrap();
                assert_eq!(written, accepted);
            });
        });
    }
}

#[test]
fn save_and_cancelled_export_leave_armed_mask_creation_ready_for_an_ordinary_drag() {
    for size in [egui::vec2(1100., 800.), egui::vec2(800., 600.)] {
        workflow::calibrated_workflow(|mut h, _, directory| {
            h.set_size(size);
            h.try_run_realtime().unwrap();
            save_initial(&mut h, directory.path());
            let accepted = h.read(|editor| editor.document.state.project.clone());
            h.get_by_label("Add radial").scroll_to_me();
            h.try_run_realtime().unwrap();
            h.get_by_label("Add radial").click();
            h.step();
            h.read(|editor| assert!(editor.creating_mask() && !editor.mask_dragging()));
            save_existing_project(&mut h);
            h.read(|editor| assert!(editor.creating_mask()));
            native_file_action(&mut h, Action::Export);
            h.read(|editor| assert!(editor.document.export.form.is_some()));
            h.key_press(egui::Key::Escape);
            h.step();
            h.read(|editor| {
                assert!(editor.document.export.form.is_none());
                assert!(editor.creating_mask());
                assert_eq!(editor.document.state.project, accepted);
            });
            let image = h.read(|editor| editor.active_canvas().last_layout.unwrap().0.image);
            drag(
                &mut h,
                image.center(),
                image.center() + egui::vec2(35., 25.),
            );
            h.read(|editor| {
                assert!(!editor.masking(), "{}", editor.status);
                let source = editor.document.visit.current_source.unwrap();
                assert_eq!(
                    editor
                        .document
                        .project
                        .source(source)
                        .unwrap()
                        .adjustment_layers
                        .as_ref()
                        .unwrap()
                        .layers
                        .len(),
                    2
                );
                assert!(editor.document.dirty());
                assert_eq!(
                    crate::project::Project::load(editor.document.path.as_ref().unwrap()).unwrap(),
                    accepted
                );
            });
        });
    }
}
