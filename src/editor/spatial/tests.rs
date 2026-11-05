use super::*;
use crate::editor::crop_editor::tests::request_auto_crop;
use crate::{
    application::test_support::{TestApplication, with_application},
    editor::{analysis, history, source_loading, tests::authored_fixture},
};
use std::sync::Mutex;
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

mod concurrency;
mod consumers;

const LIMIT: Duration = Duration::from_secs(30);

// Supply numerical output, not an editor completion: callers still await the real preparation task.
fn programmed(editor: &mut Editor) -> oneshot::Sender<Result<Option<Metadata>, String>> {
    let (send, receive) = oneshot::channel();
    editor.document.spatial_reply = Some(receive);
    send
}

fn fixture(scheduler: source_loading::SourceLoader) -> Editor {
    let mut editor = authored_fixture(&["roll-0.dng", "roll-1.dng"]);
    let original = std::mem::replace(
        &mut editor.source_services.as_mut().unwrap().developments,
        scheduler,
    );
    editor.document.cache.set_budget(0);
    original.close_and_wait().unwrap();
    editor.sync_document();
    editor
}

type SourceStarts = Arc<Mutex<std::collections::BTreeMap<std::path::PathBuf, usize>>>;

fn observed_by_source() -> (source_loading::SourceLoader, Arc<AtomicUsize>, SourceStarts) {
    let count = Arc::new(AtomicUsize::new(0));
    let starts = count.clone();
    let sources = SourceStarts::default();
    let observed = sources.clone();
    let scheduler = source_loading::SourceLoader::new_observed(move |path, phase, _| {
        if matches!(phase, source_loading::NativePhase::UnpackBegin) {
            starts.fetch_add(1, Ordering::SeqCst);
            *observed.lock().unwrap().entry(path.to_owned()).or_default() += 1;
        }
        Ok(())
    });
    (scheduler, count, sources)
}

fn confirm_without_native(
    app: &mut TestApplication<'_, Editor>,
    scheduler: &source_loading::SourceLoader,
) {
    assert!(app.read(|e| e.document.sampling_inputs.retained_bytes() > 0));
    scheduler.close_and_wait().unwrap();
    app.enter(|editor, cx| editor.confirm_analysis_review(cx));
    app.run_until(LIMIT, |editor| {
        !editor.interaction_blocked() && editor.document.analysis_operation().is_none()
    });
    app.read(|editor| {
        assert!(editor.status.starts_with("Analyzed 2"), "{}", editor.status);
        assert_eq!(editor.document.project.analysis_regions.len(), 2);
    });
}

fn observed() -> (source_loading::SourceLoader, Arc<AtomicUsize>) {
    let (scheduler, count, _) = observed_by_source();
    (scheduler, count)
}

fn assert_once_per_source(counts: &SourceStarts, document: &Document) {
    let counts = counts.lock().unwrap();
    assert!(
        !counts.is_empty(),
        "positive control must exercise native acquisition"
    );
    for (path, count) in counts.iter() {
        assert!(
            document
                .project
                .order
                .iter()
                .any(|id| document.source_path(*id).is_ok_and(|p| p == *path)),
            "unexpected source {}",
            path.display()
        );
        assert!(
            *count <= 1,
            "source developed more than once during area detection: {} ({count})",
            path.display()
        );
    }
}

