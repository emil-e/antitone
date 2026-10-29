use super::*;
use crate::dust_repair::RepairId;

mod persistence;

fn click(h: &mut Harness<'_, Editor>, p: egui::Pos2, modifiers: egui::Modifiers) {
    let at = h.read(|editor| editor.document.visit.last_layout.unwrap().0.screen_point(p));
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
        !e.dust_search_pending()
            && e.dust_signature()
                .is_some_and(|s| e.dust_evaluations(s).len() == count)
            && e.document.visit.working_view.idle()
    });
}

fn pixels(e: &Editor, source: SourceId, id: RepairId) -> usize {
    e.dust_repairs(source)
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
    tools::calibrated_harness(|mut h| {
        stage(&mut h, "Photo");
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Dust repair")
            .click();
        h.step();
        h.try_run_realtime().unwrap();
        let source = h.read(|editor| editor.document.visit.current_source.unwrap());
        let project = h.read(|editor| editor.document.state.project.clone());
        h.read(|editor| assert!(editor.document.dirty()));
        h.read(|editor| assert!(editor.document.visit.dust.cleanup));
        h.read(|editor| assert!(editor.active_pick().is_none()));
        assert!(
            h.get_by_label("Reset dust repairs")
                .accesskit_node()
                .is_disabled()
        );
        let view = h.read(|editor| editor.document.visit.viewport);
        let diameter = h.read(|editor| editor.document.visit.dust.brush_size);
        let hover = h.read(|editor| editor.document.visit.last_layout.unwrap().0.image.center());
        h.hover_at(hover);
        h.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0., 24.),
            modifiers: egui::Modifiers::NONE,
            phase: egui::TouchPhase::Move,
        });
        for _ in 0..4 {
            h.step();
            h.read(|editor| assert_eq!(editor.document.visit.viewport, view));
        }
        h.read(|editor| assert!(editor.document.visit.dust.brush_size > diameter));
        click(&mut h, egui::pos2(96., 32.), egui::Modifiers::NONE);
        ready(&mut h, 1);
        let sig = h.read(|editor| editor.dust_signature().unwrap());
        let first = h.read(|editor| editor.dust_evaluations(sig)[0].clone());
        let initial = h.read(|editor| {
            editor
                .document
                .visit
                .working_view
                .regional()
                .unwrap()
                .clone()
        });
        let initial_neutral =
            h.read(|editor| editor.document.visit.working_view.neutral_source().unwrap());
        let layout = h.read(|editor| editor.document.visit.last_layout.unwrap().0);
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
        h.read(|editor| {
            assert!(
                editor.dust_repair_at(layout, empty).is_none()
                    && !editor.dust_donor_at(layout, empty)
            )
        });
        h.hover_at(empty);
        h.step();
        assert_eq!(
            h.output().platform_output.cursor_icon,
            egui::CursorIcon::Crosshair
        );
        assert!(brush_ring_at(&h, empty));
        h.read(|editor| assert_ne!(editor.document.state.project, project));
        h.read(|editor| assert!(editor.document.dirty()));
        h.read(|editor| assert!(editor.active_pick().is_none()));
        h.update(|editor| editor.document.roll.exposure = 0.3);
        h.update(|editor| {
            editor.document.roll.wb = Balance {
                warmth: 0.03,
                tint: -0.02,
            }
        });
        h.update(|editor| editor.document.visit.adjustment_layer_settings.exposure = 0.7);
        h.update(|editor| editor.document.visit.adjustment_layer_settings.contrast = 1.1);
        h.update(|editor| editor.document.visit.adjustment_layer_settings.saturation = 0.8);
        h.step();
        ready(&mut h, 1);
        h.read(|editor| assert_eq!(editor.dust_signature(), Some(sig)));
        h.read(|editor| assert!(Arc::ptr_eq(&first, &editor.dust_evaluations(sig)[0])));
        h.read(|editor| {
            assert!(
                editor
                    .document
                    .visit
                    .working_view
                    .regional()
                    .unwrap()
                    .same_source_as(&initial)
            )
        });
        let viewport = h.read(|editor| editor.document.visit.viewport);
        click(&mut h, egui::pos2(96., 32.), egui::Modifiers::NONE);
        h.read(|editor| assert_eq!(editor.selected_dust_repair(), Some(RepairId(1))));
        h.read(|editor| assert_eq!(editor.document.visit.viewport, viewport));
        h.read(|editor| assert!(editor.active_pick().is_none()));
        h.update(|editor| editor.document.visit.dust.brush_size = 2);
        let before = h.read(|editor| pixels(editor, source, RepairId(1)));
        click(&mut h, egui::pos2(96., 32.), egui::Modifiers::ALT);
        ready(&mut h, 1);
        h.read(|editor| assert!(pixels(editor, source, RepairId(1)) < before));
        click(&mut h, egui::pos2(96., 32.), egui::Modifiers::SHIFT);
        ready(&mut h, 1);
        h.read(|editor| assert_eq!(pixels(editor, source, RepairId(1)), before));
        let regions = h.read(|editor| editor.dust_donor_regions());
        let (target, donor) = (regions[0].target, regions[0].donor);
        let offset = [
            donor.x as i32 - target.x as i32,
            donor.y as i32 - target.y as i32,
        ];
        let point = h.read(|editor| {
            editor.dust_repairs(source).unwrap().repairs()[0]
                .support
                .selected_points()
                .next()
                .unwrap()
        });
        let start = egui::pos2(
            point[0] as f32 + offset[0] as f32 + 0.5,
            point[1] as f32 + offset[1] as f32 + 0.5,
        );
        let layout = h.read(|editor| editor.document.visit.last_layout.unwrap().0);
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
        h.read(|editor| {
            assert_eq!(
                editor.dust_repairs(source).unwrap().repairs()[0].offset,
                desired
            )
        });
        ready(&mut h, 1);
        let region = h.read(|editor| editor.dust_donor_regions().remove(0));
        let selected = region.support.selected_points().next().unwrap();
        let donor = h.read(|editor| {
            editor
                .document
                .visit
                .last_layout
                .unwrap()
                .0
                .screen_point(egui::pos2(
                    selected[0] as f32 + desired[0] as f32 + 0.5,
                    selected[1] as f32 + desired[1] as f32 + 0.5,
                ))
        });
        h.hover_at(donor);
        h.step();
        assert_eq!(
            h.output().platform_output.cursor_icon,
            egui::CursorIcon::Grab
        );
        assert!(!brush_ring_at(&h, donor));
        let same = h.read(|editor| editor.dust_evaluations(sig)[0].clone());
        click(&mut h, egui::pos2(32., 96.), egui::Modifiers::NONE);
        ready(&mut h, 2);
        h.read(|editor| assert!(Arc::ptr_eq(&same, &editor.dust_evaluations(sig)[0])));
        h.read(|editor| {
            assert!(Arc::ptr_eq(
                &initial_neutral,
                &editor.document.visit.working_view.neutral_source().unwrap()
            ))
        });
        h.read(|editor| {
            assert!(
                !editor
                    .document
                    .visit
                    .working_view
                    .regional()
                    .unwrap()
                    .same_source_as(&initial)
            )
        });
        click(&mut h, egui::pos2(96., 32.), egui::Modifiers::NONE);
        h.read(|editor| assert_eq!(editor.selected_dust_repair(), Some(RepairId(1))));
        key(&mut h, egui::Key::Delete);
        h.read(|editor| assert!(editor.document.removal_pending.is_none()));
        h.read(|editor| assert_eq!(editor.dust_repairs(source).unwrap().repairs().len(), 1));
        ready(&mut h, 1);
        let remaining = h.read(|editor| editor.dust_evaluations(sig)[0].clone());
        h.get_by_label("100%").click();
        h.step();
        ready(&mut h, 1);
        h.read(|editor| assert!(Arc::ptr_eq(&remaining, &editor.dust_evaluations(sig)[0])));
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
        let crate::editor::scopes::ScopeInput::Rendered(scope, _) =
            h.read(|editor| editor.scope_request().unwrap())
        else {
            panic!("scope forked from repaired viewer")
        };
        assert!(Arc::ptr_eq(&scope, &view));
        h.update(|editor| editor.set_gamma(0.8));
        h.step();
        h.read(|editor| assert_ne!(editor.dust_signature(), Some(sig)));
        h.read(|editor| {
            assert!(
                editor
                    .dust_evaluations(editor.dust_signature().unwrap())
                    .is_empty()
            )
        });
        ready(&mut h, 1);
        h.read(|editor| {
            assert!(!Arc::ptr_eq(
                &remaining,
                &editor.dust_evaluations(editor.dust_signature().unwrap())[0]
            ))
        });
        h.get_by_label("Reset dust repairs").click();
        h.step();
        h.read(|editor| assert!(editor.dust_repairs(source).is_none()));
        ready(&mut h, 0);
        h.get_by_label("Done").click();
        h.step();
        h.read(|editor| assert!(!editor.document.visit.dust.cleanup));
        h.read(|editor| assert!(!editor.document.visit.dust.show_masks));
    });
}

