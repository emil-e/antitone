use super::*;

fn exact_values(repair: &crate::editor::dust::render_testing::Image) -> Vec<([u32; 2], [u32; 3])> {
    repair
        .pixels()
        .map(|(point, value)| (point, value.map(f32::to_bits)))
        .collect()
}

#[test]
fn pending_stroke_shows_no_donor_and_accepts_one_result_after_real_search() {
    tools::calibrated_harness_with_thumbnails(true, |mut h| {
        stage(&mut h, "Photo");
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Dust repair")
            .click();
        h.step();
        wait_for(&mut h, |e| e.document.visit.working_view.idle());
        let before = h.read(|e| e.document.state.project.clone());
        let dirty = h.read(|e| e.document.dirty());
        let admission = crate::background::testing::pause();
        h.update(|e| e.document.visit.dust.brush_size = 12);
        click(&mut h, egui::pos2(96., 32.), egui::Modifiers::NONE);
        h.step();
        h.read(|e| {
            assert!(e.dust_search_pending());
            assert_eq!(e.document.state.project, before);
            assert_eq!(e.document.dirty(), dirty);
            assert!(e.dust_donor_regions().is_empty());
            assert!(!e.file_available());
        });
        let (canvas, marked) = h.read(|e| {
            let geometry = e.active_canvas().last_layout.unwrap().0;
            let stroke = &e.pending_dust_strokes()[0];
            let [x, y] = stroke.support.origin();
            let [width, height] = stroke.support.size();
            (
                geometry.canvas,
                geometry.screen_region(crate::geometry::SourceRegion {
                    x,
                    y,
                    width,
                    height,
                }),
            )
        });
        // Move away so the brush cursor cannot stand in for pending-mask feedback.
        h.hover_at(canvas.min - egui::vec2(1., 1.));
        h.step();
        let marked = marked.expand(marked.size().max_elem() * 0.5);
        assert!(
            h.output().shapes.iter().any(|shape| {
                let bounds = shape.shape.visual_bounding_rect();
                !matches!(&shape.shape, egui::Shape::Callback(_))
                    && bounds.is_positive()
                    && marked.contains_rect(bounds)
            }),
            "the pending support must be visible before a donor exists"
        );
        assert!(h.output().shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Callback(callback) if canvas.contains(callback.rect.center()))));
        save_capture(&h.render().unwrap(), "dust-search-pending-offscreen");
        drop(admission);
        ready(&mut h, 1);
        h.read(|e| {
            assert!(!e.dust_search_pending());
            let source = e.document.visit.current_source.unwrap();
            assert_eq!(e.dust_repairs(source).unwrap().repairs().len(), 1);
            assert!(!e.dust_donor_regions().is_empty());
            assert_eq!(
                e.history_label(crate::editor::history::Direction::Undo),
                Some("Dust mask")
            );
        });
    });
}

#[test]
fn leaving_the_photo_discards_pending_artwork_and_returning_can_apply_a_fresh_stroke() {
    let directory = tempfile::tempdir().unwrap();
    let other = directory.path().join("other.dng");
    std::fs::copy(fixture("patches.dng"), &other).unwrap();
    tools::calibrated_harness_with_thumbnails(true, |mut h| {
        h.enter(|editor, cx| editor.add_raws(vec![other], cx));
        wait_opened_photo(&mut h);
        h.get_by_label("patches.dng").click();
        h.step();
        wait_for(&mut h, |editor| !editor.interaction_blocked());
        stage(&mut h, "Photo");
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Dust repair")
            .click();
        h.step();
        wait_for(&mut h, |editor| editor.document.visit.working_view.idle());
        let source = h.read(|editor| editor.document.visit.current_source.unwrap());
        let undo = h.read(|editor| editor.history_label(crate::editor::history::Direction::Undo));
        let admission = crate::background::testing::pause();
        click(&mut h, egui::pos2(96., 32.), egui::Modifiers::NONE);
        h.read(|editor| {
            assert!(editor.dust_search_pending());
            assert!(!editor.pending_dust_strokes().is_empty());
            assert!(editor.dust_repairs(source).is_none());
        });
        h.get_by_label("other.dng").click();
        h.step();
        h.read(|editor| {
            assert_ne!(editor.document.visit.current_source, Some(source));
            assert!(!editor.dust_search_pending());
            assert!(editor.pending_dust_strokes().is_empty());
            assert!(!editor.dusting());
        });
        drop(admission);
        wait_for(&mut h, |editor| {
            !editor.interaction_blocked() && editor.document.visit.working_view.idle()
        });
        h.get_by_label("patches.dng").click();
        h.step();
        wait_for(&mut h, |editor| {
            !editor.interaction_blocked() && editor.document.visit.working_view.idle()
        });
        h.read(|editor| {
            assert_eq!(editor.document.visit.current_source, Some(source));
            assert!(editor.dust_repairs(source).is_none());
            assert!(editor.pending_dust_strokes().is_empty());
            assert_eq!(
                editor.history_label(crate::editor::history::Direction::Undo),
                undo
            );
        });
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Dust repair")
            .click();
        h.step();
        click(&mut h, egui::pos2(96., 32.), egui::Modifiers::NONE);
        ready(&mut h, 1);
        h.read(|editor| assert_eq!(editor.dust_repairs(source).unwrap().repairs().len(), 1));
    });
}

