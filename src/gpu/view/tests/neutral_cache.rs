use super::*;

#[test]
fn neutral_definitions_and_live_values_do_not_change_with_processing_padding() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let scan = scan();
        let area = scan.area;
        let composition = Composition::with_crop(
            area,
            Default::default(),
            Default::default(),
            Some(crate::crop::Crop {
                center: [
                    f64::from(area.x) + f64::from(area.width) / 2.,
                    f64::from(area.y) + f64::from(area.height) / 2.,
                ],
                size: [area.width / 2, area.height / 2],
                angle: 0.21,
            }),
        )
        .unwrap();
        let frame = FrameRaster::full(composition).unwrap();
        let spec = SourceRequest::new(frame, calibration(), scan.camera_to_ap1, None).unwrap();
        let padded = |radius| {
            spec.clone()
                .with_pipeline(crate::pixel_pipeline::PixelPipeline {
                    stages: vec![crate::pixel_pipeline::PixelStage::new(
                        crate::pixel_pipeline::authoring_examples::TentConvolution::new(radius)
                            .unwrap(),
                    )],
                })
                .unwrap()
        };
        let large_spec = padded(8.);
        let small_spec = padded(2.);
        assert_ne!(
            large_spec.input_frame().unwrap(),
            small_spec.input_frame().unwrap()
        );
        assert_eq!(large_spec.plan.output(), small_spec.plan.output());
        let large_region = large_spec.input_region().unwrap();
        let small_region = small_spec.input_region().unwrap();
        assert!(large_region.contains_region(small_region));
        assert!(large_region.x < small_region.x && large_region.y < small_region.y);
        let cache = renderer.evaluation().test_materializations(false);
        let gate = SubmissionGate::default();
        let acquire = |spec| {
            ViewSource::acquire_originals_fixture(
                &device,
                &queue,
                &gate,
                renderer.clone(),
                kernels.clone(),
                &scan,
                None,
                vec![spec],
                &cache,
                || false,
            )
        };
        let large = acquire(large_spec).await.unwrap().unwrap().remove(0);
        let small = acquire(small_spec).await.unwrap().unwrap().remove(0);
        assert_eq!(large.definition.domain().bounds, None);
        assert_eq!(
            large.definition.identity(),
            small.definition.identity(),
            "downstream padding is a stored region, not a different neutral calculation"
        );
        assert_eq!(renderer.evaluation().retained_bytes(), 0);
        let whole = read(&device, &queue, &large.pixels);
        let subset = read(&device, &queue, &small.pixels);
        assert!(
            whole.windows(2).any(|pair| pair[0][..3] != pair[1][..3]),
            "fixture must distinguish misplaced or constant samples"
        );
        let dx = (small_region.x - large_region.x) as usize;
        let dy = (small_region.y - large_region.y) as usize;
        let mut expected = Vec::new();
        for row in 0..small_region.height as usize {
            let first = (dy + row) * large_region.width as usize + dx;
            expected.extend_from_slice(&whole[first..first + small_region.width as usize]);
        }
        assert_eq!(
            subset, expected,
            "a live containing value supplies exactly the same samples and flags"
        );
    });
}

