use super::*;
use crate::dust_repair::{DonorChoice, RepairId};

fn click(h: &mut Harness<'_, Editor>, p: egui::Pos2, modifiers: egui::Modifiers) {
    let at = h.state().last_layout.unwrap().0.screen_point(p);
    h.hover_at(at);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers,
        });
    }
    h.step();
}

fn brush_ring_at(h: &Harness<'_, Editor>, point: egui::Pos2) -> bool {
    h.output().shapes.iter().any(|shape| {
        matches!(
            &shape.shape,
            egui::Shape::Circle(circle)
                if circle.center.distance(point) < 0.01
                    && circle.fill == egui::Color32::TRANSPARENT
                    && circle.stroke.width == 1.5
        )
    })
}

fn ready(h: &mut Harness<'_, Editor>, count: usize) {
    wait_for(h, |e| {
        e.dust_signature()
            .is_some_and(|s| e.dust_evaluations(s).len() == count)
            && e.working.idle()
    });
}

fn pixels(e: &Editor, source: SourceId, id: RepairId) -> usize {
    e.dust
        .repairs(source)
        .unwrap()
        .repairs()
        .iter()
        .find(|r| r.id == id)
        .unwrap()
        .support
        .selected_points()
        .count()
}

fn key(h: &mut Harness<'_, Editor>, key: egui::Key) {
    for pressed in [true, false] {
        h.event(egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        h.step();
    }
}

#[test]
fn dust_direct_cleanup_reuses_neutral_heals_and_gpu_sources_through_grading_and_navigation() {
    let mut h = tools::calibrated_harness();
    stage(&mut h, "Photo");
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Dust repair")
        .click();
    h.step();
    let source = h.state().current_source.unwrap();
    let project = h.state().document.project.clone();
    let dirty = h.state().document.dirty();
    assert!(h.state().dust.cleanup);
    assert!(h.state().pick.is_none());
    assert!(
        h.get_by_label("Reset dust repairs")
            .accesskit_node()
            .is_disabled()
    );
    let view = h.state().viewport;
    let diameter = h.state().dust.brush_size;
    let hover = h.state().last_layout.unwrap().0.image.center();
    h.hover_at(hover);
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0., 24.),
        modifiers: egui::Modifiers::NONE,
        phase: egui::TouchPhase::Move,
    });
    for _ in 0..4 {
        h.step();
        assert_eq!(h.state().viewport, view);
    }
    assert!(h.state().dust.brush_size > diameter);
    click(&mut h, egui::pos2(96., 32.), egui::Modifiers::NONE);
    ready(&mut h, 1);
    let sig = h.state().dust_signature().unwrap();
    let first = h.state().dust_evaluations(sig)[0].clone();
    let initial = h.state().working.regional().unwrap().clone();
    let initial_neutral = h.state().working.neutral_source().unwrap();
    let layout = h.state().last_layout.unwrap().0;
    let target = layout.screen_point(egui::pos2(96., 32.));
    h.hover_at(target);
    h.step();
    assert_eq!(
        h.output().platform_output.cursor_icon,
        egui::CursorIcon::PointingHand
    );
    assert!(!brush_ring_at(&h, target));
    for modifiers in [egui::Modifiers::SHIFT, egui::Modifiers::ALT] {
        h.event(egui::Event::ModifiersChanged(modifiers));
        h.hover_at(target);
        h.step();
        assert_eq!(
            h.output().platform_output.cursor_icon,
            egui::CursorIcon::Crosshair
        );
        assert!(brush_ring_at(&h, target));
    }
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    h.step();
    assert_eq!(
        h.output().platform_output.cursor_icon,
        egui::CursorIcon::PointingHand
    );
    h.event(egui::Event::Key {
        key: egui::Key::Space,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.step();
    assert_eq!(
        h.output().platform_output.cursor_icon,
        egui::CursorIcon::Grab
    );
    assert!(!brush_ring_at(&h, target));
    h.event(egui::Event::Key {
        key: egui::Key::Space,
        physical_key: None,
        pressed: false,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.step();
    assert_eq!(
        h.output().platform_output.cursor_icon,
        egui::CursorIcon::PointingHand
    );
    let empty = layout.screen_point(egui::pos2(180., 130.));
    assert!(
        h.state().dust_repair_at(layout, empty).is_none()
            && !h.state().dust_donor_at(layout, empty)
    );
    h.hover_at(empty);
    h.step();
    assert_eq!(
        h.output().platform_output.cursor_icon,
        egui::CursorIcon::Crosshair
    );
    assert!(brush_ring_at(&h, empty));
    assert_eq!(h.state().document.project, project);
    assert_eq!(h.state().document.dirty(), dirty);
    assert!(h.state().pick.is_none());
    h.state_mut().roll.exposure = 0.3;
    h.state_mut().roll.wb = Balance {
        warmth: 0.03,
        tint: -0.02,
    };
    h.state_mut().photo.exposure = 0.7;
    h.state_mut().photo.contrast = 1.1;
    h.state_mut().photo.saturation = 0.8;
    h.step();
    ready(&mut h, 1);
    assert_eq!(h.state().dust_signature(), Some(sig));
    assert!(Arc::ptr_eq(&first, &h.state().dust_evaluations(sig)[0]));
    assert!(
        h.state()
            .working
            .regional()
            .unwrap()
            .same_source_as(&initial)
    );
    let viewport = h.state().viewport;
    click(&mut h, egui::pos2(96., 32.), egui::Modifiers::NONE);
    assert_eq!(h.state().selected_dust_repair(), Some(RepairId(1)));
    assert_eq!(h.state().viewport, viewport);
    assert!(h.state().pick.is_none());
    h.state_mut().dust.brush_size = 2;
    let before = pixels(h.state(), source, RepairId(1));
    click(&mut h, egui::pos2(96., 32.), egui::Modifiers::ALT);
    assert!(pixels(h.state(), source, RepairId(1)) < before);
    click(&mut h, egui::pos2(96., 32.), egui::Modifiers::SHIFT);
    assert_eq!(pixels(h.state(), source, RepairId(1)), before);
    ready(&mut h, 1);
    let regions = h.state().dust_donor_regions();
    let (target, donor) = (regions[0].target, regions[0].donor);
    let offset = [
        donor.x as i32 - target.x as i32,
        donor.y as i32 - target.y as i32,
    ];
    let point = h.state().dust.repairs(source).unwrap().repairs()[0]
        .support
        .selected_points()
        .next()
        .unwrap();
    let start = egui::pos2(
        point[0] as f32 + offset[0] as f32 + 0.5,
        point[1] as f32 + offset[1] as f32 + 0.5,
    );
    let layout = h.state().last_layout.unwrap().0;
    let desired = [50, 50];
    drag(
        &mut h,
        layout.screen_point(start),
        layout.screen_point(
            start
                + egui::vec2(
                    (desired[0] - offset[0]) as f32,
                    (desired[1] - offset[1]) as f32,
                ),
        ),
    );
    assert_eq!(
        h.state().dust.repairs(source).unwrap().repairs()[0].donor,
        DonorChoice::Offset(desired)
    );
    ready(&mut h, 1);
    let region = h.state().dust_donor_regions().remove(0);
    let selected = region.support.selected_points().next().unwrap();
    let donor = h.state().last_layout.unwrap().0.screen_point(egui::pos2(
        selected[0] as f32 + desired[0] as f32 + 0.5,
        selected[1] as f32 + desired[1] as f32 + 0.5,
    ));
    h.hover_at(donor);
    h.step();
    assert_eq!(
        h.output().platform_output.cursor_icon,
        egui::CursorIcon::Grab
    );
    assert!(!brush_ring_at(&h, donor));
    let same = h.state().dust_evaluations(sig)[0].clone();
    click(&mut h, egui::pos2(32., 96.), egui::Modifiers::NONE);
    ready(&mut h, 2);
    assert!(Arc::ptr_eq(&same, &h.state().dust_evaluations(sig)[0]));
    assert!(Arc::ptr_eq(
        &initial_neutral,
        &h.state().working.neutral_source().unwrap()
    ));
    assert!(
        !h.state()
            .working
            .regional()
            .unwrap()
            .same_source_as(&initial)
    );
    click(&mut h, egui::pos2(96., 32.), egui::Modifiers::NONE);
    assert_eq!(h.state().selected_dust_repair(), Some(RepairId(1)));
    key(&mut h, egui::Key::Delete);
    assert!(h.state().removal_pending.is_none());
    assert_eq!(h.state().dust.repairs(source).unwrap().repairs().len(), 1);
    ready(&mut h, 1);
    let remaining = h.state().dust_evaluations(sig)[0].clone();
    h.get_by_label("100%").click();
    h.step();
    ready(&mut h, 1);
    assert!(Arc::ptr_eq(&remaining, &h.state().dust_evaluations(sig)[0]));
    let view = h.state().working.displayed.as_ref().unwrap().clone();
    let crate::editor::scopes::ScopeInput::Rendered(scope, _) = h.state().scope_request().unwrap()
    else {
        panic!("scope forked from repaired viewer")
    };
    assert!(Arc::ptr_eq(&scope, &view));
    h.state_mut().roll.gamma = 0.8;
    h.step();
    assert_ne!(h.state().dust_signature(), Some(sig));
    assert!(
        h.state()
            .dust_evaluations(h.state().dust_signature().unwrap())
            .is_empty()
    );
    ready(&mut h, 1);
    assert!(!Arc::ptr_eq(
        &remaining,
        &h.state()
            .dust_evaluations(h.state().dust_signature().unwrap())[0]
    ));
    h.get_by_label("Reset dust repairs").click();
    h.step();
    assert!(h.state().dust.repairs(source).is_none());
    ready(&mut h, 0);
    h.get_by_label("Done").click();
    h.step();
    assert!(!h.state().dust.cleanup);
    assert!(!h.state().dust.show_masks);
}

#[test]
fn dust_is_suspended_in_fullscreen_and_never_claims_saved_or_exported_output() {
    let mut h = tools::calibrated_harness();
    stage(&mut h, "Photo");
    h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure")
        .scroll_to_me();
    h.try_run_realtime().unwrap();
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Dust repair")
        .click();
    for _ in 0..3 {
        h.step();
    }
    let shelf = h
        .get_by_role_and_label(egui::accesskit::Role::Button, "Dust repair")
        .rect();
    let brush = h.get_by_label("Brush size").rect();
    assert!(brush.top() > shelf.bottom() && brush.bottom() < h.ctx.content_rect().bottom());
    click(&mut h, egui::pos2(96., 32.), egui::Modifiers::NONE);
    ready(&mut h, 1);
    let source = h.state().current_source.unwrap();
    let project = h.state().document.project.clone();
    assert!(h.get_by_label("View source").accesskit_node().is_disabled());
    h.get_by_label("View source").click();
    h.step();
    assert!(!h.state().source && h.state().dust.cleanup);
    key(&mut h, egui::Key::F);
    assert!(h.state().fullscreen.is_some());
    let brush = h.state().dust.brush_size;
    click(&mut h, egui::pos2(32., 96.), egui::Modifiers::NONE);
    assert_eq!(h.state().dust.repairs(source).unwrap().repairs().len(), 1);
    assert_eq!(h.state().dust.brush_size, brush);
    key(&mut h, egui::Key::F);
    h.state_mut().fullscreen_changed(false);
    h.step();
    assert!(h.state().fullscreen.is_none());
    assert!(h.state().dust.cleanup);
    assert_eq!(h.state().document.project, project);
    h.get_by_label("Done").click();
    h.step();
    let stored = serde_json::to_value(&h.state().document.project).unwrap();
    assert_eq!(stored, serde_json::to_value(&project).unwrap());
    ready(&mut h, 1);
    assert!(
        h.get_by_label("Display P3 · Unsaved repairs")
            .rect()
            .height()
            > 0.
    );
}
