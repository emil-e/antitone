use super::*;
use crate::adjustment_layers::AdjustmentLayerSettings;
#[cfg(feature = "ocio-reference")]
use crate::raw::sony_fixture as sony;

#[cfg(feature = "ocio-reference")]
#[test]
fn sony_corrected_source_inspection_preserves_headroom_flat_field_and_rendered_pixels() {
    pollster::block_on(async {
        let order = sony::Order::Little;
        let mut tags = sony::profile_tags(order);
        tags.push((0x7032, 8, order.signed(&[2, 8192, 8192])));
        let file = sony::source(order, tags, false, false);
        let mut scan = Scan::open(file.path(), Some([1.; 4])).unwrap();
        let codes = scan.tile_region(scan.area, 0, 0, 64, 64).unwrap();
        assert!(codes.iter().any(|p| p[..3].iter().any(|v| *v > 65535.)));
        let (device, queue, renderer, kernels) = context().await;
        let gate = SubmissionGate::default();
        let ctx = egui::Context::default();
        let cpu = crate::rendering::CpuRenderer::new(OutputTarget::Srgb).unwrap();
        let input = ProcessingInput::from(ProcessingParameters::source_inspection(0.).unwrap());
        for corrected in [false, true] {
            let field = corrected.then(|| {
                Arc::new(
                    crate::flat_field::FlatField::from_nodes(
                        [0, 0, 64, 64],
                        [2, 2],
                        vec![
                            DVec3::new(1.1, 0.9, 1.2),
                            DVec3::new(0.8, 1.2, 1.),
                            DVec3::new(1.05, 1., 1.1),
                            DVec3::new(1.2, 0.85, 0.9),
                        ],
                    )
                    .unwrap(),
                )
            });
            scan.set_flat_field(field.clone()).unwrap();
            let gpu_field = field
                .as_ref()
                .map(|f| Arc::new(GpuFlatField::new(&device, f.clone()).unwrap()));
            let spec =
                ViewSourceSpec::inspection(91, frame(&scan), scan.camera_to_ap1, field.clone())
                    .unwrap();
            let source = ViewSource::acquire(
                &device,
                &queue,
                &gate,
                renderer.clone(),
                kernels.clone(),
                &scan,
                gpu_field,
                spec,
                || false,
            )
            .unwrap()
            .unwrap();
            let expected: Vec<_> = codes
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let gain = field.as_ref().map_or(DVec3::ONE, |f| {
                        f.gain([(i % 64) as f64 + 0.5, (i / 64) as f64 + 0.5])
                            .unwrap()
                    });
                    scan.camera_to_ap1
                        * (DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64) / 65535. * gain)
                })
                .collect();
            for (a, b) in read(&device, &queue, &source.pixels).iter().zip(&expected) {
                for c in 0..3 {
                    assert!((a[c] as f64 - b[c]).abs() <= 2e-6 * b.abs().max_element().max(1.));
                }
            }
            let rendered =
                RenderedView::inspection(&device, source, &input, OutputTarget::Srgb, None)
                    .unwrap();
            rendered.ensure_submitted(&device, &queue, &gate, &ctx);
            let expected = cpu.render(&expected).unwrap();
            for (a, b) in image(&device, &queue, &rendered.image).iter().zip(expected) {
                for c in 0..3 {
                    assert!(
                        (a[c] - b[c]).abs() <= 2e-5,
                        "flat_field={corrected} {a:?} {b:?}"
                    );
                }
            }
        }
        assert_eq!(scan.tile_region(scan.area, 0, 0, 64, 64).unwrap(), codes);
    });
}

#[test]
fn physical_view_source_inspection_preserves_normalization_correction_and_reuses_linear_support() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let gate = SubmissionGate::default();
        let context = egui::Context::default();
        let mut scan = scan();
        let frame = frame(&scan);
        let spec = ViewSourceSpec::inspection(21, frame, scan.camera_to_ap1, None).unwrap();
        let positive =
            ViewSourceSpec::new(21, frame, calibration(), scan.camera_to_ap1, None).unwrap();
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
                        "no calibration, inversion, WB, contrast, saturation or layers"
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
                        &device,
                        calibration(),
                        RollSettings::default(),
                        AdjustmentLayerSettings::default(),
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
                            RollSettings::default(),
                            AdjustmentLayerSnapshot::new(vec![
                                crate::adjustment_layers::AdjustmentLayer::global(
                                    AdjustmentLayerSettings::default()
                                )
                            ])
                            .unwrap(),
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
fn source_view_applies_camera_white_balance_after_acquisition_without_changing_support() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let gate = SubmissionGate::default();
        let ctx = egui::Context::default();
        let scan = scan();
        let spec = ViewSourceSpec::inspection(27, frame(&scan), scan.camera_to_ap1, None).unwrap();
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
        let neutral = read(&device, &queue, &source.pixels);
        let gains = DVec3::new(2., 1., 0.5);
        let transform =
            scan.camera_to_ap1 * DMat3::from_diagonal(gains) * scan.camera_to_ap1.inverse();
        let ev = 0.37_f64;
        let input = ProcessingInput::from(
            ProcessingParameters::source_preview_with_transform(Some([1.; 4]), ev, transform)
                .unwrap(),
        );
        for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
            let view =
                RenderedView::inspection(&device, source.clone(), &input, target, None).unwrap();
            view.ensure_submitted(&device, &queue, &gate, &ctx);
            let actual = read(&device, &queue, &view.graded.as_ref().unwrap().pixels);
            for (actual, neutral) in actual.iter().zip(&neutral) {
                let working = DVec3::new(neutral[0] as f64, neutral[1] as f64, neutral[2] as f64);
                let camera = scan.camera_to_ap1.inverse() * working;
                let expected = scan.camera_to_ap1 * (camera * gains) * 2. * ev.exp2();
                for c in 0..3 {
                    assert!(
                        (f64::from(actual[c]) - expected[c]).abs()
                            <= 2e-5 + 2e-4 * expected[c].abs()
                    );
                }
                assert_eq!(actual[3], neutral[3]);
            }
        }
        assert_eq!(read(&device, &queue, &source.pixels), neutral);
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
