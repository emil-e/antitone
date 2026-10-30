use super::*;

fn show_confirmation(harness: &mut Harness<'_, Editor>) {
    harness.step();
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Analyze")
        .scroll_to_me();
    harness.try_run_realtime().unwrap();
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Analyze")
        .click();
    harness.step();
    harness.step();
    wait_for(harness, |editor| {
        editor.document.analysis_review().is_some() || !editor.busy
    });
    harness.read(|editor| {
        assert!(
            editor.document.analysis_review().is_some(),
            "{}",
            editor.status
        )
    });
    harness.read(|editor| assert!(!editor.analysis_fitting()));
    harness.read(|editor| assert!(!editor.busy));
    harness.read(|editor| {
        let source =
            ProcessingInput::from(editor.source_parameters(editor.source_exposure).unwrap());
        let signature = editor
            .document
            .visit
            .canvas
            .grade
            .original
            .signature
            .as_ref()
            .expect("review uses original-source tool presentation");
        assert_eq!(signature.0, source.signature());
        assert_eq!(
            signature.3,
            Composition::new(
                editor.document.visit.canvas.ready.as_ref().unwrap().area,
                Default::default(),
                Default::default()
            )
            .unwrap()
        );
        assert!(matches!(editor.scope_request(), Err("Positive grade only")));
    });
    if harness.read(|editor| editor.document.analysis_review().unwrap().replaces_photos) {
        let warning = harness.get_by_label("Tool warning");
        let description = warning.accesskit_node().description().unwrap();
        assert!(description.contains("Photo layers"));
        assert!(description.contains("adjustment layers and masks"));
    }
}

fn select_area(h: &mut Harness<'_, Editor>, picture: bool) {
    let geometry = h.read(|editor| editor.active_canvas().last_layout.unwrap().0);
    let point = h.read(|editor| {
        (0..21)
            .flat_map(|y| {
                (0..21).map(move |x| {
                    egui::pos2(
                        geometry.canvas.left()
                            + 2.
                            + (geometry.canvas.width() - 4.) * x as f32 / 20.,
                        geometry.canvas.top()
                            + 2.
                            + (geometry.canvas.height() - 4.) * y as f32 / 20.,
                    )
                })
            })
            .find(|&p| editor.review_area_at(geometry, p) == Some(picture))
            .expect("selectable sampling area")
    });
    h.hover_at(point);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
    }
    h.step();
    h.step();
    h.read(|editor| assert_eq!(editor.document.analysis_review().unwrap().picture, picture));
}

fn confirm(harness: &mut Harness<'_, Editor>) {
    let mut buttons = harness
        .get_all_by_role_and_label(egui::accesskit::Role::Button, "Analyze roll")
        .filter(|node| !node.accesskit_node().is_disabled());
    let button = buttons.next().expect("enabled reviewed Analyze action");
    assert!(buttons.next().is_none());
    button.click();
    drop(buttons);
    harness.step();
    harness.read(|editor| assert!(editor.document.analysis_review().is_some()));
    harness.read(|editor| assert!(editor.analysis_fitting()));
    harness.read(|editor| assert!(editor.busy));
}

fn draw_picture(harness: &mut Harness<'_, Editor>, area: Rect) {
    harness.read(|editor| assert!(editor.document.analysis_review().is_some()));
    select_area(harness, true);
    harness.get_by_label("Fit").click();
    harness.step();
    let geometry = harness.read(|editor| editor.active_canvas().last_layout.unwrap().0);
    let start = geometry.screen_point(egui::pos2(area.x as f32, area.y as f32));
    let end = geometry.screen_point(egui::pos2(
        (area.x + area.width) as f32,
        (area.y + area.height) as f32,
    ));
    let region = harness.read(|editor| editor.pick_region());
    if let Some(region) = region {
        let bounds = geometry.screen_coverage(region);
        if bounds.min.distance(start) > 0.01 {
            drag(harness, bounds.min, start);
        }
        let region = harness.read(|editor| editor.pick_region().unwrap());
        drag(harness, geometry.screen_coverage(region).max, end);
    } else {
        drag(harness, start, end);
    }
    harness.read(|editor| {
        assert_eq!(
            editor
                .pick_region()
                .unwrap()
                .exact_rectangle(editor.document.visit.canvas.ready.as_ref().unwrap().area),
            Some(area)
        )
    });
}

