use super::*;
use crate::looks::{material, monochrome::Paper as MonochromePaper};
mod catalogue;
mod switching;

fn open_component(h: &mut Harness<'_, Editor>, label: &str) {
    h.get_by_role_and_label(egui::accesskit::Role::ComboBox, label)
        .scroll_to_me();
    h.try_run_realtime().unwrap();
    h.get_by_role_and_label(egui::accesskit::Role::ComboBox, label)
        .click();
    h.step();
    h.step();
}

#[test]
fn named_bw_paper_prepares_persists_exports_and_only_admits_supported_material_choices() {
    fixture_editor(|mut h, _| {
        let paper = MonochromePaper::default();
        let before = h.read(|e| e.document.project.clone());
        let input = h.read(|e| e.document.project.input);
        choose_component(&mut h, "Paper", paper.name());
        let selected = h.read(|e| {
            let asset = e.document.project.output.film_and_paper.as_ref().unwrap();
            assert_eq!(MonochromePaper::from_origin(asset.origin()), Some(paper));
            assert_eq!(e.document.project.output.grain, before.output.grain);
            assert_eq!(e.document.project.input, input);
            assert_eq!(e.document.project.output.lut, before.output.lut);
            asset.clone()
        });
        open_component(&mut h, "Negative");
        assert!(
            !h.get_by_label("Kodak Portra 400")
                .accesskit_node()
                .is_disabled()
        );
        h.key_press(egui::Key::Escape);
        h.step();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("ilford-paper.antitone");
        h.enter(|e, cx| e.save_project(path.clone(), crate::project::Save::New, None, cx));
        wait_for(&mut h, |e| !e.interaction_blocked());
        h.read(|e| assert!(!e.dialogs.is_open() && !e.document.dirty()));
        assert_eq!(
            crate::project::Project::load(&path)
                .unwrap()
                .output
                .film_and_paper,
            Some(selected.clone())
        );
        let export = |h: &mut Harness<'_, Editor>, path: PathBuf| {
            h.enter(|e, cx| {
                e.export(
                    path,
                    OutputFormat::Tiff(OutputTarget::Srgb),
                    Overwrite::Refuse,
                    cx,
                )
            });
            wait_for(h, |e| e.document.export.operation.is_none());
            h.read(|e| assert!(e.dialogs.message().is_none()));
            h.get_by_role_and_label(egui::accesskit::Role::Button, "OK")
                .click();
            h.step();
        };
        let treated = directory.path().join("paper.tif");
        export(&mut h, treated.clone());
        choose_component(&mut h, "Paper", "None");
        h.read(|e| assert!(e.document.project.output.film_and_paper.is_none()));
        let bypassed = directory.path().join("bypassed.tif");
        export(&mut h, bypassed.clone());
        assert_ne!(
            tiff_codes(&treated),
            tiff_codes(&bypassed),
            "fixture must distinguish paper from bypass"
        );
        open_project(&mut h, path);
        h.get_by_label("Discard changes").click();
        h.step();
        wait_for(&mut h, |e| {
            !e.interaction_blocked() && e.document.visit.canvas.ready.is_some()
        });
        paint_until(&mut h, positive_ready);
        h.read(|e| {
            assert_eq!(
                e.document.project.output.film_and_paper.as_ref(),
                Some(&selected)
            )
        });
        if let Some(output) = std::env::var_os("ANTITONE_UI_CAPTURE") {
            let folder = PathBuf::from(output);
            std::fs::create_dir_all(&folder).unwrap();
            h.render()
                .unwrap()
                .save(folder.join("ilford-paper.png"))
                .unwrap();
        }
        // Positive control: colour media enable negative stock selection. This
        // rules out a universally disabled Negative menu as an apparent success.
        choose_component(&mut h, "Paper", Paper::default().name());
        choose_component(&mut h, "Negative", "Kodak Portra 400");
        open_component(&mut h, "Paper");
        h.get_by_label(paper.name()).scroll_to_me();
        h.try_run_realtime().unwrap();
        assert!(!h.get_by_label(paper.name()).accesskit_node().is_disabled());
        h.key_press(egui::Key::Escape);
        h.step();
        h.read(|e| {
            let s = material::Selection::from_origin(
                e.document
                    .project
                    .output
                    .film_and_paper
                    .as_ref()
                    .unwrap()
                    .origin(),
            )
            .unwrap();
            assert_eq!(
                s.negative(),
                Negative::from_id("kodak_portra_400").map(Into::into)
            );
        });
    });
}
