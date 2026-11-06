use super::*;
use crate::{
    application::test_support::with_application,
    editor::{
        analysis::{
            testing,
            workflow_tests::{LIMIT, fixture},
        },
        source_loading,
    },
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::{Arc, Weak, mpsc},
};

#[test]
fn tool_entry_replaces_the_owned_interaction_without_accepting_its_draft() {
    let sources = source_loading::SourceLoader::new();
    let editor = fixture(sources.clone());
    let before = editor.document.project.clone();
    with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.begin_crop();
            editor.document.crop_edit_mut().unwrap().draft.size = [100, 100];
            editor.begin_dust_cleanup();
            assert!(editor.current_tool().unwrap().is::<dust::DustTool>());
            assert!(editor.document.visit.masks.gesture().is_none());
            editor.begin_linear_mask(None);
            assert!(editor.creating_mask());
            assert!(
                editor
                    .current_tool()
                    .unwrap()
                    .is::<mask_editor::CreationTool>()
            );
            assert!(editor.document.visit.masks.gesture().is_none());
            editor.begin_test_picker(PickerCase::Dmin, cx);
        });
        app.pump();
        app.read(|editor| {
            assert!(editor.test_picker_active());
            assert!(editor.document.visit.masks.gesture().is_none());
        });
        app.pump();
        app.read(|editor| assert_eq!(editor.test_pick(), Some(PickerCase::Dmin.display())));
        app.enter(|editor, cx| {
            editor.request_analysis(cx);
            assert!(editor.current_tool().unwrap().is::<AnalyzeTool>());
            assert!(editor.document.visit.masks.gesture().is_none());
            let reference = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/raw/bare-light.dng");
            editor.begin_flat_edit(reference, None, cx);
            assert!(
                editor.current_tool().unwrap().is::<AnalyzeTool>(),
                "working modal rejects replacement"
            );
            editor.cancel_analysis();
            assert_eq!(editor.document.project, before);
            assert!(editor.history_label(history::Direction::Undo).is_none());
        });
        app.pump();
        app.update(Editor::begin_crop);
        app.read(|editor| {
            assert!(editor.document.crop_edit().is_some());
            assert_eq!(editor.document.project, before);
        });
    });
    sources.close_and_wait().unwrap();
}

#[test]
fn changing_input_from_review_retires_its_projection_on_success_and_failure() {
    for fail in [false, true] {
        let sources = source_loading::SourceLoader::new();
        let mut editor = fixture(sources.clone());
        editor.preferences.scopes_visible = true;
        editor.scopes.visible = true;
        with_application(editor, |app| {
            app.enter(|editor, cx| editor.request_analysis(cx));
            app.run_until(LIMIT, |editor| editor.document.analysis_review().is_some());
            let before = app.read(|editor| editor.document.project.clone());
            app.enter(|editor, cx| {
                editor.request_input(InputSettings::Digital, cx);
                assert!(editor.dialogs.is_open());
                if fail {
                    editor.source_services = None;
                }
                editor.dialogs.answer_for_test("Switch stage");
            });
            app.pump();
            app.read(|editor| {
                assert!(editor.document.analysis_operation().is_none());
                assert!(editor.test_pick().is_none());
                assert!(editor.scopes.visible);
            });
            app.run_until(LIMIT, |editor| !editor.interaction_blocked());
            app.read(|editor| {
                assert!(editor.test_pick().is_none());
                assert!(editor.scopes.visible && editor.preferences.scopes_visible);
                if fail {
                    assert_eq!(editor.document.project, before);
                    assert!(editor.status.contains("Document services unavailable"));
                } else {
                    assert_eq!(editor.document.input, InputSettings::Digital);
                }
            });
        });
        sources.close_and_wait().unwrap();
    }
}

#[derive(Clone, Default)]
struct ContinuationProbe {
    retired: Rc<Cell<bool>>,
    continued: Rc<Cell<bool>>,
}
struct Retirement(Rc<Cell<bool>>);
impl Drop for Retirement {
    fn drop(&mut self) {
        self.0.set(true);
    }
}

#[allow(clippy::too_many_arguments)]
fn held_work<T: Send + 'static>(
    sources: Arc<dyn SourceProvider>,
    exposures: Arc<crate::exposure_metadata::Store>,
    output: T,
    entered: mpsc::Sender<Weak<crate::scan::DevelopedSource>>,
    release: mpsc::Receiver<()>,
    returned: mpsc::Sender<()>,
    probe: ContinuationProbe,
) -> testing::Work<T> {
    Box::new(move |request, stopped| {
        let retired = Retirement(probe.retired.clone());
        Box::pin(async move {
            let _retired = retired;
            // This scripted task acquires an unviewed original to probe physical-input
            // cleanup, not to model production review assembly's metadata-only input.
            let scan = request
                .document
                .acquire_source(
                    request.document.project.order[1],
                    sources.as_ref(),
                    &exposures,
                    &stopped,
                )
                .await
                .map_err(|error| error.to_string())?
                .expect("unviewed original");
            let result = background::compute(&stopped, move || {
                entered.send(Arc::downgrade(scan.source())).unwrap();
                release.recv_timeout(LIMIT).unwrap();
                drop(scan);
                returned.send(()).unwrap();
                Ok(output)
            })
            .await;
            probe.continued.set(true);
            result.map_err(|error| error.to_string())
        })
    })
}

