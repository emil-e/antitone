use super::*;

#[test]
fn physical_view_source_inspection_preserves_normalization_correction_and_reuses_linear_support() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let gate = SubmissionGate::default();
        let context = egui::Context::default();
        let mut scan = scan();
        let frame = frame(&scan);
        let spec = ViewSourceSpec::inspection(21, frame, scan.camera_to_ap1, None).unwrap();
        let positive = ViewSourceSpec::new(
            21,
            frame,
            calibration(),
            RollSettings::new_grade(),
            scan.camera_to_ap1,
            None,
        )
        .unwrap();
        assert!(!spec.same_source(&positive));
        let codes = scan
            .tile_region(scan.area, 0, 0, scan.area.width, scan.area.height)
            .unwrap();
        let mut previous: Option<Arc<RenderedView>> = None;
        let mut uncorrected = None;
        let mut worst_linear = 0f32;
        for corrected in [false, true] {
            if corrected {
                scan.set_flat_field(Some(Arc::new(
                    crate::flat_field::FlatField::from_nodes(
                        [scan.area.x, scan.area.y, scan.area.width, scan.area.height],
                        [2, 2],
                        vec![
                            DVec3::splat(0.8),
                            DVec3::new(1.2, 1.1, 0.9),
                            DVec3::splat(1.1),
                            DVec3::new(0.9, 1.2, 1.),
                        ],
                    )
                    .unwrap(),
                )))
                .unwrap();
            }
            let field = scan.flat_field().cloned();
            let gpu_field = field
                .as_ref()
                .map(|f| Arc::new(GpuFlatField::new(&device, f.clone()).unwrap()));
            let spec = ViewSourceSpec::inspection(21, frame, scan.camera_to_ap1, field).unwrap();
            let source = ViewSource::acquire(
                &device,
                &queue,
                &gate,
                renderer.clone(),
                kernels.clone(),
                &scan,
                gpu_field.clone(),
                spec.clone(),
                || false,
            )
            .unwrap()
            .unwrap();
            let neutral = read(&device, &queue, &source.pixels);
            if let Some(old) = &uncorrected {
                assert_ne!(
                    &neutral, old,
                    "inspection preserves accepted source correction"
                );
            } else {
                uncorrected = Some(neutral.clone());
            }
            for (i, (pixel, code)) in neutral.iter().zip(&codes).enumerate() {
                let point = [
                    f64::from(scan.area.x + (i as u32 % scan.area.width)) + 0.5,
                    f64::from(scan.area.y + (i as u32 / scan.area.width)) + 0.5,
                ];
                let gain = scan
                    .flat_field()
                    .map_or(DVec3::ONE, |f| f.gain(point).unwrap());
                let expected = scan.camera_to_ap1
                    * (DVec3::new(f64::from(code[0]), f64::from(code[1]), f64::from(code[2]))
                        / 65535.
                        * gain);
                assert_eq!(pixel[3], 8.);
                for c in 0..3 {
                    assert!(
                        (f64::from(pixel[c]) - expected[c]).abs()
                            <= 2e-5 + 2e-4 * expected[c].abs()
                    );
                }
            }
            for ev in [-16., -2., 0., 0.37, 2., 16.] {
                let input = ProcessingInput::from(
                    ProcessingParameters::source_preview(Some([2., 1., 1.5, 1.]), ev).unwrap(),
                );
                for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
                    let view = RenderedView::inspection(
                        &device,
                        source.clone(),
                        &input,
                        target,
                        previous.as_ref(),
                    )
                    .unwrap();
                    assert!(
                        view.graded.as_ref().unwrap().grade.is_none(),
                        "no calibration, inversion, WB, contrast, saturation or locals"
                    );
                    if let Some(old) = &previous
                        && old.processing == input.signature()
                        && old.spec == source.spec
                    {
                        assert!(Arc::ptr_eq(
                            view.graded.as_ref().unwrap(),
                            old.graded.as_ref().unwrap()
                        ));
                    }
                    view.ensure_submitted(&device, &queue, &gate, &context);
                    let graded = read(&device, &queue, &view.graded.as_ref().unwrap().pixels);
                    for (pixel, neutral) in graded.iter().zip(&neutral) {
                        assert_eq!(pixel[3], neutral[3]);
                        for c in 0..3 {
                            let expected = f64::from(neutral[c]) * 4. * ev.exp2();
                            assert!(
                                (f64::from(pixel[c]) - expected).abs()
                                    <= 2e-5 + 2e-4 * expected.abs()
                            );
                        }
                    }
                    let generation = AtomicU64::new(9);
                    let reference = crate::refinement::refine(
                        &renderer,
                        &device,
                        (&queue, &gate),
                        (&scan, gpu_field.clone()),
                        input.clone(),
                        target,
                        (&generation, 9),
                        crate::refinement::ViewRequest {
                            area: scan.area,
                            output: frame.output,
                            frame: Some(frame),
                        },
                    )
                    .unwrap()
                    .unwrap();
                    for (a, b) in image(&device, &queue, &view.image)
                        .iter()
                        .zip(image(&device, &queue, &reference))
                    {
                        for c in 0..3 {
                            let error = (a[c] - b[c]).abs();
                            worst_linear = worst_linear.max(error);
                            assert!(error <= 2e-5);
                        }
                        assert_eq!(a[3], b[3]);
                    }
                    let positive_input = processing_input(
                        calibration(),
                        RollSettings::new_grade(),
                        PhotoSettings::default(),
                        &scan,
                    );
                    assert!(
                        RenderedView::inspection(
                            &device,
                            source.clone(),
                            &positive_input,
                            target,
                            None
                        )
                        .is_err()
                    );
                    assert!(
                        RenderedView::new(
                            &device,
                            source.clone(),
                            calibration(),
                            RollSettings::new_grade(),
                            PhotoGradeSnapshot::new(PhotoSettings::default(), vec![]).unwrap(),
                            &positive_input,
                            target,
                            None
                        )
                        .is_err()
                    );
                    previous = Some(view);
                }
            }
            assert_eq!(read(&device, &queue, &source.pixels), neutral);
        }
        eprintln!("source_working_view maximum_reference_linear_error={worst_linear}");
    });
}