fn settled_targets(harness: &mut Harness<'_, Editor>, case: &str) {
    let mut project = harness.read(|editor| editor.document.state.project.clone());
    stage(harness, "Rendering");
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        select_target(harness, target);
        project.output.tiff = if target == OutputTarget::Srgb {
            crate::project::TiffTarget::Srgb
        } else {
            crate::project::TiffTarget::DisplayP3
        };
        super::workflow::wait_settled(harness);
        harness.read(|editor| {
            let view = editor
                .document
                .visit
                .working_view
                .displayed
                .as_ref()
                .unwrap();
            assert_eq!(
                [view.image.linear.width(), view.image.linear.height()],
                view.spec.frame().output
            );
            assert!(view.spec.frame().output.into_iter().all(|size| size > 0));
            assert!(!editor.document.source);
        });
        let timing =
            harness.read(|editor| Arc::new(SelectionTiming::new(editor.id, Instant::now())));
        harness
            .update(|editor| editor.document.visit.canvas.selection_timing = Some(timing.clone()));
        harness.step();
        let png = harness.render().unwrap();
        assert!(timing.ready_was_painted());
        save_capture(
            &png,
            &format!(
                "analyze-{case}-{}-offscreen",
                if target == OutputTarget::Srgb {
                    "srgb"
                } else {
                    "p3"
                }
            ),
        );
        harness.read(|editor| assert_eq!(editor.document.state.project, project));
    }
    stage(harness, "Roll");
}

