use super::*;
use crate::{
    application::test_support::with_application, coverage::Coverage, processing::Calibration,
};
use std::time::Duration;

type App<'a> = crate::application::test_support::TestApplication<'a, Editor>;

fn lock(app: &mut App<'_>) {
    app.enter(|editor, cx| editor.begin_test_picker(super::super::PickerCase::GammaPivot, cx));
    app.pump();
    app.enter(|editor, cx| {
        let area = editor.selected_area().unwrap();
        editor
            .document
            .visit
            .picker
            .set_region(Some(Coverage::rectangle(area)));
        editor.apply_pick(cx);
    });
    app.run_until(Duration::from_secs(10), |editor| {
        !editor.interaction_blocked()
    });
    app.read(|editor| {
        assert!(
            editor.document.gamma_pivot.anchor.is_some(),
            "{}",
            editor.status
        )
    });
}

fn propose(app: &App<'_>, value: f64, active: bool) {
    app.enter(|editor, cx| {
        let mut candidate = editor.document.project_snapshot().as_ref().clone();
        if let InputSettings::Negative { gamma, .. } = &mut candidate.input {
            *gamma = value;
        }
        editor.roll_control(
            super::super::controls::ControlResponse {
                active,
                ..super::super::controls::ControlResponse::action(
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
        let candidate = editor.document.project_snapshot().as_ref().clone();
        editor.roll_control(
            super::super::controls::ControlResponse {
                active: false,
                ..super::super::controls::ControlResponse::action(
                    egui::Id::new("test-document-field"),
                    true,
                )
            },
            cx,
            move |v| *v = candidate,
        );
    });
}

fn settle(app: &mut App<'_>) {
    app.run_until(Duration::from_secs(10), |editor| !editor.controls_pending());
}

fn setup() -> (crate::fixture_path::FixturePath, Editor) {
    let file = crate::raw::fixture::bayer_dng_with_rgb([16000, 20000, 24000]);
    let mut editor = crate::editor::tests::authored_fixture(&[file.path().to_str().unwrap()]);
    editor.document.project.input = Calibration {
        dmin: glam::DVec3::splat(0.8),
        dmax: glam::DVec3::splat(0.1),
        input_floor: crate::processing::CAMERA_INPUT_FLOOR,
    }
    .into();

    (file, editor)
}

#[test]
fn capture_waits_for_apply_and_manual_gamma_accepts_one_compensated_transaction() {
    let (_file, editor) = setup();
    with_application(editor, |app| {
        let before = app.enter(|editor, cx| {
            let before = editor.history_snapshot();
            editor.begin_test_picker(super::super::PickerCase::GammaPivot, cx);
            before
        });
        app.pump();
        app.enter(|editor, _| {
            let area = editor.selected_area().unwrap();
            editor
                .document
                .visit
                .picker
                .set_region(Some(Coverage::rectangle(area)));
            assert!(editor.document.gamma_pivot.anchor.is_none());
            assert_eq!(editor.history_snapshot(), before);
        });
        app.enter(|editor, cx| editor.apply_pick(cx));
        app.run_until(Duration::from_secs(10), |editor| {
            !editor.interaction_blocked()
        });
        app.enter(|editor, cx| {
            assert!(
                editor.document.gamma_pivot.anchor.is_some(),
                "{}",
                editor.status
            );
            assert_eq!(editor.history_snapshot(), before);
            let previous = editor.document.project_snapshot();
            let mut candidate = previous.as_ref().clone();
            if let InputSettings::Negative { gamma, .. } = &mut candidate.input {
                *gamma = 0.8;
            }
            editor.roll_control(
                super::super::controls::ControlResponse {
                    active: false,
                    ..super::super::controls::ControlResponse::action(
                        egui::Id::new("test-document-field"),
                        true,
                    )
                },
                cx,
                move |v| *v = candidate,
            );
            assert_eq!(editor.document.project.input, previous.input);
            assert_eq!(editor.document.project.roll, previous.roll);
        });
        app.run_until(Duration::from_secs(10), |editor| !editor.controls_pending());
        app.enter(|editor, cx| {
            assert_ne!(
                editor.document.project.input, before.input,
                "{}",
                editor.status
            );
            assert_ne!(editor.document.project.roll.wb, before.roll.wb);
            assert_eq!(
                editor.history_label(super::super::history::Direction::Undo),
                Some("Gamma & balance")
            );
            editor.restore_history(super::super::history::Direction::Undo, cx);
        });
        app.run_until(Duration::from_secs(10), |editor| {
            !editor.interaction_blocked()
        });
        app.enter(|editor, _| {
            assert_eq!(editor.history_snapshot(), before);
            assert!(editor.document.gamma_pivot.anchor.is_some());
        });
    });
}

#[test]
fn no_anchor_is_uncompensated_and_a_locked_gesture_returns_exactly_to_its_start() {
    let (_file, editor) = setup();
    with_application(editor, |app| {
        let wb = app.read(|editor| editor.document.project.roll.wb);
        propose(app, 0.7, false);
        app.read(|editor| {
            assert_eq!(editor.document.project.input.gamma(), 0.7);
            assert_eq!(editor.document.project.roll.wb, wb);
            assert!(!editor.controls_pending());
        });
        lock(app);
        let before = app.enter(|editor, _| editor.history_snapshot());
        let prior_undo =
            app.read(|editor| editor.history_label(super::super::history::Direction::Undo));

        propose(app, 0.9, true);
        settle(app);
        app.read(|editor| {
            assert_ne!(editor.document.project_snapshot().roll.wb, wb);
            assert_eq!(
                editor.document.project.roll.wb, wb,
                "unfinished gesture stays a draft"
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
                super::super::controls::ControlResponse {
                    active: false,
                    ..super::super::controls::ControlResponse::action(
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
fn cancelling_and_failed_repicks_preserve_evidence_and_late_workers_cannot_publish() {
    let (_file, editor) = setup();
    with_application(editor, |app| {
        lock(app);
        let anchor = app.read(|editor| editor.document.gamma_pivot.anchor.clone().unwrap());
        app.enter(|editor, cx| editor.begin_test_picker(super::super::PickerCase::GammaPivot, cx));
        app.pump();
        app.enter(|editor, cx| editor.finish_pick(cx));
        app.pump();
        app.read(|editor| {
            assert!(Arc::ptr_eq(
                editor.document.gamma_pivot.anchor.as_ref().unwrap(),
                &anchor
            ))
        });

        let gate = app.update(measurement::testing::Gate::install);
        app.enter(|editor, cx| editor.begin_test_picker(super::super::PickerCase::GammaPivot, cx));
        app.pump();
        app.enter(|editor, cx| {
            editor
                .document
                .visit
                .picker
                .set_region(Some(Coverage::rectangle(editor.selected_area().unwrap())));
            editor.apply_pick(cx);
        });
        let failed = gate.next(app);
        failed.panic();
        app.run_until(Duration::from_secs(10), |editor| {
            !editor.interaction_blocked()
        });
        app.read(|editor| {
            assert!(Arc::ptr_eq(
                editor.document.gamma_pivot.anchor.as_ref().unwrap(),
                &anchor
            ));
            assert_eq!(editor.status, "Measurement worker panicked");
        });

        let gate = measurement::testing::Gate::after_work();
        app.update(|editor| editor.document.visit.measurement_gate = Some(gate.clone()));
        for failure in [false, true] {
            app.enter(|editor, cx| {
                editor.begin_test_picker(super::super::PickerCase::GammaPivot, cx)
            });
            app.pump();
            app.enter(|editor, cx| {
                editor
                    .document
                    .visit
                    .picker
                    .set_region(Some(Coverage::rectangle(editor.selected_area().unwrap())));
                editor.apply_pick(cx);
            });
            let stale = gate.next(app);
            app.update(|editor| {
                editor.replace_photo_scope(900);
                editor.status = "surviving visit".into();
            });
            app.pump();
            assert!(stale.cancelled.is_cancelled());
            let completed = stale.completed.clone();
            if failure {
                stale.panic();
            } else {
                stale.run();
            }
            completed.wait(app);
            app.pump();
            app.read(|editor| {
                assert_eq!(editor.status, "surviving visit");
                assert!(Arc::ptr_eq(
                    editor.document.gamma_pivot.anchor.as_ref().unwrap(),
                    &anchor
                ));
            });
            if !failure {
                // A new visit needs actual readiness; restore it through normal source loading.
                let path =
                    app.read(|editor| editor.document.source_path(anchor.recipe.source).unwrap());
                app.enter(|editor, cx| editor.load(path, cx));
                app.run_until(Duration::from_secs(10), |editor| {
                    !editor.interaction_blocked() && !editor.document.visit.source.pending()
                });
                app.update(|editor| {
                    editor.document.visit.measurement_gate = Some(gate.clone());
                });
            }
        }
    });
}

#[test]
fn stale_gamma_completion_and_invalid_gamma_leave_accepted_state_unchanged() {
    let (_file, editor) = setup();
    with_application(editor, |app| {
        lock(app);
        let before = app.enter(|editor, _| editor.history_snapshot());
        propose(app, 0.01, false);
        settle(app);
        app.enter(|editor, _| {
            assert_eq!(editor.history_snapshot(), before);
            assert!(editor.status.starts_with("Edit rejected:"));
            assert!(editor.document.gamma_pivot.anchor.is_some());
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
        propose(app, 0.8, false);
        let stale = gate.next(app);
        app.enter(|editor, cx| editor.toggle_gamma_pivot(cx));
        app.pump();
        assert!(stale.cancelled.is_cancelled());
        let completed = stale.completed.clone();
        stale.panic();
        completed.wait(app);
        app.pump();
        app.enter(|editor, _| {
            assert_eq!(editor.history_snapshot(), before);
            assert!(editor.document.gamma_pivot.anchor.is_none());
            assert_eq!(editor.status, "Gamma pivot released");
        });
    });
}

#[test]
fn completed_gamma_work_cannot_publish_to_a_replacement_document() {
    let (_file, editor) = setup();
    with_application(editor, |app| {
        lock(app);
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
            assert!(!editor.document.gamma_pivot.locked());
            assert_eq!(editor.status, "replacement document");
        });
    });
}

#[test]
fn a_no_net_gamma_gesture_preserves_redo_and_replay_restores_the_exact_pair() {
    let (_file, editor) = setup();
    with_application(editor, |app| {
        lock(app);
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
                super::super::controls::ControlResponse {
                    active: false,
                    ..super::super::controls::ControlResponse::action(
                        egui::Id::new("test-document-field"),
                        true,
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
fn downstream_changes_preserve_evidence_and_basis_or_source_removal_retires_it() {
    let (_file, editor) = setup();
    with_application(editor, |app| {
        lock(app);
        app.update(|editor| {
            editor.document.project.roll.exposure = 1.5;
            editor.document.set_target(super::super::OutputTarget::Srgb);
            editor.photo_settings_mut().exposure = -1.;
            editor.document.visit.canvas.viewport.fit();
            editor.observe_gamma_pivot();
            assert!(editor.document.gamma_pivot.anchor.is_some());
            if let InputSettings::Negative { response, .. } = &mut editor.document.project.input {
                response.toe = 1.2;
            }
            editor.observe_gamma_pivot();
            assert!(editor.document.gamma_pivot.anchor.is_none());
        });
        lock(app);
        app.update(|editor| {
            editor.document.project.order.clear();
            editor.observe_gamma_pivot();
            assert!(editor.document.gamma_pivot.anchor.is_none());
        });
    });
}
