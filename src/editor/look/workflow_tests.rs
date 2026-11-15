use super::*;
use crate::application::test_support::{TestApplication, with_application};
use std::time::Duration;
use testing::{Call, Gate};

pub(super) fn selection() -> MaterialSelection {
    material::Selection::Monochrome(crate::looks::monochrome::Paper::default().into())
}

pub(super) fn operation(editor: &Editor, slot: Slot) -> Option<&Operation> {
    match slot {
        Slot::Lut => editor.document.lut.pending.as_ref(),
        Slot::FilmAndPaper => editor.document.film_and_paper.pending.as_ref(),
    }
}

pub(super) fn pending(app: &mut TestApplication<'_, Editor>, slot: Slot, source: Source) -> Call {
    let gate = app.update(Gate::install);
    app.enter(|editor, cx| editor.select_look(source, cx));
    let call = gate.next(app);
    assert!(call.valid);
    app.read(|editor| {
        assert!(operation(editor, slot).is_some());
        assert!(!editor.editing());
        assert!(!editor.workspace_input.is_blocked());
        assert!(!editor.interaction_blocked());
    });
    call
}

fn finish(app: &mut TestApplication<'_, Editor>, slot: Slot, call: Call) {
    call.finish();
    app.run_until(Duration::from_secs(15), |editor| {
        operation(editor, slot).is_none()
    });
}

#[test]
fn independent_stages_accept_in_either_order_without_replaying_other_slots_or_later_grades() {
    for lut_first in [false, true] {
        let (editor, scheduler) = tests::editor();
        let chosen = tests::asset(0.7);
        let material = selection();
        with_application(editor, |app| {
            let lut = pending(app, Slot::Lut, Source::Loaded(chosen.clone()));
            let paper = pending(app, Slot::FilmAndPaper, Source::Photochemical(material));
            let before = app.read(|editor| editor.document.state.project.clone());
            if lut_first {
                finish(app, Slot::Lut, lut);
                app.read(|editor| {
                    assert!(operation(editor, Slot::FilmAndPaper).is_some());
                    assert_eq!(editor.document.project.output.lut, Some(chosen.clone()));
                    assert_eq!(
                        editor.document.project.output.film_and_paper,
                        before.output.film_and_paper
                    );
                });
                app.update(|editor| {
                    let mut edit = editor.edit_project("Later grading").unwrap();
                    edit.update(|project| project.roll.exposure = 0.5);
                    editor.commit_project(edit).unwrap();
                });
                finish(app, Slot::FilmAndPaper, paper);
            } else {
                finish(app, Slot::FilmAndPaper, paper);
                app.read(|editor| {
                    assert!(operation(editor, Slot::Lut).is_some());
                    assert_eq!(editor.document.project.output.lut, before.output.lut);
                    assert_eq!(
                        editor
                            .document
                            .project
                            .output
                            .film_and_paper
                            .as_ref()
                            .unwrap()
                            .origin(),
                        &material.origin()
                    );
                });
                app.update(|editor| {
                    let mut edit = editor.edit_project("Later grading").unwrap();
                    edit.update(|project| project.roll.exposure = 0.5);
                    editor.commit_project(edit).unwrap();
                });
                finish(app, Slot::Lut, lut);
            }
            app.read(|editor| {
                assert_eq!(editor.document.project.roll.exposure, 0.5);
                assert_eq!(editor.document.project.output.lut, Some(chosen.clone()));
                assert_eq!(
                    editor
                        .document
                        .project
                        .output
                        .film_and_paper
                        .as_ref()
                        .unwrap()
                        .origin(),
                    &material.origin()
                );
                assert!(!editor.dialogs.is_open());
            });
        });
        scheduler.close_and_wait().unwrap();
    }
}