fn exercise_analyze(harness: &mut Harness<'_, Editor>, picture: Rect, case: &str) {
    harness.update(|editor| {
        editor.set_gamma(0.7);
        let crate::processing::InputSettings::Negative { dmin, dmax, .. } =
            &mut editor.document.input
        else {
            panic!("Analyze fixture must use Negative");
        };
        dmin.get_or_insert(DVec3::new(0.2, 0.15, 0.1));
        dmax.get_or_insert(DVec3::new(0.008, 0.005, 0.002));
        editor.document.source = false;
        editor.document.roll_orientation.turn_clockwise();
        editor.document.visit.photo_orientation.flips.horizontal = true;
        editor.document.visit.crop = Some(
            Crop::from_region(picture)
                .straightened(
                    editor.document.visit.canvas.ready.as_ref().unwrap().area,
                    0.05,
                )
                .unwrap(),
        );
        editor.document.visit.adjustment_layer_settings.exposure = 0.5;
        editor.document.visit.adjustment_layer_settings.wb = Balance {
            warmth: 0.1,
            tint: -0.05,
        };
        editor.document.visit.adjustment_layer_settings.contrast = 1.2;
        let ids = adjustment_layers::install_layers(editor);
        let mut layers = editor
            .document
            .project
            .source(editor.document.visit.current_source.unwrap())
            .unwrap()
            .adjustment_layers
            .clone()
            .unwrap();
        layers.next_id = 40;
        layers.layers[0].adjustment.photo.exposure = 0.25;
        layers.layers[1].adjustment.enabled = false;
        for id in editor.document.state.project.order.clone() {
            editor
                .document
                .project
                .source_mut(id)
                .unwrap()
                .adjustment_layers = Some(layers.clone());
        }
        if let Some(reference_id) = editor
            .document
            .project
            .flat_field()
            .map(|field| field.reference)
        {
            editor
                .document
                .project
                .source_mut(reference_id)
                .unwrap()
                .adjustment_layers = Some(layers);
        }
        editor.select_adjustment_layer(super::super::adjustment_layers::Target::Layer(ids[0]));
        editor.sync_document();
        editor
            .validate_document_candidate(&editor.document)
            .unwrap();
    });
    harness.step();
    let before = harness.read(|editor| editor.document.state.project.clone());
    let correction = harness.read(|editor| editor.document.correction.clone());
    show_confirmation(harness);
    draw_picture(harness, picture);
    harness.read(|editor| assert_eq!(editor.document.state.project, before));
    harness.get_by_label("Cancel").click();
    harness.step();
    harness.read(|editor| assert!(editor.document.analysis_review().is_none()));
    harness.read(|editor| assert!(!editor.analysis_fitting()));
    harness.read(|editor| assert!(editor.status.contains("areas and grade unchanged")));
    harness.read(|editor| assert_eq!(editor.document.state.project, before));

    show_confirmation(harness);
    // Keep fitting queued at physical admission until the Cancel control is exercised.
    let admission = crate::background::testing::pause();
    confirm(harness);
    harness.read(|editor| assert_eq!(editor.document.state.project, before));
    harness.update(|editor| editor.tasks.elapse(Duration::from_secs(1)));
    for _ in 0..3 {
        harness.step();
    }
    harness.get_by_label("Cancel analysis").click();
    harness.step();
    harness.read(|editor| {
        assert!(
            editor
                .document
                .analysis_operation()
                .unwrap()
                .cancelled
                .is_cancelled(),
            "Cancel must retire the captured fit before physical admission is released"
        );
        assert_eq!(editor.document.state.project, before);
    });
    drop(admission);
    wait_for(harness, |editor| !editor.busy);
    harness.read(|editor| assert!(!editor.analysis_fitting()));
    harness.read(|editor| assert_eq!(editor.status, "Analysis cancelled"));
    harness.read(|editor| assert_eq!(editor.document.state.project, before));

    show_confirmation(harness);
    for id in &before.order {
        let label = before
            .source(*id)
            .unwrap()
            .path
            .file_name()
            .unwrap()
            .to_str()
            .unwrap();
        harness.get_by_label(label).click();
        harness.step();
        wait_for(harness, |editor| {
            !editor.busy && editor.document.visit.canvas.ready.is_some()
        });
        draw_picture(harness, picture);
    }
    let selected = harness.read(|editor| editor.document.visit.current_source);
    confirm(harness);
    harness.read(|editor| assert_eq!(editor.document.state.project, before));
    wait_for(harness, |editor| !editor.busy);
    harness.read(|editor| assert!(!editor.analysis_fitting()));
    harness.read(|editor| assert!(editor.status.starts_with("Analyzed 2"), "{}", editor.status));
    harness.read(|editor| {
        assert_eq!(
            editor.adjustment_layer_target(),
            super::super::adjustment_layers::Target::Layer(crate::project::AdjustmentLayerId(40))
        )
    });
    harness.read(|editor| {
        let after = &editor.document.state.project;
        assert_eq!(after.order, before.order);
        assert!(after.analysis_area.is_none());
        assert_eq!(after.analysis_regions.len(), after.order.len());
        assert!(
            after
                .analysis_regions
                .iter()
                .all(|e| e.areas.picture
                    == Some(Coverage::rectangle(picture).corners().map(|p| p.0)))
        );
        assert_eq!(after.input.gamma(), before.input.gamma());
        assert_eq!(
            Editor::response_of(after.input),
            Editor::response_of(before.input)
        );
        assert_eq!(after.orientation, before.orientation);
        assert_eq!(after.flat_field(), before.flat_field());
        assert_eq!(editor.document.visit.current_source, selected);
        for (mut source, prior) in after.sources.clone().into_iter().zip(&before.sources) {
            if after.order.contains(&source.id) {
                assert_eq!(
                    source.adjustment_layers.as_ref().unwrap().layers[0]
                        .adjustment
                        .photo,
                    AdjustmentLayerSettings {
                        exposure: source.adjustment_layers.as_ref().unwrap().layers[0]
                            .adjustment
                            .photo
                            .exposure,
                        ..AdjustmentLayerSettings::default()
                    }
                );
                let layers = source.adjustment_layers.as_ref().unwrap();
                assert_eq!(layers.layers.len(), 1);
                assert_eq!(
                    layers.layers[0].adjustment.mask,
                    crate::adjustment_layers::AdjustmentLayerCoverage::Global {}
                );
                assert_eq!(
                    layers.next_id,
                    prior.adjustment_layers.as_ref().unwrap().next_id + 1
                );
                source.adjustment_layers = prior.adjustment_layers.clone();
            }
            source.adjustment_layers.as_mut().unwrap().layers[0]
                .adjustment
                .photo = prior.adjustment_layers.as_ref().unwrap().layers[0]
                .adjustment
                .photo;
            assert_eq!(&source, prior);
        }
        if let Some(correction) = correction {
            assert!(Arc::ptr_eq(
                &correction,
                editor.document.correction.as_ref().unwrap()
            ));
        } else {
            assert!(
                after
                    .dmin()
                    .unwrap()
                    .into_iter()
                    .all(|value| (value - 3000. / 4031.).abs() <= 2. / 65535.)
            );
            let exposures: Vec<_> = after
                .order
                .iter()
                .map(|id| {
                    after
                        .source(*id)
                        .unwrap()
                        .adjustment_layers
                        .as_ref()
                        .unwrap()
                        .layers[0]
                        .adjustment
                        .photo
                        .exposure
                })
                .collect();
            assert!(exposures[0] > exposures[1]);
        }
    });
    settled_targets(harness, case);
    harness.update(|editor| {
        let adjustment = before
            .source(selected.unwrap())
            .unwrap()
            .adjustment_layers
            .as_ref()
            .unwrap()
            .layers[0]
            .adjustment;
        editor
            .edit_adjustment_layers(|layers| {
                layers.add("Survives failed Analyze".into(), adjustment)?;
                Ok(())
            })
            .unwrap();
    });

    let failed = harness.read(|editor| editor.document.state.project.clone());
    // Default and adjusted Photos both review areas; only adjusted Photos show the reset warning.
    show_confirmation(harness);
    draw_picture(
        harness,
        Rect {
            width: 8,
            height: 8,
            ..picture
        },
    );
    confirm(harness);
    wait_for(harness, |editor| !editor.busy);
    harness.read(|editor| {
        assert!(
            editor.status.contains("picture samples"),
            "{}",
            editor.status
        )
    });
    harness.read(|editor| assert_eq!(editor.document.state.project, failed));
    harness.read(|editor| assert_eq!(editor.document.visit.current_source, selected));
    harness.read(|editor| assert!(editor.document.visit.canvas.ready.is_some()));
    harness.read(|editor| assert!(editor.preference_store.is_none()));
}

