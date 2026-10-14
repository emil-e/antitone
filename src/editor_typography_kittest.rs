use super::*;

#[test]
fn typography_keeps_tabs_readouts_and_tool_actions_coherent_at_both_densities() {
    let (mut harness, _capture) = editor_harness();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng");
    harness.state_mut().add_raws(vec![fixture]);
    wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
    {
        let editor = harness.state_mut();
        editor.dmin = Some(DVec3::splat(3000. / 4031.));
        editor.dmax = Some(DVec3::splat(128. / 4031.));
        editor.roll.gamma = 0.75;
        editor.photo.exposure = 0.5;
        editor.scopes.visible = false;
        editor.sync_document();
    }
    let project = harness.state().document.project.clone();
    let source = harness.state().current_source;
    for scale in [1., 2.] {
        harness
            .input_mut()
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .native_pixels_per_point = Some(scale);
        harness.step();
        for (layout, size, width) in [
            ("normal", egui::vec2(1100., 800.), 315.),
            ("narrow", egui::vec2(800., 600.), 270.),
        ] {
            set_layout(&mut harness, size, width);
            for (scene, stage) in [
                ("roll", "Roll"),
                ("photo", "Photo"),
                ("rendering", "Rendering"),
                ("picker", "Roll"),
            ] {
                if harness.state().pick.is_some() {
                    harness.state_mut().finish_pick();
                }
                harness
                    .get_by_role_and_label(egui::accesskit::Role::Tab, stage)
                    .click();
                harness.step();
                harness.state_mut().source = scene == "photo";
                if scene == "picker" {
                    harness.state_mut().begin_pick(Pick::Dmin);
                }
                harness.step();
                harness.try_run_realtime().unwrap();
                harness.input_mut().events.push(egui::Event::PointerGone);
                harness.step();
                for label in ["Roll", "Photo", "Rendering"] {
                    let tab = harness.get_by_role_and_label(egui::accesskit::Role::Tab, label);
                    assert_eq!(tab.accesskit_node().is_selected(), Some(label == stage));
                    let rect = tab.rect();
                    assert!(rect.left() >= size.x - width && rect.right() <= size.x + 0.01);
                    assert_eq!(rect.height(), 28.);
                    let marking = label.to_uppercase();
                    let text = harness
                        .output()
                        .shapes
                        .iter()
                        .find_map(|shape| match &shape.shape {
                            egui::Shape::Text(text) if text.galley.job.text == marking => {
                                Some(text)
                            }
                            _ => None,
                        })
                        .unwrap();
                    assert!(text.pos.x - rect.left() >= 12.);
                    assert!(rect.right() - (text.pos.x + text.galley.size().x) >= 12.);
                    assert!(text.galley.job.sections.iter().all(|section| matches!(
                        section.format.font_id.family,
                        egui::FontFamily::Name(_)
                    )));
                }
                if scene == "picker" {
                    let identity = harness
                        .get_by_role_and_label(egui::accesskit::Role::Label, "Active tool: Dmin")
                        .rect();
                    for label in ["Cancel", "Apply"] {
                        let action =
                            harness.get_by_role_and_label(egui::accesskit::Role::Button, label);
                        assert_eq!(action.rect().height(), 28.);
                        assert!(action.rect().left() > identity.right());
                        assert!(action.rect().right() <= size.x - width);
                    }
                    assert!(
                        harness
                            .get_by_role_and_label(egui::accesskit::Role::Button, "Apply")
                            .accesskit_node()
                            .is_disabled()
                    );
                }
                assert_eq!(harness.state().document.project, project);
                assert_eq!(harness.state().current_source, source);
                let png = harness.render().unwrap();
                assert_eq!(
                    png.dimensions(),
                    ((size.x * scale) as u32, (size.y * scale) as u32)
                );
                if let Some(directory) = std::env::var_os("ANTITONE_UI_CAPTURE") {
                    let directory = PathBuf::from(directory);
                    std::fs::create_dir_all(&directory).unwrap();
                    png.save(directory.join(format!(
                        "typography-{scene}-{layout}-{}x-offscreen.png",
                        scale as u32
                    )))
                    .unwrap();
                }
            }
        }
    }
}