#[test]
fn repaired_selected_preview_paints_until_the_detailed_view_is_actually_ready() {
    tools::calibrated_harness_with_thumbnails(true, |mut h| {
        stage(&mut h, "Photo");
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Dust repair")
            .click();
        h.step();
        h.update(|e| e.document.visit.dust.brush_size = 12);
        click(&mut h, egui::pos2(96., 32.), egui::Modifiers::NONE);
        ready(&mut h, 1);
        wait_for(&mut h, |e| e.document.visit.working_view.idle());
        // Recreate the source-ready / detailed-acquisition-pending boundary,
        // keeping accepted repair instructions. Hold physical admission so the
        // placeholder must paint rather than winning a worker timing race.
        let admission = crate::background::testing::pause();
        let timing = h.update(|e| {
            e.seed_thumbnail();
            let source = e.document.visit.current_source.unwrap();
            e.document.visit.pending_preview = Some(
                e.document
                    .thumbnails
                    .selected_preview(e.id, &e.document.state, source)
                    .unwrap()
                    .expect("selected camera preview is cached"),
            );
            e.document.visit.working_view.clear();
            e.document.visit.canvas.preview_rendered = None;
            let timing = Arc::new(SelectionTiming::new(e.id, Instant::now()));
            e.document.visit.canvas.selection_timing = Some(timing.clone());
            timing
        });
        h.step();
        h.read(|e| {
            assert!(e.document.visit.pending_preview.is_some());
            assert!(e.document.visit.working_view.displayed.is_none());
            let preview = &e
                .document
                .visit
                .canvas
                .preview_rendered
                .as_ref()
                .expect("usable placeholder")
                .2;
            assert_eq!(preview.capture.recipe().unwrap().repairs.len(), 1);
            assert!(e.document.visit.working_view.completed_capture().is_none());
            assert_eq!(
                e.dust_render_state(),
                "Repairs pending",
                "a proxy retaining complete intent has not completed the requested healing"
            );
        });
        h.render().unwrap();
        assert!(timing.pending_was_painted());
        assert!(!timing.ready_was_painted());
        drop(admission);
        wait_for(&mut h, |e| {
            e.document
                .visit
                .working_view
                .displayed
                .as_ref()
                .is_some_and(|view| {
                    view.capture
                        .recipe()
                        .is_some_and(|recipe| recipe.repairs.len() == 1)
                        && view.spec.plan.sampling()
                            == crate::acquisition::Sampling::NeutralFootprints
                })
        });
        h.step();
        h.render().unwrap();
        assert!(timing.ready_was_painted());
        h.read(|e| {
            assert!(e.document.visit.pending_preview.is_none());
            assert_eq!(
                e.document
                    .visit
                    .working_view
                    .displayed
                    .as_ref()
                    .unwrap()
                    .capture
                    .recipe()
                    .unwrap()
                    .repairs
                    .len(),
                1
            );
        });
    });
}

