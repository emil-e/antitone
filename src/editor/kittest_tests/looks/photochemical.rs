use super::*;
use crate::looks::spektrafilm::{Negative, Paper, Selection};
mod monochrome;
mod supersession;

pub(super) fn choose_component(harness: &mut Harness<'_, Editor>, label: &str, value: &str) {
    harness
        .get_by_role_and_label(egui::accesskit::Role::ComboBox, label)
        .scroll_to_me();
    harness.try_run_realtime().unwrap();
    harness
        .get_by_role_and_label(egui::accesskit::Role::ComboBox, label)
        .click();
    harness.step();
    harness.step();
    harness.get_by_label(value).scroll_to_me();
    harness.try_run_realtime().unwrap();
    harness.get_by_label(value).click();
    harness.step();
    paint_until(harness, positive_ready);
}

#[test]
fn negative_and_paper_select_independently_with_real_generation_and_paper_bypass_admission() {
    fixture_editor(|mut h, _| {
        let input = h.read(|e| e.document.input);
        let texture = h.read(|e| e.document.project.output.grain);
        let original = h.read(|e| e.document.state.project.clone());
        assert!(original.output.film_and_paper.is_none());
        choose_component(&mut h, "Negative", "Kodak Portra 400");
        h.read(|e| {
            let selected = Selection::from_origin(
                e.document
                    .project
                    .output
                    .film_and_paper
                    .as_ref()
                    .unwrap()
                    .origin(),
            )
            .unwrap();
            assert_eq!(selected.negative, Negative::from_id("kodak_portra_400"));
            assert_eq!(selected.paper, Paper::default());
            assert_eq!(e.document.input, input);
        });
        h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Paper")
            .scroll_to_me();
        h.try_run_realtime().unwrap();
        h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Paper")
            .click();
        h.step();
        h.step();
        assert!(h.get_by_label("None").accesskit_node().is_disabled());
        let print_film = Paper::from_id("kodak_2383").unwrap().name();
        h.get_by_label(print_film).scroll_to_me();
        h.try_run_realtime().unwrap();
        h.get_by_label(print_film).click();
        h.step();
        paint_until(&mut h, positive_ready);
        h.read(|e| {
            let s = Selection::from_origin(
                e.document
                    .project
                    .output
                    .film_and_paper
                    .as_ref()
                    .unwrap()
                    .origin(),
            )
            .unwrap();
            assert_eq!(s.negative, Negative::from_id("kodak_portra_400"));
            assert_eq!(s.paper, Paper::from_id("kodak_2383").unwrap());
        });
        choose_component(&mut h, "Negative", "None");
        h.read(|e| {
            let s = Selection::from_origin(
                e.document
                    .project
                    .output
                    .film_and_paper
                    .as_ref()
                    .unwrap()
                    .origin(),
            )
            .unwrap();
            assert_eq!(s.negative, None);
            assert_eq!(s.paper, Paper::from_id("kodak_2383").unwrap());
            assert_eq!(e.document.project.output.grain, texture);
        });
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("paper-only.antitone");
        let accepted = h.read(|e| e.document.state.project.clone());
        h.enter(|e, cx| e.save_project(path.clone(), crate::project::Save::New, None, cx));
        wait_for(&mut h, |e| !e.busy && e.document_pending.is_none());
        h.read(|e| assert!(!e.document.dirty() && e.failure.is_none()));
        let reopened = crate::project::Project::load(&path).unwrap();
        assert_eq!(
            reopened.output.film_and_paper,
            accepted.output.film_and_paper
        );
        let treated = directory.path().join("treated.tif");
        h.enter(|e, cx| {
            e.export(
                treated.clone(),
                OutputFormat::Tiff(OutputTarget::Srgb),
                Overwrite::Refuse,
                cx,
            )
        });
        wait_for(&mut h, |e| e.document.export.operation.is_none());
        h.read(|e| assert!(e.failure.is_none()));
        h.get_by_role_and_label(egui::accesskit::Role::Button, "OK")
            .click();
        h.step();
        choose_component(&mut h, "Paper", "None");
        h.read(|e| assert!(e.document.project.output.film_and_paper.is_none()));
        let bypassed = directory.path().join("bypassed.tif");
        h.enter(|e, cx| {
            e.export(
                bypassed.clone(),
                OutputFormat::Tiff(OutputTarget::Srgb),
                Overwrite::Refuse,
                cx,
            )
        });
        wait_for(&mut h, |e| e.document.export.operation.is_none());
        h.read(|e| assert!(e.failure.is_none()));
        h.get_by_role_and_label(egui::accesskit::Role::Button, "OK")
            .click();
        h.step();
        assert_ne!(tiff_codes(&treated), tiff_codes(&bypassed));
        open_project(&mut h, path);
        h.get_by_label("Discard changes").click();
        h.step();
        wait_for(&mut h, |e| {
            !e.busy && e.document_pending.is_none() && e.document.visit.canvas.ready.is_some()
        });
        paint_until(&mut h, positive_ready);
        h.read(|e| {
            assert_eq!(
                e.document.project.output.film_and_paper,
                accepted.output.film_and_paper
            )
        });
        // Switching interpretation uses production redevelop/admission, not a test-only input mutation.
        h.enter(|e, cx| e.apply_input(crate::processing::InputSettings::Digital, cx));
        wait_for(&mut h, |e| {
            !e.busy && e.document.input == crate::processing::InputSettings::Digital
        });
        paint_until(&mut h, positive_ready);
        choose_component(&mut h, "Paper", Paper::default().name());
        h.read(|e| {
            let s = Selection::from_origin(
                e.document
                    .project
                    .output
                    .film_and_paper
                    .as_ref()
                    .unwrap()
                    .origin(),
            )
            .unwrap();
            assert_eq!(s.negative, None);
            assert_eq!(s.paper, Paper::default());
        });
        if let Some(output) = std::env::var_os("ANTITONE_UI_CAPTURE") {
            let folder = PathBuf::from(output);
            std::fs::create_dir_all(&folder).unwrap();
            h.render()
                .unwrap()
                .save(folder.join("photochemical-selectors.png"))
                .unwrap();
        }
    });
}