#[test]
fn cancelling_either_stage_preserves_the_other_request_and_its_publication() {
    for cancel_lut in [false, true] {
        let (editor, scheduler) = tests::editor();
        let chosen = tests::asset(0.7);
        with_application(editor, |app| {
            let lut = pending(app, Slot::Lut, Source::Loaded(chosen.clone()));
            let paper = pending(app, Slot::FilmAndPaper, Source::Photochemical(selection()));
            let slot = if cancel_lut {
                Slot::Lut
            } else {
                Slot::FilmAndPaper
            };
            let other = if cancel_lut {
                Slot::FilmAndPaper
            } else {
                Slot::Lut
            };
            let (activity, retained) = app.read(|editor| {
                (
                    operation(editor, slot).unwrap().activity.clone(),
                    operation(editor, other).unwrap().activity.clone(),
                )
            });
            activity.request_cancel();
            app.run_until(Duration::from_secs(15), |editor| {
                operation(editor, slot).is_none()
            });
            app.read(|editor| assert_eq!(operation(editor, other).unwrap().activity, retained));
            if cancel_lut {
                assert!(lut.cancelled.is_cancelled());
                finish(app, Slot::FilmAndPaper, paper);
                lut.finish();
            } else {
                assert!(paper.cancelled.is_cancelled());
                finish(app, Slot::Lut, lut);
                paper.finish();
            }
            app.pump();
            app.read(|editor| {
                assert_eq!(editor.document.project.output.lut.is_none(), cancel_lut);
                assert_eq!(
                    editor.document.project.output.film_and_paper.is_none(),
                    !cancel_lut
                );
                assert!(!editor.dialogs.is_open());
            });
        });
        scheduler.close_and_wait().unwrap();
    }
}

#[test]
fn superseding_a_stage_cannot_retire_its_peer_or_publish_the_old_choice() {
    for replace_lut in [false, true] {
        let (editor, scheduler) = tests::editor();
        let chosen = tests::asset(0.7);
        with_application(editor, |app| {
            let lut = pending(app, Slot::Lut, Source::Loaded(tests::asset(0.3)));
            let paper = pending(app, Slot::FilmAndPaper, Source::Photochemical(selection()));
            let slot = if replace_lut {
                Slot::Lut
            } else {
                Slot::FilmAndPaper
            };
            let next_material = MaterialSelection::new(
                None,
                material::Paper::Monochrome(
                    crate::looks::monochrome::Paper::from_id(
                        "foma_fomabrom_variant_glossy_filter_2",
                    )
                    .unwrap(),
                ),
            )
            .unwrap();
            let replacement = pending(
                app,
                slot,
                if replace_lut {
                    Source::Loaded(chosen.clone())
                } else {
                    Source::Photochemical(next_material)
                },
            );
            if replace_lut {
                assert!(lut.cancelled.is_cancelled());
                finish(app, slot, replacement);
                lut.finish();
                finish(app, Slot::FilmAndPaper, paper);
            } else {
                assert!(paper.cancelled.is_cancelled());
                finish(app, slot, replacement);
                paper.finish();
                finish(app, Slot::Lut, lut);
            }
            app.read(|editor| {
                assert_eq!(
                    editor.document.project.output.lut,
                    Some(if replace_lut {
                        chosen.clone()
                    } else {
                        tests::asset(0.3)
                    })
                );
                assert_eq!(
                    editor
                        .document
                        .project
                        .output
                        .film_and_paper
                        .as_ref()
                        .unwrap()
                        .origin(),
                    &(if replace_lut {
                        selection()
                    } else {
                        next_material
                    })
                    .origin()
                );
                assert!(!editor.dialogs.is_open());
            });
        });
        scheduler.close_and_wait().unwrap();
    }
}

#[test]
fn a_prepared_result_rejects_a_later_live_edit_without_cancelling_it_or_retrying_on_release() {
    let (editor, scheduler) = tests::editor();
    with_application(editor, |app| {
        let call = pending(app, Slot::Lut, Source::Loaded(tests::asset(0.7)));
        let before = app.read(|editor| editor.document.state.project.clone());
        let mut edit = app.update(|editor| editor.edit_project("Live control").unwrap());
        edit.update(|project| project.roll.exposure = 0.5);
        finish(app, Slot::Lut, call);
        app.read(|editor| {
            assert_eq!(editor.document.state.project, before);
            assert!(edit.preview().active());
            assert!(
                editor
                    .dialogs
                    .message()
                    .unwrap()
                    .details
                    .contains("finish or cancel the current edit")
            );
        });
        app.update(|editor| editor.commit_project(edit).unwrap());
        app.pump();
        app.read(|editor| {
            assert_eq!(editor.document.project.roll.exposure, 0.5);
            assert!(editor.document.project.output.lut.is_none());
            assert!(editor.look.loaded.is_empty());
        });
    });
    scheduler.close_and_wait().unwrap();
}