#[test]
fn independent_neutral_consumers_share_the_value_with_and_without_optional_retention() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let scan = scan();
        let frame = frame(&scan);
        let gate = SubmissionGate::default();
        let spec = SourceRequest::new(frame, calibration(), scan.camera_to_ap1, None).unwrap();
        let cache = renderer.evaluation().test_materializations(true);
        let source = ViewSource::acquire_originals_fixture(
            &device,
            &queue,
            &gate,
            renderer.clone(),
            kernels.clone(),
            &scan,
            None,
            vec![spec.clone()],
            &cache,
            || false,
        )
        .await
        .unwrap()
        .unwrap()
        .remove(0);
        let wanted = source.spec.clone();
        let found = ViewSource::cached_neutral(
            &cache,
            renderer.clone(),
            kernels.clone(),
            &wanted,
            &scan,
            None,
        )
        .unwrap();
        assert!(found.spec == wanted);
        assert!(found.same_pixels_as(&source));
        let repeated = ViewSource::acquire_originals_fixture(
            &device,
            &queue,
            &gate,
            renderer.clone(),
            kernels.clone(),
            &scan,
            None,
            vec![wanted.clone()],
            &cache,
            || false,
        )
        .await
        .unwrap()
        .unwrap()
        .remove(0);
        assert!(repeated.same_pixels_as(&source));
        let program = *wanted.program().unwrap();
        let mut changed_input = program.input;
        if let InputSettings::Negative { gamma, .. } = &mut changed_input {
            *gamma = gamma.next_up();
        }
        let changed_program = GpuInputProgram::new(changed_input, program.matrix).unwrap();
        assert_ne!(program, changed_program);
        assert_eq!(
            bytemuck::bytes_of(&program.neutral_parameters().unwrap()),
            bytemuck::bytes_of(&changed_program.neutral_parameters().unwrap()),
            "control must collide after shader packing"
        );
        let changed_gamma = SourceRequest::new(
            wanted.frame(),
            changed_input,
            wanted.matrix(),
            wanted.field.clone(),
        )
        .unwrap()
        .with_identity(wanted.identity.clone())
        .with_pipeline(wanted.plan.pipeline().clone())
        .unwrap();
        assert!(
            ViewSource::cached_neutral(
                &cache,
                renderer.clone(),
                kernels.clone(),
                &changed_gamma,
                &scan,
                None
            )
            .is_none(),
            "a retained value must keep its complete captured input definition"
        );
        let changed = SourceRequest::new(
            wanted.frame(),
            wanted.program().unwrap().input,
            wanted.matrix() * 1.01,
            wanted.field.clone(),
        )
        .unwrap()
        .with_identity(wanted.identity.clone())
        .with_pipeline(wanted.plan.pipeline().clone())
        .unwrap();
        assert!(
            ViewSource::cached_neutral(
                &cache,
                renderer.clone(),
                kernels.clone(),
                &changed,
                &scan,
                None
            )
            .is_none()
        );
        let moved = wanted
            .clone()
            .with_frame(FrameRaster {
                output: [frame.output[0] / 2, frame.output[1] / 2],
                ..frame
            })
            .unwrap();
        assert!(
            ViewSource::cached_neutral(
                &cache,
                renderer.clone(),
                kernels.clone(),
                &moved,
                &scan,
                None
            )
            .is_none()
        );
        let foreign = crate::gpu::evaluation::Context::new(&device, Default::default())
            .test_materializations(true);
        assert!(
            ViewSource::cached_neutral(
                &foreign,
                renderer.clone(),
                kernels.clone(),
                &wanted,
                &scan,
                None
            )
            .is_none()
        );
        let active = found.pixels.clone();
        let evaluation = renderer.evaluation().clone();
        let owner = Arc::downgrade(&renderer);
        assert_eq!(evaluation.retained_bytes(), source.pixels.size());
        drop((source, found, repeated, renderer));
        assert!(
            owner.upgrade().is_none(),
            "retained values must not own their renderer"
        );
        cache.clear();
        assert_eq!(evaluation.retained_bytes(), 0);
        assert!(!read(&device, &queue, &active).is_empty());

        let off_renderer = crate::renderer::GpuRenderer::new_with_cache_policy(
            &device,
            &queue,
            Default::default(),
            crate::gpu::evaluation::CachePolicy { generated_bytes: 0 },
        )
        .await
        .unwrap();
        let off_cache = off_renderer.evaluation().test_materializations(false);
        let off = ViewSource::acquire_originals_fixture(
            &device,
            &queue,
            &gate,
            off_renderer.clone(),
            kernels.clone(),
            &scan,
            None,
            vec![spec],
            &off_cache,
            || false,
        )
        .await
        .unwrap()
        .unwrap()
        .remove(0);
        let joined = ViewSource::cached_neutral(
            &off_cache,
            off_renderer.clone(),
            kernels.clone(),
            &off.spec,
            &scan,
            None,
        )
        .unwrap();
        assert!(
            joined.same_pixels_as(&off),
            "active ownership remains discoverable without retention"
        );
        assert_eq!(off_renderer.evaluation().retained_bytes(), 0);
        assert_eq!(
            read(&device, &queue, &off.pixels),
            read(&device, &queue, &active)
        );
        let wanted = off.spec.clone();
        drop((off, joined));
        assert!(
            ViewSource::cached_neutral(&off_cache, off_renderer, kernels, &wanted, &scan, None)
                .is_none()
        );
    });
}