#[test]
fn physical_view_source_inspection_composite_covers_missing_regional_support() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let gate = SubmissionGate::default();
        let context = egui::Context::default();
        let scan = scan();
        let full = frame(&scan);
        let regional = FrameRaster::new(
            full.composition,
            full.min,
            crate::geometry::FramePoint([full.max.0[0] / 2., full.max.0[1]]),
            [full.output[0] / 2, full.output[1]],
        )
        .unwrap();
        let spec = ViewSourceSpec::inspection(23, regional, scan.camera_to_ap1, None).unwrap();
        let mut coarse = spec.underlay().unwrap();
        coarse.frame.output = [31, 23];
        let mut sources = ViewSource::acquire_many(
            &device,
            &queue,
            &gate,
            renderer,
            kernels,
            &scan,
            None,
            vec![spec, coarse],
            || false,
        )
        .unwrap()
        .unwrap();
        let base = sources.pop().unwrap();
        let detail = sources.pop().unwrap();
        for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
            let input = ProcessingInput::from(
                ProcessingParameters::source_preview(Some(scan.fixed_wb), 0.37).unwrap(),
            );
            let a =
                RenderedView::inspection(&device, detail.clone(), &input, target, None).unwrap();
            let b = RenderedView::inspection(&device, base.clone(), &input, target, None).unwrap();
            let view = RenderedView::composite(&device, a.clone(), b.clone(), full, None).unwrap();
            view.ensure_submitted(&device, &queue, &gate, &context);
            let actual = image(&device, &queue, &view.image);
            assert!(
                actual
                    .iter()
                    .all(|p| p[3] == 1. && p.iter().all(|v| v.is_finite())),
                "source fallback leaves no holes"
            );
            let dense = image(&device, &queue, &a.image);
            for y in 0..regional.output[1] {
                for x in 0..regional.output[0] {
                    let p = actual[(x + y * full.output[0]) as usize];
                    let q = dense[(x + y * regional.output[0]) as usize];
                    for c in 0..3 {
                        assert!(
                            (p[c] - q[c]).abs() <= 2e-5,
                            "available source detail must win over fallback"
                        );
                    }
                }
            }
            let stale = ProcessingInput::from(ProcessingParameters::source_inspection(0.).unwrap());
            let stale =
                RenderedView::inspection(&device, base.clone(), &stale, target, None).unwrap();
            assert!(RenderedView::composite(&device, a, stale, full, None).is_err());
        }
    });
}
