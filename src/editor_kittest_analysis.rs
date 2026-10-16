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
        editor.analysis_review.is_some() || !editor.busy
    });
    assert!(
        harness.state().analysis_review.is_some(),
        "{}",
        harness.state().status
    );
    assert!(harness.state().analysis_pending.is_none());
    assert!(!harness.state().busy);
    let editor = harness.state();
    let source = ProcessingInput::from(editor.source_parameters(editor.source_exposure).unwrap());
    let signature = editor
        .original
        .signature
        .as_ref()
        .expect("review uses original-source tool presentation");
    assert_eq!(signature.0, source.signature());
    assert_eq!(
        signature.3,
        Composition::new(
            editor.ready.as_ref().unwrap().area,
            Default::default(),
            Default::default()
        )
        .unwrap()
    );
    assert!(matches!(editor.scope_request(), Err("Positive grade only")));
    if harness
        .state()
        .analysis_review
        .as_ref()
        .unwrap()
        .replaces_photos
    {
        let warning = harness.get_by_label("Tool warning");
        let description = warning.accesskit_node().description().unwrap();
        assert!(description.contains("Photo adjustments"));
        assert!(description.contains("local adjustments and masks"));
        assert!(description.contains("cannot be undone"));
    }
}

fn select_area(h: &mut Harness<'_, Editor>, picture: bool) {
    let geometry = h.state().last_layout.unwrap().0;
    let point = (0..21)
        .flat_map(|y| {
            (0..21).map(move |x| {
                egui::pos2(
                    geometry.canvas.left() + 2. + (geometry.canvas.width() - 4.) * x as f32 / 20.,
                    geometry.canvas.top() + 2. + (geometry.canvas.height() - 4.) * y as f32 / 20.,
                )
            })
        })
        .find(|&p| h.state().review_area_at(geometry, p) == Some(picture))
        .expect("selectable sampling area");
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
    assert_eq!(h.state().analysis_review.as_ref().unwrap().picture, picture);
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
    harness.get_by_label("Sampling areas").scroll_to_me();
    harness.try_run_realtime().unwrap();
    if harness
        .query_by_label("Edit shared picture area…")
        .is_none()
    {
        harness.get_by_label("Sampling areas").click();
        harness.try_run_realtime().unwrap();
    }
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
        editor.dmin.get_or_insert(DVec3::new(0.2, 0.15, 0.1));
        editor.dmax.get_or_insert(DVec3::new(0.008, 0.005, 0.002));
        editor.source = false;
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
        let ids = adjustments::install_locals(editor);
        let mut locals = editor
            .document
            .project
            .source(editor.current_source.unwrap())
            .unwrap()
            .local_adjustments
            .clone()
            .unwrap();
        locals.next_id = 40;
        locals.adjustments[0].adjustment.photo.exposure = 0.25;
        locals.adjustments[1].adjustment.enabled = false;
        for id in editor.document.project.order.clone() {
            editor
                .document
                .project
                .source_mut(id)
                .unwrap()
                .local_adjustments = Some(locals.clone());
        }
        let mut retired = editor
            .document
            .project
            .source(editor.current_source.unwrap())
            .unwrap()
            .clone();
        retired.id = SourceId(0);
        retired.path = "retired-local-reference.dng".into();
        retired.photo = PhotoSettings::default();
        editor
            .document
            .register_reference(
                retired,
                editor.document.project.development.clone().unwrap(),
            )
            .unwrap();
        editor.select_adjustment(super::super::adjustments::Target::Local(ids[0]));
        editor.sync_document();
        editor
            .validate_document_candidate(&editor.document)
            .unwrap();
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
    assert_eq!(
        harness.state().adjustment_target(),
        super::super::adjustments::Target::Global
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
            let locals = source.local_adjustments.as_ref().unwrap();
            assert!(locals.adjustments.is_empty());
            assert_eq!(
                locals.next_id,
                prior.local_adjustments.as_ref().unwrap().next_id
            );
            source.local_adjustments = prior.local_adjustments.clone();
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
    {
        let editor = harness.state_mut();
        let adjustment = before
            .source(selected.unwrap())
            .unwrap()
            .local_adjustments
            .as_ref()
            .unwrap()
            .adjustments[0]
            .adjustment;
        editor
            .edit_adjustments(|locals| {
                locals.add("Survives failed Analyze".into(), adjustment)?;
                Ok(())
            })
            .unwrap();
    }

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
fn missing_area_cues_follow_the_photo_and_clear_without_an_inspector_panel() {
    let (mut h, _) = editor_harness_with_thumbnails(true);
    h.state_mut()
        .add_raws(vec![fixture("roll-0.dng"), fixture("roll-1.dng")]);
    wait_for(&mut h, |e| !e.busy && e.document_pending.is_none());
    show_confirmation(&mut h);
    assert!(!h.state().analysis_review.as_ref().unwrap().replaces_photos);
    let before = h.state().document.project.clone();
    let ids = before.order.clone();
    assert!(h.query_by_label("Tool warning").is_none());
    assert!(
        h.query_by_role_and_label(egui::accesskit::Role::Label, "Analyze")
            .is_none()
    );
    let picture = h.state().review_areas(ids[0]).unwrap().picture;
    h.state_mut()
        .analysis_review
        .as_mut()
        .unwrap()
        .approved
        .entries[0]
        .areas
        .picture = None;
    h.state_mut().refresh_review_region();
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
    wait_for(&mut h, |e| !e.busy && e.ready.is_some());
    assert!(h.state().source_tool_warning(ids[1]).is_none());
    assert!(
        h.get_by_label("Tool warning")
            .accesskit_node()
            .description()
            .unwrap()
            .contains("1 photo")
    );
    h.state_mut()
        .analysis_review
        .as_mut()
        .unwrap()
        .approved
        .entries[0]
        .areas
        .picture = picture;
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
    assert_eq!(h.state().document.project, before);
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
            select_area(&mut harness, picture);
            harness.run();
            assert_eq!(
                harness.state().analysis_review.as_ref().unwrap().picture,
                picture
            );
            assert!(harness.state().source);
            assert!(!harness.state().scopes.visible);
            assert_eq!(harness.state().preferences.scopes_visible, scopes_visible);
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
                assert!(
                    rect.min.x >= 0.
                        && rect.max.x <= harness.state().last_layout.unwrap().0.canvas.max.x
                        && rect.max.y <= size.y,
                    "{layout}: {rect:?}"
                );
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
    select_area(&mut harness, false);
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
    select_area(&mut harness, true);
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
    let source_epoch = harness
        .state()
        .source_epoch
        .load(std::sync::atomic::Ordering::Relaxed);
    let original_revision = harness.state().original.revision();
    harness.get_by_label("roll-1.dng").click();
    harness.step();
    wait_for(&mut harness, |e| !e.busy);
    assert_ne!(harness.state().current_source, first);
    assert_ne!(
        harness
            .state()
            .source_epoch
            .load(std::sync::atomic::Ordering::Relaxed),
        source_epoch
    );
    assert!(harness.state().original.revision() > original_revision);
    assert!(matches!(
        harness.state().scope_request(),
        Err("Positive grade only")
    ));
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
