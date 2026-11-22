use super::*;

mod continuity;
mod rendering;
use crate::editor::history::Direction;

mod history_tests;

pub(super) fn open(h: &mut Harness<'_, Editor>, kind: PickerCase) {
    let (tab, label) = match kind {
        PickerCase::Dmin => ("Roll", "Pick Dmin (film base)"),
        PickerCase::Dmax => ("Roll", "Pick Dmax"),
        PickerCase::Gamma => ("Roll", calibration::GAMMA_PICK_LABEL),
        PickerCase::RollWb => ("Roll", "Pick neutral for white balance"),
        PickerCase::Wb => ("Photo", "Pick neutral for white balance"),
    };
    stage(h, tab);
    h.get_by_label(label).scroll_to_me();
    h.try_run_realtime().unwrap();
    h.get_by_label(label).click();
    h.step();
    h.try_run_realtime().unwrap();
}

#[test]
fn incompatible_reference_fails_only_the_positive_view_and_can_be_repicked() {
    fn pick_at(h: &mut Harness<'_, Editor>, point: egui::Pos2) {
        if !h.read(|editor| editor.document.source) {
            h.get_by_label("View source").click();
            h.step();
        }
        open(h, PickerCase::Dmin);
        let layout = h.read(|editor| editor.active_canvas().last_layout.unwrap().0);
        let first = layout.screen_point(point - egui::vec2(8., 8.));
        let last = layout.screen_point(point + egui::vec2(8., 8.));
        h.hover_at(first);
        h.event(egui::Event::PointerButton {
            pos: first,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
        h.step();
        h.hover_at(last);
        h.step();
        h.event(egui::Event::PointerButton {
            pos: last,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        h.step();
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Apply")
            .click();
        h.step();
        wait_interaction(h);
        h.get_by_label("View source").click();
        h.step();
    }

    workflow::calibrated_workflow(|mut h, _, _directory| {
        h.get_by_label("patches.dng").click();
        h.step();
        wait_for(&mut h, |editor| !editor.document.visit.source.pending());
        workflow::wait_settled(&mut h);
        for (size, width, suffix) in [
            (egui::vec2(1100., 800.), 315., "normal"),
            (egui::vec2(800., 600.), 270., "narrow"),
        ] {
            set_layout(&mut h, size, width);
            let before = h.read(|editor| editor.input());
            pick_at(&mut h, egui::pos2(96., 32.));
            h.run_steps(3);
            h.read(|editor| {
                assert_ne!(editor.input(), before);
                assert!(editor.input().calibration().unwrap().validate().is_err());
                assert!(editor.document.visit.canvas.ready.is_some());
                assert!(!editor.document.source && !editor.workspace_input.is_blocked());
                assert_eq!(
                    editor.history_label(Direction::Undo),
                    Some("Calibration pick")
                );
            });
            let error = h
                .get_by_label_contains("Positive preview unavailable:")
                .rect();
            let canvas = h.read(|editor| editor.active_canvas().last_layout.unwrap().0.canvas);
            assert!(canvas.contains_rect(error));
            assert!(error.bottom() < h.get_by_label("View source").rect().top());
            save_capture(
                &h.render().unwrap(),
                &format!("calibration-incompatible-{suffix}"),
            );
            pick_at(&mut h, egui::pos2(32., 32.));
            workflow::wait_settled(&mut h);
            h.read(|editor| {
                assert!(editor.input().calibration().unwrap().validate().is_ok());
                assert!(!editor.document.source && !editor.workspace_input.is_blocked());
            });
            assert!(
                h.query_by_label_contains("Positive preview unavailable:")
                    .is_none()
            );
            save_capture(
                &h.render().unwrap(),
                &format!("calibration-recovered-{suffix}"),
            );
        }
    });
}

#[test]
fn measurement_modes_own_apply_readiness_and_leave_normal_view_preferences_intact() {
    workflow::calibrated_workflow(|mut h, _, _directory| {
        for (size, width, suffix) in [
            (egui::vec2(1100., 800.), 315., "normal"),
            (egui::vec2(800., 600.), 270., "narrow"),
        ] {
            set_layout(&mut h, size, width);
            for kind in [
                PickerCase::Dmin,
                PickerCase::Dmax,
                PickerCase::Gamma,
                PickerCase::RollWb,
                PickerCase::Wb,
            ] {
                let source_preference = matches!(
                    kind,
                    PickerCase::Gamma | PickerCase::RollWb | PickerCase::Wb
                );
                h.update(|editor| editor.document.source = source_preference);
                h.step();
                let viewport = h.read(|editor| editor.document.visit.canvas.viewport);
                let before = h.read(|editor| editor.document.state.project.clone());
                open(&mut h, kind);
                h.read(|editor| {
                    assert!(
                        editor
                            .current_tool()
                            .unwrap()
                            .is::<super::super::picking::PickTool>()
                    );
                    assert_eq!(editor.test_pick(), Some(kind.display()));
                    assert_eq!(
                        editor.document.source, source_preference,
                        "mode does not commandeer the normal source preference"
                    );
                    assert_eq!(editor.canvas_inspection(), source_preference);
                });
                let apply = h.get_by_role_and_label(egui::accesskit::Role::Button, "Apply");
                assert!(apply.accesskit_node().is_disabled());
                h.get_by_label(&format!("Active tool: {}", kind.display().label));
                let point =
                    h.read(|editor| editor.active_canvas().last_layout.unwrap().0.image.center());
                h.hover_at(point);
                for pressed in [true, false] {
                    h.event(egui::Event::PointerButton {
                        pos: point,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    });
                    h.step();
                }
                h.step();
                assert!(
                    !h.get_by_role_and_label(egui::accesskit::Role::Button, "Apply")
                        .accesskit_node()
                        .is_disabled()
                );
                h.read(|editor| {
                    assert!(editor.test_pick_region().is_some());
                    assert_eq!(editor.document.state.project, before);
                });
                h.event(egui::Event::PointerGone);
                h.step();
                save_capture(
                    &h.render().unwrap(),
                    &format!(
                        "measurement-{}-{suffix}",
                        match kind {
                            PickerCase::Dmin => "dmin",
                            PickerCase::Dmax => "dmax",
                            PickerCase::Gamma => "gamma",
                            PickerCase::RollWb => "roll-wb",
                            PickerCase::Wb => "layer-wb",
                        }
                    ),
                );
                h.get_by_role_and_label(egui::accesskit::Role::Button, "Cancel")
                    .click();
                h.step();
                h.try_run_realtime().unwrap();
                h.read(|editor| {
                    assert!(editor.current_tool().is_none());
                    assert!(editor.test_pick().is_none());
                    assert_eq!(editor.document.source, source_preference);
                    assert_eq!(editor.canvas_inspection(), source_preference);
                    assert_eq!(editor.document.visit.canvas.viewport, viewport);
                    assert_eq!(editor.document.state.project, before);
                });
            }
        }
    });
}

#[test]
fn apply_keeps_the_current_view_and_measurement_uses_only_cancellable_activity() {
    workflow::calibrated_workflow(|mut h, _, _directory| {
        for (size, width, layout) in [
            (egui::vec2(1100., 800.), 315., "normal"),
            (egui::vec2(800., 600.), 270., "narrow"),
        ] {
            set_layout(&mut h, size, width);
            h.update(|editor| editor.document.source = false);
            h.run_steps(3);
            let before = h.read(|editor| {
                (
                    editor.document.project.clone(),
                    editor.document.visit.canvas.viewport,
                )
            });
            let gate = h.update(crate::editor::measurement::testing::Gate::install);
            open(&mut h, PickerCase::Dmin);
            let point =
                h.read(|editor| editor.active_canvas().last_layout.unwrap().0.image.center());
            h.hover_at(point);
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                });
                h.step();
            }
            h.get_by_role_and_label(egui::accesskit::Role::Button, "Apply")
                .click();
            h.step();
            let worker = gate.next(&mut h.application);
            h.update(|editor| editor.activities.elapse(Duration::from_secs(1)));
            h.run_steps(3);
            h.read(|editor| {
                assert!(!editor.has_hosted_tool());
                assert!(!editor.document.source && !editor.canvas_inspection());
                assert_eq!(editor.document.visit.canvas.viewport, before.1);
                assert!(editor.measurement_pending() && editor.interaction_blocked());
                assert_eq!(editor.document.project, before.0);
            });
            assert!(
                h.query_by_role_and_label(egui::accesskit::Role::Button, "Apply")
                    .is_none()
            );
            h.get_by_role_and_label(egui::accesskit::Role::Button, "Cancel measurement");
            save_capture(
                &h.render().unwrap(),
                &format!("measurement-running-{layout}"),
            );
            h.get_by_role_and_label(egui::accesskit::Role::Button, "Cancel measurement")
                .click();
            h.step();
            wait_for(&mut h, |editor| !editor.measurement_pending());
            assert!(worker.cancelled.is_cancelled());
            let completed = worker.completed.clone();
            worker.run();
            completed.wait(&mut h.application);
            h.read(|editor| {
                assert!(editor.activities.view().is_none());
                assert_eq!(editor.document.project, before.0);
            });
        }
    });
}
