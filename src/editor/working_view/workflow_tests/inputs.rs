use super::*;
use crate::editor::correction::Resources;
use futures_channel::oneshot;

fn held_correction(
    editor: &mut Editor,
    input: &mut source::Context,
) -> (Resources, oneshot::Receiver<()>, oneshot::Sender<()>) {
    editor
        .document
        .register(crate::project::Source {
            id: SourceId(0),
            path: input.source.path().to_path_buf(),
            development: crate::project::SourceDevelopment::from_metadata(
                &input.source.metadata(),
                input.source.white_balance(),
            )
            .unwrap(),
            development_mode: crate::project::DevelopmentMode::Unity,
            repairs: Default::default(),
            adjustment_layers: None,
            orientation: Default::default(),
            crop: None,
        })
        .unwrap();
    let area = input.source.area();
    let map = Arc::new(
        crate::flat_field::FlatField::from_nodes(
            [area.x, area.y, area.width, area.height],
            [2, 2],
            vec![glam::DVec3::new(1.4, 0.7, 1.2); 4],
        )
        .unwrap(),
    );
    editor
        .document
        .project
        .set_flat_field(Some(crate::project::FlatFieldSettings::new(
            "reference.dng",
            map,
        )));
    let (started, observed) = oneshot::channel();
    let (release, resume) = oneshot::channel();
    let resources = editor.correction().after(async move {
        started.send(()).unwrap();
        resume
            .await
            .expect("release captured correction preparation");
    });
    editor.document.correction = resources.clone().into();
    input.correction = resources.clone();
    (resources, observed, release)
}

