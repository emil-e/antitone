use super::*;
use crate::gpu::parameters::ProcessingParameters;
use crate::gpu::test_inputs::AuthoredPhotograph;
use crate::gpu::view::request::SourceRequest;

#[test]
fn fallible_paint_admission_rejects_uncaptured_photos_but_accepts_inspection() {
    pollster::block_on(async {
        let (device, _queue, renderer, _kernels) = context().await;
        let image = Arc::new(
            GpuImage::new_shared(
                renderer.evaluation().clone(),
                renderer.looks().clone(),
                renderer.drt(OutputTarget::Srgb),
                [2, 2],
                &[[32000, 20000, 10000, 0]; 4],
                DMat3::IDENTITY,
                None,
            )
            .await
            .unwrap(),
        );
        let presentation = crate::viewer::Presentation::new(&device).await.unwrap();
        let packed = ProcessingParameters::interpret(
            InputSettings::Digital,
            Default::default(),
            DMat3::IDENTITY,
        )
        .unwrap();
        assert!(matches!(
            crate::viewer::ImagePaint::with_processing(
                &presentation,
                image.clone(),
                Some(packed.into()),
                [0., 0., 1., 1.],
            ),
            Err(Error::InvalidParameter(
                "photographic input requires captured interpretation and grading"
            ))
        ));
        let inspection = AuthoredPhotograph::from(ProcessingParameters::source());
        assert!(
            crate::viewer::ImagePaint::with_processing(
                &presentation,
                image,
                Some(inspection),
                [0., 0., 1., 1.]
            )
            .is_ok()
        );
    });
}

#[test]
fn fresh_resident_camera_reuses_warm_programs_after_the_startup_handles_retire() {
    pollster::block_on(async {
        let instance = crate::gpu::instance();
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = crate::gpu::submission::request_device(&adapter, &Default::default())
            .await
            .unwrap();
        let evaluation = crate::gpu::evaluation::Context::new(&device, Default::default());
        let looks = Arc::new(crate::gpu::lut3d::Looks::new(&device));
        let targets = crate::gpu::drt::Targets::new(&device, &queue)
            .await
            .unwrap();
        crate::gpu::photograph::prewarm_viewer(&evaluation, &targets).unwrap();
        let image = GpuImage::new_shared(
            evaluation.clone(),
            looks,
            targets.get(OutputTarget::Srgb),
            [4, 2],
            &[[32000, 20000, 10000, 0]; 8],
            DMat3::IDENTITY,
            None,
        )
        .await
        .unwrap();
        drop(targets);
        let compiled = evaluation.programs.len();
        for input in [
            AuthoredPhotograph::from(ProcessingParameters::source()),
            AuthoredPhotograph::new(
                &device.limits(),
                InputSettings::Digital,
                Default::default(),
                AdjustmentLayerSnapshot::new(vec![]).unwrap(),
                DMat3::IDENTITY,
                [0, 0, 4, 2].into(),
            )
            .unwrap(),
        ] {
            let mut encoder = device.create_command_encoder(&Default::default());
            image
                .encode_processing(&device, &mut encoder, &input)
                .unwrap();
            assert_eq!(
                evaluation.programs.len(),
                compiled,
                "camera paint must use initialization's compiled endpoint (positive: {})",
                input.is_positive()
            );
            queue.submit([encoder.finish()]);
        }
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(10)),
            })
            .unwrap();
    });
}

#[test]
fn fresh_context_support_can_paint_default_grading_inspection_and_first_active_layer_without_compiling()
 {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let scan = scan();
        let frame = frame(&scan);
        let source = ViewImage::prefix(
            &device,
            &queue,
            renderer.clone(),
            kernels.clone(),
            &scan,
            None,
            SourceRequest::new(frame, calibration(), scan.camera_to_ap1, None).unwrap(),
            || false,
        )
        .unwrap()
        .unwrap();
        let inspection = ViewImage::prefix(
            &device,
            &queue,
            renderer.clone(),
            kernels,
            &scan,
            None,
            SourceRequest::inspection_fixture(frame, scan.camera_to_ap1, None).unwrap(),
            || false,
        )
        .unwrap()
        .unwrap();
        // This renderer/device has never prepared thumbnails or photographic views.
        let compiled = renderer.evaluation.programs.len();
        let context = egui::Context::default();
        let complete = |source: Arc<ViewImage>, input: &AuthoredPhotograph, target| {
            let capture =
                crate::gpu::photograph::request::PhotographRequest::Fixture(input.clone());
            let (prefix, plan) = capture
                .filtered(&device.limits(), &source.spec, source.definition.clone())
                .unwrap();
            let endpoint = capture
                .display(prefix, renderer.looks(), renderer.drt(target))
                .unwrap();
            let evaluation =
                ViewImage::evaluation(renderer.evaluation(), endpoint.clone()).unwrap();
            let work = renderer
                .evaluation()
                .prepare_supplied(evaluation, vec![source.supplied().unwrap()])
                .unwrap();
            let spec = capture
                .planned_request(&device.limits(), source.spec.clone(), plan)
                .unwrap();
            let region = endpoint.domain().finite().unwrap();
            let image = ViewImage::from_recorded(
                renderer.clone(),
                source.kernels().clone(),
                spec,
                endpoint,
                region,
                work.record(renderer.evaluation()).unwrap(),
            )
            .unwrap();
            RenderedView::from_image(&device, image, capture, target).unwrap()
        };
        for photo in [
            AdjustmentLayerSettings::default(),
            AdjustmentLayerSettings {
                exposure: 0.75,
                ..Default::default()
            },
        ] {
            let input = processing_input(&device, calibration(), Default::default(), photo, &scan);
            let view = complete(source.clone(), &input, OutputTarget::Srgb);
            view.ensure_submitted(&device, &queue, &context);
            assert_eq!(
                renderer.evaluation.programs.len(),
                compiled,
                "paint must reuse background-initialized component programs"
            );
        }
        let input = AuthoredPhotograph::from(ProcessingParameters::source_inspection(1.2).unwrap());
        let view = complete(inspection, &input, OutputTarget::DisplayP3);
        view.ensure_submitted(&device, &queue, &context);
        assert_eq!(
            renderer.evaluation.programs.len(),
            compiled,
            "first inspection paint must not compile"
        );
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(10)),
            })
            .unwrap();
    });
}
