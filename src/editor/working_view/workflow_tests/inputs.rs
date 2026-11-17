use super::*;

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
    let submissions = editor.submissions.clone();
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
            &submissions,
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
            &submissions,
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
            &submissions,
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
    let submissions = editor.submissions.clone();
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
            &submissions,
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
                &submissions,
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
    let submissions = editor.submissions.clone();
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
            &submissions,
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