#[test]
fn working_request_awaits_its_captured_correction_without_an_inspector_observer() {
    let (mut editor, mut input, spec, scheduler) = fixture();
    let (resources, started, release) = held_correction(&mut editor, &mut input);
    let services = editor.source_services.clone().unwrap();
    with_application(editor, |app| {
        app.enter(|editor, cx| request_working(editor, spec.clone(), input.clone(), cx));
        let waiting = app
            .access()
            .spawn(move |_| async move { started.await.unwrap() })
            .unwrap();
        app.run_until(Duration::from_secs(15), |_| waiting.is_finished());
        app.read(|editor| {
            let working = &editor.document.visit.working_view;
            assert!(resources.pending());
            assert!(!working.support_ready() && working.frame.is_none());
            assert!(working.error.is_none());
            assert!(working.wanted.as_ref().unwrap().rendering.get().is_none());
        });
        app.update(|editor| editor.document.project.set_flat_field(None));
        release.send(()).unwrap();
        app.run_until(Duration::from_secs(15), |editor| {
            editor.document.visit.working_view.support_ready()
        });
        let correction = pollster::block_on(resources.acquire()).unwrap().unwrap();
        let frame = app.read(|editor| {
            let working = &editor.document.visit.working_view;
            assert!(working.error.is_none());
            let frame = working.frame.as_ref().unwrap();
            assert!(Arc::ptr_eq(
                &frame.request.pixels.correction.gpu().unwrap().unwrap(),
                &correction.gpu,
            ));
            let camera = frame
                .source
                .definition
                .ancestors()
                .find_map(|node| {
                    (node.operation().as_ref() as &dyn std::any::Any)
                        .downcast_ref::<crate::gpu::photograph::operations::Camera>()
                })
                .expect("completed graph must contain corrected camera light");
            assert!(Arc::ptr_eq(
                camera.captured.field.as_ref().unwrap(),
                &correction.gpu
            ));
            frame.clone()
        });
        complete_frame(&frame, &services);
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn replacing_a_working_request_does_not_wait_for_the_retired_corrections_preparation() {
    let (mut editor, mut input, spec, scheduler) = fixture();
    let (resources, started, release) = held_correction(&mut editor, &mut input);
    with_application(editor, |app| {
        app.enter(|editor, cx| request_working(editor, spec.clone(), input.clone(), cx));
        let waiting = app
            .access()
            .spawn(move |_| async move { started.await.unwrap() })
            .unwrap();
        app.run_until(Duration::from_secs(15), |_| waiting.is_finished());
        input.correction = Resources::default();
        app.enter(|editor, cx| {
            editor.document.project.set_flat_field(None);
            request_working(editor, spec.clone(), input.clone(), cx);
        });
        app.run_until(Duration::from_secs(15), |editor| {
            editor.document.visit.working_view.support_ready()
        });
        let completed = app.read(|editor| {
            let working = &editor.document.visit.working_view;
            assert!(
                resources.pending(),
                "replacement must finish before the old map is released"
            );
            assert!(working.error.is_none());
            let frame = working.frame.as_ref().unwrap();
            assert!(crate::gpu::photograph::PhotographGraph::has_correction(
                &frame.source.definition,
                None
            ));
            frame.clone()
        });
        release.send(()).unwrap();
        let retired = app
            .access()
            .spawn(move |_| async move { resources.acquire().await.unwrap() })
            .unwrap();
        app.run_until(Duration::from_secs(15), |_| retired.is_finished());
        app.pump();
        app.read(|editor| {
            let working = &editor.document.visit.working_view;
            assert!(working.error.is_none());
            assert!(Arc::ptr_eq(working.frame.as_ref().unwrap(), &completed));
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn gamma_edits_reuse_budgeted_source_values_and_pressure_preserves_completed_frames() {
    use crate::gpu::evaluation::{ImageLayout, camera_source};
    let (editor, input, frame_spec, scheduler) = fixture();
    let scan = input.scan().unwrap();
    let calibration = (crate::processing::Calibration {
        dmin: glam::DVec3::splat(0.8),
        dmax: glam::DVec3::splat(0.2),
    })
    .validate()
    .unwrap();
    let spec = |value| {
        let mut settings = crate::processing::InputSettings::from(calibration);
        if let crate::processing::InputSettings::Negative { gamma, .. } = &mut settings {
            *gamma = value;
        }
        SourceRequest::new(frame_spec.frame(), settings, scan.camera_to_ap1, None).unwrap()
    };
    let services = editor.source_services.as_ref().unwrap().clone();
    services.renderers.set_retention_budget(8 << 20);
    let renderer = services
        .renderers
        .get(&services.device, &services.queue)
        .unwrap();
    let common = renderer.evaluation().materializations();
    common.set_test_pressure(memory::Pressure::Normal);
    let original = camera_source(&scan).unwrap();
    let key = original.identity().unwrap().clone();
    with_application(editor, |app| {
        app.enter(|editor, cx| request_working(editor, spec(0.6), input.clone(), cx));
        app.run_until(Duration::from_secs(15), |editor| {
            editor.document.visit.working_view.support_ready()
        });
        complete_frame(
            &app.read(|editor| {
                editor
                    .document
                    .visit
                    .working_view
                    .frame
                    .as_ref()
                    .unwrap()
                    .clone()
            }),
            &services,
        );
        let before: Vec<_> = common
            .ready::<ImageLayout>(&key)
            .into_iter()
            .map(|image| Arc::downgrade(&image.image))
            .collect();
        assert!(
            !before.is_empty(),
            "the cold frame must materialize real source pixels"
        );
        app.enter(|editor, cx| request_working(editor, spec(0.62), input.clone(), cx));
        app.run_until(Duration::from_secs(15), |editor| {
            editor.document.visit.working_view.support_ready()
        });
        complete_frame(
            &app.read(|editor| {
                editor
                    .document
                    .visit
                    .working_view
                    .frame
                    .as_ref()
                    .unwrap()
                    .clone()
            }),
            &services,
        );
        let after = common.ready::<ImageLayout>(&key);
        assert!(
            before
                .iter()
                .any(|a| after.iter().any(|b| a.ptr_eq(&Arc::downgrade(&b.image)))),
            "changed interpretation must reuse actual immutable source allocations"
        );
        assert!(renderer.evaluation().retained_bytes() > 0);
        assert!(renderer.evaluation().retained_bytes() <= 8 << 20);
        drop((before, after));
        common.set_test_pressure(memory::Pressure::Warning);
        assert_eq!(renderer.evaluation().retained_bytes(), 0);
        app.read(|editor| {
            assert!(
                editor.document.visit.working_view.support_ready(),
                "pressure must not discard the completed frame"
            )
        });
        app.enter(|editor, cx| request_working(editor, spec(0.64), input.clone(), cx));
        app.run_until(Duration::from_secs(15), |editor| {
            editor.document.visit.working_view.support_ready()
        });
        assert_eq!(renderer.evaluation().retained_bytes(), 0);
        common.set_test_pressure(memory::Pressure::Normal);
        app.enter(|editor, cx| request_working(editor, spec(0.66), input.clone(), cx));
        app.run_until(Duration::from_secs(15), |editor| {
            editor.document.visit.working_view.support_ready()
        });
        complete_frame(
            &app.read(|editor| {
                editor
                    .document
                    .visit
                    .working_view
                    .frame
                    .as_ref()
                    .unwrap()
                    .clone()
            }),
            &services,
        );
        assert!(renderer.evaluation().retained_bytes() > 0);
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn inspection_and_positive_views_share_budgeted_inputs_but_not_view_results() {
    use crate::gpu::evaluation::{ImageLayout, camera_source};
    let (editor, pixels, inspection, scheduler) = fixture();
    let positive = positive_spec(inspection.frame(), &pixels, 0.25);
    let source = camera_source(&pixels.scan().unwrap()).unwrap();
    let services = editor.source_services.as_ref().unwrap().clone();
    services.renderers.set_retention_budget(8 << 20);
    let renderer = services
        .renderers
        .get(&services.device, &services.queue)
        .unwrap();
    let values = renderer.evaluation().materializations();
    values.set_test_pressure(memory::Pressure::Normal);
    with_application(editor, |app| {
        app.enter(|editor, cx| request_working(editor, inspection.clone(), pixels.clone(), cx));
        app.run_until(Duration::from_secs(15), |editor| {
            editor.document.visit.working_view.support_ready()
        });
        complete_frame(
            &app.read(|editor| {
                editor
                    .document
                    .visit
                    .working_view
                    .frame
                    .as_ref()
                    .unwrap()
                    .clone()
            }),
            &services,
        );
        let initial: Vec<_> = values
            .ready::<ImageLayout>(source.identity().unwrap())
            .into_iter()
            .map(|image| Arc::downgrade(&image.image))
            .collect();
        assert!(!initial.is_empty());
        for spec in [positive, inspection] {
            app.enter(|editor, cx| request_working(editor, spec.clone(), pixels.clone(), cx));
            app.run_until(Duration::from_secs(15), |editor| {
                editor.document.visit.working_view.support_ready()
            });
            app.read(|editor| {
                let frame = editor.document.visit.working_view.frame.as_ref().unwrap();
                assert_eq!(frame.request.capture.is_inspection(), spec.is_inspection());
                assert_eq!(
                    frame
                        .source
                        .definition
                        .ancestors()
                        .any(|node| node.domain().meaning
                            == crate::gpu::evaluation::Meaning::Inspection),
                    spec.is_inspection()
                );
            });
            complete_frame(
                &app.read(|editor| {
                    editor
                        .document
                        .visit
                        .working_view
                        .frame
                        .as_ref()
                        .unwrap()
                        .clone()
                }),
                &services,
            );
            let next = values.ready::<ImageLayout>(source.identity().unwrap());
            assert!(
                initial
                    .iter()
                    .any(|a| next.iter().any(|b| a.ptr_eq(&Arc::downgrade(&b.image))))
            );
        }
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn departing_visit_keeps_reusable_values_only_through_common_budgeted_ownership() {
    use crate::gpu::evaluation::ImageLayout;
    let (editor, input, spec, scheduler) = fixture();
    let services = editor.source_services.as_ref().unwrap().clone();
    let renderer = services
        .renderers
        .get(&services.device, &services.queue)
        .unwrap();
    let values = renderer.evaluation().materializations();
    values.set_test_pressure(memory::Pressure::Normal);
    with_application(editor, |app| {
        app.enter(|editor, cx| request_working(editor, spec, input.clone(), cx));
        app.run_until(Duration::from_secs(15), |editor| {
            editor.document.visit.working_view.support_ready()
        });
        complete_frame(
            &app.read(|editor| {
                editor
                    .document
                    .visit
                    .working_view
                    .frame
                    .as_ref()
                    .unwrap()
                    .clone()
            }),
            &services,
        );
        let (key, weak) = app.read(|editor| {
            let source = editor.document.visit.working_view.source().unwrap();
            let key = source.definition.identity().unwrap().clone();
            let image = values.find_ready::<ImageLayout>(&key, |_| true).unwrap();
            (key, Arc::downgrade(&image.image))
        });
        app.update(|editor| editor.replace_photo_scope(8));
        let cached = values.find_ready::<ImageLayout>(&key, |_| true).unwrap();
        assert!(Arc::ptr_eq(&cached.image, &weak.upgrade().unwrap()));
        values.set_retention_enabled(false);
        assert!(
            weak.upgrade().is_some(),
            "the independent consumer keeps its allocation usable"
        );
        drop(cached);
        assert!(
            weak.upgrade().is_none(),
            "no hidden visit-owned retention remains"
        );
    });
    scheduler.close_and_wait().unwrap();
}
