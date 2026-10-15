use super::*;

fn show_confirmation(harness: &mut Harness<'_, Editor>) {
    harness.step();
    harness.get_by_label("Analyze roll").scroll_to_me();
    harness.try_run_realtime().unwrap();
    harness.get_by_label("Analyze roll").click();
    harness.step();
    harness.step();
    wait_for(harness, |editor| {
        editor.analysis_review.is_some() || !editor.busy
    });
    assert!(
        harness.state().analysis_review.is_some(),
        "{}",
        harness.state().status
    );
    assert!(harness.state().analysis_pending.is_none());
    assert!(!harness.state().busy);
    if harness
        .state()
        .analysis_review
        .as_ref()
        .unwrap()
        .replaces_photos
    {
        harness.get_by_label("Replace Photo adjustments");
        harness.get_by_label("Warning");
    }
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
    assert!(harness.state().analysis_review.is_some());
    assert!(harness.state().analysis_pending.is_some());
    assert!(harness.state().busy);
}

fn draw_picture(harness: &mut Harness<'_, Editor>, area: Rect) {
    harness
        .get_by_label("Edit shared picture area…")
        .scroll_to_me();
    harness.try_run_realtime().unwrap();
    harness.get_by_label("Edit shared picture area…").click();
    harness.step();
    assert_eq!(harness.state().pick, Some(Pick::Analysis));
    wait_for(harness, |editor| editor.analysis_area_pending.is_none());
    harness.get_by_label("Fit").click();
    harness.step();
    let geometry = harness.state().last_layout.unwrap().0;
    let start = geometry.screen_point(egui::pos2(area.x as f32, area.y as f32));
    let end = geometry.screen_point(egui::pos2(
        (area.x + area.width) as f32,
        (area.y + area.height) as f32,
    ));
    if let Some(region) = harness.state().region {
        let bounds = geometry.screen_coverage(region);
        if bounds.min.distance(start) > 0.01 {
            drag(harness, bounds.min, start);
        }
        let region = harness.state().region.unwrap();
        drag(harness, geometry.screen_coverage(region).max, end);
    } else {
        drag(harness, start, end);
    }
    assert_eq!(
        harness
            .state()
            .region
            .unwrap()
            .exact_rectangle(harness.state().ready.as_ref().unwrap().area),
        Some(area)
    );
}

fn settled_targets(harness: &mut Harness<'_, Editor>, case: &str) {
    let mut project = harness.state().document.project.clone();
    stage(harness, "Rendering");
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        select_target(harness, target);
        project.output.tiff = if target == OutputTarget::Srgb {
            crate::project::TiffTarget::Srgb
        } else {
            crate::project::TiffTarget::DisplayP3
        };
        super::workflow::wait_settled(harness);
        let view = harness.state().working.displayed.as_ref().unwrap();
        assert_eq!(
            [view.image.linear.width(), view.image.linear.height()],
            view.spec.frame.output
        );
        assert!(view.spec.frame.output.into_iter().all(|size| size > 0));
        assert!(!harness.state().source);
        let timing = Arc::new(SelectionTiming::new(harness.state().id, Instant::now()));
        harness.state_mut().selection_timing = Some(timing.clone());
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
        assert_eq!(harness.state().document.project, project);
    }
    stage(harness, "Roll");
}

