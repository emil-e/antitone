use super::*;
use crate::application::test_support::with_application;
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

#[test]
fn fitting_waits_for_its_captured_map_and_cancellation_preserves_another_reader() {
    let (mut request, _directory, device, queue) = request();
    approved(&mut request);
    let source = request.document.project.source(request.selected).unwrap();
    let settings = crate::project::FlatFieldSettings::new(
        "reference.dng",
        Arc::new(
            crate::flat_field::FlatField::from_nodes(
                source.development.active_area,
                [2, 2],
                vec![glam::DVec3::new(1.1, 0.9, 1.05); 4],
            )
            .unwrap(),
        ),
    );
    request
        .document
        .project
        .set_flat_field(Some(settings.clone()));
    let (release, resume) = futures_channel::oneshot::channel();
    let started = Arc::new(AtomicBool::new(false));
    let signal = started.clone();
    let correction =
        crate::editor::correction::Resources::prepare(Some(settings), Some(device.clone())).after(
            async move {
                signal.store(true, Ordering::Release);
                resume.await.unwrap();
            },
        );
    let (scheduler, loads) = observed();
    let renderer = crate::editor::display::DisplayFixture::for_device(&device)
        .get(&device, &queue)
        .unwrap();
    let cancelled = Cancellation::default();
    with_application(Vec::<Result<Option<Fit>, Error>>::new(), |app| {
        let mut tasks = Vec::new();
        for stop in [cancelled.clone(), Cancellation::default()] {
            let evaluation = renderer.evaluation().clone();
            let queue_and_gate = queue.clone();
            let request = request.clone();
            let resources = super::super::Resources {
                correction: correction.clone(),
                sources: scheduler.provider(None),
                sampling_inputs: Default::default(),
                initialization: Arc::default(),
            };
            tasks.push(
                app.access()
                    .spawn(move |cx| async move {
                        let result =
                            Fit::acquire(evaluation, queue_and_gate, request, resources, &stop)
                                .await;
                        cx.update(move |results| results.push(result));
                    })
                    .unwrap(),
            );
        }
        app.run_until(Duration::from_secs(30), |_| started.load(Ordering::Acquire));
        app.read(|results| assert!(results.is_empty()));
        assert_eq!(
            loads.load(Ordering::Relaxed),
            0,
            "fitting has not acquired sources before its held map"
        );
        cancelled.cancel();
        app.run_until(Duration::from_secs(30), |results| results.len() == 1);
        app.read(|results| assert!(results[0].as_ref().unwrap().is_none()));
        assert!(correction.pending());
        release.send(()).unwrap();
        app.run_until(Duration::from_secs(30), |results| results.len() == 2);
        let prepared = pollster::block_on(correction.acquire()).unwrap().unwrap();
        app.update(|results| {
            let mut fit = results.pop().unwrap().unwrap().unwrap();
            assert!(Arc::ptr_eq(
                &fit.correction.as_ref().unwrap().gpu,
                &prepared.gpu
            ));
            fit.geometry().unwrap();
            assert_eq!(fit.cameras.len(), 2);
        });
        drop(tasks);
    });
    scheduler.close_and_wait().unwrap();
}