#[test]
fn fullscreen_suspends_dust_interaction_without_changing_committed_repairs() {
    tools::calibrated_harness(|mut h| {
        stage(&mut h, "Photo");
        h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure")
            .scroll_to_me();
        h.try_run_realtime().unwrap();
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Dust repair")
            .click();
        h.try_run_realtime().unwrap();
        let shelf = h
            .get_by_role_and_label(egui::accesskit::Role::Button, "Dust repair")
            .rect();
        let brush = h.get_by_label("Brush size").rect();
        assert!(brush.top() > shelf.bottom() && brush.bottom() < h.ctx.content_rect().bottom());
        click(&mut h, egui::pos2(96., 32.), egui::Modifiers::NONE);
        ready(&mut h, 1);
        let source = h.read(|editor| editor.document.visit.current_source.unwrap());
        let project = h.read(|editor| editor.document.state.project.clone());
        assert!(h.get_by_label("View source").accesskit_node().is_disabled());
        h.get_by_label("View source").click();
        h.step();
        h.read(|editor| assert!(!editor.document.source && editor.document.visit.dust.cleanup));
        key(&mut h, egui::Key::F);
        h.read(|editor| assert!(editor.fullscreen.is_some()));
        let brush = h.read(|editor| editor.document.visit.dust.brush_size);
        click(&mut h, egui::pos2(32., 96.), egui::Modifiers::NONE);
        h.read(|editor| assert_eq!(editor.dust_repairs(source).unwrap().repairs().len(), 1));
        h.read(|editor| assert_eq!(editor.document.visit.dust.brush_size, brush));
        key(&mut h, egui::Key::F);
        h.update(|editor| editor.fullscreen_changed(false));
        h.step();
        h.read(|editor| assert!(editor.fullscreen.is_none()));
        h.read(|editor| assert!(editor.document.visit.dust.cleanup));
        h.read(|editor| assert_eq!(editor.document.state.project, project));
        h.get_by_label("Done").click();
        h.step();
        let stored = h.read(|editor| serde_json::to_value(&editor.document.state.project).unwrap());
        assert_eq!(stored, serde_json::to_value(&project).unwrap());
        ready(&mut h, 1);
        assert!(h.get_by_label("Display P3 · Repairs").rect().height() > 0.);
    });
}

