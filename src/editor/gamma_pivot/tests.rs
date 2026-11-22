use super::*;
use crate::{application::test_support::with_application, processing::Calibration};
use std::time::Duration;

type App<'a> = crate::application::test_support::TestApplication<'a, Editor>;

fn propose(app: &App<'_>, value: f64, active: bool) {
    app.enter(|editor, cx| {
        let mut candidate = editor.document.project_snapshot().as_ref().clone();
        if let InputSettings::Negative { gamma, .. } = &mut candidate.input {
            *gamma = value;
        }
        editor.roll_control(
            super::super::ui::ControlResponse {
                active,
                ..super::super::ui::ControlResponse::action(
                    egui::Id::new("test-document-field"),
                    true,
                )
            },
            cx,
            move |v| *v = candidate,
        );
    });
}

fn release(app: &App<'_>) {
    app.enter(|editor, cx| {
        editor.roll_control(
            ControlResponse::action(egui::Id::new("test-document-field"), false),
            cx,
            |_| {},
        );
    });
}

fn settle(app: &mut App<'_>) {
    app.run_until(Duration::from_secs(10), |editor| {
        !editor.document.gamma_pivot.calculating()
    });
}

fn setup() -> (crate::fixture_path::FixturePath, Editor) {
    let file = crate::raw::fixture::bayer_dng_with_rgb([16000, 20000, 24000]);
    let mut editor = crate::editor::tests::authored_fixture(&[file.path().to_str().unwrap()]);
    editor.document.project.input = Calibration {
        dmin: glam::DVec3::splat(0.8),
        dmax: glam::DVec3::splat(0.1),
    }
    .into();

    (file, editor)
}

#[test]
fn a_gesture_keeps_its_photo_but_the_next_uses_the_new_selection() {
    let mut editor =
        crate::editor::tests::authored_fixture(&["patches.dng", "illuminated-patches.dng"]);
    editor.set_dmin(Some(glam::DVec3::splat(3000. / 4031.)));
    editor.set_dmax(Some(glam::DVec3::splat(128. / 4031.)));
    editor.set_gamma(0.75);
    with_application(editor, |app| {
        let expected = |app: &App<'_>, gamma| {
            let (source, input, wb, gpu) = app.read(|editor| {
                let project = &editor.document.state.project;
                (
                    capture_reference(editor, project.input).unwrap(),
                    project.input.negative_input().unwrap(),
                    project.roll.wb,
                    measurement::capture_gpu(editor).unwrap(),
                )
            });
            let reference = pollster::block_on(source).unwrap();
            let proposed = crate::processing::NegativeInput { gamma, ..input };
            reference
                .evidence
                .compensate(
                    (&gpu.0, &gpu.1),
                    input,
                    proposed,
                    reference.camera_to_ap1,
                    wb,
                    &|| false,
                )
                .unwrap()
        };
        let start_source = app.read(|editor| editor.document.visit.current_source.unwrap());
        let expected_first = expected(app, 0.9);
        let gate = measurement::testing::Gate::default();
        app.update(|editor| editor.document.gamma_pivot.edit_gate = Some(gate.clone()));
        propose(app, 0.85, true);
        let first = gate.next(app);
        app.enter(|editor, cx| {
            let other = editor
                .document
                .project
                .order
                .iter()
                .position(|&id| id != start_source)
                .unwrap();
            editor.select_roll_photo(other, egui::Modifiers::NONE, cx);
        });
        app.run_until(Duration::from_secs(10), |editor| {
            editor.document.visit.current_source != Some(start_source)
                && editor.document.visit.canvas.ready.is_some()
        });
        app.read(|editor| {
            assert_eq!(editor.document.project.input.gamma(), 0.75);
            assert_eq!(editor.document.project.roll.wb, Balance::default());
        });
        let wrong_photo = expected(app, 0.9);
        assert!(
            (wrong_photo.warmth - expected_first.warmth).abs() > 0.005
                || (wrong_photo.tint - expected_first.tint).abs() > 0.005
        );
        propose(app, 0.9, true);
        let second = gate.next(app);
        assert!(first.cancelled.is_cancelled());
        first.run();
        second.run();
        settle(app);
        release(app);
        app.read(|editor| {
            assert_eq!(editor.document.project.roll.wb, expected_first);
            assert!(editor.document.gamma_pivot.gesture.is_none());
        });

        let expected_second = expected(app, 0.8);
        app.update(|editor| editor.document.gamma_pivot.edit_gate = None);
        propose(app, 0.8, false);
        settle(app);
        app.read(|editor| {
            assert_eq!(editor.document.project.input.gamma(), 0.8);
            assert_eq!(editor.document.project.roll.wb, expected_second);
            assert!(editor.document.gamma_pivot.gesture.is_none());
        });
    });
}