#[test]
fn authored_auto_crop_first_reuses_regions_for_analyze_without_accepting_either_consumers_edits() {
    let (scheduler, starts, by_source) = observed_by_source();
    let editor = fixture(scheduler.clone());
    let accepted = editor.document.state.project.clone();
    let dirty = editor.document.dirty();
    let undo = editor.history_label(history::Direction::Undo);
    with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.review_crop_proposals(editor.document.project.order.clone(), "Whole roll", cx)
        });
        assert!(app.read(|editor| editor.document.spatial_preparing.is_some()));
        app.run_until(LIMIT, |editor| editor.document.spatial_preparing.is_none());
        let regions = app.read(|editor| {
            assert!(!editor.interaction_blocked(), "{}", editor.status);
            assert!(!editor.analysis_preparing());
            assert!(!editor.analysis_fitting());
            assert!(editor.document.analysis_review().is_none());
            assert_eq!(editor.document.state.project, accepted);
            assert_eq!(editor.document.dirty(), dirty);
            assert_eq!(editor.history_label(history::Direction::Undo), undo);
            let regions = editor.document.spatial.ready();
            assert_eq!(
                regions
                    .groups
                    .iter()
                    .map(|g| g.sources.len())
                    .sum::<usize>(),
                2
            );
            assert!(editor.document.auto_crop.reviewing(), "{}", editor.status);
            assert!(
                regions
                    .groups
                    .iter()
                    .flat_map(|g| &g.sources)
                    .all(|s| s.sampling_areas().ready()),
                "Auto-crop must finish sampling detection too, not defer it until Analyze"
            );
            regions
        });
        app.read(|editor| assert_once_per_source(&by_source, &editor.document.capture()));
        let acquired = starts.load(Ordering::SeqCst);
        assert!(
            acquired > 0,
            "the first request must exercise native acquisition, not just a supplied metadata fixture"
        );
        app.enter(|editor, cx| {
            editor.finish_auto_crop(false);
            editor.auto_crop_margin = 1.;
            editor.review_crop_proposals(editor.document.project.order.clone(), "Whole roll", cx);
        });
        app.run_until(LIMIT, |editor| editor.document.spatial_preparing.is_none());
        app.read(|editor| {
            assert_eq!(*editor.document.spatial.ready(), *regions);
            assert_eq!(editor.document.state.project, accepted);
            assert!(editor.document.auto_crop.reviewing(), "{}", editor.status);
        });
        assert_eq!(
            starts.load(Ordering::SeqCst),
            acquired,
            "changing a consumer margin must not reacquire RAWs or redetect"
        );
        app.enter(|editor, cx| {
            editor.finish_auto_crop(false);
            editor.request_analysis(cx);
        });
        app.run_until(LIMIT, |editor| {
            editor.document.spatial_preparing.is_none() && !editor.analysis_preparing()
        });
        assert_eq!(
            starts.load(Ordering::SeqCst),
            acquired,
            "Analyze must not acquire pixels to detect areas after Auto-crop"
        );
        app.read(|editor| {
            assert!(
                editor.document.analysis_review().is_some(),
                "{}",
                editor.status
            );
            assert_eq!(*editor.document.spatial.ready(), *regions);
            assert_eq!(editor.document.state.project, accepted);
            assert_eq!(editor.history_label(history::Direction::Undo), undo);
        });
        confirm_without_native(app, &scheduler);
        assert_eq!(starts.load(Ordering::SeqCst), acquired);
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn authored_analyze_first_keeps_regions_after_review_cancel_and_auto_crop_reuses_them() {
    let (scheduler, starts, by_source) = observed_by_source();
    let editor = fixture(scheduler.clone());
    let accepted = editor.document.state.project.clone();
    let undo = editor.history_label(history::Direction::Undo);
    with_application(editor, |app| {
        app.enter(|editor, cx| editor.request_analysis(cx));
        app.run_until(LIMIT, |editor| {
            editor.document.spatial_preparing.is_none() && !editor.analysis_preparing()
        });
        let regions = app.read(|editor| {
            assert!(
                editor.document.analysis_review().is_some(),
                "{}",
                editor.status
            );
            assert_eq!(editor.document.state.project, accepted);
            assert_eq!(editor.history_label(history::Direction::Undo), undo);
            assert_eq!(
                editor
                    .document
                    .spatial
                    .ready()
                    .groups
                    .iter()
                    .map(|g| g.sources.len())
                    .sum::<usize>(),
                2
            );
            editor.document.spatial.ready()
        });
        app.read(|editor| assert_once_per_source(&by_source, &editor.document.capture()));
        let acquired = starts.load(Ordering::SeqCst);
        assert!(
            regions
                .groups
                .iter()
                .flat_map(|g| &g.sources)
                .all(|s| s.sampling_areas().ready()),
            "finished shared results must include usable sampling suggestions for the authored positive control"
        );
        app.enter(|editor, cx| editor.finish_pick(cx));
        app.run_until(LIMIT, |editor| !editor.interaction_blocked());
        assert!(app.read(|editor| editor.document.analysis_review().is_none()));
        app.enter(|editor, cx| {
            editor.review_crop_proposals(editor.document.project.order.clone(), "Whole roll", cx)
        });
        app.run_until(LIMIT, |editor| editor.document.spatial_preparing.is_none());
        app.read(|editor| {
            assert_eq!(*editor.document.spatial.ready(), *regions);
            assert_eq!(editor.document.state.project, accepted);
            assert!(editor.document.auto_crop.reviewing(), "{}", editor.status);
            assert_eq!(editor.history_label(history::Direction::Undo), undo);
        });
        assert_eq!(starts.load(Ordering::SeqCst), acquired);
        app.enter(|editor, cx| {
            editor.finish_auto_crop(false);
            editor.request_analysis(cx);
        });
        app.run_until(LIMIT, |editor| editor.document.analysis_review().is_some());
        confirm_without_native(app, &scheduler);
        assert_eq!(starts.load(Ordering::SeqCst), acquired);
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn restored_regions_collect_missing_sampling_pixels_without_replacing_finished_detection() {
    let (scheduler, starts) = observed();
    let editor = fixture(scheduler.clone());
    with_application(editor, |app| {
        app.enter(|e, cx| {
            e.review_crop_proposals(e.document.project.order.clone(), "Whole roll", cx)
        });
        app.run_until(LIMIT, |e| e.document.spatial_preparing.is_none());
        let retained = app.read(|e| {
            assert!(e.document.sampling_inputs.retained_bytes() > 0);
            let mut metadata = (*e.document.spatial.ready()).clone();
            assert!(metadata.groups[0].sources[0].sampling_areas().ready());
            metadata.groups[0].sources[0].sampling =
                crate::spatial_metadata::SamplingAreas::new(&Default::default());
            metadata.validate().unwrap();
            metadata
        });
        app.update(|e| {
            e.finish_auto_crop(false);
            e.document.sampling_inputs = Default::default();
            e.document.spatial = crate::spatial_metadata::Store::restored(retained.clone());
        });
        let before = starts.load(Ordering::SeqCst);
        app.enter(|e, cx| e.request_analysis(cx));
        app.run_until(LIMIT, |e| e.document.spatial_preparing.is_none());
        app.read(|e| {
            assert_eq!(
                *e.document.spatial.ready(),
                retained,
                "collecting pixels must not rerun or replace finished detection"
            );
            assert!(e.document.sampling_inputs.retained_bytes() > 0);
        });
        assert!(
            starts.load(Ordering::SeqCst) > before,
            "missing patches must actually be recollected with zero optional RAW retention"
        );
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn queued_shared_preparation_cancels_without_native_work_or_accepted_changes() {
    let (scheduler, starts) = observed();
    let editor = fixture(scheduler.clone());
    let accepted = editor.document.state.project.clone();
    let admission = crate::background::testing::pause();
    with_application(editor, |app| {
        app.enter(|editor, cx| editor.request_analysis(cx));
        app.pump();
        app.update(Editor::cancel_analysis);
        app.pump();
        app.read(|editor| {
            assert!(editor.document.spatial_preparing.is_none());
            assert!(!editor.analysis_preparing());
            assert!(editor.document.analysis_review().is_none());
            assert!(!editor.interaction_blocked());
            assert_eq!(editor.document.state.project, accepted);
            assert!(editor.activities.view().is_none());
        });
        assert_eq!(starts.load(Ordering::SeqCst), 0);
        drop(admission);
    });
    scheduler.close_and_wait().unwrap();
}

fn paused() -> (
    source_loading::SourceLoader,
    mpsc::Receiver<()>,
    mpsc::Sender<()>,
) {
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let resume = Mutex::new(resume);
    let once = AtomicUsize::new(0);
    let scheduler = source_loading::SourceLoader::new_observed(move |_, phase, _| {
        if matches!(phase, source_loading::NativePhase::UnpackBegin)
            && once.fetch_add(1, Ordering::SeqCst) == 0
        {
            entered.send(()).unwrap();
            resume.lock().unwrap().recv_timeout(LIMIT).unwrap();
        }
        Ok(())
    });
    (scheduler, starts, release)
}

fn await_native(app: &mut TestApplication<'_, Editor>, starts: &mpsc::Receiver<()>) {
    let deadline = Instant::now() + LIMIT;
    loop {
        app.pump();
        if starts.try_recv().is_ok() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{}",
            app.read(|editor| editor.status.clone())
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn an_arbitrary_awaiter_can_use_regions_or_depart_without_retiring_document_preparation() {
    for depart in [false, true] {
        let (scheduler, starts, release) = paused();
        let editor = fixture(scheduler.clone());
        let accepted = editor.document.state.project.clone();
        with_application(editor, |app| {
            let regions = app.enter(|editor, cx| editor.prepare_regions(cx).unwrap());
            let received = std::rc::Rc::new(std::cell::RefCell::new(None));
            let output = received.clone();
            let mut caller = Some(
                app.access()
                    .spawn(move |_| async move {
                        let metadata = regions.await.unwrap();
                        let ids: Vec<_> = metadata
                            .groups
                            .iter()
                            .flat_map(|group| group.sources.iter().map(|row| row.input.source))
                            .collect();
                        *output.borrow_mut() = Some(ids);
                    })
                    .unwrap(),
            );
            await_native(app, &starts);
            if depart {
                caller = None;
                app.pump();
                assert!(app.read(|editor| editor.document.spatial_preparing.is_some()));
            }
            release.send(()).unwrap();
            app.run_until(LIMIT, |editor| {
                editor.document.spatial_preparing.is_none()
                    && (depart || received.borrow().is_some())
            });
            app.read(|editor| {
                let mut expected = editor.document.project.order.clone();
                expected.sort_by_key(|id| id.0);
                let mut ready: Vec<_> = editor
                    .document
                    .spatial
                    .ready()
                    .groups
                    .iter()
                    .flat_map(|group| group.sources.iter().map(|row| row.input.source))
                    .collect();
                ready.sort_by_key(|id| id.0);
                assert_eq!(ready, expected);
                if !depart {
                    let mut observed = received
                        .borrow_mut()
                        .take()
                        .expect("the awaited result never arrived");
                    observed.sort_by_key(|id| id.0);
                    assert_eq!(observed, expected);
                } else {
                    assert!(received.borrow().is_none());
                }
                assert_eq!(editor.document.state.project, accepted);
                assert!(!editor.document.auto_crop.reviewing());
                assert!(editor.document.analysis_review().is_none());
            });
            drop(caller);
        });
        scheduler.close_and_wait().unwrap();
    }
}

#[test]
fn retiring_a_photo_visit_does_not_retire_document_region_preparation() {
    let (scheduler, starts, release) = paused();
    let editor = fixture(scheduler.clone());
    with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.review_crop_proposals(editor.document.project.order.clone(), "Whole roll", cx)
        });
        await_native(app, &starts);
        let request = app.read(|editor| {
            editor
                .document
                .spatial_preparing
                .as_ref()
                .unwrap()
                .activity
                .clone()
        });
        app.update(|editor| {
            let visit = editor.request_id();
            editor.replace_photo_scope(visit);
            assert_eq!(
                editor.document.spatial_preparing.as_ref().unwrap().activity,
                request
            );
        });
        release.send(()).unwrap();
        app.run_until(LIMIT, |editor| editor.document.spatial_preparing.is_none());
        app.read(|editor| {
            assert_eq!(
                editor
                    .document
                    .spatial
                    .ready()
                    .groups
                    .iter()
                    .map(|g| g.sources.len())
                    .sum::<usize>(),
                2
            );
            assert!(editor.document.analysis_review().is_none());
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn document_retirement_cancels_publication_without_blocking_unrelated_cpu_work() {
    let (scheduler, starts, release) = paused();
    let editor = fixture(scheduler.clone());
    with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.review_crop_proposals(editor.document.project.order.clone(), "Whole roll", cx)
        });
        await_native(app, &starts);
        let (cancelled, snapshot) = app.read(|editor| {
            (
                editor
                    .document
                    .spatial_preparing
                    .as_ref()
                    .unwrap()
                    .cancelled
                    .clone(),
                Arc::downgrade(&editor.document.cache),
            )
        });
        app.update(|editor| editor.document = Document::default().into());
        app.pump();
        assert!(cancelled.is_cancelled());
        assert!(snapshot.upgrade().is_none());
        let (entered, observed) = mpsc::channel();
        let probe = app
            .access()
            .spawn(move |_| async move {
                crate::background::run(move |_| entered.send(()).unwrap())
                    .await
                    .unwrap();
            })
            .unwrap();
        app.run_until(LIMIT, |_| observed.try_recv().is_ok());
        release.send(()).unwrap();
        drop(probe);
        app.read(|editor| {
            assert!(editor.document.spatial.ready().groups.is_empty());
            assert!(!editor.document.auto_crop.reviewing());
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn application_retirement_cancels_shared_continuation_before_native_cleanup_returns() {
    let (scheduler, starts, release) = paused();
    let editor = fixture(scheduler.clone());
    with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.review_crop_proposals(editor.document.project.order.clone(), "Whole roll", cx)
        });
        await_native(app, &starts);
        let cancelled = app.read(|editor| {
            editor
                .document
                .spatial_preparing
                .as_ref()
                .unwrap()
                .cancelled
                .clone()
        });
        app.close_execution();
        assert!(cancelled.is_cancelled());
        app.read(|editor| {
            assert!(editor.document.spatial.ready().groups.is_empty());
            assert!(!editor.document.auto_crop.reviewing());
        });
        release.send(()).unwrap();
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn shared_region_waiters_join_one_producer_and_modal_prevents_tool_replacement() {
    let (scheduler, _) = observed();
    let mut editor = fixture(scheduler.clone());
    let metadata = consumers::declared(&editor);
    let reply = programmed(&mut editor);
    let before = editor.document.project.clone();
    let before_history = editor.history_label(history::Direction::Undo);
    with_application(editor, |app| {
        let preparation = app.enter(|editor, cx| {
            editor.request_analysis(cx);
            editor
                .document
                .spatial_preparing
                .as_ref()
                .unwrap()
                .activity
                .clone()
        });
        app.pump();
        let completed = std::rc::Rc::new(std::cell::Cell::new(false));
        let received = completed.clone();
        let _waiter = app.enter(|editor, cx| {
            editor.review_crop_proposals(editor.document.project.order.clone(), "Whole roll", cx);
            assert!(editor.document.analysis_operation().is_some());
            assert!(!editor.document.auto_crop.active());
            assert_eq!(
                editor.document.spatial_preparing.as_ref().unwrap().activity,
                preparation
            );
            let regions = editor.prepare_regions(cx).unwrap();
            cx.spawn(move |_| async move {
                received.set(regions.await.is_some());
            })
            .unwrap()
        });
        reply.send(Ok(Some(metadata))).unwrap();
        app.run_until(LIMIT, |editor| {
            editor.document.analysis_review().is_some() && completed.get()
        });
        app.enter(|editor, cx| {
            editor.review_crop_proposals(editor.document.project.order.clone(), "Whole roll", cx)
        });
        app.run_until(LIMIT, |editor| editor.document.auto_crop.reviewing());
        app.read(|editor| {
            assert!(
                editor
                    .document
                    .project
                    .order
                    .iter()
                    .all(|photo| editor.auto_crop_preview(*photo).is_some())
            );
            assert!(editor.document.auto_crop.reviewing());
            assert!(editor.document.analysis_review().is_none());
            assert_eq!(editor.document.project, before);
            assert_eq!(
                editor.history_label(history::Direction::Undo),
                before_history
            );
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn retiring_a_photo_discards_its_crop_consumer_but_keeps_shared_regions_for_the_document() {
    let (scheduler, _) = observed();
    let mut editor = fixture(scheduler.clone());
    let metadata = consumers::declared(&editor);
    let reply = programmed(&mut editor);
    with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.begin_crop();
            request_auto_crop(editor.document.visit.current_source.unwrap(), editor, cx);
        });
        app.pump();
        app.update(|editor| {
            let ready = editor.document.visit.canvas.ready.clone().unwrap();
            let visit = editor.request_id();
            editor.replace_photo_scope(visit);
            assert!(editor.document.spatial_preparing.is_some());
            editor.document.visit.canvas.ready = Some(ready);
            editor.begin_crop();
            assert!(
                !editor.cropping(),
                "the region producer still owns its modal"
            );
        });
        reply.send(Ok(Some(metadata))).unwrap();
        app.run_until(LIMIT, |editor| editor.document.spatial_preparing.is_none());
        let replacement_draft = app.update(|editor| {
            editor.begin_crop();
            let draft = &mut editor.document.crop_edit_mut().unwrap().draft;
            draft.size = [17, 19];
            *draft
        });

        app.update(|editor| {
            assert!(!editor.document.spatial.ready().groups.is_empty());
            assert_eq!(
                editor.document.crop_edit().unwrap().draft,
                replacement_draft
            );
        });
        app.enter(|editor, cx| {
            request_auto_crop(editor.document.visit.current_source.unwrap(), editor, cx)
        });
        app.run_until(LIMIT, |editor| !editor.document.crop_auto_crop_pending());
        app.read(|editor| {
            assert_ne!(
                editor.document.crop_edit().unwrap().draft,
                replacement_draft,
                "the retained regions must provide a usable proposal"
            );
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn failed_or_rejected_photo_region_preparation_can_retry_in_the_same_crop_visit() {
    for rejected_publication in [false, true] {
        let (scheduler, _) = observed();
        let mut editor = fixture(scheduler.clone());
        let metadata = consumers::declared(&editor);
        let first = programmed(&mut editor);
        with_application(editor, |app| {
            app.enter(|editor, cx| {
                editor.begin_crop();
                request_auto_crop(editor.document.visit.current_source.unwrap(), editor, cx);
            });
            let before = app.read(|editor| editor.document.crop_edit().unwrap().draft);
            if rejected_publication {
                app.update(|editor| editor.document.spatial.cancel());
                first.send(Ok(Some(metadata.clone()))).unwrap();
            } else {
                first
                    .send(Err("authored region preparation failure".into()))
                    .unwrap();
            }
            app.run_until(LIMIT, |editor| {
                editor.document.spatial_preparing.is_none()
                    && !editor.document.crop_auto_crop_pending()
            });
            let retry = app.enter(|editor, cx| {
                assert_eq!(editor.document.crop_edit().unwrap().draft, before);
                let retry = programmed(editor);
                request_auto_crop(editor.document.visit.current_source.unwrap(), editor, cx);
                assert!(
                    editor.document.spatial_preparing.is_some(),
                    "failure stranded retry admission"
                );
                retry
            });
            retry.send(Ok(Some(metadata.clone()))).unwrap();
            app.run_until(LIMIT, |editor| !editor.document.crop_auto_crop_pending());
            app.read(|editor| assert_ne!(editor.document.crop_edit().unwrap().draft, before));
        });
        scheduler.close_and_wait().unwrap();
    }
}
