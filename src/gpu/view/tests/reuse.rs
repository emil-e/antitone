use super::*;
use crate::gpu::input::ProcessingInput;
use crate::gpu::parameters::ProcessingParameters;
use crate::gpu::submission::SubmissionGate;
use crate::gpu::view::request::SourceRequest;

#[test]
fn equal_acquired_calculations_share_inspection_work_across_independent_view_handles() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let scan = scan();
        let spec =
            SourceRequest::inspection_fixture(frame(&scan), scan.camera_to_ap1, None).unwrap();
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
        let independent = ViewSource::from_acquired(source.kernels.clone(), source.acquired());
        assert!(!Arc::ptr_eq(&source, &independent));
        assert!(source.spec == independent.spec);
        assert_eq!(source.pixels, independent.pixels);
        assert!(first.matches(&independent.spec, &input, OutputTarget::Srgb));
        let next = RenderedView::inspection(
            &device,
            independent,
            &input,
            OutputTarget::Srgb,
            Some(&first),
        )
        .unwrap();
        assert!(
            Arc::ptr_eq(
                first.graded.as_ref().unwrap(),
                next.graded.as_ref().unwrap()
            ),
            "independent handles do not change the calculation"
        );
    });
}

#[test]
fn submitted_positive_is_reused_by_an_independent_handle_without_regrading_or_changing_pixels() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let scan = scan();
        let matrix = scan.positive_matrix(&InputSettings::Digital).unwrap();
        let input = ProcessingInput::from(Arc::new(
            ProcessingInput::new(
                &device.limits(),
                InputSettings::Digital,
                Default::default(),
                AdjustmentLayerSnapshot::new(vec![]).unwrap(),
                matrix,
                scan.area,
            )
            .unwrap(),
        ))
        .with_pixel_pipeline(
            crate::pixel_pipeline::PixelPipeline::emulation(
                crate::grain::GrainPreset::ThirtyFiveMm.settings(),
                crate::halation::HalationSettings {
                    enabled: true,
                    size: crate::halation::MAX_SIZE,
                    ..Default::default()
                },
                frame(&scan).composition.crop().size,
                17,
            )
            .unwrap(),
        );
        assert!(input.pixel_pipeline().stages.iter().any(|stage| {
            stage
                .get::<crate::halation::HalationParameters>()
                .is_some_and(|parameters| parameters.strength > 0.)
        }));
        let spec = SourceRequest::new(frame(&scan), InputSettings::Digital, matrix, None)
            .unwrap()
            .with_pipeline(input.pixel_pipeline())
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
            ViewSource::from_acquired(source.kernels.clone(), source.acquired()),
            &input,
            OutputTarget::Srgb,
            Some(&first),
        )
        .unwrap();
        assert!(first.spec == next.spec);
        assert!(Arc::ptr_eq(
            first.graded.as_ref().unwrap(),
            next.graded.as_ref().unwrap()
        ));
        assert!(first.matches(&next.spec, &input, OutputTarget::Srgb));
        next.ensure_submitted(&device, &queue, &gate, &context);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        assert_eq!(
            pixels,
            crate::viewer::readback::image_pixels(&device, &queue, &next.image)
        );
    });
}

#[test]
fn anonymous_supplied_values_with_equal_view_descriptions_do_not_alias_previous_photographic_work()
{
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let scan = scan();
        let matrix = scan.positive_matrix(&InputSettings::Digital).unwrap();
        let spec = SourceRequest::new(frame(&scan), InputSettings::Digital, matrix, None).unwrap();
        let region = spec.input_region().unwrap();
        let source = |pixel: [f32; 4]| {
            Arc::new(ViewSource::from_neutral_samples(
                spec.clone(),
                renderer.clone(),
                kernels.clone(),
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("actual anonymous supplied value"),
                    contents: bytemuck::cast_slice(&vec![
                        pixel;
                        (region.width * region.height) as usize
                    ]),
                    usage: wgpu::BufferUsages::STORAGE,
                }),
            ))
        };
        let a = source([0.125, 0.25, 0.5, 8.]);
        let b = source([0.375, 0.5, 0.75, 8.]);
        assert!(a.definition.identity().is_none());
        assert!(b.definition.identity().is_none());
        assert!(a.spec == b.spec);
        let input = ProcessingInput::new(
            &device.limits(),
            InputSettings::Digital,
            RollSettings {
                exposure: 1.,
                ..Default::default()
            },
            AdjustmentLayerSnapshot::new(Vec::new()).unwrap(),
            matrix,
            scan.area,
        )
        .unwrap();
        let first =
            RenderedView::new(&device, a.clone(), &input, OutputTarget::Srgb, None).unwrap();
        let shared = RenderedView::new(
            &device,
            ViewSource::from_acquired(a.kernels.clone(), a.acquired()),
            &input,
            OutputTarget::Srgb,
            Some(&first),
        )
        .unwrap();
        assert!(
            Arc::ptr_eq(
                first.graded.as_ref().unwrap(),
                shared.graded.as_ref().unwrap()
            ),
            "the same anonymous argument and definition can reuse recorded work locally"
        );
        let other =
            RenderedView::new(&device, b, &input, OutputTarget::Srgb, Some(&first)).unwrap();
        let gate = SubmissionGate::default();
        let ui = egui::Context::default();
        first.ensure_submitted(&device, &queue, &gate, &ui);
        other.ensure_submitted(&device, &queue, &gate, &ui);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        for (view, expected) in [(&first, [0.25, 0.5, 1.]), (&other, [0.75, 1., 1.5])] {
            let pixels = read(&device, &queue, &view.graded.as_ref().unwrap().pixels);
            assert_eq!(
                pixels.len(),
                (spec.frame().output[0] * spec.frame().output[1]) as usize
            );
            for pixel in pixels {
                for channel in 0..3 {
                    assert!((pixel[channel] - expected[channel]).abs() < 1e-6);
                }
                assert_eq!(pixel[3], 8.);
            }
        }
    });
}

#[test]
fn original_adapter_rejects_a_different_capture_even_when_program_and_layout_match() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let scan = scan();
        let file = crate::dng_fixture::Fixture::default().write();
        let unrelated = Scan::open(file.path(), None).unwrap();
        let spec = SourceRequest::inspection_fixture(frame(&scan), scan.camera_to_ap1, None)
            .unwrap()
            .with_identity(crate::gpu::source::identity::Identity::camera(
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