#[test]
fn missing_area_cues_follow_the_photo_and_clear_without_an_inspector_panel() {
    editor_harness_with_thumbnails(true, |mut h, _| {
        h.enter(|editor, cx| {
            editor.add_raws(vec![fixture("roll-0.dng"), fixture("roll-1.dng")], cx)
        });
        wait_for(&mut h, |e| !e.busy && e.document_pending.is_none());
        show_confirmation(&mut h);
        h.read(|editor| assert!(!editor.document.analysis_review().unwrap().replaces_photos));
        let before = h.read(|editor| editor.document.state.project.clone());
        let ids = before.order.clone();
        assert!(h.query_by_label("Tool warning").is_none());
        assert!(
            h.query_by_role_and_label(egui::accesskit::Role::Label, "Analyze")
                .is_none()
        );
        let picture = h.read(|editor| editor.review_areas(ids[0]).unwrap().picture);
        h.update(|editor| {
            editor
                .document
                .analysis_review_mut()
                .unwrap()
                .approved
                .entries[0]
                .areas
                .picture = None
        });
        h.update(|editor| editor.refresh_review_region());
        h.step();
        h.step();
        assert!(
            h.get_by_label("Tool warning")
                .accesskit_node()
                .description()
                .unwrap()
                .contains("Picture sampling area is missing")
        );
        assert_eq!(
            h.get_all_by_role_and_label(egui::accesskit::Role::Image, "Photo warning")
                .count(),
            1
        );
        assert!(
            h.get_by_label("Analyze roll")
                .accesskit_node()
                .is_disabled()
        );
        save_capture(&h.render().unwrap(), "analysis-missing-area-cue");
        h.get_by_label("roll-1.dng").click();
        h.step();
        wait_for(&mut h, |e| {
            !e.busy && e.document.visit.canvas.ready.is_some()
        });
        h.read(|editor| assert!(editor.source_tool_warning(ids[1]).is_none()));
        assert!(
            h.get_by_label("Tool warning")
                .accesskit_node()
                .description()
                .unwrap()
                .contains("1 photo")
        );
        h.update(|editor| {
            editor
                .document
                .analysis_review_mut()
                .unwrap()
                .approved
                .entries[0]
                .areas
                .picture = picture
        });
        h.step();
        h.step();
        assert!(h.query_by_label("Tool warning").is_none());
        assert_eq!(
            h.query_all_by_role_and_label(egui::accesskit::Role::Image, "Photo warning")
                .count(),
            0
        );
        h.get_by_label("Cancel").click();
        h.step();
        wait_for(&mut h, |e| !e.busy);
        h.read(|editor| assert_eq!(editor.document.state.project, before));
    });
}

