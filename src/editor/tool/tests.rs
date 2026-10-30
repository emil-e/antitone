use super::*;
use crate::{
    application::test_support::with_application,
    editor::{
        analysis_services::Services,
        analysis_workflow::{
            testing,
            tests::{LIMIT, fixture},
        },
        source_loading,
    },
};
use std::{
    cell::RefCell,
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
            assert!(matches!(editor.document.tool, Some(Tool::Dust(_))));
            editor.begin_linear_mask(None);
            assert!(matches!(editor.document.tool, Some(Tool::Mask(_))));
            editor.begin_pick(Pick::Dmin, cx);
            assert!(matches!(editor.document.tool, Some(Tool::Pick(_))));
        });
        app.pump();
        app.read(|editor| assert_eq!(editor.active_pick(), Some(Pick::Dmin.display())));
        app.enter(|editor, cx| {
            editor.request_analysis(cx);
            assert!(matches!(editor.document.tool, Some(Tool::Analyze(_))));
            let reference = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/raw/bare-light.dng");
            editor.begin_flat_edit(reference, None, cx);
            assert!(matches!(editor.document.tool, Some(Tool::Flat(_))));
            editor.begin_crop();
            assert!(matches!(editor.document.tool, Some(Tool::Crop(_))));
            assert_eq!(editor.document.project, before);
            assert!(editor.history_label(history::Direction::Undo).is_none());
        });
        app.run_until(LIMIT, |editor| editor.document.spatial_preparing.is_none());
        app.read(|editor| {
            assert!(matches!(editor.document.tool, Some(Tool::Crop(_))));
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
            app.enter(|editor, cx| {
                let request = editor.analysis_request().unwrap();
                editor.prepare_analysis_review(request, cx);
            });
            app.run_until(LIMIT, |editor| editor.document.analysis_review().is_some());
            let before = app.read(|editor| editor.document.project.clone());
            app.enter(|editor, cx| {
                editor.request_input(InputSettings::Digital, cx);
                let target = editor
                    .document
                    .input_switch
                    .take()
                    .expect("stage-change confirmation");
                if fail {
                    editor.source_services = None;
                }
                editor.apply_input(target, cx);
                assert!(editor.document.analysis_operation().is_none());
                assert!(editor.active_pick().is_none());
                assert!(editor.scopes.visible);
            });
            app.run_until(LIMIT, |editor| !editor.busy);
            app.read(|editor| {
                assert!(editor.active_pick().is_none());
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

fn held_work<T: Send + 'static>(
    services: Services,
    output: T,
    entered: mpsc::Sender<Weak<crate::scan::DevelopedSource>>,
    release: mpsc::Receiver<()>,
    returned: mpsc::Sender<()>,
) -> testing::Work<T> {
    Box::new(move |request, stopped| {
        Box::pin(async move {
            // Acquire the unviewed original through real native services. Only the
            // numerical output is programmed; the CPU job owns its pixels through release.
            let scan = services
                .acquire(Arc::new(request), 1, stopped.clone())
                .await
                .map_err(|error| error.to_string())?
                .expect("unviewed original");
            services
                .cpu(&stopped, move || {
                    entered.send(Arc::downgrade(scan.source())).unwrap();
                    release.recv_timeout(LIMIT).unwrap();
                    drop(scan);
                    returned.send(()).unwrap();
                    Ok(output)
                })
                .await
                .map_err(|error| error.to_string())
        })
    })
}

#[test]
fn replacement_retires_running_preparation_and_fitting_while_their_physical_inputs_survive_cleanup()
{
    for fitting in [false, true] {
        for replace in [false, true] {
            let sources = source_loading::SourceLoader::new();
            let editor = fixture(sources.clone());
            with_application(editor, |app| {
                app.enter(|editor, cx| {
                    let request = editor.analysis_request().unwrap();
                    editor.prepare_analysis_review(request, cx);
                });
                app.run_until(LIMIT, |editor| editor.document.analysis_review().is_some());
                let (entered, starts) = mpsc::channel();
                let (release, resume) = mpsc::channel();
                let (returned, returns) = mpsc::channel();
                let (before, proposed) = app.enter(|editor, cx| {
                    let reviewed = editor.document.analysis_review().unwrap().approved.clone();
                    let services = Services::new(
                        editor.source_services.as_ref().unwrap(),
                        editor.source_provider().unwrap(),
                        editor.document.exposures.clone(),
                    );
                    let before = editor.document.project.clone();
                    if fitting {
                        let mut document = editor.document.capture();
                        document.project.roll.exposure += 0.75;
                        document.project.analysis_regions = reviewed.entries;
                        editor.validate_document_candidate(&document).unwrap();
                        let proposed = document.project.clone();
                        editor.document.analysis_fit_work = Some(held_work(
                            services,
                            Box::new(analysis::Outcome {
                                document,
                                frames: 2,
                                support: editor.document.visit.canvas.ready.as_ref().unwrap().area,
                            }),
                            entered,
                            resume,
                            returned,
                        ));
                        editor.confirm_analysis_review(cx);
                        (before, Some(proposed))
                    } else {
                        editor.finish_pick(cx);
                        editor.document.analysis_preparation_work =
                            Some(held_work(services, reviewed, entered, resume, returned));
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
                if replace {
                    app.update(Editor::begin_crop);
                    app.pump();
                    app.read(|editor| {
                        assert!(matches!(editor.document.tool, Some(Tool::Crop(_))));
                        assert!(!editor.busy);
                        assert_eq!(editor.document.project, before);
                    });
                    assert!(
                        pixels.upgrade().is_some(),
                        "running CPU work must retain its native input"
                    );
                    assert!(matches!(returns.try_recv(), Err(mpsc::TryRecvError::Empty)));
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
                if replace {
                    app.read(|editor| {
                        assert!(matches!(editor.document.tool, Some(Tool::Crop(_))));
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
            });
            sources.close_and_wait().unwrap();
        }
    }
}