#[test]
fn owner_retirement_releases_running_preparation_and_fitting_while_physical_inputs_survive_cleanup()
{
    for fitting in [false, true] {
        for replace in [false, true] {
            let sources = source_loading::SourceLoader::new();
            let editor = fixture(sources.clone());
            with_application(editor, |app| {
                app.enter(|editor, cx| editor.request_analysis(cx));
                app.run_until(LIMIT, |editor| editor.document.analysis_review().is_some());
                let (entered, starts) = mpsc::channel();
                let (release, resume) = mpsc::channel();
                let (returned, returns) = mpsc::channel();
                let probe = ContinuationProbe::default();
                let (before, proposed) = app.enter(|editor, cx| {
                    let reviewed = editor.document.analysis_review().unwrap().approved.clone();
                    let sources = editor.source_provider().unwrap();
                    let exposures = editor.document.exposures.clone();
                    let before = editor.document.project.clone();
                    if fitting {
                        let mut document = editor.document.capture();
                        document.project.roll.exposure += 0.75;
                        document.project.analysis_regions = reviewed.entries;
                        editor.validate_document_candidate(&document).unwrap();
                        let proposed = document.project.clone();
                        editor.document.analysis_fit_work = Some(held_work(
                            sources,
                            exposures,
                            Box::new(analysis::Outcome {
                                document,
                                frames: 2,
                                support: editor.document.visit.canvas.ready.as_ref().unwrap().area,
                            }),
                            entered,
                            resume,
                            returned,
                            probe.clone(),
                        ));
                        editor.confirm_analysis_review(cx);
                        (before, Some(proposed))
                    } else {
                        editor.finish_pick(cx);
                        editor.document.analysis_preparation_work = Some(held_work(
                            sources,
                            exposures,
                            reviewed,
                            entered,
                            resume,
                            returned,
                            probe.clone(),
                        ));
                        let request = editor.analysis_request().unwrap();
                        editor.prepare_analysis_review(request, cx);
                        (before, None)
                    }
                });
                let pixels = RefCell::new(None);
                let deadline = std::time::Instant::now() + LIMIT;
                while pixels.borrow().is_none() {
                    app.pump();
                    if let Ok(source) = starts.try_recv() {
                        *pixels.borrow_mut() = Some(source);
                    }
                    assert!(
                        std::time::Instant::now() < deadline,
                        "CPU input did not arrive"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                let pixels = pixels.into_inner().unwrap();
                assert!(!probe.retired.get() && !probe.continued.get());
                if replace {
                    app.update(Editor::end_tool);
                    app.pump();
                    app.update(Editor::begin_crop);
                    app.read(|editor| {
                        assert!(editor.document.crop_edit().is_some());
                        assert!(!editor.interaction_blocked());
                        assert_eq!(editor.document.project, before);
                    });
                    assert!(
                        pixels.upgrade().is_some(),
                        "running CPU work must retain its native input"
                    );
                    assert!(matches!(returns.try_recv(), Err(mpsc::TryRecvError::Empty)));
                    assert!(
                        probe.retired.get(),
                        "replacement must drop the waiting continuation before physical cleanup"
                    );
                    assert!(!probe.continued.get());
                }
                release.send(()).unwrap();
                let deadline = std::time::Instant::now() + LIMIT;
                while pixels.upgrade().is_some() {
                    app.pump();
                    assert!(
                        std::time::Instant::now() < deadline,
                        "retired physical input was retained"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                returns.recv_timeout(LIMIT).unwrap();
                let turned = Rc::new(Cell::new(false));
                let reached = turned.clone();
                let turn = app.enter(|_, cx| {
                    cx.spawn(move |_| async move {
                        futures_lite::future::yield_now().await;
                        reached.set(true);
                    })
                    .unwrap()
                });
                app.run_until(LIMIT, |_| turned.get());
                drop(turn);
                if replace {
                    assert!(
                        !probe.continued.get(),
                        "retired work cannot resume after cleanup"
                    );
                    app.read(|editor| {
                        assert!(editor.document.crop_edit().is_some());
                        assert_eq!(editor.document.project, before);
                        assert!(editor.history_label(history::Direction::Undo).is_none());
                    });
                } else if let Some(proposed) = proposed {
                    app.run_until(LIMIT, |editor| {
                        editor.document.analysis_operation().is_none()
                    });
                    app.read(|editor| {
                        assert_eq!(editor.document.project, proposed);
                        assert!(editor.history_label(history::Direction::Undo).is_some());
                    });
                } else {
                    app.run_until(LIMIT, |editor| editor.document.analysis_review().is_some());
                    app.read(|editor| assert_eq!(editor.document.project, before));
                }
                if !replace {
                    assert!(
                        probe.continued.get() && probe.retired.get(),
                        "unreplaced work must deliver its result through the observed continuation"
                    );
                }
            });
            sources.close_and_wait().unwrap();
        }
    }
}
