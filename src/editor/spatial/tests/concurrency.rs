use super::*;
use std::{collections::BTreeSet, sync::Condvar};

#[derive(Default)]
struct Gate {
    released: Mutex<bool>,
    changed: Condvar,
}

impl Gate {
    fn wait(&self) {
        let released = self.released.lock().unwrap();
        let (released, timeout) = self
            .changed
            .wait_timeout_while(released, LIMIT, |released| !*released)
            .unwrap();
        drop(released);
        assert!(!timeout.timed_out(), "native test gate was not released");
    }

    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.changed.notify_all();
    }
}

struct HeldSources {
    started: mpsc::Receiver<std::path::PathBuf>,
    first: Arc<Gate>,
    other: Arc<Gate>,
}

impl Drop for HeldSources {
    fn drop(&mut self) {
        self.first.release();
        self.other.release();
    }
}

fn held_preparation(
    fail_first: bool,
) -> (
    Editor,
    source_loading::SourceLoader,
    HeldSources,
    tempfile::TempDir,
) {
    let files = tempfile::tempdir().unwrap();
    let fixture_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw");
    let paths: Vec<_> = (0..4)
        .map(|index| {
            let path = files.path().join(format!("source-{index}.dng"));
            std::fs::copy(fixture_root.join(format!("roll-{}.dng", index % 2)), &path).unwrap();
            path
        })
        .collect();
    let editor = authored_fixture(&[]);
    let mut editor = crate::editor::tests::complete_document_operation(editor, LIMIT, |e, cx| {
        e.add_raws(paths, cx)
    });
    assert!(
        editor.document.visit.canvas.ready.is_some(),
        "{}",
        editor.status
    );
    editor.document.cache.set_budget(0);
    let visit = editor.request_id();
    editor.replace_photo_scope(visit);
    let first = Arc::new(Gate::default());
    let other = Arc::new(Gate::default());
    let a = first.clone();
    let b = other.clone();
    let (entered, started) = mpsc::channel();
    let scheduler = source_loading::SourceLoader::new_observed(move |path, phase, _| {
        if matches!(phase, source_loading::NativePhase::UnpackBegin) {
            entered.send(path.to_owned()).unwrap();
            let is_first = path.file_name().unwrap() == "source-0.dng";
            if is_first {
                a.wait();
            } else {
                b.wait();
            }
            if is_first && fail_first {
                return Err("authored acquisition failure beside a running sibling".into());
            }
        }
        Ok(())
    });
    let previous = std::mem::replace(
        &mut editor.source_services.as_mut().unwrap().developments,
        scheduler.clone(),
    );
    previous.close_and_wait().unwrap();
    editor.sync_document();
    (
        editor,
        scheduler,
        HeldSources {
            started,
            first,
            other,
        },
        files,
    )
}

fn await_two_sources(app: &mut TestApplication<'_, Editor>, holds: &HeldSources) {
    let deadline = Instant::now() + LIMIT;
    let mut started = BTreeSet::new();
    while started.len() < 2 {
        app.pump();
        while let Ok(path) = holds.started.try_recv() {
            assert!(
                started.insert(path.file_name().unwrap().to_str().unwrap().to_owned()),
                "duplicate source development"
            );
        }
        assert!(
            Instant::now() < deadline,
            "two independent acquisitions must overlap before either is released"
        );
        std::thread::yield_now();
    }
    assert_eq!(
        started,
        BTreeSet::from(["source-0.dng".to_owned(), "source-1.dng".to_owned()])
    );
}

fn assert_no_regions(
    app: &mut TestApplication<'_, Editor>,
    waiter: impl Future<Output = Option<Arc<Metadata>>>,
) {
    let mut waiter = std::pin::pin!(waiter);
    let deadline = Instant::now() + LIMIT;
    loop {
        app.pump();
        if let Some(regions) =
            futures_lite::future::block_on(futures_lite::future::poll_once(&mut waiter))
        {
            assert!(regions.is_none());
            return;
        }
        assert!(Instant::now() < deadline, "region waiter was not retired");
        std::thread::yield_now();
    }
}

#[test]
fn cancelling_two_running_sources_withholds_regions_and_patches_without_waiting_for_cleanup() {
    let (editor, scheduler, holds, _files) = held_preparation(false);
    let accepted = editor.document.state.project.clone();
    with_application(editor, |app| {
        // Request the real document producer without requiring a selected-source lease.
        let waiter = app.enter(|e, cx| e.prepare_regions(cx).unwrap());
        await_two_sources(app, &holds);
        app.update(Editor::cancel_spatial);
        assert_no_regions(app, waiter);
        let (entered, done) = mpsc::channel();
        let unrelated = app
            .access()
            .spawn(move |_| async move {
                crate::background::run(move |_| entered.send(()).unwrap())
                    .await
                    .unwrap();
            })
            .unwrap();
        app.run_until(LIMIT, |_| done.try_recv().is_ok());
        holds.first.release();
        holds.other.release();
        scheduler.close_and_wait().unwrap();
        app.pump();
        app.read(|e| {
            assert_eq!(e.document.state.project, accepted);
            assert!(e.document.spatial.ready().groups.is_empty());
            assert_eq!(e.document.sampling_inputs.retained_bytes(), 0);
            assert!(e.document.spatial_preparing.is_none());
        });
        drop(unrelated);
    });
}

#[test]
fn acquisition_failure_retires_the_sibling_without_waiting_for_its_native_cleanup_or_publishing() {
    let (editor, scheduler, holds, _files) = held_preparation(true);
    let accepted = editor.document.state.project.clone();
    with_application(editor, |app| {
        let waiter = app.enter(|e, cx| e.prepare_regions(cx).unwrap());
        await_two_sources(app, &holds);
        holds.first.release();
        app.run_until(LIMIT, |e| e.document.spatial_preparing.is_none());
        assert_no_regions(app, waiter);
        app.read(|e| {
            assert!(
                e.status.contains("authored acquisition failure"),
                "{}",
                e.status
            );
            assert_eq!(e.document.state.project, accepted);
            assert!(e.document.spatial.ready().groups.is_empty());
            assert_eq!(e.document.sampling_inputs.retained_bytes(), 0);
        });
        holds.other.release();
        scheduler.close_and_wait().unwrap();
        app.pump();
    });
}
