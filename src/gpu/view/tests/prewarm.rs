use super::*;

#[test]
fn fallible_paint_admission_rejects_uncaptured_photos_but_accepts_inspection() {
    pollster::block_on(async {
        let (device, _queue, renderer, _kernels) = context().await;
        let image = Arc::new(
            GpuImage::new_shared(
                &renderer,
                &device,
                2,
                2,
                &[[32000, 20000, 10000, 0]; 4],
                DMat3::IDENTITY,
                OutputTarget::Srgb,
            )
            .await
            .unwrap(),
        );
        let packed = ProcessingParameters::interpret(
            InputSettings::Digital,
            Default::default(),
            DMat3::IDENTITY,
        )
        .unwrap();
        assert!(matches!(
            crate::viewer::ImagePaint::with_processing(
                image.clone(),
                Some(packed.into()),
                [0., 0., 1., 1.],
            ),
            Err(Error::InvalidParameter(
                "photographic input requires captured interpretation and grading"
            ))
        ));
        let inspection = ProcessingInput::from(ProcessingParameters::source());
        assert!(
            crate::viewer::ImagePaint::with_processing(image, Some(inspection), [0., 0., 1., 1.],)
                .is_ok()
        );
    });
}

#[test]
fn fresh_resident_camera_paints_without_incidental_acquisition_or_thumbnail_compilation() {
    pollster::block_on(async {
        let (device, queue, renderer, _kernels) = context().await;
        let image = GpuImage::new_shared(
            &renderer,
            &device,
            4,
            2,
            &[[32000, 20000, 10000, 0]; 8],
            DMat3::IDENTITY,
            OutputTarget::Srgb,
        )
        .await
        .unwrap();
        let compiled = renderer.evaluation.programs.len();
        for input in [
            ProcessingInput::from(ProcessingParameters::source()),
            ProcessingInput::new(
                &device,
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
                renderer.evaluation.programs.len(),
                compiled,
                "camera paint must use initialization's compiled source component"
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
        let source = ViewSource::acquire(
            &device,
            &queue,
            &Default::default(),
            renderer.clone(),
            kernels.clone(),
            &scan,
            None,
            ViewSourceSpec::new(1, frame, calibration(), scan.camera_to_ap1, None).unwrap(),
            || false,
        )
        .unwrap()
        .unwrap();
        let inspection = ViewSource::acquire(
            &device,
            &queue,
            &Default::default(),
            renderer.clone(),
            kernels,
            &scan,
            None,
            ViewSourceSpec::inspection(2, frame, scan.camera_to_ap1, None).unwrap(),
            || false,
        )
        .unwrap()
        .unwrap();
        // This renderer/device has never prepared thumbnails or photographic views.
        let compiled = renderer.evaluation.programs.len();
        let context = egui::Context::default();
        let gate = SubmissionGate::default();
        for photo in [
            AdjustmentLayerSettings::default(),
            AdjustmentLayerSettings {
                exposure: 0.75,
                ..Default::default()
            },
        ] {
            let input = processing_input(&device, calibration(), Default::default(), photo, &scan);
            let view = RenderedView::new(&device, source.clone(), &input, OutputTarget::Srgb, None)
                .unwrap();
            view.ensure_submitted(&device, &queue, &gate, &context);
            assert_eq!(
                renderer.evaluation.programs.len(),
                compiled,
                "paint must reuse background-initialized component programs"
            );
        }
        let input = ProcessingInput::from(ProcessingParameters::source_inspection(1.2).unwrap());
        let view =
            RenderedView::inspection(&device, inspection, &input, OutputTarget::DisplayP3, None)
                .unwrap();
        view.ensure_submitted(&device, &queue, &gate, &context);
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
