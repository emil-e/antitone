use super::*;
use crate::application::test_support::with_application;
use crate::platform::menu::Action;
use std::time::Duration;
use workflow_tests::pending;

#[test]
fn save_as_keeps_its_written_asset_baseline_when_preparation_accepts_before_or_after_the_write() {
    for accept_before_write in [false, true] {
        let (mut editor, scheduler) = tests::editor();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("saved.antitone");
        let accepted = tests::asset(0.2);
        let chosen = tests::asset(0.7);
        editor.document.project.output.lut = Some(accepted.clone());
        with_application(editor, |app| {
            let call = pending(app, Slot::Lut, Source::Loaded(chosen.clone()));
            let hold = crate::background::testing::pause();
            app.enter(|editor, cx| {
                assert!(editor.application_menu_state().enabled(Action::SaveAs));
                editor.save_destination_action(
                    super::super::files::Destination {
                        path: path.clone(),
                        policy: crate::project::Save::New,
                        confirm_replace: false,
                    },
                    None,
                    cx,
                );
                assert!(editor.document.write.pending());
                assert!(editor.document.lut.pending.is_some());
            });
            app.pump();
            if accept_before_write {
                call.finish();
                app.run_until(Duration::from_secs(15), |editor| {
                    editor.document.lut.pending.is_none()
                });
                assert!(app.read(|editor| editor.document.write.pending()));
                drop(hold);
                app.run_until(Duration::from_secs(15), |editor| {
                    !editor.document.write.pending()
                });
            } else {
                drop(hold);
                app.run_until(Duration::from_secs(15), |editor| {
                    !editor.document.write.pending()
                });
                app.read(|editor| {
                    assert!(!editor.document.dirty() && editor.document.lut.pending.is_some())
                });
                call.finish();
                app.run_until(Duration::from_secs(15), |editor| {
                    editor.document.lut.pending.is_none()
                });
            }
            assert_eq!(
                crate::project::Project::load(&path).unwrap().output.lut,
                Some(accepted)
            );
            app.read(|editor| {
                assert_eq!(editor.document.path.as_ref(), Some(&path));
                assert_eq!(editor.document.project.output.lut, Some(chosen.clone()));
                assert!(editor.document.dirty());
                assert!(!editor.dialogs.is_open());
            });
        });
        scheduler.close_and_wait().unwrap();
    }
}

fn codes(path: &std::path::Path) -> Vec<u16> {
    let mut decoder = tiff::decoder::Decoder::new(std::fs::File::open(path).unwrap()).unwrap();
    let tiff::decoder::DecodingResult::U16(codes) = decoder.read_image().unwrap() else {
        panic!("RGB16 TIFF")
    };
    codes
}

#[test]
fn export_form_captures_only_the_asset_accepted_at_confirmation_and_frozen_output_survives_later_acceptance()
 {
    for accept_before_capture in [false, true] {
        let mut editor = super::super::tests::authored_fixture(&["patches.dng"]);
        editor.set_dmin(Some(DVec3::splat(0.8)));
        editor.set_dmax(Some(DVec3::splat(0.02)));
        editor.document.source = false;
        let accepted = tests::asset(0.2);
        let chosen = tests::asset(0.7);
        editor.document.project.output.lut = Some(accepted.clone());
        let services = editor.export_services.as_ref().unwrap().clone();
        let directory = tempfile::tempdir().unwrap();
        let mut draft =
            super::super::export::Draft::new(OutputTarget::Srgb, directory.path().to_owned());
        draft.suffix = "_before".into();
        let before = editor.capture_export(&draft).unwrap();
        let before_path = before.photos[0].path.clone();
        let (captured, captured_path, after, after_path) = with_application(editor, |app| {
            let call = pending(app, Slot::Lut, Source::Loaded(chosen.clone()));
            app.update(|editor| {
                assert!(editor.application_menu_state().enabled(Action::Export));
                editor.open_export_dialog();
                assert!(editor.document.export.form.is_some());
            });
            let call = if accept_before_capture {
                call.finish();
                app.run_until(Duration::from_secs(15), |editor| {
                    editor.document.lut.pending.is_none()
                });
                None
            } else {
                Some(call)
            };
            let captured = app.update(|editor| {
                let mut form = editor.document.export.form.take().unwrap();
                form.draft.folder = directory.path().to_owned();
                form.draft.target = OutputTarget::Srgb;
                form.draft.suffix = "_captured".into();
                let request = editor.capture_export(&form.draft).unwrap();
                editor.document.export.form = Some(form);
                request
            });
            assert_eq!(
                captured.photos[0].recipe.luts.lut.as_ref(),
                Some(if accept_before_capture {
                    &chosen
                } else {
                    &accepted
                })
            );
            if let Some(call) = call {
                call.finish();
                app.run_until(Duration::from_secs(15), |editor| {
                    editor.document.lut.pending.is_none()
                });
            }
            let after = app.update(|editor| {
                editor.document.export.form = None;
                draft.suffix = "_after".into();
                editor.capture_export(&draft).unwrap()
            });
            assert_eq!(after.photos[0].recipe.luts.lut.as_ref(), Some(&chosen));
            let captured_path = captured.photos[0].path.clone();
            let after_path = after.photos[0].path.clone();
            (captured, captured_path, after, after_path)
        });
        for request in [before, captured, after] {
            let summary = pollster::block_on(services.clone().export_batch(
                request,
                crate::cancellation::Cancellation::default(),
                |_, _| {},
            ));
            assert_eq!((summary.exported(), summary.failed()), (1, 0));
        }
        let before = codes(&before_path);
        let after = codes(&after_path);
        assert_ne!(
            before, after,
            "the two assets must produce contrasting files"
        );
        assert_eq!(
            codes(&captured_path),
            if accept_before_capture { after } else { before }
        );
    }
}
