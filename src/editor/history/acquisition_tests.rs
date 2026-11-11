use super::*;
use crate::application::test_support::{TestApplication, with_application};
use std::sync::{Mutex, mpsc};

struct Release(Option<mpsc::Sender<()>>);
impl Release {
    fn now(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}
impl Drop for Release {
    fn drop(&mut self) {
        self.now();
    }
}

fn held_second_photo() -> (
    Editor,
    source_loading::SourceLoader,
    mpsc::Receiver<()>,
    Release,
) {
    let mut editor = crate::editor::tests::authored_fixture(&["roll-0.dng", "roll-1.dng"]);
    editor.document.history = History::default();
    editor.document.thumbnails = Thumbnails::default().with_sources(editor.document.cache.clone());
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let resume = Mutex::new(resume);
    let loader = source_loading::SourceLoader::new_observed(move |path, phase, _| {
        if path.ends_with("roll-1.dng") && matches!(phase, source_loading::NativePhase::UnpackBegin)
        {
            entered.send(()).unwrap();
            resume
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(30))
                .unwrap();
        }
        Ok(())
    });
    editor.source_services.as_mut().unwrap().developments = loader.clone();
    editor.developments = Some(loader.clone());
    (editor, loader, starts, Release(Some(release)))
}

fn wait_for_unpack(app: &mut TestApplication<'_, Editor>, starts: &mpsc::Receiver<()>) {
    let until = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        app.pump();
        if starts.try_recv().is_ok() {
            break;
        }
        assert!(
            std::time::Instant::now() < until,
            "second photograph never started native unpack"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn grading_history_preserves_the_pending_visit_and_its_native_acquisition() {
    let (editor, loader, starts, mut release) = held_second_photo();
    with_application(editor, |app| {
        app.enter(|editor, cx| {
            let second = editor.document.project.order[1];
            editor.load(editor.document.source_path(second).unwrap(), cx);
            let before = editor.history_snapshot();
            editor.document.project.roll.exposure = 0.75;
            editor.record_edit(before, "Roll exposure", None);
            editor.document.visit.canvas.viewport.zoom = Some(1.75);
        });
        wait_for_unpack(app, &starts);
        app.enter(|editor, cx| {
            let view = editor.document.visit.view;
            let viewport = editor.document.visit.canvas.viewport;
            editor.restore_history(Direction::Undo, cx);
            assert_eq!(editor.document.project.roll.exposure, 0.);
            assert_eq!(editor.document.visit.view, view);
            assert_eq!(editor.document.visit.canvas.viewport, viewport);
            assert!(editor.document.visit.source.pending());
            assert!(!editor.interaction_blocked());
            assert_eq!(editor.history_label(Direction::Redo), Some("Roll exposure"));
            editor.restore_history(Direction::Redo, cx);
            assert_eq!(editor.document.project.roll.exposure, 0.75);
            assert_eq!(editor.document.visit.view, view);
        });
        release.now();
        app.run_until(Duration::from_secs(15), |editor| {
            !editor.document.visit.source.pending()
        });
        app.read(|editor| {
            assert!(
                editor
                    .document
                    .visit
                    .canvas
                    .ready
                    .as_ref()
                    .unwrap()
                    .path
                    .ends_with("roll-1.dng")
            );
            assert_eq!(editor.document.project.roll.exposure, 0.75);
            assert_eq!(editor.history_label(Direction::Undo), Some("Roll exposure"));
        });
    });
    loader.close_and_wait().unwrap();
}

#[test]
fn rapid_history_retires_old_photo_presentation_in_both_native_completion_orders() {
    for complete_first in [false, true] {
        let (mut editor, loader, starts, mut release) = held_second_photo();
        let ids = editor.document.project.order.clone();
        let original = editor.history_snapshot();
        for (id, value) in [(ids[0], 0.5), (ids[1], 0.75)] {
            let before = editor.history_snapshot();
            editor
                .document
                .project
                .source_mut(id)
                .unwrap()
                .orientation
                .turn_clockwise();
            editor.document.project.roll.exposure = value;
            editor.record_edit(before, "Photo orientation", Some(id));
        }
        with_application(editor, |app| {
            app.enter(|editor, cx| {
                editor.restore_history(Direction::Undo, cx);
                assert_eq!(editor.document.visit.current_source, Some(ids[1]));
                assert_eq!(editor.document.project.roll.exposure, 0.5);
            });
            wait_for_unpack(app, &starts);
            if complete_first {
                release.now();
                app.run_until(Duration::from_secs(15), |editor| {
                    !editor.document.visit.source.pending()
                });
                app.read(|editor| {
                    assert!(
                        editor
                            .document
                            .visit
                            .canvas
                            .ready
                            .as_ref()
                            .unwrap()
                            .path
                            .ends_with("roll-1.dng")
                    )
                });
            }
            app.enter(|editor, cx| {
                for direction in [
                    Direction::Undo,
                    Direction::Redo,
                    Direction::Redo,
                    Direction::Undo,
                    Direction::Undo,
                ] {
                    editor.restore_history(direction, cx);
                    assert!(editor.document_pending.is_none() && !editor.interaction_blocked());
                    assert_eq!(
                        editor
                            .source_epoch
                            .load(std::sync::atomic::Ordering::Relaxed),
                        editor.id
                    );
                }
                assert_eq!(editor.document.project, original);
                assert_eq!(editor.document.visit.current_source, Some(ids[0]));
                assert_eq!(
                    (
                        editor.document.history.undo.len(),
                        editor.document.history.redo.len()
                    ),
                    (0, 2)
                );
            });
            app.run_until(Duration::from_secs(15), |editor| {
                !editor.document.visit.source.pending()
            });
            app.read(|editor| {
                assert!(
                    editor
                        .document
                        .visit
                        .canvas
                        .ready
                        .as_ref()
                        .unwrap()
                        .path
                        .ends_with("roll-0.dng")
                )
            });
            release.now();
            loader.close_and_wait().unwrap();
            app.pump();
            app.read(|editor| {
                assert_eq!(editor.document.project, original);
                assert_eq!(editor.document.visit.current_source, Some(ids[0]));
                assert!(
                    editor
                        .document
                        .visit
                        .canvas
                        .ready
                        .as_ref()
                        .unwrap()
                        .path
                        .ends_with("roll-0.dng")
                );
                assert_eq!(
                    (
                        editor.document.history.undo.len(),
                        editor.document.history.redo.len()
                    ),
                    (0, 2)
                );
            });
        });
        loader.close_and_wait().unwrap();
    }
}
