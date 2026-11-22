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
                    Some(FrontierSelection {
                        negative: None,
                        variant
                    })
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
        choose_component(
            &mut h,
            "Negative",
            crate::looks::monochrome::Negative::all()
                .next()
                .unwrap()
                .name(),
        );
        h.read(|e| {
            assert!(e.dialogs.is_open() && e.document.film_and_paper.pending.is_none());
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

#[test]
fn frontier_colour_stock_controls_preserve_pairs_history_other_slots_and_saved_pixels() {
    fixture_editor(|mut h, _| {
        let mut nodes = Vec::new();
        for b in 0..2 {
            for g in 0..2 {
                for r in 0..2 {
                    nodes.push([r, g, b].map(|v| -0.5 + 1.6 * v as f32));
                }
            }
        }
        let imported = Arc::new(
            crate::looks::LookAsset::new(
                "Authored identity".into(),
                crate::looks::LookOrigin::Imported,
                Arc::new(crate::looks::Lut3D::new(2, [-0.5; 3], [1.1; 3], nodes).unwrap()),
            )
            .unwrap(),
        );
        h.enter(|e, _| {
            let mut edit = e.edit_project("Authored grade and LUT").unwrap();
            edit.update(|project| {
                project.output.lut = Some(imported.clone());
                project.roll.exposure = 0.35;
                project.sources[0]
                    .adjustment_layers
                    .as_mut()
                    .unwrap()
                    .layers[0]
                    .adjustment
                    .photo
                    .saturation = 0.89;
            });
            e.commit_project(edit).unwrap();
        });
        paint_until(&mut h, positive_ready);
        let before = h.read(|e| e.document.state.project.clone());
        let portra = Negative::from_id("kodak_portra_400").unwrap();
        let assert_state = |e: &Editor, negative, variant| {
            assert!(!e.dialogs.is_open());
            assert_eq!(e.document.project.input, before.input);
            assert_eq!(e.document.project.roll, before.roll);
            assert_eq!(e.document.project.sources, before.sources);
            assert_eq!(e.document.project.output.lut, before.output.lut);
            assert_eq!(e.document.project.output.grain, before.output.grain);
            assert_eq!(
                FrontierSelection::from_origin(
                    e.document
                        .project
                        .output
                        .film_and_paper
                        .as_ref()
                        .unwrap()
                        .origin()
                ),
                Some(FrontierSelection { negative, variant })
            );
        };
        choose_component(&mut h, "Negative", portra.name());
        choose_component(&mut h, "Paper", Variant::A.name());
        h.read(|e| assert_state(e, Some(portra), Variant::A));
        h.event(egui::Event::PointerGone);
        h.step();
        save_capture(&h.render().unwrap(), "frontier-negative-a");
        let a = h.read(|e| e.document.project.output.film_and_paper.clone().unwrap());
        choose_component(&mut h, "Paper", Variant::B.name());
        h.read(|e| assert_state(e, Some(portra), Variant::B));
        h.event(egui::Event::PointerGone);
        h.step();
        save_capture(&h.render().unwrap(), "frontier-negative-b");
        let b = h.read(|e| e.document.project.output.film_and_paper.clone().unwrap());
        assert_ne!(a.id(), b.id());
        let ektar = Negative::from_id("kodak_ektar_100").unwrap();
        choose_component(&mut h, "Negative", ektar.name());
        h.read(|e| assert_state(e, Some(ektar), Variant::B));
        assert_ne!(
            h.read(|e| e
                .document
                .project
                .output
                .film_and_paper
                .as_ref()
                .unwrap()
                .id()),
            b.id()
        );
        choose_component(&mut h, "Negative", "None");
        h.read(|e| {
            assert_state(e, None, Variant::B);
            assert_eq!(
                e.document
                    .project
                    .output
                    .film_and_paper
                    .as_ref()
                    .unwrap()
                    .id()
                    .to_string(),
                "d291a5d512263f1759ae2145925060263dc12973fe04f31571363cd5045de849"
            );
        });
        let none = h.read(|e| e.document.project.output.film_and_paper.clone());
        choose_component(&mut h, "Negative", portra.name());
        h.read(|e| assert_eq!(e.document.project.output.film_and_paper.as_ref(), Some(&b)));
        h.enter(|e, cx| e.restore_history(crate::editor::history::Direction::Undo, cx));
        paint_until(&mut h, positive_ready);
        h.read(|e| assert_eq!(e.document.project.output.film_and_paper, none));
        h.enter(|e, cx| e.restore_history(crate::editor::history::Direction::Redo, cx));
        paint_until(&mut h, positive_ready);
        h.read(|e| assert_state(e, Some(portra), Variant::B));

        choose_component(
            &mut h,
            "Negative",
            crate::looks::monochrome::Negative::all()
                .next()
                .unwrap()
                .name(),
        );
        h.read(|e| {
            assert!(e.dialogs.is_open() && e.document.film_and_paper.pending.is_none());
            assert_eq!(e.document.project.output.film_and_paper.as_ref(), Some(&b));
        });
        h.key_press(egui::Key::Escape);
        h.step();
        h.read(|e| assert_state(e, Some(portra), Variant::B));

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("frontier-negative.antitone");
        h.enter(|e, cx| e.save_project(path.clone(), crate::project::Save::New, None, cx));
        wait_for(&mut h, |e| !e.interaction_blocked());
        h.read(|e| assert!(!e.dialogs.is_open() && !e.document.dirty()));
        let saved = crate::project::Project::load(&path).unwrap();
        assert_eq!(saved.output.film_and_paper.as_ref(), Some(&b));
        assert_eq!(saved.output.lut, before.output.lut);
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
        let treated = export(&mut h, "treated.tif");
        h.get_by_label("Reset Film & paper").scroll_to_me();
        h.try_run_realtime().unwrap();
        h.get_by_label("Reset Film & paper").click();
        h.step();
        paint_until(&mut h, positive_ready);
        h.read(|e| {
            assert!(e.document.project.output.film_and_paper.is_none());
            assert_eq!(e.document.project.output.lut, before.output.lut);
        });
        assert_ne!(export(&mut h, "bypassed.tif"), treated);
        open_project(&mut h, path);
        h.get_by_label("Discard changes").click();
        h.step();
        wait_for(&mut h, |e| {
            !e.interaction_blocked() && e.document.visit.canvas.ready.is_some()
        });
        paint_until(&mut h, positive_ready);
        h.read(|e| {
            assert_state(e, Some(portra), Variant::B);
            assert_eq!(e.document.project.output.film_and_paper.as_ref(), Some(&b));
            assert!(e.document.film_and_paper.pending.is_none());
        });
        assert_eq!(export(&mut h, "reopened.tif"), treated);
    });
}
