use super::*;
use crate::application::test_support::{TestApplication, with_application};
use std::sync::atomic::{AtomicBool, Ordering};

fn filmstrip_frame(
    app: &TestApplication<'_, Editor>,
    context: &egui::Context,
    budgets: memory::CacheBudgets,
) -> egui::FullOutput {
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900., 600.),
            )),
            ..Default::default()
        },
        |ui| {
            app.enter(|editor, cx| {
                editor.filmstrip_with_budgets(ui, budgets, cx);
            });
        },
    );
    // These component frames inspect callbacks without submitting egui texture updates.
    output.textures_delta.clear();
    output
}

#[test]
fn failed_map_preserves_source_thumbnail_paint_but_still_fails_precision_acquisition() {
    let mut editor = crate::editor::tests::authored_fixture(&["roll-0.dng"]);
    editor.set_dmin(None);
    editor.set_dmax(None);
    let id = editor.document.project.order[0];
    let source = editor.document.project.source(id).unwrap().clone();
    let map = Arc::new(
        crate::flat_field::FlatField::from_nodes(
            [1 << 24, 0, 1, 1],
            [2, 2],
            vec![glam::DVec3::ONE; 4],
        )
        .unwrap(),
    );
    editor
        .document
        .project
        .set_flat_field(Some(crate::project::FlatFieldSettings::new(
            "reference.dng",
            map,
        )))
        .unwrap();
    let correction = editor.correction();
    let failure = pollster::block_on(correction.clone().acquire())
        .err()
        .unwrap();
    assert!(failure.contains("coordinate"), "{failure}");
    let services = editor.source_services.as_ref().unwrap().clone();
    editor.document.thumbnails = Thumbnails::with_disk(
        services.device.clone(),
        services.queue.clone(),
        services.renderers.clone(),
        None,
    )
    .with_sources(editor.document.cache.clone());
    let budgets = memory::CacheBudgets::new(
        cache_limits::CacheLimits {
            cpu_bytes: 8 << 20,
            gpu_bytes: 8 << 20,
        },
        memory::Pressure::Normal,
    );
    editor.document.thumbnails.apply_budgets(budgets);
    let path = editor.document.source_path(id).unwrap();
    let key = editor.thumbnail_key(&source, &path, false).unwrap();
    assert!(key.recipe.as_ref().unwrap().is_none());
    let job = service_job(&editor.document.thumbnails, key, path, source, None, true);
    complete_service_request(&mut editor.document.thumbnails, &editor.document.state, job);
    let context = egui::Context::default();
    with_application(editor, |app| {
        let frame = filmstrip_frame(app, &context, budgets);
        assert!(
            frame
                .shapes
                .iter()
                .any(|shape| matches!(shape.shape, egui::Shape::Callback(_))),
            "failed correction must leave an actual source-inspection paint available"
        );
        let captured = app.read(|editor| {
            assert!(editor.document.project.active_flat_field().is_some());
            assert_eq!(editor.correction().error(), Some(failure.as_str()));
            editor.capture_source_context().unwrap()
        });
        assert_eq!(
            pollster::block_on(captured).err().unwrap(),
            failure,
            "precision acquisition retains the accepted map rather than the inspection fallback"
        );
    });
}

#[test]
fn camera_thumbnail_paint_requests_its_map_without_waiting_or_using_an_absent_correction() {
    let mut editor = crate::editor::tests::authored_fixture(&["roll-0.dng"]);
    editor.set_dmin(None);
    editor.set_dmax(None);
    let id = editor.document.project.order[0];
    let source = editor.document.project.source(id).unwrap().clone();
    let map = Arc::new(
        crate::flat_field::FlatField::from_nodes(
            source.development.active_area,
            [2, 2],
            vec![glam::DVec3::splat(1.2); 4],
        )
        .unwrap(),
    );
    editor
        .document
        .project
        .set_flat_field(Some(crate::project::FlatFieldSettings::new(
            "reference.dng",
            map,
        )))
        .unwrap();
    let services = editor.source_services.as_ref().unwrap().clone();
    editor.document.thumbnails = Thumbnails::with_disk(
        services.device.clone(),
        services.queue.clone(),
        services.renderers.clone(),
        None,
    )
    .with_sources(editor.document.cache.clone());
    let budgets = memory::CacheBudgets::new(
        cache_limits::CacheLimits {
            cpu_bytes: 8 << 20,
            gpu_bytes: 8 << 20,
        },
        memory::Pressure::Normal,
    );
    editor.document.thumbnails.apply_budgets(budgets);
    let (release, resume) = futures_channel::oneshot::channel();
    let started = Arc::new(AtomicBool::new(false));
    let signal = started.clone();
    let correction = editor.correction().after(async move {
        signal.store(true, Ordering::Release);
        resume.await.unwrap();
    });
    editor.document.correction = correction.clone().into();
    let path = editor.document.source_path(id).unwrap();
    let key = editor.thumbnail_key(&source, &path, false).unwrap();
    assert!(key.recipe.as_ref().unwrap().is_none());
    let job = service_job(
        &editor.document.thumbnails,
        key.clone(),
        path,
        source,
        None,
        true,
    );
    complete_service_request(&mut editor.document.thumbnails, &editor.document.state, job);
    assert!(
        editor
            .document
            .thumbnails
            .cache
            .get(&key)
            .unwrap()
            .as_ref()
            .unwrap()
            .positive
            .is_none()
    );
    assert!(!started.load(Ordering::Acquire));
    let context = egui::Context::default();
    with_application(editor, |app| {
        let pending = filmstrip_frame(app, &context, budgets);
        assert!(
            !pending
                .shapes
                .iter()
                .any(|shape| matches!(shape.shape, egui::Shape::Callback(_))),
            "camera pixels must not be interpreted without the requested map"
        );
        app.run_until(Duration::from_secs(30), |_| started.load(Ordering::Acquire));
        assert!(
            correction.pending(),
            "drawing only requests asynchronous preparation"
        );
        release.send(()).unwrap();
        app.run_until(Duration::from_secs(30), |_| !correction.pending());
        let ready = filmstrip_frame(app, &context, budgets);
        assert!(
            ready
                .shapes
                .iter()
                .any(|shape| matches!(shape.shape, egui::Shape::Callback(_))),
            "the prepared map permits a real photographic paint callback"
        );
    });
}