#[test]
fn repair_evaluation_failures_preserve_usable_navigation_previews() {
    tools::calibrated_harness_with_thumbnails(true, |mut h| {
        wait_for(&mut h, |editor| {
            editor
                .document
                .thumbnails
                .ready_count(&editor.document, editor.document.target())
                == 1
        });
        let source = h.read(|editor| editor.document.visit.current_source.unwrap());
        let area = h.read(|editor| editor.document.visit.canvas.ready.as_ref().unwrap().area);
        // A support touching the source edge cannot satisfy the biharmonic
        // exterior, so this is an evaluation failure rather than mask rejection.
        let repair = h.update(|editor| {
            editor
                .document
                .project
                .source_mut(source)
                .unwrap()
                .repairs
                .insert(
                    crate::dust_repair::SupportMask::new(
                        [area.x, area.y],
                        [16, 16],
                        vec![true; 16 * 16],
                    )
                    .unwrap(),
                    crate::dust_repair::RepairOrigin::Manual,
                    [64, 0],
                )
                .unwrap()
        });
        h.step();
        assert_navigation_preview(&h);
        wait_for(&mut h, |editor| {
            editor
                .document
                .visit
                .working_view
                .failed_capture()
                .is_some()
        });
        for size in [egui::vec2(1100., 800.), egui::vec2(800., 600.)] {
            h.set_size(size);
            h.step();
            assert_navigation_preview(&h);
            wait_for(&mut h, |editor| {
                editor
                    .document
                    .visit
                    .working_view
                    .failed_capture()
                    .is_some()
            });
            h.read(|editor| {
                assert!(
                    editor
                        .document
                        .visit
                        .working_view
                        .failed_capture()
                        .is_some(),
                    "the current failed view remains unavailable while navigation stays usable"
                );
            });
            save_capture(
                &h.render().unwrap(),
                &format!(
                    "navigation-thumbnail-with-repair-failure-{}x{}",
                    size.x as u32, size.y as u32
                ),
            );
        }
        h.update(|editor| {
            editor
                .document
                .project
                .source_mut(source)
                .unwrap()
                .repairs
                .set_enabled(repair, false)
                .unwrap()
        });
        h.step();
        assert_navigation_preview(&h);
    });
}

fn assert_navigation_preview(h: &Harness<'_, Editor>) {
    let name = h.read(|e| {
        e.document
            .source_path(e.document.visit.current_source.unwrap())
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned()
    });
    let card = h.get_by_label(&name);
    assert!(
        !card.accesskit_node().is_disabled(),
        "the photo remains selectable"
    );
    let rect = card.rect();
    assert!(h.output().shapes.iter().any(|shape| matches!(&shape.shape,
        egui::Shape::Callback(callback) if callback.rect.is_positive() && rect.contains(callback.rect.center())
    )), "the navigation card retains an image while repair work changes");
}

