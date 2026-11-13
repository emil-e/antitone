use super::*;
use crate::looks::frontier::{Selection as FrontierSelection, Variant};

#[test]
fn frontier_density_choices_render_export_and_reopen_without_regrading() {
    fixture_editor(|mut h, _| {
        h.enter(|e, _| {
            let mut edit = e.edit_project("Authored grade fixture").unwrap();
            edit.update(|project| {
                project.roll.exposure = 0.35;
                project.roll.wb = crate::color::Balance {
                    warmth: 0.04,
                    tint: -0.02,
                };
                for (index, source) in project.sources.iter_mut().enumerate() {
                    let photo = &mut source.adjustment_layers.as_mut().unwrap().layers[0]
                        .adjustment
                        .photo;
                    photo.exposure = -0.17 + 0.05 * index as f64;
                    photo.wb = crate::color::Balance {
                        warmth: -0.03,
                        tint: 0.02,
                    };
                    photo.contrast = 1.12;
                    photo.saturation = 0.89;
                }
            });
            e.commit_project(edit).unwrap();
        });
        paint_until(&mut h, positive_ready);
        let before = h.read(|e| e.document.state.project.clone());
        let assert_grade = |e: &Editor| {
            assert_eq!(e.document.project.roll, before.roll);
            assert_eq!(e.document.project.sources, before.sources);
        };
        let directory = tempfile::tempdir().unwrap();
        let mut assets = Vec::new();
        for variant in Variant::all() {
            choose_component(&mut h, "Paper", variant.name());
            h.read(|e| {
                assert!(!e.dialogs.is_open());
                let asset = e.document.project.output.film_and_paper.as_ref().unwrap();
                assert_eq!(
                    FrontierSelection::from_origin(asset.origin()),
                    Some(FrontierSelection { variant })
                );
                assert_eq!(asset.lut().size(), crate::looks::authoring::DEFAULT_GRID);
                assert_eq!(e.document.project.input, before.input);
                assert_eq!(e.document.project.output.lut, before.output.lut);
                assert_eq!(e.document.project.output.grain, before.output.grain);
                assert_grade(e);
                assets.push(asset.clone());
            });
            h.event(egui::Event::PointerGone);
            h.step();
            save_capture(
                &h.render().unwrap(),
                match variant {
                    Variant::A => "frontier-a",
                    Variant::B => "frontier-b",
                },
            );
        }
        assert_ne!(assets[0].id(), assets[1].id());
        h.enter(|e, cx| e.restore_history(crate::editor::history::Direction::Undo, cx));
        paint_until(&mut h, positive_ready);
        h.read(|e| {
            assert_eq!(
                e.document.project.output.film_and_paper,
                Some(assets[0].clone())
            );
            assert_grade(e);
        });
        h.enter(|e, cx| e.restore_history(crate::editor::history::Direction::Redo, cx));
        paint_until(&mut h, positive_ready);
        h.read(|e| {
            assert_eq!(
                e.document.project.output.film_and_paper,
                Some(assets[1].clone())
            );
            assert_grade(e);
        });
        choose_component(&mut h, "Negative", "Kodak Portra 400");
        h.read(|e| {
            assert!(e.dialogs.is_open() && e.document.look.is_none());
            assert_eq!(
                e.document.project.output.film_and_paper,
                Some(assets[1].clone())
            );
            assert_grade(e);
        });
        h.key_press(egui::Key::Escape);
        h.step();
        h.read(|e| {
            assert!(
                !e.dialogs.is_open()
                    && e.document.project.output.film_and_paper == Some(assets[1].clone())
            );
            assert_grade(e);
        });
        let path = directory.path().join("frontier.antitone");
        h.enter(|e, cx| e.save_project(path.clone(), crate::project::Save::New, None, cx));
        wait_for(&mut h, |e| !e.interaction_blocked());
        h.read(|e| assert!(!e.dialogs.is_open() && !e.document.dirty()));
        assert_eq!(
            crate::project::Project::load(&path)
                .unwrap()
                .output
                .film_and_paper,
            Some(assets[1].clone())
        );

        let export = |h: &mut Harness<'_, Editor>, name: &str| {
            let output = directory.path().join(name);
            h.enter(|e, cx| {
                e.export(
                    output.clone(),
                    OutputFormat::Tiff(OutputTarget::DisplayP3),
                    Overwrite::Refuse,
                    cx,
                )
            });
            wait_for(h, |e| e.document.export.operation.is_none());
            h.read(|e| assert!(e.dialogs.message().is_none()));
            h.get_by_role_and_label(egui::accesskit::Role::Button, "OK")
                .click();
            h.step();
            tiff_codes(&output)
        };
        let treated = export(&mut h, "frontier.tif");
        choose_component(&mut h, "Paper", "None");
        let bypassed = export(&mut h, "bypassed.tif");
        assert_eq!(treated.len(), bypassed.len());
        assert_ne!(
            treated, bypassed,
            "the native choice must change actual exported pixels"
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
                e.document.project.output.film_and_paper,
                Some(assets[1].clone())
            );
            assert_grade(e);
        });
        assert_eq!(export(&mut h, "reopened.tif"), treated);
    });
}
