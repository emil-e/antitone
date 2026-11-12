use super::*;
use crate::editor::history::Direction;

pub(super) fn depart(harness: &mut Harness<'_, Editor>, keyboard: bool) {
    if keyboard {
        harness.key_press(egui::Key::ArrowLeft);
    } else {
        harness.get_by_label("dmax.dng").click();
    }
    harness.step();
    harness.read(|editor| {
        assert_eq!(
            editor.document.visit.current_source,
            Some(editor.document.project.order[1])
        );
    });
    wait_for(harness, |editor| !editor.document.visit.source.pending());
}

pub(super) fn return_to_photo(harness: &mut Harness<'_, Editor>) {
    harness.get_by_label("working-photo.dng").click();
    harness.step();
    wait_for(harness, |editor| !editor.document.visit.source.pending());
}

#[test]
fn filmstrip_and_arrows_retire_crop_drafts_only_when_the_active_photo_changes() {
    workflow::calibrated_workflow(|mut harness, _, _directory| {
        for (name, size, width) in [
            ("normal", egui::vec2(1100., 800.), 315.),
            ("narrow", egui::vec2(800., 600.), 270.),
        ] {
            set_layout(&mut harness, size, width);
            for keyboard in [false, true] {
                stage(&mut harness, "Photo");
                let before = harness.read(|editor| editor.document.state.project.clone());
                let history = harness.read(|editor| editor.history_label(Direction::Undo));
                let visit = harness.read(|editor| editor.document.visit.view);
                let draft = tools::crop_draft(&mut harness);
                harness.get_by_label("working-photo.dng").click();
                harness.step();
                harness.read(|editor| {
                    assert_eq!(editor.document.visit.view, visit);
                    assert_eq!(editor.document.crop_edit().unwrap().draft, draft);
                    assert_eq!(editor.document.state.project, before);
                });
                save_capture(
                    &harness.render().unwrap(),
                    &format!("crop-browsing-{name}-{keyboard}"),
                );

                depart(&mut harness, keyboard);
                harness.read(|editor| {
                    assert_ne!(editor.document.visit.view, visit);
                    assert!(!editor.cropping());
                    assert_eq!(editor.document.state.project, before);
                    assert_eq!(editor.history_label(Direction::Undo), history);
                });
                return_to_photo(&mut harness);
                harness.read(|editor| {
                    assert!(!editor.cropping());
                    assert_eq!(editor.document.state.project, before);
                });
            }
        }
    });
}

#[test]
fn filmstrip_and_arrows_retire_real_picker_callers_without_measuring_or_accepting() {
    workflow::calibrated_workflow(|mut harness, _, _directory| {
        for (size, width) in [
            (egui::vec2(1100., 800.), 315.),
            (egui::vec2(800., 600.), 270.),
        ] {
            set_layout(&mut harness, size, width);
            for kind in [
                PickerCase::Dmin,
                PickerCase::Dmax,
                PickerCase::Gamma,
                PickerCase::GammaPivot,
                PickerCase::RollWb,
                PickerCase::Wb,
            ] {
                for keyboard in [false, true] {
                    let before = harness.read(|editor| editor.document.state.project.clone());
                    let history = harness.read(|editor| editor.history_label(Direction::Undo));
                    picking::open(&mut harness, kind);
                    harness.read(|editor| assert_eq!(editor.test_pick(), Some(kind.display())));
                    let image =
                        harness.read(|editor| editor.active_canvas().last_layout.unwrap().0.image);
                    drag(
                        &mut harness,
                        image.center() - egui::vec2(10., 10.),
                        image.center() + egui::vec2(10., 10.),
                    );
                    harness.read(|editor| assert!(editor.test_pick_region().is_some()));
                    assert!(!harness.get_by_label("Apply").accesskit_node().is_disabled());
                    let visit = harness.read(|editor| editor.document.visit.view);
                    harness.get_by_label("working-photo.dng").click();
                    harness.step();
                    harness.read(|editor| {
                        assert_eq!(editor.document.visit.view, visit);
                        assert_eq!(editor.test_pick(), Some(kind.display()));
                    });

                    depart(&mut harness, keyboard);
                    harness.read(|editor| {
                        assert!(editor.test_pick().is_none());
                        assert!(editor.document.visit.interaction_action.is_none());
                        assert!(!editor.measurement_pending());
                        assert_eq!(editor.document.state.project, before);
                        assert_eq!(editor.history_label(Direction::Undo), history);
                    });
                    return_to_photo(&mut harness);
                    harness.read(|editor| assert!(editor.test_pick().is_none()));
                }
            }
        }
    });
}

#[test]
fn flat_reference_survives_browsing_and_apply_uses_its_captured_reference() {
    workflow::calibrated_workflow(|mut harness, _, _directory| {
        harness.enter(|editor, cx| editor.begin_flat_edit(fixture("bare-light.dng"), None, cx));
        wait_for(&mut harness, |editor| !editor.workspace_input.is_blocked());
        workflow::wait_settled(&mut harness);
        harness.get_by_label("100%").click();
        harness.step();
        harness.get_by_label("Zoom in").click();
        harness.step();
        let viewport = harness.update(|editor| editor.with_tool_viewport(|viewport| *viewport));
        let before = harness.read(|editor| editor.document.state.project.clone());
        let (reference, region) = harness.read(|editor| {
            let state = editor.document.flat_edit().unwrap();
            (
                state.view.ready.as_ref().unwrap().developed.clone(),
                state.region.unwrap(),
            )
        });
        for keyboard in [false, true] {
            harness.get_by_label("working-photo.dng").click();
            harness.step();
            depart(&mut harness, keyboard);
            workflow::wait_settled(&mut harness);
            assert_eq!(
                harness.update(|editor| editor.with_tool_viewport(|viewport| *viewport)),
                viewport
            );
            harness.read(|editor| {
                let state = editor.document.flat_edit().unwrap();
                assert!(Arc::ptr_eq(
                    &reference,
                    &state.view.ready.as_ref().unwrap().developed
                ));
                assert_eq!(state.region, Some(region));
                assert_eq!(editor.document.state.project, before);
                assert_eq!(
                    editor
                        .document
                        .visit
                        .canvas
                        .ready
                        .as_ref()
                        .unwrap()
                        .path
                        .file_name()
                        .unwrap(),
                    "dmax.dng"
                );
            });
            save_capture(
                &harness.render().unwrap(),
                &format!("flat-browsing-{keyboard}"),
            );
            return_to_photo(&mut harness);
        }
        depart(&mut harness, false);
        let active = harness.read(|editor| editor.document.visit.current_source);
        harness.get_by_label("Apply").click();
        harness.step();
        wait_for(&mut harness, |editor| !editor.workspace_input.is_blocked());
        harness.read(|editor| {
            assert!(editor.document.flat_edit().is_none());
            assert!(
                editor.document.correction.prepared().is_some(),
                "{}",
                editor.status
            );
            assert_eq!(
                editor.document.project.flat_field().unwrap().reference_name,
                "bare-light.dng"
            );
            assert_eq!(editor.document.visit.current_source, active);
            assert_eq!(editor.document.project.order, before.order);
            assert_eq!(editor.history_label(Direction::Undo), Some("Correction"));
        });
    });
}