fn exercise_analyze(harness: &mut Harness<'_, Editor>, picture: Rect, case: &str) {
    {
        let editor = harness.state_mut();
        editor.roll.gamma = 0.7;
        editor.roll_orientation.turn_clockwise();
        editor.photo_orientation.flips.horizontal = true;
        editor.crop = Some(
            Crop::from_region(picture)
                .straightened(editor.ready.as_ref().unwrap().area, 0.05)
                .unwrap(),
        );
        editor.photo.exposure = 0.5;
        editor.photo.wb = Balance {
            warmth: 0.1,
            tint: -0.05,
        };
        editor.photo.contrast = 1.2;
        editor.sync_document();
    }
    harness.step();
    let selected = harness.state().current_source;
    let original = harness.state().document.project.clone();
    let correction = harness.state().correction.clone();
    draw_picture(harness, picture);
    assert_eq!(harness.state().document.project, original);
    harness.get_by_label("Cancel").click();
    harness.step();
    assert!(harness.state().pick.is_none());
    assert_eq!(harness.state().document.project, original);
    draw_picture(harness, picture);
    harness.get_by_label("Apply").click();
    harness.step();
    assert!(harness.state().pick.is_none());
    let mut before = original;
    before.analysis_area = Some([picture.x, picture.y, picture.width, picture.height]);
    assert_eq!(harness.state().document.project, before);

    show_confirmation(harness);
    assert_eq!(harness.state().document.project, before);
    harness.get_by_label("Cancel").click();
    harness.step();
    assert!(harness.state().analysis_review.is_none());
    assert!(harness.state().analysis_pending.is_none());
    assert!(harness.state().status.contains("areas and grade unchanged"));
    assert_eq!(harness.state().document.project, before);

    show_confirmation(harness);
    // Hold only reply delivery so the fast authored worker cannot install before the
    // Cancel widget is painted. Cancellation still uses the real dispatched worker.
    let (_hold, inbox) = mpsc::channel();
    let replies = std::mem::replace(&mut harness.state_mut().messages, inbox);
    confirm(harness);
    assert_eq!(harness.state().document.project, before);
    harness.step();
    harness.get_by_label("Cancel analysis").click();
    harness.step();
    assert!(harness.state().busy);
    assert_eq!(harness.state().document.project, before);
    harness.state_mut().messages = replies;
    wait_for(harness, |editor| !editor.busy);
    assert!(harness.state().analysis_pending.is_none());
    assert_eq!(harness.state().status, "Analysis cancelled");
    assert_eq!(harness.state().document.project, before);

    show_confirmation(harness);
    confirm(harness);
    assert_eq!(harness.state().document.project, before);
    wait_for(harness, |editor| !editor.busy);
    assert!(harness.state().analysis_pending.is_none());
    assert!(
        harness.state().status.starts_with("Analyzed 2"),
        "{}",
        harness.state().status
    );
    let after = &harness.state().document.project;
    assert_eq!(after.order, before.order);
    assert!(after.analysis_area.is_none());
    assert_eq!(after.analysis_regions.len(), after.order.len());
    assert!(
        after
            .analysis_regions
            .iter()
            .all(|e| e.areas.picture == Some(Coverage::rectangle(picture).corners().map(|p| p.0)))
    );
    assert_eq!(after.roll.gamma, before.roll.gamma);
    assert_eq!(after.roll.response, before.roll.response);
    assert_eq!(after.orientation, before.orientation);
    assert_eq!(after.flat_field, before.flat_field);
    assert_eq!(harness.state().current_source, selected);
    for (mut source, prior) in after.sources.clone().into_iter().zip(&before.sources) {
        if after.order.contains(&source.id) {
            assert_eq!(
                source.photo,
                PhotoSettings {
                    exposure: source.photo.exposure,
                    ..PhotoSettings::default()
                }
            );
        }
        source.photo = prior.photo;
        assert_eq!(&source, prior);
    }
    if let Some(correction) = correction {
        assert!(Arc::ptr_eq(
            &correction,
            harness.state().correction.as_ref().unwrap()
        ));
    } else {
        assert!(
            after
                .dmin
                .unwrap()
                .into_iter()
                .all(|value| (value - 3000. / 4031.).abs() <= 2. / 65535.)
        );
        let exposures: Vec<_> = after
            .order
            .iter()
            .map(|id| after.source(*id).unwrap().photo.exposure)
            .collect();
        assert!(exposures[0] > exposures[1]);
    }
    settled_targets(harness, case);

    draw_picture(
        harness,
        Rect {
            width: 8,
            height: 8,
            ..picture
        },
    );
    harness.get_by_label("Apply").click();
    harness.step();
    let failed = harness.state().document.project.clone();
    // Default and adjusted Photos both review areas; only adjusted Photos show the reset warning.
    show_confirmation(harness);
    confirm(harness);
    wait_for(harness, |editor| !editor.busy);
    assert!(
        harness.state().status.contains("picture samples"),
        "{}",
        harness.state().status
    );
    assert_eq!(harness.state().document.project, failed);
    assert_eq!(harness.state().current_source, selected);
    assert!(harness.state().ready.is_some());
    assert!(harness.state().preference_store.is_none());
}