#[test]
fn roll_area_review_supports_normal_and_narrow_layouts_and_exception_browsing() {
    editor_harness_with_thumbnails(true, |mut harness, _| {
        harness.enter(|editor, cx| {
            editor.add_raws(vec![fixture("roll-0.dng"), fixture("roll-1.dng")], cx)
        });
        wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
        harness.update(|editor| editor.document.visit.adjustment_layer_settings.exposure = 1.25);
        harness.update(|editor| editor.sync_document());
        let before = harness.read(|editor| editor.document.state.project.clone());
        let first = harness.read(|editor| editor.document.visit.current_source);
        let scopes_visible = harness.read(|editor| editor.scopes.visible);
        show_confirmation(&mut harness);
        wait_for(&mut harness, |e| {
            e.document
                .thumbnails
                .ready_count(&e.document, e.document.target)
                == 2
        });
        for (layout, size) in [
            ("normal", egui::vec2(1100., 800.)),
            ("narrow", egui::vec2(800., 600.)),
        ] {
            harness.set_size(size);
            harness.run();
            for picture in [false, true] {
                select_area(&mut harness, picture);
                harness.run();
                harness.read(|editor| {
                    assert_eq!(editor.document.analysis_review().unwrap().picture, picture)
                });
                harness.read(|editor| assert!(editor.document.source));
                harness.read(|editor| assert!(!editor.scopes.visible));
                harness
                    .read(|editor| assert_eq!(editor.preferences.scopes_visible, scopes_visible));
                assert!(harness.query_by_label("Roll analysis").is_none());
                assert!(harness.query_by_label("Picture statistics").is_none());
                harness.get_by_label("Scopes");
                for name in ["Roll", "Photo", "Rendering"] {
                    assert!(
                        !harness
                            .get_by_role_and_label(egui::accesskit::Role::Tab, name)
                            .accesskit_node()
                            .is_disabled()
                    );
                }
                let apply = harness.get_by_label("Analyze roll");
                let cancel = harness.get_by_label("Cancel");
                assert!(!apply.accesskit_node().is_disabled());
                for rect in [apply.rect(), cancel.rect()] {
                    harness.read(|editor| {
                        assert!(
                            rect.min.x >= 0.
                                && rect.max.x
                                    <= editor
                                        .document
                                        .visit
                                        .canvas
                                        .last_layout
                                        .unwrap()
                                        .0
                                        .canvas
                                        .max
                                        .x
                                && rect.max.y <= size.y,
                            "{layout}: {rect:?}"
                        )
                    });
                }
                assert!(!apply.rect().intersects(cancel.rect()));
                let warning = harness.get_by_label("Tool warning").rect();
                assert!(
                    harness
                        .query_by_role_and_label(egui::accesskit::Role::Label, "Analyze")
                        .is_none(),
                    "Analyze has no parameter panel"
                );
                assert!(
                    warning.min.y > 0. && warning.max.y < apply.rect().max.y,
                    "{layout}: warning hidden: {warning:?}"
                );
                save_capture(
                    &harness.render().unwrap(),
                    &format!(
                        "analyze-review-{layout}-{}-offscreen",
                        if picture { "picture" } else { "opening" }
                    ),
                );
                harness.read(|editor| assert_eq!(editor.document.state.project, before));
            }
            stage(&mut harness, "Photo");
            for role in [
                egui::accesskit::Role::Slider,
                egui::accesskit::Role::SpinButton,
            ] {
                assert!(
                    harness
                        .get_by_role_and_label(role, "Exposure")
                        .accesskit_node()
                        .is_disabled()
                );
            }
            let mut resets = 0;
            for reset in
                harness.get_all_by_role_and_label(egui::accesskit::Role::Button, "Reset Exposure")
            {
                assert!(reset.accesskit_node().is_disabled());
                reset.click();
                resets += 1;
            }
            assert!(resets > 0);
            harness.run();
            stage(&mut harness, "Rendering");
            assert!(
                harness
                    .get_by(|node| node.role() == egui::accesskit::Role::ComboBox
                        && node.value().as_deref() == Some("Display P3"))
                    .accesskit_node()
                    .is_disabled()
            );
            harness.read(|editor| assert_eq!(editor.document.state.project, before));
            stage(&mut harness, "Roll");
        }
        assert_eq!(
            harness
                .query_all_by_role_and_label(egui::accesskit::Role::Button, "Next area to inspect")
                .count(),
            0
        );
        select_area(&mut harness, false);
        harness.run();
        let prior = harness.read(|editor| editor.review_areas(first.unwrap()).unwrap().clone());
        assert!(prior.picture_geometry.is_some() && !prior.picture_edited);
        let q = harness.read(|editor| editor.pick_region().unwrap().corners().map(|p| p.0));
        let p = prior.picture.unwrap()[0];
        let geometry = harness.read(|editor| editor.active_canvas().last_layout.unwrap().0);
        drag(
            &mut harness,
            geometry.screen_point(egui::pos2(q[0][0] as f32, q[0][1] as f32)),
            geometry.screen_point(egui::pos2(p[0] as f32 + 12., p[1] as f32 + 12.)),
        );
        let edited = harness.read(|editor| editor.review_areas(first.unwrap()).unwrap().clone());
        assert!(edited.opening_edited && !edited.picture_edited);
        assert_ne!(edited.picture, prior.picture);
        harness.read(|editor| assert_eq!(editor.document.state.project, before));
        select_area(&mut harness, true);
        harness.run();
        harness.read(|editor| {
            assert_eq!(
                editor.pick_region().unwrap().corners().map(|p| p.0),
                editor
                    .review_areas(first.unwrap())
                    .unwrap()
                    .picture
                    .unwrap()
            )
        });
        let source_epoch = harness.read(|editor| {
            editor
                .source_epoch
                .load(std::sync::atomic::Ordering::Relaxed)
        });
        let (original_generation, original_revision) = harness.read(|editor| {
            (
                editor
                    .document
                    .visit
                    .canvas
                    .grade
                    .original
                    .generation
                    .clone(),
                editor.document.visit.canvas.grade.original.revision(),
            )
        });
        harness.get_by_label("roll-1.dng").click();
        harness.step();
        wait_for(&mut harness, |e| !e.busy);
        harness.read(|editor| assert_ne!(editor.document.visit.current_source, first));
        harness.read(|editor| {
            assert_ne!(
                editor
                    .source_epoch
                    .load(std::sync::atomic::Ordering::Relaxed),
                source_epoch
            )
        });
        harness.read(|editor| {
            assert!(!Arc::ptr_eq(
                &editor.document.visit.canvas.grade.original.generation,
                &original_generation
            ));
            assert!(
                original_generation.load(std::sync::atomic::Ordering::Relaxed) > original_revision
            );
        });
        harness
            .read(|editor| assert!(matches!(editor.scope_request(), Err("Positive grade only"))));
        harness.read(|editor| {
            assert!(
                !editor
                    .review_areas(editor.document.visit.current_source.unwrap())
                    .unwrap()
                    .opening_edited
            )
        });
        harness.read(|editor| assert_eq!(editor.document.state.project, before));
        harness.get_by_label("Cancel").click();
        harness.step();
        wait_for(&mut harness, |e| !e.busy);
        harness.read(|editor| {
            assert_eq!(
                editor.document.visit.current_source,
                Some(editor.document.project.order[1])
            )
        });
        harness.read(|editor| assert_eq!(editor.scopes.visible, scopes_visible));
        harness.read(|editor| assert_eq!(editor.preferences.scopes_visible, scopes_visible));
        harness.read(|editor| assert_eq!(editor.document.state.project, before));
    });
}

