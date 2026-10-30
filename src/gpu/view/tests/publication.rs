use super::*;

#[test]
fn captured_pixels_and_photographic_work_survive_a_new_ticket_but_old_publication_does_not() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let scan = scan();
        let spec = ViewSourceSpec::inspection(7, frame(&scan), scan.camera_to_ap1, None).unwrap();
        let source = ViewSource::acquire(
            &device,
            &queue,
            &SubmissionGate::default(),
            renderer,
            kernels,
            &scan,
            None,
            spec,
            || false,
        )
        .unwrap()
        .unwrap();
        let input = ProcessingInput::from(ProcessingParameters::source());
        let first =
            RenderedView::inspection(&device, source.clone(), &input, OutputTarget::Srgb, None)
                .unwrap();
        let published = source.published(8);
        assert!(source.spec.same_source(&published.spec));
        assert!(source.spec.satisfies(&published.spec));
        assert!(
            source.spec != published.spec,
            "publication tickets remain request identity"
        );
        assert_eq!(
            source.pixels, published.pixels,
            "publication must not copy or reacquire pixels"
        );
        assert!(!first.matches(&published.spec, &input.signature(), OutputTarget::Srgb));
        let next =
            RenderedView::inspection(&device, published, &input, OutputTarget::Srgb, Some(&first))
                .unwrap();
        assert_eq!(next.spec.authority, 8);
        assert!(
            Arc::ptr_eq(
                first.graded.as_ref().unwrap(),
                next.graded.as_ref().unwrap()
            ),
            "photographic content does not depend on publication"
        );
        assert!(
            RenderedView::composite(&device, first, next.clone(), next.spec.frame(), None).is_err(),
            "matching content cannot authorize mixing tickets"
        );
    });
}

#[test]
fn submitted_positive_rebinds_publication_without_regrading_or_changing_pixels() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let scan = scan();
        let matrix = scan.positive_matrix(&InputSettings::Digital).unwrap();
        let input = ProcessingInput::from(Arc::new(
            ProcessingInput::new(
                &device,
                InputSettings::Digital,
                Default::default(),
                AdjustmentLayerSnapshot::new(vec![]).unwrap(),
                matrix,
                scan.area,
            )
            .unwrap(),
        ))
        .with_film_emulation(crate::film_emulation::FilmPreset::ThirtyFiveMm.settings())
        .with_frame_size(frame(&scan).composition.crop().size)
        .with_film_seed(17);
        let spec = ViewSourceSpec::new(7, frame(&scan), InputSettings::Digital, matrix, None)
            .unwrap()
            .with_pipeline(input.pixel_pipeline().unwrap())
            .unwrap();
        let gate = SubmissionGate::default();
        let source = ViewSource::acquire(
            &device,
            &queue,
            &gate,
            renderer,
            kernels,
            &scan,
            None,
            spec,
            || false,
        )
        .unwrap()
        .unwrap();
        let first =
            RenderedView::new(&device, source.clone(), &input, OutputTarget::Srgb, None).unwrap();
        let context = egui::Context::default();
        first.ensure_submitted(&device, &queue, &gate, &context);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let pixels = crate::viewer::readback::image_pixels(&device, &queue, &first.image);
        assert!(pixels.iter().any(|p| p[..3].iter().any(|&c| c > 0.)));
        let next = RenderedView::new(
            &device,
            source.published(8),
            &input,
            OutputTarget::Srgb,
            Some(&first),
        )
        .unwrap();
        assert_eq!(next.spec.authority, 8);
        assert!(Arc::ptr_eq(
            first.graded.as_ref().unwrap(),
            next.graded.as_ref().unwrap()
        ));
        assert!(!first.matches(&next.spec, &input.signature(), OutputTarget::Srgb));
        next.ensure_submitted(&device, &queue, &gate, &context);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        assert_eq!(
            pixels,
            crate::viewer::readback::image_pixels(&device, &queue, &next.image)
        );
    });
}

#[test]
fn original_adapter_rejects_a_different_capture_even_when_program_layout_and_ticket_match() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let scan = scan();
        let file = crate::dng_fixture::Fixture::default().write();
        let unrelated = Scan::open(file.path(), None).unwrap();
        let spec = ViewSourceSpec::inspection(7, frame(&scan), scan.camera_to_ap1, None)
            .unwrap()
            .with_identity(crate::gpu::execution::source::Identity::camera(
                unrelated.source().identity(),
            ));
        let gate = SubmissionGate::default();
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed = count.clone();
        gate.observe_submissions(Arc::new(move || {
            observed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }));
        assert!(
            ViewSource::acquire(
                &device,
                &queue,
                &gate,
                renderer,
                kernels,
                &scan,
                None,
                spec,
                || false
            )
            .is_err()
        );
        assert_eq!(count.load(std::sync::atomic::Ordering::Relaxed), 0);
    });
}