#[test]
fn repeated_held_input_keeps_the_same_gamma_calculation() {
    let (_file, editor) = setup();
    with_application(editor, |app| {
        let gate = measurement::testing::Gate::default();
        app.update(|editor| editor.document.gamma_pivot.edit_gate = Some(gate.clone()));
        propose(app, 0.9, true);
        let solve = gate.next(app);
        propose(app, 0.9, true);
        app.pump();
        assert!(
            !solve.cancelled.is_cancelled(),
            "a stationary control must not restart identical work"
        );
        solve.run();
        settle(app);
        propose(app, 0.9, true);
        app.pump();
        app.read(|editor| {
            assert!(
                !editor.document.gamma_pivot.calculating(),
                "a completed identical value must not restart work"
            )
        });
        release(app);
        app.read(|editor| {
            assert_eq!(editor.document.project.input.gamma(), 0.9);
            assert!(!editor.editing());
        });
    });
}

#[test]
fn an_unchanged_new_gesture_and_pin_toggle_leave_the_submitted_edit_intact() {
    let (_file, editor) = setup();
    with_application(editor, |app| {
        let gate = measurement::testing::Gate::default();
        app.update(|editor| editor.document.gamma_pivot.edit_gate = Some(gate.clone()));
        let before = app.read(|editor| editor.document.project.roll.wb);
        propose(app, 0.9, true);
        let solve = gate.next(app);
        release(app);
        app.enter(|editor, cx| {
            editor.roll_control(
                ControlResponse {
                    started: true,
                    active: true,
                    ..ControlResponse::action(egui::Id::new("test-document-field"), false)
                },
                cx,
                |_| {},
            );
        });
        release(app);
        app.update(Editor::toggle_gamma_pivot);
        app.pump();
        assert!(!solve.cancelled.is_cancelled());
        solve.run();
        settle(app);
        app.read(|editor| {
            assert_eq!(editor.document.project.input.gamma(), 0.9);
            assert_ne!(editor.document.project.roll.wb, before);
            assert!(!editor.document.gamma_pivot.enabled());
        });
    });
}

#[test]
fn gamma_calculation_does_not_reserve_tool_admission() {
    let (_file, editor) = setup();
    with_application(editor, |app| {
        let gate = measurement::testing::Gate::default();
        app.update(|editor| editor.document.gamma_pivot.edit_gate = Some(gate.clone()));
        propose(app, 0.9, false);
        let solve = gate.next(app);
        app.enter(|editor, cx| {
            assert!(!editor.editing());
            editor.apply_photo_action(super::super::editing::PhotoAction::Crop, cx);
            assert!(
                editor.cropping(),
                "ordinary tool admission remains available"
            );
            assert!(editor.document.gamma_pivot.calculating());
        });
        assert!(!solve.cancelled.is_cancelled());
        solve.run();
        settle(app);
        app.read(|editor| {
            assert_eq!(editor.document.project.input.gamma(), 0.9);
            assert!(
                editor.cropping(),
                "publication does not end the independent tool"
            );
        });
    });
}