#[test]
fn navigating_back_to_a_repaired_photo_keeps_inactive_dust_artwork_hidden() {
    let directory = tempfile::tempdir().unwrap();
    let other = directory.path().join("other.dng");
    std::fs::copy(fixture("patches.dng"), &other).unwrap();
    let artwork = |h: &Harness<'_, Editor>| {
        let context = egui::Context::default();
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(h.ctx.content_rect()),
                ..Default::default()
            },
            |ui| {
                h.read(|editor| editor.paint_dust(ui, editor.document.visit.last_layout.unwrap().0))
            },
        );
        output.textures_delta.clear();
        output
            .shapes
            .iter()
            .any(|shape| !matches!(shape.shape, egui::Shape::Noop))
    };
    tools::calibrated_harness(|mut h| {
        h.enter(|editor, cx| editor.add_raws(vec![other], cx));
        wait_for(&mut h, |editor| {
            !editor.busy && editor.document_pending.is_none()
        });
        stage(&mut h, "Photo");
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Dust repair")
            .click();
        h.step();
        click(&mut h, egui::pos2(96., 32.), egui::Modifiers::NONE);
        ready(&mut h, 1);
        let source = h.read(|editor| editor.document.visit.current_source.unwrap());
        assert!(
            artwork(&h),
            "the control must contain visible repair artwork while Dust is active"
        );
        h.get_by_label("Done").click();
        h.step();
        assert!(!artwork(&h));
        h.get_by_label("other.dng").click();
        h.step();
        wait_for(&mut h, |editor| {
            !editor.busy && editor.document.visit.current_source != Some(source)
        });
        h.get_by_label("patches.dng").click();
        h.step();
        wait_for(&mut h, |editor| {
            !editor.busy && editor.document.visit.current_source == Some(source)
        });
        ready(&mut h, 1);
        h.read(|editor| {
            assert!(!editor.document.visit.dust.cleanup);
            assert_eq!(editor.dust_repairs(source).unwrap().repairs().len(), 1);
        });
        assert!(
            !artwork(&h),
            "visit replacement restored repair overlays without entering Dust"
        );
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Dust repair")
            .click();
        h.step();
        assert!(
            artwork(&h),
            "entering Dust must expose the retained repair again"
        );
    });
}
