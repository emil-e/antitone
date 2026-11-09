use super::*;

#[test]
fn gamma_edits_reuse_active_camera_storage_and_pressure_retires_it_without_losing_the_frame() {
    use crate::gpu::evaluation::{ImageBacking, ImageLayout, camera_source};
    let (e, input, frame_spec, scheduler) = fixture();
    let scan = input.scan().unwrap();
    let calibration = (crate::processing::Calibration {
        dmin: glam::DVec3::splat(0.8),
        dmax: glam::DVec3::splat(0.2),
    })
    .validate()
    .unwrap();
    let spec = |warmth| {
        let mut settings = crate::processing::InputSettings::from(calibration);
        if let crate::processing::InputSettings::Negative { balance, .. } = &mut settings {
            balance.warmth = warmth;
        }
        SourceRequest::new(frame_spec.frame(), settings, scan.camera_to_ap1, None).unwrap()
    };
    let services = e.source_services.as_ref().unwrap();
    let renderer = services
        .renderers
        .get(&services.device, &services.queue)
        .unwrap();
    let common = renderer.evaluation().materializations();
    common.set_retention_enabled(true);
    let original = camera_source(&scan).unwrap();
    let key = original.identity().unwrap().clone();
    let bytes = u64::from(scan.area.width)
        * u64::from(scan.area.height)
        * original.source().unwrap().storage_format().pixel_bytes();
    with_application(e, |app| {
        app.enter(|e, cx| request_working(e, spec(0.), input.clone(), cx));
        app.run_until(Duration::from_secs(15), |e| {
            e.document.visit.working_view.support_ready()
        });
        let before = renderer
            .evaluation()
            .materializations()
            .find_ready::<ImageLayout>(&key, |layout| layout.backing == ImageBacking::Texture)
            .unwrap();
        assert_eq!(renderer.evaluation().active_input_bytes(), bytes);
        app.enter(|e, cx| request_working(e, spec(0.02), input.clone(), cx));
        app.run_until(Duration::from_secs(15), |e| {
            e.document.visit.working_view.support_ready()
        });
        let after = renderer
            .evaluation()
            .materializations()
            .find_ready::<ImageLayout>(&key, |layout| layout.backing == ImageBacking::Texture)
            .unwrap();
        assert!(
            Arc::ptr_eq(&before.image, &after.image),
            "gamma must not re-upload unchanged camera samples"
        );
        drop((before, after));
        app.read(|e| {
            e.document
                .visit
                .working_view
                .mailbox
                .pressure(memory::Pressure::Warning)
        });
        common.set_retention_enabled(false);
        assert_eq!(renderer.evaluation().active_input_bytes(), 0);
        app.read(|e| {
            assert!(
                e.document.visit.working_view.support_ready(),
                "pressure must not discard the completed frame"
            )
        });
        app.enter(|e, cx| request_working(e, spec(-0.02), input.clone(), cx));
        app.run_until(Duration::from_secs(15), |e| {
            e.document.visit.working_view.support_ready()
        });
        assert_eq!(
            renderer.evaluation().active_input_bytes(),
            0,
            "pressure uses bounded source production"
        );
        app.read(|e| {
            e.document
                .visit
                .working_view
                .mailbox
                .pressure(memory::Pressure::Normal)
        });
        common.set_retention_enabled(true);
        app.enter(|e, cx| request_working(e, spec(0.03), input.clone(), cx));
        app.run_until(Duration::from_secs(15), |e| {
            e.document.visit.working_view.support_ready()
        });
        assert_eq!(renderer.evaluation().active_input_bytes(), bytes);
        app.update(|e| e.replace_photo_scope(8));
        assert_eq!(renderer.evaluation().active_input_bytes(), 0);
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn departing_visit_releases_active_input_but_keeps_reusable_values_in_the_common_store() {
    use crate::gpu::evaluation::ImageLayout;
    let (e, input, spec, scheduler) = fixture();
    let services = e.source_services.as_ref().unwrap();
    let renderer = services
        .renderers
        .get(&services.device, &services.queue)
        .unwrap();
    let values = renderer.evaluation().materializations();
    values.set_retention_enabled(true);
    with_application(e, |app| {
        app.enter(|e, cx| request_working(e, spec, input.clone(), cx));
        app.run_until(Duration::from_secs(15), |e| {
            e.document.visit.working_view.support_ready()
        });
        let (key, weak) = app.read(|e| {
            let source = e.document.visit.working_view.source().unwrap();
            let key = source.definition.identity().unwrap().clone();
            let image = values.find_ready::<ImageLayout>(&key, |_| true).unwrap();
            (key, Arc::downgrade(&image.image))
        });
        assert!(renderer.evaluation().active_input_bytes() > 0);
        app.update(|e| e.replace_photo_scope(8));
        assert_eq!(renderer.evaluation().active_input_bytes(), 0);
        let cached = values.find_ready::<ImageLayout>(&key, |_| true).unwrap();
        assert!(Arc::ptr_eq(&cached.image, &weak.upgrade().unwrap()));
        values.set_retention_enabled(false);
        assert!(
            weak.upgrade().is_some(),
            "the new consumer keeps its value alive"
        );
        drop(cached);
        assert!(
            weak.upgrade().is_none(),
            "no owner remains after optional ownership retires"
        );
    });
    scheduler.close_and_wait().unwrap();
}