#[test]
fn bypass_is_uncompensated_and_a_locked_gesture_returns_exactly_to_its_start() {
    let (_file, editor) = setup();
    with_application(editor, |app| {
        let wb = app.read(|editor| editor.document.project.roll.wb);
        app.update(Editor::toggle_gamma_pivot);
        propose(app, 0.7, false);
        app.read(|editor| {
            assert_eq!(editor.document.project.input.gamma(), 0.7);
            assert_eq!(editor.document.project.roll.wb, wb);
            assert!(!editor.document.gamma_pivot.calculating());
        });
        app.update(Editor::toggle_gamma_pivot);
        let before = app.enter(|editor, _| editor.history_snapshot());
        let prior_undo =
            app.read(|editor| editor.history_label(super::super::history::Direction::Undo));

        propose(app, 0.9, true);
        settle(app);
        app.read(|editor| {
            assert_ne!(editor.document.project_snapshot().roll.wb, wb);
            assert_eq!(
                editor.document.project.roll.wb,
                editor.document.project_snapshot().roll.wb
            );
            assert!(
                !editor.editing(),
                "accepted values retain no mutation authority"
            );
        });
        propose(app, 0.7, true);
        settle(app);
        release(app);
        app.enter(|editor, _| {
            assert_eq!(editor.history_snapshot(), before);
            assert_eq!(
                editor.history_label(super::super::history::Direction::Undo),
                prior_undo
            );
        });
        // A deliberate WB starts a new held colour, not a return to capture-time WB.
        app.enter(|editor, cx| {
            let previous = editor.document.project_snapshot();
            let mut candidate = previous.as_ref().clone();
            candidate.roll.wb = Balance {
                warmth: 0.25,
                tint: -0.1,
            };
            editor.roll_control(
                super::super::ui::ControlResponse {
                    active: false,
                    ..super::super::ui::ControlResponse::action(
                        egui::Id::new("test-document-field"),
                        true,
                    )
                },
                cx,
                move |v| *v = candidate,
            );
        });
        let manual = app.read(|editor| editor.document.project.roll.wb);
        propose(app, 0.8, true);
        settle(app);
        propose(app, 0.7, true);
        settle(app);
        release(app);
        app.read(|editor| assert_eq!(editor.document.project.roll.wb, manual));
    });
}

#[test]
fn stale_gamma_completion_and_invalid_gamma_leave_accepted_state_unchanged() {
    let (_file, editor) = setup();
    with_application(editor, |app| {
        let before = app.enter(|editor, _| editor.history_snapshot());
        propose(app, 0.01, false);
        settle(app);
        app.enter(|editor, _| {
            assert_eq!(editor.history_snapshot(), before);
            assert!(editor.status.starts_with("Edit rejected:"));
            assert!(editor.document.gamma_pivot.enabled());
        });
        let gate = measurement::testing::Gate::default();
        app.update(|editor| editor.document.gamma_pivot.edit_gate = Some(gate.clone()));
        propose(app, 0.9, true);
        let stale = gate.next(app);
        // Returning to accepted gamma before an older solve completes is an exact no-op.
        propose(app, before.input.gamma(), true);
        app.pump();
        assert!(stale.cancelled.is_cancelled());
        let completed = stale.completed.clone();
        stale.run();
        completed.wait(app);
        release(app);
        app.pump();
        app.enter(|editor, _| assert_eq!(editor.history_snapshot(), before));
        let gate = measurement::testing::Gate::after_work();
        app.update(|editor| editor.document.gamma_pivot.edit_gate = Some(gate.clone()));
        propose(app, 0.8, true);
        let stale = gate.next(app);
        app.enter(|editor, cx| {
            editor.roll_control(
                ControlResponse {
                    cancelled: true,
                    ..ControlResponse::action(egui::Id::new("test-document-field"), false)
                },
                cx,
                |_| {},
            );
        });
        app.pump();
        assert!(stale.cancelled.is_cancelled());
        let completed = stale.completed.clone();
        stale.panic();
        completed.wait(app);
        app.pump();
        app.enter(|editor, _| {
            assert_eq!(editor.history_snapshot(), before);
            assert!(editor.document.gamma_pivot.enabled());
            assert!(editor.document.gamma_pivot.gesture.is_none());
        });
    });
}

