use super::*;
#[path = "editor_kittest_auto_crop_margin.rs"]
mod margin;
#[path = "editor_kittest_auto_crop_overlay.rs"]
mod overlay;
use crate::{
    project::SourceAnalysis,
    scan_regions::{Areas, PictureGeometry},
};

fn setup<R>(test: impl FnOnce(Harness<'_, Editor>, tempfile::TempDir) -> R) -> R {
    let directory = tempfile::tempdir().unwrap();
    let paths = (0..3)
        .map(|i| {
            let path = directory.path().join(format!("crop-{i}.dng"));
            std::fs::copy(fixture("illuminated-patches.dng"), &path).unwrap();
            path
        })
        .collect();
    editor_harness_with_thumbnails(true, |mut h, _capture| {
        h.update(|editor| editor.add_raws(paths));
        wait_for(&mut h, |e| !e.busy && e.document_pending.is_none());
        h.update(|e| {
            assert!(e.document.visit.ready.is_some(), "{}", e.status);
            e.dmin = Some(DVec3::splat(3000. / 4031.));
            e.dmax = Some(DVec3::splat(128. / 4031.));
            e.roll.gamma = 0.75;
            e.photo.exposure = 0.5;
            e.source = false;
            e.roll_orientation.turn_clockwise();
            e.photo_orientation.flips.horizontal = true;
            e.document.project.analysis_regions = e
                .document
                .project
                .order
                .iter()
                .map(|&source| {
                    let active = e
                        .document
                        .project
                        .source(source)
                        .unwrap()
                        .development
                        .active_area
                        .into();
                    let geometry = PictureGeometry {
                        size: [768, 512],
                        angle: 0.75,
                        sides: [Some(100.), Some(660.), Some(80.), Some(430.)],
                    };
                    SourceAnalysis {
                        source,
                        areas: Areas {
                            opening: Some(Coverage::rectangle(active).corners().map(|p| p.0)),
                            picture: geometry.frame(active),
                            picture_geometry: Some(geometry),
                            picture_observed: [true; 4],
                            ..Default::default()
                        },
                    }
                })
                .collect();
            e.sync_document();
        });
        h.step();
        h.update(|editor| {
            editor.queue_save(
                directory.path().join("accepted.json"),
                crate::project::Save::New,
                None,
            )
        });
        wait_for(&mut h, |e| !e.busy && e.document_pending.is_none());
        h.read(|editor| assert!(!editor.document.dirty(), "{}", editor.status));
        test(h, directory)
    })
}

fn click_action(h: &mut Harness<'_, Editor>, label: &str) {
    h.get_by_role_and_label(egui::accesskit::Role::Button, label)
        .scroll_to_me();
    h.try_run_realtime().unwrap();
    h.get_by_role_and_label(egui::accesskit::Role::Button, label)
        .click();
    h.step();
    h.step();
}

fn prior_crop(h: &mut Harness<'_, Editor>) -> Crop {
    let crop = h.update(|e| {
        let a = e.document.visit.ready.as_ref().unwrap().area;
        let crop = Crop::from_region(Rect {
            x: a.x + a.width / 4,
            y: a.y + a.height / 4,
            width: a.width / 2,
            height: a.height / 2,
        });
        e.crop = Some(crop);
        e.sync_document();
        crop
    });
    h.step();
    save_existing_project(h);
    crop
}

fn wait_crop(h: &mut Harness<'_, Editor>, crop: Crop) {
    h.read(|editor| assert!(editor.document.auto_crop_edit.is_none()));
    wait_for(h, |e| {
        assert!(e.working.error.is_none(), "{:?}", e.working.error);
        !e.working.updating
            && e.working
                .displayed
                .as_ref()
                .is_some_and(|v| v.spec.frame.composition.crop() == crop)
    });
    h.read(|editor| assert_eq!(editor.last_layout.unwrap().0.composition.crop(), crop));
    let timing = h.read(|editor| Arc::new(SelectionTiming::new(editor.id, Instant::now())));
    h.update(|editor| editor.selection_timing = Some(timing.clone()));
    h.step();
    h.render().unwrap();
    assert!(timing.ready_was_painted());
    let input = h.read(|editor| editor.scope_request().unwrap());
    assert_eq!(input.frame().composition.crop(), crop);
    match input {
        scopes::ScopeInput::Rendered(view, _) => h.read(|editor| {
            assert!(Arc::ptr_eq(
                &view,
                editor.working.displayed.as_ref().unwrap()
            ))
        }),
        _ => panic!("scopes must consume the accepted physical working/original view"),
    }
}

fn assert_lifecycle(h: &Harness<'_, Editor>, size: egui::Vec2) {
    let canvas = h.read(|editor| {
        editor
            .document
            .visit
            .crop_edit
            .as_ref()
            .and_then(|e| e.layout)
            .map(|l| l.canvas)
            .or_else(|| editor.last_layout.map(|(l, _)| l.canvas))
            .unwrap()
    });
    for label in ["Apply", "Cancel"] {
        let rect = h
            .get_by_role_and_label(egui::accesskit::Role::Button, label)
            .rect();
        assert!(
            rect.left() >= canvas.left()
                && rect.right() <= canvas.right()
                && rect.top() >= canvas.bottom()
                && rect.bottom() <= size.y,
            "{label}: {rect:?}, {canvas:?}"
        );
    }
    assert!(
        h.query_by_role_and_label(egui::accesskit::Role::Button, "Done")
            .is_none()
    );
    assert!(h.query_by_label("Show crop overlay").is_none());
}

#[test]
fn unavailable_outlines_have_compact_tool_and_thumbnail_cues_at_both_layouts() {
    for (name, size, width) in [
        ("normal", egui::vec2(1100., 800.), 315.),
        ("narrow", egui::vec2(800., 600.), 270.),
    ] {
        setup(|mut h, _directory| {
            set_layout(&mut h, size, width);
            let ids = h.read(|editor| editor.document.project.order.clone());
            h.update(|e| {
                e.document
                    .project
                    .analysis_regions
                    .retain(|r| r.source != ids[0]);
                let areas = &mut e.document.project.analysis_regions[0].areas;
                areas.picture_observed[0] = false;
                areas.picture_weak[0] = true;
            });
            h.step();
            save_existing_project(&mut h);
            let before = h.read(|editor| editor.document.project.clone());
            stage(&mut h, "Roll");
            click_action(&mut h, "Auto-crop roll");
            overlay::wait_context(&mut h);
            h.read(|editor| {
                assert!(
                    editor
                        .auto_crop_outline(editor.last_layout.unwrap().0)
                        .is_none()
                )
            });
            let cue = h.get_by_label("Tool warning");
            assert!(
                cue.accesskit_node()
                    .description()
                    .unwrap()
                    .contains("no retained frame evidence")
            );
            assert_eq!(
                h.get_all_by_role_and_label(egui::accesskit::Role::Image, "Photo warning")
                    .count(),
                2
            );
            let point = h
                .get_all_by_role_and_label(egui::accesskit::Role::Image, "Photo warning")
                .find(|n| {
                    n.accesskit_node()
                        .description()
                        .unwrap()
                        .contains("four measured picture edges")
                })
                .unwrap()
                .rect()
                .center();
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
            wait_for(&mut h, |e| !e.busy && e.document.visit.ready.is_some());
            h.read(|editor| {
                assert_eq!(
                    editor.current_source,
                    Some(ids[1]),
                    "badge hover must not steal photo selection"
                )
            });
            assert!(
                h.get_by_label("Tool warning")
                    .accesskit_node()
                    .description()
                    .unwrap()
                    .contains("four measured picture edges")
            );
            assert_lifecycle(&h, size);
            save_capture(&h.render().unwrap(), &format!("crop-warning-{name}"));
            h.get_by_label("crop-2.dng").click();
            h.step();
            wait_for(&mut h, |e| !e.busy && e.document.visit.ready.is_some());
            overlay::wait_context(&mut h);
            assert!(h.query_by_label("Tool warning").is_none());
            h.read(|editor| {
                assert!(
                    editor
                        .auto_crop_outline(editor.last_layout.unwrap().0)
                        .is_some()
                )
            });
            click_action(&mut h, "Cancel");
            h.read(|editor| assert_eq!(editor.document.project, before));
            assert_eq!(
                h.query_all_by_role_and_label(egui::accesskit::Role::Image, "Photo warning")
                    .count(),
                0
            );
            for name in ["crop-0.dng", "crop-1.dng"] {
                assert!(
                    !h.get_by_label(name)
                        .accesskit_node()
                        .description()
                        .is_some_and(|d| d.contains("crop proposal"))
                );
            }
            h.update(|editor| editor.document.project.analysis_regions.clear());
            h.step();
            save_existing_project(&mut h);
            let before = h.read(|editor| editor.document.project.clone());
            click_action(&mut h, "Auto-crop roll");
            overlay::wait_context(&mut h);
            assert!(h.get_by_label("Apply").accesskit_node().is_disabled());
            assert!(
                h.query_by_role_and_label(egui::accesskit::Role::Slider, "Inward margin")
                    .is_none()
            );
            assert_eq!(
                h.get_all_by_role_and_label(egui::accesskit::Role::Image, "Photo warning")
                    .count(),
                3
            );
            h.get_by_label("Tool warning");
            assert_lifecycle(&h, size);
            save_capture(
                &h.render().unwrap(),
                &format!("crop-all-unavailable-{name}"),
            );
            click_action(&mut h, "Cancel");
            h.read(|editor| assert_eq!(editor.document.project, before));
            h.read(|editor| assert!(!editor.document.dirty()));
            stage(&mut h, "Photo");
            click_action(&mut h, "Crop & straighten");
            workflow::enter_numeric(&mut h, "Angle", "2.3");
            let draft = h.read(|editor| editor.document.visit.crop_edit.as_ref().unwrap().draft);
            click_action(&mut h, "Auto-crop");
            h.read(|editor| {
                assert_eq!(
                    editor.document.visit.crop_edit.as_ref().unwrap().draft,
                    draft
                )
            });
            assert!(
                h.get_by_label("Tool warning")
                    .accesskit_node()
                    .description()
                    .unwrap()
                    .contains("Automatic crop unavailable")
            );
            assert_eq!(
                h.get_all_by_role_and_label(egui::accesskit::Role::Image, "Photo warning")
                    .count(),
                1
            );
            click_action(&mut h, "Cancel");
            h.read(|editor| assert_eq!(editor.document.project, before));
            assert!(h.query_by_label("Tool warning").is_none());
            assert_eq!(
                h.query_all_by_role_and_label(egui::accesskit::Role::Image, "Photo warning")
                    .count(),
                0
            );
        });
    }
}

#[test]
fn photo_auto_crop_initializes_the_normal_crop_draft_at_both_layouts() {
    for (name, size, width) in [
        ("normal", egui::vec2(1100., 800.), 315.),
        ("narrow", egui::vec2(800., 600.), 270.),
    ] {
        setup(|mut h, _directory| {
            set_layout(&mut h, size, width);
            stage(&mut h, "Photo");
            let old = prior_crop(&mut h);
            let before = h.read(|editor| editor.document.project.clone());
            let source = h.read(|editor| editor.current_source.unwrap());
            assert!(
                h.query_by_role_and_label(egui::accesskit::Role::Button, "Auto-crop")
                    .is_none()
            );
            let shelf = h
                .get_by_role_and_label(egui::accesskit::Role::Button, "Crop & straighten")
                .rect();
            click_action(&mut h, "Crop & straighten");
            workflow::enter_numeric(&mut h, "Angle", "2.3");
            click_action(&mut h, "Auto-crop");
            let proposed = margin::expected_crop(&h, source, 0.5);
            h.read(|editor| {
                assert_eq!(
                    editor.document.visit.crop_edit.as_ref().unwrap().draft,
                    proposed
                )
            });
            h.read(|editor| assert!(editor.document.auto_crop_edit.is_none()));
            h.read(|editor| assert_eq!(editor.document.project, before));
            h.read(|editor| assert_eq!(editor.crop, Some(old)));
            h.read(|editor| assert!(!editor.document.dirty()));
            assert_eq!(
                h.get_by_role_and_label(egui::accesskit::Role::Button, "Crop & straighten")
                    .rect(),
                shelf
            );
            assert_lifecycle(&h, size);
            save_capture(&h.render().unwrap(), &format!("simple-photo-crop-{name}"));
            click_action(&mut h, "Cancel");
            h.read(|editor| assert!(editor.document.visit.crop_edit.is_none()));
            h.read(|editor| assert_eq!(editor.document.project, before));
            wait_crop(&mut h, old);
            click_action(&mut h, "Crop & straighten");
            click_action(&mut h, "Auto-crop");
            click_action(&mut h, "Apply");
            let mut expected = before;
            expected.source_mut(source).unwrap().crop = Some(proposed);
            h.read(|editor| assert_eq!(editor.document.project, expected));
            h.read(|editor| assert_eq!(editor.crop, Some(proposed)));
            wait_crop(&mut h, proposed);
            click_action(&mut h, "Dust repair");
            h.read(|editor| assert!(editor.document.dust.cleanup));
            click_action(&mut h, "Done");
            h.read(|editor| assert_eq!(editor.document.project, expected));
        });
    }
}

#[test]
fn roll_shelf_keeps_orientation_and_tools_fixed_and_preserves_photo_exceptions() {
    for (name, size, width) in [
        ("normal", egui::vec2(1100., 800.), 315.),
        ("narrow", egui::vec2(800., 600.), 270.),
    ] {
        setup(|mut h, _directory| {
            set_layout(&mut h, size, width);
            stage(&mut h, "Roll");
            let before = h.read(|editor| editor.document.project.clone());
            for label in [
                "Turn left",
                "Turn right",
                "Flip horizontal",
                "Flip vertical",
                "Reset scan orientation",
                "Analyze",
                "Auto-crop roll",
            ] {
                let rect = h
                    .get_by_role_and_label(egui::accesskit::Role::Button, label)
                    .rect();
                assert!(
                    rect.min.x >= size.x - width
                        && rect.max.x <= size.x
                        && rect.min.y > 0.
                        && rect.max.y < size.y,
                    "{label}: {rect:?}"
                );
            }
            let shelf = h
                .get_by_role_and_label(egui::accesskit::Role::Button, "Turn left")
                .rect();
            click_action(&mut h, "Reset scan orientation");
            click_action(&mut h, "Turn left");
            click_action(&mut h, "Flip horizontal");
            let mut expected = before;
            expected.orientation = RollOrientation::default();
            expected.orientation.turn_counterclockwise();
            expected.orientation.horizontal = true;
            h.read(|editor| assert_eq!(editor.document.project, expected));
            h.get_by_role_and_label(egui::accesskit::Role::Button, "Exposure")
                .scroll_to_me();
            h.try_run_realtime().unwrap();
            assert_eq!(
                h.get_by_role_and_label(egui::accesskit::Role::Button, "Turn left")
                    .rect(),
                shelf
            );
            save_capture(&h.render().unwrap(), &format!("simple-roll-shelf-{name}"));
        });
    }
}

#[test]
fn whole_roll_entry_ignores_selection_and_keeps_abstentions_unchanged() {
    setup(|mut h, _directory| {
        let ids = h.read(|editor| editor.document.project.order.clone());
        h.update(|e| {
            let a = &mut e.document.project.analysis_regions[1].areas;
            a.picture_observed[0] = false;
            a.picture_weak[0] = true;
        });
        h.step();
        save_existing_project(&mut h);
        let before = h.read(|editor| editor.document.project.clone());
        h.read(|editor| {
            assert!(
                editor.roll_selection.contains(ids[0]) && !editor.roll_selection.contains(ids[2])
            )
        });
        stage(&mut h, "Roll");
        click_action(&mut h, "Auto-crop roll");
        overlay::wait_context(&mut h);
        h.read(|editor| {
            assert!(
                editor.status.contains("2 proposed, 1 skipped"),
                "{}",
                editor.status
            )
        });
        h.read(|editor| {
            assert!(
                editor.auto_crop_preview(ids[0]).is_some()
                    && editor.auto_crop_preview(ids[2]).is_some()
                    && editor.auto_crop_preview(ids[1]).is_none()
            )
        });
        h.read(|editor| assert_eq!(editor.document.project, before));
        h.read(|editor| {
            assert!(editor.transition.is_none() && editor.document.analysis_review.is_none())
        });
        let mut expected = before;
        for id in [ids[0], ids[2]] {
            expected.source_mut(id).unwrap().crop = Some(margin::expected_crop(&h, id, 0.5));
        }
        click_action(&mut h, "Apply");
        h.read(|editor| assert_eq!(editor.document.project, expected));
        h.read(|editor| assert!(editor.document.auto_crop_edit.is_none()));
    });
}

#[test]
fn selected_look_cards_validate_auto_crop_preview_framing_without_mutating_the_accepted_recipe() {
    setup(|mut h, _directory| {
        h.update(|editor| {
            editor.document.project.output.look = Some(Arc::new(
                crate::looks::LookAsset::new(
                    "Preview look".into(),
                    crate::looks::LookOrigin::Imported,
                    Arc::new(
                        crate::looks::Lut3D::new(2, [0.; 3], [1.; 3], vec![[0.4, 0.3, 0.2]; 8])
                            .unwrap(),
                    ),
                )
                .unwrap(),
            ))
        });
        h.step();
        save_existing_project(&mut h);
        let before = h.read(|editor| editor.document.project.clone());
        let source = h.read(|editor| editor.current_source.unwrap());
        stage(&mut h, "Roll");
        click_action(&mut h, "Auto-crop roll");
        overlay::wait_context(&mut h);
        let record = before.source(source).unwrap();
        let projection = h.read(|editor| {
            Composition::with_crop(
                record.development.active_area.into(),
                before.orientation,
                record.orientation,
                editor.auto_crop_preview(source).unwrap(),
            )
            .unwrap()
        });
        assert_ne!(projection, before.image_recipe(source).unwrap().composition);
        wait_for(&mut h, |editor| {
            editor
                .thumbnails
                .projection_ready(&editor.document, source, editor.target, projection)
        });
        h.read(|editor| assert_eq!(editor.document.project, before));
        h.read(|editor| assert!(!editor.document.dirty()));
        assert!(
            h.query_by_role_and_label(egui::accesskit::Role::Image, "Photo error")
                .is_none()
        );
        click_action(&mut h, "Cancel");
        h.read(|editor| assert!(editor.document.auto_crop_edit.is_none()));
        wait_for(&mut h, |editor| {
            editor
                .thumbnails
                .ready_count(&editor.document, editor.target)
                == 3
        });
        h.read(|editor| assert_eq!(editor.document.project, before));
    });
}

#[test]
fn fullscreen_pauses_auto_crop_and_escape_returns_to_the_tool_before_cancelling_it() {
    setup(|mut h, _directory| {
        stage(&mut h, "Photo");
        let old = prior_crop(&mut h);
        let before = h.read(|editor| editor.document.project.clone());
        stage(&mut h, "Roll");
        click_action(&mut h, "Auto-crop roll");
        overlay::wait_context(&mut h);
        workflow::enter_numeric(&mut h, "Inward margin", "2");
        let draft = h.read(|editor| editor.auto_crop_preview(editor.current_source.unwrap()));
        h.key_press(egui::Key::F);
        h.step();
        h.step();
        h.read(|editor| {
            assert!(
                editor.fullscreen.is_some()
                    && editor.document.auto_crop_edit.is_some()
                    && !editor.auto_crop_context()
            )
        });
        assert!(
            h.query_by_role_and_label(egui::accesskit::Role::Button, "Apply")
                .is_none()
        );
        wait_for(&mut h, |e| {
            !e.working.updating
                && e.working
                    .displayed
                    .as_ref()
                    .is_some_and(|v| v.spec.frame.composition.crop() == old)
        });
        h.key_press(egui::Key::Enter);
        h.step();
        h.read(|editor| assert!(editor.document.auto_crop_edit.is_some()));
        h.read(|editor| assert_eq!(editor.document.project, before));
        h.key_press(egui::Key::Escape);
        h.step();
        h.update(|editor| editor.fullscreen_changed(false));
        h.step();
        overlay::wait_context(&mut h);
        h.read(|editor| assert!(editor.fullscreen.is_none()));
        h.read(|editor| {
            assert_eq!(
                editor.auto_crop_preview(editor.current_source.unwrap()),
                draft
            )
        });
        h.key_press(egui::Key::Escape);
        h.step();
        h.step();
        h.read(|editor| assert!(editor.document.auto_crop_edit.is_none()));
        h.read(|editor| assert_eq!(editor.document.project, before));
        h.read(|editor| assert!(!editor.document.dirty()));
        wait_crop(&mut h, old);
    });
}
