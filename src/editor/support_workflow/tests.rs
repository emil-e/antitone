use super::*;
use crate::{
    application::test_support::{TestApplication, with_application},
    editor::{Pick, Preparation, Thumbnails},
};
use std::{path::PathBuf, time::Duration};

fn editor() -> Editor {
    let (device, queue) = pollster::block_on(async {
        let instance = crate::gpu::instance();
        instance
            .request_adapter(&Default::default())
            .await
            .unwrap()
            .request_device(&Default::default())
            .await
            .unwrap()
    });
    let mut e = Editor::new(&device, &queue);
    e.document.thumbnails = (Thumbnails::default()).with_sources(e.document.cache.clone());
    e
}

fn imported(app: &mut TestApplication<'_, Editor>) {
    app.enter(|e, access| {
        e.add_raws(
            vec![
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/raw/illuminated-patches.dng"),
            ],
            access,
        );
    });
    app.run_until(Duration::from_secs(20), |e| !e.busy);
    app.read(|e| assert!(e.document.visit.canvas.ready.is_some(), "{}", e.status));
}

#[test]
fn lazy_support_progresses_without_paint_and_does_not_accept_a_picture_or_change_processing() {
    let e = editor();
    let scheduler = e.developments.as_ref().unwrap().clone();
    with_application(e, |app| {
        imported(app);
        let (before, expected) = app.update(|e| {
            assert!(
                e.document
                    .visit
                    .canvas
                    .ready
                    .as_ref()
                    .unwrap()
                    .analysis_support
                    .is_none()
            );
            (
                e.history_snapshot(),
                e.source_context()
                    .unwrap()
                    .scan()
                    .unwrap()
                    .propose_flat_field_area()
                    .unwrap(),
            )
        });
        app.enter(|e, access| {
            e.begin_pick(Pick::Analysis, access);
        });
        app.run_until(Duration::from_secs(20), |e| {
            e.document.visit.support.pending.is_none()
        });
        app.read(|e| {
            assert_eq!(
                e.document
                    .visit
                    .canvas
                    .ready
                    .as_ref()
                    .unwrap()
                    .analysis_support,
                Some(expected)
            );
            assert_eq!(e.document.state.project, before);
            assert!(!e.busy);
        });
        app.enter(|e, access| {
            e.finish_pick(access);
            e.begin_pick(Pick::Analysis, access);
        });
        app.read(|e| {
            assert!(
                e.document.visit.support.pending.is_none(),
                "known support, including absence, is cached on readiness"
            )
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn queued_support_respects_drawn_geometry_and_reopened_picker_identity() {
    for reopen in [false, true] {
        let e = editor();
        let scheduler = e.developments.as_ref().unwrap().clone();
        with_application(e, |app| {
            imported(app);
            let hold = crate::background::testing::pause();
            let old = app.enter(|e, access| {
                e.begin_pick(Pick::Analysis, access);

                e.document.visit.support.pending.as_ref().unwrap().id
            });
            app.pump();
            let drawn = crate::coverage::Coverage::rectangle(Rect {
                x: 30,
                y: 30,
                width: 40,
                height: 40,
            });
            app.enter(|e, access| {
                if reopen {
                    e.finish_pick(access);
                    e.begin_pick(Pick::Analysis, access);
                }
                e.set_pick_region(Some(drawn));
            });
            if reopen {
                app.read(|e| {
                    assert_ne!(e.document.visit.support.pending.as_ref().unwrap().id, old)
                });
            }
            drop(hold);
            app.run_until(Duration::from_secs(20), |e| {
                e.document.visit.support.pending.is_none()
            });
            app.update(|e| e.analysis_support_result(old, e.id, Err("stale".into())));
            app.read(|e| {
                assert_eq!(e.pick_region(), Some(drawn));
                assert!(
                    e.document
                        .visit
                        .canvas
                        .ready
                        .as_ref()
                        .unwrap()
                        .analysis_support
                        .is_some()
                );
                assert_ne!(e.status, "stale");
            });
        });
        scheduler.close_and_wait().unwrap();
    }
}

#[test]
fn document_replacement_retires_support_without_waiting_for_a_paint_or_accepting_stale_geometry() {
    let e = editor();
    let scheduler = e.developments.as_ref().unwrap().clone();
    with_application(e, |app| {
        imported(app);
        let hold = crate::background::testing::pause();
        let (old, loaded) = app.enter(|e, access| {
            e.begin_pick(Pick::Analysis, access);
            let pending = e.document.visit.support.pending.as_ref().unwrap();

            (pending.id, pending.loaded)
        });
        app.pump();
        app.enter(|e, access| {
            e.finish_pick(access);
            e.start_document(Preparation::New, access);
        });
        app.update(|e| {
            e.analysis_support_result(
                old,
                loaded,
                Ok(Some(Rect {
                    x: 30,
                    y: 30,
                    width: 40,
                    height: 40,
                })),
            )
        });
        app.read(|e| {
            assert!(e.document.visit.support.pending.is_none());
            assert!(e.pick_region().is_none());
        });
        drop(hold);
        app.run_until(Duration::from_secs(20), |e| !e.busy);
        app.read(|e| {
            assert!(
                e.document.state.project.order.is_empty()
                    && e.document.visit.canvas.ready.is_none()
            )
        });
    });
    scheduler.close_and_wait().unwrap();
}
