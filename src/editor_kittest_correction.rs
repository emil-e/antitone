use super::workflow::{enter_numeric, wait_settled};
use super::*;

fn pick_patch(harness: &mut Harness<'_, Editor>, action: &str, area: Rect) {
    harness.get_by_label(action).scroll_to_me();
    harness.try_run_realtime().unwrap();
    harness.get_by_label(action).click();
    harness.step();
    draw_region(harness, area);
    harness.get_by_label("Apply").click();
    harness.step();
    wait_for(harness, |e| !e.busy);
    assert!(harness.state().pick.is_none(), "{}", harness.state().status);
}

#[test]
fn correction_widgets_measure_exactly_render_thumbnails_toggle_and_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let reference = directory.path().join("bare-light.dng");
    std::fs::copy(fixture("bare-light.dng"), &reference).unwrap();
    let (mut harness, _) = editor_harness_with_thumbnails(true);
    harness
        .state_mut()
        .add_raws(vec![fixture("illuminated-patches.dng")]);
    wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
    assert!(
        harness.state().ready.is_some(),
        "{}",
        harness.state().status
    );
    let before = harness.state().document.project.clone();
    let selected = harness.state().current_source;
    // The reference chooser itself remains native acceptance; use its resulting path.
    harness.state_mut().begin_flat_edit(reference.clone(), None);
    wait_for(&mut harness, |e| !e.busy);
    wait_settled(&mut harness);
    let geometry = harness.state().last_layout.unwrap().0;
    let original = harness.state().region.unwrap();
    assert!(geometry.canvas.expand(0.1).contains_rect(geometry.image));
    assert!(
        geometry
            .canvas
            .expand(0.1)
            .contains_rect(geometry.screen_coverage(original))
    );
    let center = geometry.screen_coverage(original).center();
    drag(
        &mut harness,
        center,
        center + egui::vec2(geometry.scale * 8., 0.),
    );
    let moved = harness.state().region.unwrap();
    assert_ne!(moved, original);
    let corner = geometry.screen_coverage(moved).right_bottom();
    drag(
        &mut harness,
        corner,
        corner - egui::vec2(geometry.scale * 4., geometry.scale * 4.),
    );
    let shrunk = harness
        .state()
        .region
        .unwrap()
        .exact_rectangle(harness.state().ready.as_ref().unwrap().area)
        .unwrap();
    let moved = moved
        .exact_rectangle(harness.state().ready.as_ref().unwrap().area)
        .unwrap();
    assert!(shrunk.width < moved.width && shrunk.height < moved.height);
    harness.get_by_label("Cancel").click();
    harness.step();
    assert_eq!(harness.state().document.project, before);
    assert_eq!(harness.state().current_source, selected);
    harness.state_mut().begin_flat_edit(reference.clone(), None);
    wait_for(&mut harness, |e| !e.busy);
    let geometry = harness.state().last_layout.unwrap().0;
    let region = harness.state().region.unwrap();
    drag(
        &mut harness,
        geometry.screen_coverage(region).right_bottom(),
        geometry.screen_coverage(region).right_bottom()
            - egui::vec2(geometry.scale * 8., geometry.scale * 8.),
    );
    let accepted = harness
        .state()
        .region
        .unwrap()
        .exact_rectangle(harness.state().ready.as_ref().unwrap().area)
        .unwrap();
    harness.get_by_label("Apply").click();
    harness.step();
    wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
    assert!(
        harness.state().correction.is_some(),
        "{}",
        harness.state().status
    );
    assert_eq!(harness.state().document.project.order, before.order);
    assert_eq!(harness.state().document.project.dmin, before.dmin);
    assert_eq!(harness.state().document.project.dmax, before.dmax);
    let settings = harness.state().document.project.flat_field.clone().unwrap();
    assert_eq!(
        settings.map.area(),
        [accepted.x, accepted.y, accepted.width, accepted.height]
    );
    assert!(
        harness
            .state()
            .document
            .project
            .source(settings.reference)
            .unwrap()
            .reference_only
    );
    assert!(
        !harness
            .state()
            .document
            .project
            .order
            .contains(&settings.reference)
    );
    let mut scan = Scan::open(
        &fixture("illuminated-patches.dng"),
        Some(
            harness
                .state()
                .document
                .project
                .development
                .as_ref()
                .unwrap()
                .fixed_wb,
        ),
    )
    .unwrap();
    scan.set_flat_field(Some(settings.map.clone())).unwrap();
    let [x, y, w, h] = settings.map.area();
    let mut patches = Vec::new();
    for gy in 0..9 {
        for gx in 0..9 {
            let region = Rect {
                x: x + 32 + gx * (w - 96) / 8,
                y: y + 32 + gy * (h - 96) / 8,
                width: 32,
                height: 32,
            };
            if let Ok(rgb) = scan.reference(region) {
                patches.push((region, rgb));
            }
        }
    }
    let dmin = *patches
        .iter()
        .max_by(|a, b| a.1.element_sum().total_cmp(&b.1.element_sum()))
        .expect("usable corrected patches");
    let dmax = *patches
        .iter()
        .filter(|p| (dmin.1 / p.1).min_element() > 1.00001)
        .max_by(|a, b| {
            (dmin.1 / a.1)
                .min_element()
                .total_cmp(&(dmin.1 / b.1).min_element())
        })
        .expect("ordered corrected RGB pair");
    pick_patch(&mut harness, "Pick Dmin (film base)", dmin.0);
    assert_eq!(harness.state().dmin, Some(dmin.1));
    pick_patch(&mut harness, "Pick Dmax", dmax.0);
    assert_eq!(harness.state().dmax, Some(dmax.1));
    let calibration = harness.state().calibration().unwrap();
    let (region, balance) = patches
        .iter()
        .find_map(|(r, _)| {
            scan.roll_neutral(*r, calibration, harness.state().roll)
                .ok()
                .map(|b| (*r, b))
        })
        .expect("usable corrected roll-neutral patch");
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "White balance")
        .scroll_to_me();
    harness.try_run_realtime().unwrap();
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "White balance")
        .click();
    harness.step();
    harness.try_run_realtime().unwrap();
    pick_patch(&mut harness, "Pick neutral…", region);
    assert_eq!(harness.state().roll.balance, balance);
    let (region, balance) = patches
        .iter()
        .find_map(|(r, _)| {
            scan.photo_neutral(*r, calibration, harness.state().roll, harness.state().photo)
                .ok()
                .map(|b| (*r, b))
        })
        .expect("usable corrected photo-neutral patch");
    stage(&mut harness, "Photo");
    pick_patch(&mut harness, "Pick neutral…", region);
    assert_eq!(harness.state().photo.wb, balance);
    if harness.state().source {
        harness.get_by_label("View source").click();
        harness.step();
    }
    stage(&mut harness, "Rendering");
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        select_target(&mut harness, target);
        wait_settled(&mut harness);
        wait_for(&mut harness, |e| {
            e.thumbnails.ready_count(&e.document, target) > 0
        });
        let geometry = harness.state().last_layout.unwrap().0;
        assert!(geometry.canvas.expand(0.1).contains_rect(geometry.image));
        let timing = Arc::new(SelectionTiming::new(harness.state().id, Instant::now()));
        harness.state_mut().selection_timing = Some(timing.clone());
        harness.step();
        let png = harness.render().unwrap();
        assert!(timing.ready_was_painted());
        save_capture(
            &png,
            if target == OutputTarget::Srgb {
                "correction-positive-srgb-offscreen"
            } else {
                "correction-positive-p3-offscreen"
            },
        );
    }
    harness.get_by_label("View source").click();
    harness.step();
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Source inspection")
        .click();
    harness.step();
    harness.step();
    enter_numeric(&mut harness, "Exposure", "2");
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Source inspection")
        .click();
    harness.step();
    set_layout(&mut harness, egui::vec2(600., 600.), 315.);
    harness.get_by_label("100%").click();
    harness.step();
    assert_eq!(harness.state().viewport.zoom, Some(1.));
    stage(&mut harness, "Roll");
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Flat-field correction")
        .scroll_to_me();
    harness.try_run_realtime().unwrap();
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Flat-field correction")
        .click();
    harness.step();
    harness.get_by_label("Enabled").scroll_to_me();
    harness.try_run_realtime().unwrap();
    let viewport = harness.state().viewport;
    let corrected = harness.state().document.project.clone();
    for enabled in [false, true] {
        harness.get_by_label("Enabled").click();
        harness.step();
        wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
        assert_eq!(harness.state().correction.is_some(), enabled);
        assert!(harness.state().source);
        assert_eq!(harness.state().source_exposure, 2.);
        assert_eq!(harness.state().viewport, viewport);
        let mut expected = corrected.clone();
        expected.flat_field.as_mut().unwrap().enabled = enabled;
        assert_eq!(harness.state().document.project, expected);
    }
    wait_settled(&mut harness);
    let rendered = harness.state().working.displayed.as_ref().unwrap();
    let frame = rendered.spec.frame;
    assert!(frame.bounds().unwrap().width < harness.state().ready.as_ref().unwrap().area.width);
    // Physical 100% fixes pixel pitch, not the integer bounding ROI: fractional window edges
    // can require an extra original row/column without adding a display pixel.
    let origin = glam::DVec2::from_array(frame.source_point([0., 0.]).0);
    for point in [[1., 0.], [0., 1.]] {
        let step = glam::DVec2::from_array(frame.source_point(point).0) - origin;
        assert!((step.length() - 1.).abs() <= 1e-9);
    }
    assert_eq!(
        [
            rendered.image.linear.width(),
            rendered.image.linear.height()
        ],
        frame.output
    );
    let png = harness.render().unwrap();
    save_capture(&png, "correction-source-native-offscreen");
    set_layout(&mut harness, egui::vec2(1100., 800.), 315.);
    stage(&mut harness, "Rendering");
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Source inspection")
        .click();
    harness.step();
    harness.step();
    enter_numeric(&mut harness, "Exposure", "0");
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Source inspection")
        .click();
    harness.step();
    harness.get_by_label("View source").click();
    harness.step();
    harness.get_by_label("Fit").click();
    harness.step();
    {
        let e = harness.state_mut();
        e.document = e
            .document
            .save(
                &directory.path().join("correction.antitone.json"),
                crate::project::Save::New,
            )
            .unwrap();
    }
    save_existing_project(&mut harness);
    let saved = harness.state().document.project.clone();
    stage(&mut harness, "Roll");
    harness.get_by_label("Clear").scroll_to_me();
    harness.try_run_realtime().unwrap();
    harness.get_by_label("Clear").click();
    harness.step();
    wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
    assert!(harness.state().correction.is_none());
    let mut cleared = saved.clone();
    cleared.flat_field = None;
    assert_eq!(harness.state().document.project, cleared);
    open_project(
        &mut harness,
        directory.path().join("correction.antitone.json"),
    );
    harness.get_by_label("Discard changes").click();
    harness.step();
    wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
    assert_eq!(harness.state().document.project, saved);
    assert_eq!(
        harness.state().correction.as_ref().unwrap().settings.map,
        settings.map
    );
    wait_settled(&mut harness);
    wait_for(&mut harness, |e| {
        e.thumbnails.ready_count(&e.document, e.target) > 0
    });
    set_layout(&mut harness, egui::vec2(800., 600.), 270.);
    harness.get_by_label("Edit area…").scroll_to_me();
    harness.try_run_realtime().unwrap();
    harness.get_by_label("Edit area…").click();
    harness.step();
    wait_for(&mut harness, |e| !e.busy);
    harness.get_by_label("Fit").click();
    harness.step();
    wait_settled(&mut harness);
    let geometry = harness.state().last_layout.unwrap().0;
    assert!(geometry.canvas.expand(0.1).contains_rect(geometry.image));
    assert!(
        geometry
            .canvas
            .expand(0.1)
            .contains_rect(geometry.screen_coverage(harness.state().region.unwrap()))
    );
    let png = harness.render().unwrap();
    save_capture(&png, "correction-edit-narrow-offscreen");
    harness.get_by_label("Cancel").click();
    harness.step();
    assert_eq!(harness.state().document.project, saved);
    assert_eq!(harness.state().current_source, selected);
    std::fs::remove_file(&reference).unwrap();
    open_project(
        &mut harness,
        directory.path().join("correction.antitone.json"),
    );
    wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
    assert_eq!(harness.state().document.project, saved);
    assert!(harness.state().correction.is_some());
    let request = harness.state().next_request;
    harness.get_by_label("Edit area…").click();
    harness.step();
    assert!(
        harness.state().next_request > request,
        "reference edit did not dispatch"
    );
    wait_for(&mut harness, |e| !e.busy);
    assert!(harness.state().flat_edit.is_none());
    assert!(harness.state().pick.is_none());
    assert!(
        harness.state().status.contains("open"),
        "{}",
        harness.state().status
    );
    assert_eq!(harness.state().document.project, saved);
    assert!(harness.state().ready.is_some());
    assert_eq!(
        harness.state().failure.as_ref().unwrap().kind,
        FailureKind::FlatReference
    );
    harness.get_by_label("OK").click();
    harness.step();
    assert!(harness.state().failure.is_none());
    assert_eq!(harness.state().document.project, saved);
    assert!(harness.state().preference_store.is_none());
}