#[test]
fn authored_dust_save_new_open_restores_intent_exact_results_and_looked_view_reference_scopes() {
    tools::calibrated_harness_with_thumbnails(true, |mut h| {
        let nodes = (0..8)
            .map(|i| {
                [
                    0.02 + 0.9 * (i % 2) as f32,
                    0.03 + 0.8 * ((i / 2) % 2) as f32,
                    0.01 + 0.95 * (i / 4) as f32,
                ]
            })
            .collect();
        let look = Arc::new(
            crate::looks::LookAsset::new(
                "Persisted Dust look".into(),
                crate::looks::LookOrigin::Imported,
                Arc::new(crate::looks::Lut3D::new(2, [0.; 3], [1.; 3], nodes).unwrap()),
            )
            .unwrap(),
        );
        h.update(|editor| {
            // Keep the f64/f32 consistency check well-conditioned at film base;
            // the save/reopen comparison below remains bit-exact.
            editor.set_response(crate::processing::Response {
                toe: 1.,
                shoulder: 0.,
            });
            editor.document.state.project.output.lut = Some(look.clone());
        });
        h.step();
        stage(&mut h, "Photo");
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("dust.antitone");

        let saved = h.read(|editor| {
            editor
                .document
                .save(&path, crate::project::Save::New)
                .unwrap()
        });
        h.update(|editor| *editor.document = saved);

        h.get_by_role_and_label(egui::accesskit::Role::Button, "Dust repair")
            .click();
        h.step();
        h.update(|editor| editor.document.visit.dust.brush_size = 32);
        click(&mut h, egui::pos2(96., 32.), egui::Modifiers::NONE);
        ready(&mut h, 1);
        click(&mut h, egui::pos2(32., 96.), egui::Modifiers::NONE);
        ready(&mut h, 2);

        let source = h.read(|editor| editor.document.visit.current_source.unwrap());
        let first_offset =
            h.read(|editor| editor.dust_repairs(source).unwrap().repairs()[0].offset);
        let region = h.read(|editor| editor.dust_donor_regions().remove(0));
        let point = region.support.selected_points().next().unwrap();
        let offset = [
            region.donor.x as i32 - region.target.x as i32,
            region.donor.y as i32 - region.target.y as i32,
        ];
        let start = egui::pos2(
            point[0] as f32 + offset[0] as f32 + 0.5,
            point[1] as f32 + offset[1] as f32 + 0.5,
        );
        let layout = h.read(|editor| editor.active_canvas().last_layout.unwrap().0);
        drag(
            &mut h,
            layout.screen_point(start),
            layout
                .screen_point(start + egui::vec2((50 - offset[0]) as f32, (50 - offset[1]) as f32)),
        );
        ready(&mut h, 2);
        h.read(|editor| assert!(editor.document.dirty()));
        h.read(|editor| {
            assert_eq!(
                editor.dust_repairs(source).unwrap().repairs()[0].offset,
                first_offset
            )
        });
        h.read(|editor| {
            assert_eq!(
                editor.dust_repairs(source).unwrap().repairs()[1].offset,
                [50, 50]
            )
        });

        let evaluated = h.read(|editor| editor.dust_images());
        let expected = h.read(|editor| editor.document.state.project.clone());
        h.read(|editor| {
            assert!(
                !editor
                    .application_menu_state()
                    .enabled(crate::platform::menu::Action::Save)
            )
        });
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Done")
            .click();
        h.step();
        h.read(|editor| {
            assert!(!editor.dusting());
            assert_eq!(editor.document.state.project, expected);
        });
        native_file_action(&mut h, crate::platform::menu::Action::New);
        h.run();
        h.read(|editor| assert!(editor.files.pending(), "repair-only edits must guard New"));
        h.get_by_label("Cancel").click();
        h.run();
        h.read(|editor| assert!(!editor.files.pending()));
        h.read(|editor| assert_eq!(editor.document.state.project, expected));
        save_existing_project(&mut h);
        h.read(|editor| assert!(evaluated[0].same_definition(&editor.dust_images()[0])));

        native_file_action(&mut h, crate::platform::menu::Action::New);
        wait_for(&mut h, |e| !e.interaction_blocked());
        h.read(|editor| assert!(editor.document.state.project.sources.is_empty()));
        open_project(&mut h, path.clone());
        wait_opened_photo(&mut h);
        h.read(|editor| assert_eq!(editor.document.state.project, expected));
        h.read(|editor| assert!(!editor.document.dirty()));
        h.read(|editor| assert_eq!(editor.selected_dust_repair(), None));
        ready(&mut h, 2);
        wait_for(&mut h, |e| {
            e.document
                .thumbnails
                .ready_count(&e.document, e.document.target())
                == 1
        });
        let restored = h.read(|editor| editor.dust_images());
        for (before, after) in evaluated.iter().zip(&restored) {
            assert_eq!(before.offset, after.offset);
            assert_eq!(before.support(), after.support());
            assert_eq!(exact_values(before), exact_values(after));
        }

        let view = h.read(|editor| {
            editor
                .document
                .visit
                .working_view
                .displayed
                .as_ref()
                .unwrap()
                .clone()
        });
        assert_eq!(view.processing().look_id(), Some(look.lut().id()));
        let scopes::ScopeInput { view: scope, .. } =
            h.read(|editor| editor.scope_request().unwrap());
        assert!(Arc::ptr_eq(&scope, &view));
        h.get_by_label("Original-resolution reference").click();
        h.step();
        // The reference must match the complete processing intent, including admitted repairs.
        wait_for(&mut h, |e| {
            assert!(!e.document.visit.canvas.original.failed(), "{}", e.status);
            !e.document.visit.working_view.updating
                && e.document.visit.working_view.reference_ready()
                && e.document
                    .visit
                    .working_view
                    .displayed
                    .as_ref()
                    .is_some_and(|view| {
                        Some(&view.processing())
                            == e.document.visit.working_view.processing().as_ref()
                            && view.target == e.document.target()
                    })
        });
        h.render().unwrap();
        h.read(|editor| {
            let reference = editor
                .document
                .visit
                .working_view
                .displayed
                .as_ref()
                .unwrap();
            assert_eq!(reference.processing().look_id(), Some(look.lut().id()));
            assert!(Arc::ptr_eq(
                &reference.image,
                &editor
                    .document
                    .visit
                    .canvas
                    .original
                    .settled()
                    .as_ref()
                    .unwrap()
                    .1
            ));
            let scopes::ScopeInput { view: scope, .. } = editor.scope_request().unwrap();
            assert!(Arc::ptr_eq(&scope, reference));
            assert_eq!(editor.document.state.project, expected);
        });
        h.get_by_label("Original-resolution reference").click();
        h.step();
        ready(&mut h, 2);

        let raw_path = h.read(|editor| editor.document.source_path(source).unwrap());
        let scan = Scan::open(&raw_path, expected.acquisition_wb()).unwrap();
        for repair in expected.source(source).unwrap().repairs.repairs() {
            let recipe = expected.image_recipe(source).unwrap();
            let input = crate::scan::ImageSource::from_recipe(&scan, &recipe.neutral).unwrap();
            let cpu = crate::repair_source::evaluate(&input, repair).unwrap();
            let gpu = restored
                .iter()
                .find(|result| result.id == repair.id)
                .unwrap();
            assert_eq!(gpu.offset, repair.offset);
            assert_eq!(gpu.support(), &repair.support);

            let reference: Vec<_> = cpu.pixels().collect();
            let actual: Vec<_> = gpu.pixels().collect();
            assert_eq!(actual.len(), reference.len());

            // Independent f64 input and production f32 input need numerical agreement;
            // reopening the same production recipe above and below remains bit-exact.
            for ((point, actual), (reference_point, reference)) in actual.iter().zip(&reference) {
                assert_eq!(point, reference_point);

                for channel in 0..3 {
                    assert!(
                        (actual[channel] - reference[channel]).abs()
                            < 2e-6 * reference[channel].abs().max(1.)
                    );
                }
            }
        }
        // Failed Open preserves the current view, not an older one from before reference toggling.
        let view = h.read(|editor| {
            editor
                .document
                .visit
                .working_view
                .displayed
                .as_ref()
                .unwrap()
                .clone()
        });
        let hold = crate::background::testing::pause();
        open_project(&mut h, directory.path().join("missing.antitone"));
        for _ in 0..5 {
            h.step();
            h.read(|editor| assert!(editor.files.opening()));
            h.read(|editor| {
                assert!(Arc::ptr_eq(
                    &view,
                    editor
                        .document
                        .visit
                        .working_view
                        .displayed
                        .as_ref()
                        .unwrap()
                ))
            });
        }
        drop(hold);
        wait_for(&mut h, |e| !e.interaction_blocked());
        h.read(|editor| assert_eq!(editor.document.state.project, expected));
        h.read(|editor| assert!(restored[0].same_definition(&editor.dust_images()[0])));
        h.read(|editor| {
            assert!(Arc::ptr_eq(
                &view,
                editor
                    .document
                    .visit
                    .working_view
                    .displayed
                    .as_ref()
                    .unwrap()
            ))
        });
        stage(&mut h, "Photo");
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Dust repair")
            .click();
        h.step();
        h.try_run_realtime().unwrap();

        for (name, size) in [
            ("normal", egui::vec2(1100., 800.)),
            ("narrow", egui::vec2(800., 600.)),
        ] {
            h.set_size(size);
            h.step();
            h.get_by_label("Brush size").scroll_to_me();
            h.try_run_realtime().unwrap();
            let brush = h.get_by_label("Brush size").rect();
            assert!(h.ctx.content_rect().contains_rect(brush));
            save_capture(
                &h.render().unwrap(),
                &format!("dust-persistence-{name}-offscreen"),
            );
        }

        h.get_by_label("Done").click();
        h.step();
        native_file_action(&mut h, crate::platform::menu::Action::New);
        wait_for(&mut h, |e| !e.interaction_blocked());
        open_project(&mut h, path);
        wait_opened_photo(&mut h);
        ready(&mut h, 2);
        for (before, after) in h.read(|editor| evaluated.iter().zip(editor.dust_images())) {
            assert_eq!(before.offset, after.offset);
            assert_eq!(before.support(), after.support());
            assert_eq!(exact_values(before), exact_values(&after));
        }
    });
}