#[test]
fn completed_gamma_work_cannot_publish_to_a_replacement_document() {
    let (_file, editor) = setup();
    with_application(editor, |app| {
        let before = app.enter(|editor, _| editor.history_snapshot());
        let gate = measurement::testing::Gate::after_work();
        app.update(|editor| editor.document.gamma_pivot.edit_gate = Some(gate.clone()));
        propose(app, 0.9, false);
        let late = gate.next(app);
        app.update(|editor| {
            let saved = editor.document.capture();
            editor.document = editor.document.replacement(saved);
            editor.status = "replacement document".into();
        });
        app.pump();
        assert!(late.cancelled.is_cancelled());
        let completed = late.completed.clone();
        late.run();
        completed.wait(app);
        app.pump();
        app.read(|editor| {
            assert_eq!(editor.document.project, before);
            assert!(editor.document.gamma_pivot.enabled());
            assert!(editor.document.gamma_pivot.gesture.is_none());
            assert_eq!(editor.status, "replacement document");
        });
    });
}

#[test]
fn a_no_net_gamma_gesture_preserves_redo_and_replay_restores_the_exact_pair() {
    let (_file, editor) = setup();
    with_application(editor, |app| {
        let before = app.enter(|editor, _| editor.history_snapshot());
        propose(app, 0.8, false);
        settle(app);
        let after = app.enter(|editor, cx| {
            let after = editor.history_snapshot();
            editor.restore_history(super::super::history::Direction::Undo, cx);
            after
        });
        app.run_until(Duration::from_secs(10), |editor| {
            !editor.interaction_blocked()
        });

        propose(app, 0.9, true);
        settle(app);
        propose(app, before.input.gamma(), true);
        settle(app);
        app.enter(|editor, cx| {
            let candidate = editor.document.project_snapshot().as_ref().clone();
            editor.roll_control(
                super::super::ui::ControlResponse {
                    active: false,
                    ..super::super::ui::ControlResponse::action(
                        egui::Id::new("test-document-field"),
                        false,
                    )
                },
                cx,
                move |v| *v = candidate,
            );
            assert_eq!(editor.history_snapshot(), before);
            assert_eq!(
                editor.history_label(super::super::history::Direction::Redo),
                Some("Gamma & balance")
            );
            editor.restore_history(super::super::history::Direction::Redo, cx);
        });
        app.run_until(Duration::from_secs(10), |editor| {
            !editor.interaction_blocked()
        });
        app.enter(|editor, _| assert_eq!(editor.history_snapshot(), after));
    });
}

#[test]
fn a_pending_gamma_result_cannot_overwrite_newer_calibration_response_or_wb() {
    for field in ["calibration", "response", "WB"] {
        let (_file, editor) = setup();
        with_application(editor, |app| {
            let gate = measurement::testing::Gate::after_work();
            app.update(|editor| editor.document.gamma_pivot.edit_gate = Some(gate.clone()));
            propose(app, 0.8, false);
            let late = gate.next(app);
            let changed = app.enter(|editor, _| {
                let mut edit = editor.edit_project("Newer control").unwrap();
                edit.update(|project| match field {
                    "calibration" => {
                        project.set_dmin(Some([0.85; 3]));
                    }
                    "response" => {
                        if let InputSettings::Negative { response, .. } = &mut project.input {
                            response.toe = 1.2;
                        }
                    }
                    "WB" => project.roll.wb.warmth = 0.2,
                    _ => unreachable!(),
                });
                editor.commit_project(edit).unwrap();
                editor.history_snapshot()
            });
            assert!(
                !late.cancelled.is_cancelled(),
                "accepted colour is independent of {field}"
            );
            late.run();
            settle(app);
            app.read(|editor| {
                assert_eq!(editor.document.project, changed, "{field}");
                assert!(editor.document.gamma_pivot.enabled());
                assert_eq!(
                    editor.history_label(super::super::history::Direction::Undo),
                    Some("Newer control")
                );
            });
        });
    }
}