#[test]
fn analyze_widgets_preserve_area_cancel_failure_and_both_settled_targets() {
    editor_harness(|mut harness, _| {
        harness.enter(|editor, cx| {
            editor.add_raws(vec![fixture("roll-0.dng"), fixture("roll-1.dng")], cx)
        });
        wait_for(&mut harness, |editor| {
            !editor.busy && editor.document_pending.is_none()
        });
        harness.read(|editor| {
            assert!(
                editor.document.visit.canvas.ready.is_some(),
                "{}",
                editor.status
            )
        });
        exercise_analyze(
            &mut harness,
            Rect {
                x: 96,
                y: 96,
                width: 320,
                height: 320,
            },
            "roll",
        );
    });
}

#[test]
fn analyze_widgets_keep_real_correction_and_both_settled_targets() {
    let directory = tempfile::tempdir().unwrap();
    let second = directory.path().join("second-negative.dng");
    std::fs::copy(fixture("illuminated-patches.dng"), &second).unwrap();
    editor_harness(|mut harness, _| {
        harness.enter(|editor, cx| {
            editor.add_raws(vec![fixture("illuminated-patches.dng"), second], cx)
        });
        wait_for(&mut harness, |editor| {
            !editor.busy && editor.document_pending.is_none()
        });
        harness.read(|editor| {
            assert!(
                editor.document.visit.canvas.ready.is_some(),
                "{}",
                editor.status
            )
        });
        // Supply the native chooser result; map estimation/application still use production workers.
        harness.enter(|editor, cx| editor.begin_flat_edit(fixture("bare-light.dng"), None, cx));
        wait_for(&mut harness, |editor| !editor.busy);
        harness.get_by_label("Apply").click();
        harness.step();
        wait_for(&mut harness, |editor| {
            !editor.busy && editor.document_pending.is_none()
        });
        harness.read(|editor| assert!(editor.document.correction.is_some(), "{}", editor.status));
        exercise_analyze(
            &mut harness,
            Rect {
                x: 64,
                y: 64,
                width: 256,
                height: 192,
            },
            "corrected",
        );
    });
}