#[test]
fn roll_area_review_supports_normal_and_narrow_layouts_and_exception_browsing() {
    let (mut harness, _) = editor_harness_with_thumbnails(true);
    harness
        .state_mut()
        .add_raws(vec![fixture("roll-0.dng"), fixture("roll-1.dng")]);
    wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
    harness.state_mut().photo.exposure = 1.25;
    harness.state_mut().sync_document();
    let before = harness.state().document.project.clone();
    let first = harness.state().current_source;
    let scopes_visible = harness.state().scopes.visible;
    show_confirmation(&mut harness);
    wait_for(&mut harness, |e| {
        e.thumbnails.ready_count(&e.document, e.target) == 2
    });
    for (layout, size) in [
        ("normal", egui::vec2(1100., 800.)),
        ("narrow", egui::vec2(800., 600.)),
    ] {
        harness.set_size(size);
        harness.run();
        for picture in [false, true] {
            if picture {
                harness.get_by_label("Picture statistics").click();
            } else {
                harness.get_by_label("Opening").click();
            }
            harness.run();
            assert_eq!(
                harness.state().analysis_review.as_ref().unwrap().picture,
                picture
            );
            assert!(harness.state().source);
            assert!(!harness.state().scopes.visible);
            assert_eq!(harness.state().preferences.scopes_visible, scopes_visible);
            harness.get_by_label("Roll analysis");
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
                assert!(
                    rect.min.x >= 0.
                        && rect.max.x <= harness.state().last_layout.unwrap().0.canvas.max.x
                        && rect.max.y <= size.y,
                    "{layout}: {rect:?}"
                );
            }
            assert!(!apply.rect().intersects(cancel.rect()));
            for text in [
                "Replace Photo adjustments",
                "Analyze replaces Photo grading with estimated exposure and defaults for the other Photo controls.",
            ] {
                let warning = harness.get_by_label(text).rect();
                assert!(
                    warning.min.y > 0. && warning.max.y < apply.rect().max.y,
                    "{layout}: warning hidden: {warning:?}"
                );
            }
            save_capture(
                &harness.render().unwrap(),
                &format!(
                    "analyze-review-{layout}-{}-offscreen",
                    if picture { "picture" } else { "opening" }
                ),
            );
            assert_eq!(harness.state().document.project, before);
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
        assert_eq!(harness.state().document.project, before);
        stage(&mut harness, "Roll");
    }
    assert_eq!(
        harness
            .query_all_by_role_and_label(egui::accesskit::Role::Button, "Next area to inspect")
            .count(),
        0
    );
    harness.get_by_label("Opening").click();
    harness.run();
    let prior = harness
        .state()
        .review_areas(first.unwrap())
        .unwrap()
        .clone();
    assert!(prior.picture_geometry.is_some() && !prior.picture_edited);
    let q = harness.state().region.unwrap().corners().map(|p| p.0);
    let p = prior.picture.unwrap()[0];
    let geometry = harness.state().last_layout.unwrap().0;
    drag(
        &mut harness,
        geometry.screen_point(egui::pos2(q[0][0] as f32, q[0][1] as f32)),
        geometry.screen_point(egui::pos2(p[0] as f32 + 12., p[1] as f32 + 12.)),
    );
    let edited = harness.state().review_areas(first.unwrap()).unwrap();
    assert!(edited.opening_edited && !edited.picture_edited);
    assert_ne!(edited.picture, prior.picture);
    assert_eq!(harness.state().document.project, before);
    harness.get_by_label("Picture statistics").click();
    harness.run();
    assert_eq!(
        harness.state().region.unwrap().corners().map(|p| p.0),
        harness
            .state()
            .review_areas(first.unwrap())
            .unwrap()
            .picture
            .unwrap()
    );
    harness.get_by_label("roll-1.dng").click();
    harness.step();
    wait_for(&mut harness, |e| !e.busy);
    assert_ne!(harness.state().current_source, first);
    assert!(
        !harness
            .state()
            .review_areas(harness.state().current_source.unwrap())
            .unwrap()
            .opening_edited
    );
    assert_eq!(harness.state().document.project, before);
    harness.get_by_label("Cancel").click();
    harness.step();
    wait_for(&mut harness, |e| !e.busy);
    assert_eq!(harness.state().current_source, first);
    assert_eq!(harness.state().scopes.visible, scopes_visible);
    assert_eq!(harness.state().preferences.scopes_visible, scopes_visible);
    assert_eq!(harness.state().document.project, before);
}
#[test]
fn analyze_widgets_preserve_area_cancel_failure_and_both_settled_targets() {
    let (mut harness, _) = editor_harness();
    harness
        .state_mut()
        .add_raws(vec![fixture("roll-0.dng"), fixture("roll-1.dng")]);
    wait_for(&mut harness, |editor| {
        !editor.busy && editor.document_pending.is_none()
    });
    assert!(
        harness.state().ready.is_some(),
        "{}",
        harness.state().status
    );
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
}

#[test]
fn analyze_widgets_keep_real_correction_and_both_settled_targets() {
    let directory = tempfile::tempdir().unwrap();
    let second = directory.path().join("second-negative.dng");
    std::fs::copy(fixture("illuminated-patches.dng"), &second).unwrap();
    let (mut harness, _) = editor_harness();
    harness
        .state_mut()
        .add_raws(vec![fixture("illuminated-patches.dng"), second]);
    wait_for(&mut harness, |editor| {
        !editor.busy && editor.document_pending.is_none()
    });
    assert!(
        harness.state().ready.is_some(),
        "{}",
        harness.state().status
    );
    // Supply the native chooser result; map estimation/application still use production workers.
    harness
        .state_mut()
        .begin_flat_edit(fixture("bare-light.dng"), None);
    wait_for(&mut harness, |editor| !editor.busy);
    harness.get_by_label("Apply").click();
    harness.step();
    wait_for(&mut harness, |editor| {
        !editor.busy && editor.document_pending.is_none()
    });
    assert!(
        harness.state().correction.is_some(),
        "{}",
        harness.state().status
    );
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
}