#[test]
fn positive_thumbnail_waits_for_the_captured_map_and_cancellation_keeps_another_reader() {
    let mut editor = crate::editor::tests::authored_fixture(&["roll-0.dng"]);
    editor.set_dmin(Some(glam::DVec3::splat(0.8)));
    editor.set_dmax(Some(glam::DVec3::splat(0.02)));
    let id = editor.document.project.order[0];
    let source = editor.document.project.source(id).unwrap().clone();
    let map = Arc::new(
        crate::flat_field::FlatField::from_nodes(
            source.development.active_area,
            [2, 2],
            vec![glam::DVec3::new(1.1, 0.9, 1.05); 4],
        )
        .unwrap(),
    );
    editor
        .document
        .project
        .set_flat_field(Some(crate::project::FlatFieldSettings::new(
            "reference.dng",
            map.clone(),
        )))
        .unwrap();
    let services = editor.source_services.as_ref().unwrap().clone();
    let thumbnails = Thumbnails::with_disk(
        services.device.clone(),
        services.queue.clone(),
        services.renderers.clone(),
        None,
    )
    .with_sources(editor.document.cache.clone());
    let (release, resume) = futures_channel::oneshot::channel();
    let started = Arc::new(AtomicBool::new(false));
    let observed = started.clone();
    let correction = editor.correction().after(async move {
        observed.store(true, Ordering::Release);
        resume.await.unwrap();
    });
    editor.document.correction = correction.clone().into();
    let path = editor.document.source_path(id).unwrap();
    let key = editor.thumbnail_key(&source, &path, false).unwrap();
    assert_eq!(key.field.as_ref().map(|field| &field.0), Some(&map));
    assert!(key.recipe.as_ref().unwrap().is_some());
    let worker = thumbnails.services.clone().unwrap();
    let sources = thumbnails.acquisition();
    let cancelled = crate::cancellation::Cancellation::default();
    with_application(Vec::<Result<Outcome, Stopped>>::new(), |app| {
        let mut tasks = Vec::new();
        for stop in [
            cancelled.clone(),
            crate::cancellation::Cancellation::default(),
        ] {
            let mut job = service_job(
                &thumbnails,
                key.clone(),
                path.clone(),
                source.clone(),
                None,
                true,
            );
            job.correction = correction.clone();
            job.cancelled = stop;
            let worker = worker.clone();
            let sources = sources.clone();
            tasks.push(
                app.access()
                    .spawn(move |cx| async move {
                        let outcome = worker.process(job, sources).await;
                        cx.update(move |results| results.push(outcome));
                    })
                    .unwrap(),
            );
        }
        app.run_until(Duration::from_secs(30), |_| started.load(Ordering::Acquire));
        app.read(|results| assert!(results.is_empty()));
        editor.document.project.set_flat_field(None).unwrap();
        assert!(editor.correction().map().is_none());
        cancelled.cancel();
        app.run_until(Duration::from_secs(30), |results| results.len() == 1);
        app.update(|results| assert!(results.pop().unwrap().unwrap().image.unwrap().is_none()));
        assert!(correction.pending());
        release.send(()).unwrap();
        app.run_until(Duration::from_secs(30), |results| results.len() == 1);
        let prepared = pollster::block_on(correction.acquire()).unwrap().unwrap();
        app.update(|results| {
            let outcome = results.pop().unwrap().unwrap();
            assert_eq!(outcome.key.field.as_ref().map(|field| &field.0), Some(&map));
            let product = outcome.image.unwrap().unwrap();
            let positive = product.positive.as_ref().expect("actual positive product");
            assert!(positive.has_correction(Some(&prepared.gpu)));
            assert!(!positive.has_correction(None));
        });
        drop(tasks);
    });
}
