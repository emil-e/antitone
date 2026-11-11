use super::*;
use crate::adjustment_layers::AdjustmentLayerSettings;
use crate::gpu::submission::SubmissionGate;
use crate::gpu::test_inputs::AuthoredPhotograph;
use crate::gpu::view::request::SourceRequest;
use crate::{
    adjustment_layers::AdjustmentLayerSnapshot,
    color::{Balance, wb_matrix},
    dust_repair::{Repair, RepairId, RepairOrigin, SupportMask},
    gpu::repairs::fixtures::{self, Replacement},
    processing::{NegativeInput, Response},
};

fn repaired(scan: &Scan) -> (Vec<Replacement>, Arc<crate::dust_repair::EvaluatedRepair>) {
    let roll = RollSettings::default();
    let scalar = crate::processing::Pipeline::new(calibration(), roll, scan.camera_to_ap1).unwrap();
    let support =
        SupportMask::new([124, 40], [9, 9], (0..81).map(|i| i % 3 != 0).collect()).unwrap();
    let repair = Repair {
        id: RepairId(1),
        enabled: true,
        origin: RepairOrigin::Manual,
        offset: [-50, 50],
        support,
    };
    let evaluated = Arc::new(
        crate::dust_repair::evaluate(&repair, scan.area, |p| {
            Ok(scalar
                .apply(scan.pixel(p[0], p[1])?)?
                .rgb
                .as_vec3()
                .to_array())
        })
        .unwrap(),
    );
    (
        fixtures::replacements(std::slice::from_ref(&evaluated)).unwrap(),
        evaluated,
    )
}

#[test]
fn batched_fixture_demands_keep_their_own_captured_neutral_arguments() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let scan = scan();
        let (replacements, _) = repaired(&scan);
        let plain =
            SourceRequest::new(frame(&scan), calibration(), scan.camera_to_ap1, None).unwrap();
        let mut input = plain.processing_fixture();
        input.replacements = replacements;
        let changed = SourceRequest::for_processing(plain.frame(), &input).unwrap();
        let sources = ViewSource::acquire_many(
            &device,
            &queue,
            &SubmissionGate::default(),
            renderer,
            kernels,
            &scan,
            None,
            vec![plain, changed],
            || false,
        )
        .unwrap()
        .unwrap();
        assert!(
            !sources[0]
                .definition
                .same_definition(&sources[1].definition)
        );
        assert_ne!(
            read(&device, &queue, &sources[0].pixels),
            read(&device, &queue, &sources[1].pixels),
            "each demand must execute its own effective replacement, not the first batch member's capture"
        );
    });
}

#[test]
fn repaired_materialization_joins_live_consumers_after_one_waiter_retires() {
    pollster::block_on(async {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            mpsc,
        };
        let (device, queue, renderer, kernels) = context().await;
        let scan = scan();
        let (repairs, _) = repaired(&scan);
        let spec =
            SourceRequest::new(frame(&scan), calibration(), scan.camera_to_ap1, None).unwrap();
        let scope = renderer.evaluation().test_materializations(false);
        let gate = SubmissionGate::default();
        let base = ViewSource::acquire_originals_fixture(
            &device,
            &queue,
            &gate,
            renderer.clone(),
            kernels,
            &scan,
            None,
            vec![spec],
            &scope,
            || false,
        )
        .await
        .unwrap()
        .unwrap()
        .remove(0);
        let mut input = base.spec.processing_fixture();
        input.replacements = repairs;
        let repaired_spec = SourceRequest::for_processing(base.spec.frame(), &input).unwrap();
        let (entered_tx, entered_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        let release = std::sync::Mutex::new(release_rx);
        let first_submission = AtomicBool::new(true);
        gate.observe_encoded_resources(Arc::new(move || {
            if first_submission.swap(false, Ordering::SeqCst) {
                entered_tx.send(()).unwrap();
                release.lock().unwrap().recv().unwrap();
            }
        }));
        let departed = AtomicBool::new(false);
        let mut leaving = Box::pin(ViewSource::acquire_originals_fixture(
            &device,
            &queue,
            &gate,
            renderer.clone(),
            base.kernels.clone(),
            &scan,
            None,
            vec![repaired_spec.clone()],
            &scope,
            || departed.load(Ordering::SeqCst),
        ));
        let waker = futures_util::task::noop_waker();
        let mut cx = std::task::Context::from_waker(&waker);
        assert!(std::future::Future::poll(leaving.as_mut(), &mut cx).is_pending());
        entered_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        let mut first = Box::pin(ViewSource::acquire_originals_fixture(
            &device,
            &queue,
            &gate,
            renderer.clone(),
            base.kernels.clone(),
            &scan,
            None,
            vec![repaired_spec.clone()],
            &scope,
            || false,
        ));
        let mut second = Box::pin(ViewSource::acquire_originals_fixture(
            &device,
            &queue,
            &gate,
            renderer.clone(),
            base.kernels.clone(),
            &scan,
            None,
            vec![repaired_spec.clone()],
            &scope,
            || false,
        ));
        assert!(std::future::Future::poll(first.as_mut(), &mut cx).is_pending());
        assert!(std::future::Future::poll(second.as_mut(), &mut cx).is_pending());
        departed.store(true, Ordering::SeqCst);
        assert!(matches!(
            std::future::Future::poll(leaving.as_mut(), &mut cx),
            std::task::Poll::Ready(Ok(None))
        ));
        drop(leaving);
        release_tx.send(()).unwrap();
        let (first, second) = futures_util::future::join(first, second).await;
        let first = first.unwrap().unwrap().remove(0);
        let second = second.unwrap().unwrap().remove(0);
        assert!(
            first.same_pixels_as(&second),
            "independent live consumers join one repaired value"
        );
        assert_ne!(
            read(&device, &queue, &first.pixels),
            read(&device, &queue, &base.pixels),
            "fixture must execute an effective repair"
        );
        assert_eq!(renderer.evaluation().retained_bytes(), 0);
    });
}

#[allow(clippy::too_many_arguments)]
fn render_repaired(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    scan: &Scan,
    scope: &crate::gpu::evaluation::Materializations,
    base: Arc<ViewSource>,
    replacements: &[Replacement],
    (roll, photo): (RollSettings, AdjustmentLayerSettings),
    target: OutputTarget,
    previous: Option<&Arc<RenderedView>>,
) -> Arc<RenderedView> {
    let mut input = processing_input(device, calibration(), roll, photo, scan);
    input.replacements = replacements.to_vec();
    let spec = SourceRequest::for_processing(base.spec.frame(), &input).unwrap();
    let source = pollster::block_on(ViewSource::acquire_originals_fixture(
        device,
        queue,
        &SubmissionGate::default(),
        base.renderer.clone(),
        base.kernels.clone(),
        scan,
        None,
        vec![spec],
        scope,
        || false,
    ))
    .unwrap()
    .unwrap()
    .remove(0);
    let view = RenderedView::new(device, source, &input, target, previous).unwrap();
    view.ensure_submitted(
        device,
        queue,
        &SubmissionGate::default(),
        &egui::Context::default(),
    );
    view
}

fn filter_authored_repairs(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    source: Arc<ViewSource>,
    frame: FrameRaster,
    replacements: &[Replacement],
) -> Arc<ViewSource> {
    use crate::gpu::evaluation as e;
    let area = source.spec.area;
    assert_eq!(source.spec.frame().output, [area.width, area.height]);
    let domain = e::Domain {
        bounds: Some(area.try_into().unwrap()),
        grid: e::Grid::axis_aligned([0.; 2], [1.; 2]),
        meaning: e::Meaning::Neutral,
    };
    let base = fixtures::image(domain, read(device, queue, &source.pixels)).unwrap();
    let repaired =
        crate::gpu::repairs::definition::compose_images(base, replacements.to_vec()).unwrap();
    let spec = source.spec.clone().with_frame(frame).unwrap();
    let root = e::reconstruct(
        repaired,
        e::Domain {
            bounds: Some(spec.input_region().unwrap()),
            grid: e::Grid {
                coverage: 8,
                ..e::Grid::from_frame(frame)
            },
            meaning: e::Meaning::Neutral,
        },
    )
    .unwrap();
    let bounds = root.domain().finite().unwrap();
    let evaluation = source.renderer.evaluation();
    let request = evaluation
        .request(
            root.clone(),
            e::Region {
                bounds,
                grid: root.domain().grid,
            },
            e::Budget {
                tile: [bounds.width, bounds.height],
                scratch_bytes: 4 << 20,
            },
        )
        .unwrap();
    let scope = evaluation.materializations();
    let gate = SubmissionGate::default();
    let value = pollster::block_on(
        evaluation
            .materialize(
                queue,
                &gate,
                &scope,
                request,
                e::ImageFormat::Rgba32Float,
                e::ImageBacking::Buffer,
            )
            .unwrap(),
    )
    .unwrap();
    ViewSource::from_acquired(
        source.kernels.clone(),
        Arc::new(crate::gpu::view::acquisition::AcquiredViewRaster {
            spec,
            renderer: source.renderer.clone(),
            definition: root.clone(),
            pixels: value.try_into().unwrap(),
        }),
    )
}

#[test]
fn sparse_repair_projection_retains_signed_hdr_and_ignores_replaced_values() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let area = crate::geometry::SourceRegion {
            x: 11,
            y: 17,
            width: 64,
            height: 48,
        };
        let frame = FrameRaster::full(
            Composition::new(area, Default::default(), Default::default()).unwrap(),
        )
        .unwrap();
        let support = SupportMask::new([32, 32], [5, 5], vec![true; 25]).unwrap();
        let values = (0..64 * 48)
            .map(|i| {
                let point = [area.x + i % 64, area.y + i / 64];
                if support.contains(point) {
                    [1e30, 1e30, 1e30, 11.]
                } else {
                    [-0.02 + (i % 64) as f32 * 0.0001, 0.1, 4., 11.]
                }
            })
            .collect::<Vec<_>>();
        let repair = Repair {
            id: RepairId(1),
            enabled: true,
            origin: RepairOrigin::Manual,
            offset: [20, 0],
            support,
        };
        let evaluated = Arc::new(
            crate::dust_repair::evaluate(&repair, area, |p| {
                let v = values[((p[1] - area.y) * area.width + p[0] - area.x) as usize];
                Ok([v[0], v[1], v[2]])
            })
            .unwrap(),
        );
        let patches = fixtures::replacements(std::slice::from_ref(&evaluated)).unwrap();
        let original_source = colour::authored_source(&device, renderer, kernels, &values, frame);
        let source =
            filter_authored_repairs(&device, &queue, original_source.clone(), frame, &patches);
        let mut reduced = frame;
        reduced.output = [17, 13];
        let filtered =
            filter_authored_repairs(&device, &queue, original_source.clone(), reduced, &patches);
        let reduced_values = read(&device, &queue, &filtered.pixels);
        let mut alternative = values.clone();
        for p in evaluated.support().selected_points() {
            alternative[((p[1] - area.y) * area.width + p[0] - area.x) as usize] =
                [-1e20, 1e-20, 1e20, 11.];
        }
        let alternative = colour::authored_source(
            &device,
            original_source.renderer.clone(),
            original_source.kernels.clone(),
            &alternative,
            frame,
        );
        let alternative = filter_authored_repairs(&device, &queue, alternative, reduced, &patches);
        assert_eq!(
            reduced_values
                .iter()
                .map(|p| p.map(f32::to_bits))
                .collect::<Vec<_>>(),
            read(&device, &queue, &alternative.pixels)
                .iter()
                .map(|p| p.map(f32::to_bits))
                .collect::<Vec<_>>()
        );
        for (i, pixel) in reduced_values.iter().enumerate() {
            let footprint = reduced
                .footprint([(i as u32) % 17, (i as u32) / 17])
                .unwrap();
            let expected = footprint
                .pixels(area)
                .unwrap()
                .fold(DVec3::ZERO, |sum, (p, w)| {
                    let v = values[((p[1] - area.y) * area.width + p[0] - area.x) as usize];
                    sum + DVec3::from_array(evaluated.pixel(p, [v[0], v[1], v[2]]).map(f64::from))
                        * w
                })
                / footprint.area();
            for c in 0..3 {
                assert!(
                    (pixel[c] as f64 - expected[c]).abs() <= 2e-6 + 2e-5 * expected[c].abs(),
                    "mixed HDR {i}/{c}: {pixel:?} vs {expected:?}"
                );
            }
        }
        let negative = NegativeInput {
            calibration: calibration(),
            response: Response::film(),
            gamma: 0.65,
            balance: Balance::default(),
        };
        let roll = RollSettings::default();
        let mut input: AuthoredPhotograph = Arc::new(
            AuthoredPhotograph::new(
                &device.limits(),
                negative,
                roll,
                AdjustmentLayerSnapshot::new(vec![
                    crate::adjustment_layers::AdjustmentLayer::global(Default::default()),
                ])
                .unwrap(),
                source.spec.matrix(),
                area,
            )
            .unwrap(),
        )
        .into();
        input.replacements = patches;
        let view =
            RenderedView::new(&device, source.clone(), &input, OutputTarget::Srgb, None).unwrap();
        view.ensure_submitted(
            &device,
            &queue,
            &SubmissionGate::default(),
            &egui::Context::default(),
        );
        let actual = read(
            &device,
            &queue,
            &view.graded.as_ref().unwrap().source.pixels,
        );
        for (i, (pixel, base)) in actual.iter().zip(&values).enumerate() {
            let point = [area.x + i as u32 % 64, area.y + i as u32 / 64];
            if evaluated.support().contains(point) {
                let expected = evaluated.pixel(point, [0.; 3]);
                assert!(pixel[0] < 0. && pixel[2] > 1.);
                for c in 0..3 {
                    assert!((pixel[c] - expected[c]).abs() < 1e-6);
                }
            } else {
                assert_eq!(pixel.map(f32::to_bits), base.map(f32::to_bits));
            }
            assert_eq!(pixel[3], base[3]);
        }
    });
}

#[test]
fn neutral_repairs_filter_before_grading_preserve_known_pixels_and_reuse_gpu_content() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) =
            context_with_cache_policy(crate::gpu::evaluation::CachePolicy { generated_bytes: 0 })
                .await;
        let scope = renderer.evaluation().test_materializations(false);
        let scan = scan();
        let (patches, evaluated) = repaired(&scan);
        let scalar = crate::processing::Pipeline::new(
            calibration(),
            RollSettings::default(),
            scan.camera_to_ap1,
        )
        .unwrap();
        let native = frame(&scan);
        let mut rotated = native;
        rotated.composition = Composition::with_crop(
            scan.area,
            Default::default(),
            Default::default(),
            Some(crate::crop::Crop {
                center: [128., 88.],
                size: [180, 120],
                angle: 0.19,
            }),
        )
        .unwrap();
        rotated = FrameRaster::full(rotated.composition).unwrap();
        rotated.output = [31, 23];
        for frame in [native, rotated] {
            let spec = SourceRequest::new(frame, calibration(), scan.camera_to_ap1, None).unwrap();
            let source = ViewSource::acquire_originals_fixture(
                &device,
                &queue,
                &SubmissionGate::default(),
                renderer.clone(),
                kernels.clone(),
                &scan,
                None,
                vec![spec],
                &scope,
                || false,
            )
            .await
            .unwrap()
            .unwrap()
            .remove(0);
            let baseline = read(&device, &queue, &source.pixels);
            let first = render_repaired(
                &device,
                &queue,
                &scan,
                &scope,
                source.clone(),
                &patches,
                (RollSettings::default(), AdjustmentLayerSettings::default()),
                OutputTarget::Srgb,
                None,
            );
            let acquired = &first.graded.as_ref().unwrap().source;
            let mut captured = acquired.spec.processing_fixture();
            captured.replacements = patches.clone();
            let (definition, _) = crate::gpu::photograph::PhotographGraph::preview_with_input(
                &device.limits(),
                crate::gpu::evaluation::camera_source(&scan).unwrap(),
                None,
                &captured,
                acquired.spec.frame(),
                None,
            )
            .unwrap();
            assert!(
                acquired.definition.same_definition(&definition.neutral),
                "sparse production supplies the native value named by the complete graph"
            );
            let actual = read(&device, &queue, &acquired.pixels);
            let mut touched = 0;
            for (i, pixel) in actual.iter().enumerate() {
                let footprint = frame
                    .footprint([(i as u32) % frame.output[0], (i as u32) / frame.output[0]])
                    .unwrap();
                let mut expected = DVec3::ZERO;
                let mut repaired = false;
                for (p, w) in footprint.pixels(scan.area).unwrap() {
                    let base = scalar.apply(scan.pixel(p[0], p[1]).unwrap()).unwrap().rgb;
                    if evaluated.support().contains(p) {
                        expected +=
                            DVec3::from_array(evaluated.pixel(p, [0.; 3]).map(f64::from)) * w;
                        repaired = true;
                    } else {
                        expected += base * w;
                    }
                }
                expected /= footprint.area();
                assert_eq!(pixel[3], baseline[i][3]);
                if !repaired {
                    assert_eq!(pixel.map(f32::to_bits), baseline[i].map(f32::to_bits));
                } else {
                    touched += 1;
                    for c in 0..3 {
                        assert!(
                            (pixel[c] as f64 - expected[c]).abs()
                                <= 2e-6 + 2e-5 * expected[c].abs(),
                            "{i}/{c}: {pixel:?} vs {expected:?}"
                        );
                    }
                }
            }
            assert!(touched > 0);
            assert_eq!(read(&device, &queue, &source.pixels), baseline);
            let roll = RollSettings {
                exposure: 0.7,
                wb: Balance {
                    warmth: 0.07,
                    tint: -0.03,
                },
            };
            let photo = AdjustmentLayerSettings {
                exposure: 0.4,
                contrast: 1.1,
                saturation: 0.8,
                ..AdjustmentLayerSettings::default()
            };
            let graded = render_repaired(
                &device,
                &queue,
                &scan,
                &scope,
                source.clone(),
                &patches,
                (roll, photo),
                OutputTarget::Srgb,
                Some(&first),
            );
            let same_acquired = &graded.graded.as_ref().unwrap().source;
            assert!(
                acquired.same_pixels_as(same_acquired),
                "downstream grading reuses the repaired GPU value"
            );
            assert!(
                acquired
                    .definition
                    .same_definition(&same_acquired.definition)
            );
            let values = read(&device, &queue, &graded.graded.as_ref().unwrap().pixels);
            let post = wb_matrix(photo.wb).unwrap()
                * wb_matrix(roll.wb).unwrap()
                * (roll.exposure + photo.exposure).exp2();
            let grade = crate::photographic::Grade::new(photo).unwrap();
            for (neutral, pixel) in actual.iter().zip(values) {
                let expected = grade
                    .apply(
                        post * DVec3::new(neutral[0] as f64, neutral[1] as f64, neutral[2] as f64),
                    )
                    .unwrap();
                for c in 0..3 {
                    assert!(
                        (pixel[c] as f64 - expected[c]).abs() <= 2e-5 + 2e-4 * expected[c].abs()
                    );
                }
            }
            let p3 = render_repaired(
                &device,
                &queue,
                &scan,
                &scope,
                source.clone(),
                &patches,
                (roll, photo),
                OutputTarget::DisplayP3,
                Some(&graded),
            );
            assert!(Arc::ptr_eq(
                graded.graded.as_ref().unwrap(),
                p3.graded.as_ref().unwrap()
            ));
            let cleared = render_repaired(
                &device,
                &queue,
                &scan,
                &scope,
                source.clone(),
                &[],
                (roll, photo),
                OutputTarget::DisplayP3,
                Some(&p3),
            );
            assert!(
                cleared
                    .graded
                    .as_ref()
                    .unwrap()
                    .source
                    .same_pixels_as(&source)
            );
            assert_eq!(read(&device, &queue, &source.pixels), baseline);
        }
        assert_eq!(renderer.evaluation().retained_bytes(), 0);
    });
}

#[test]
fn repair_instruction_changes_restore_removed_support_and_leave_published_inputs_immutable() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let scan = scan();
        let (first, _) = repaired(&scan);
        let first = Repair {
            id: RepairId(1),
            enabled: true,
            origin: RepairOrigin::Manual,
            offset: [-50, 50],
            support: first[0].0.clone(),
        };
        let second = Repair {
            id: RepairId(2),
            enabled: true,
            origin: RepairOrigin::Manual,
            offset: [-60, 60],
            support: SupportMask::new([134, 45], [3, 3], vec![true; 9]).unwrap(),
        };
        let mut frame = frame(&scan);
        frame.output = [31, 23];
        let clean = SourceRequest::new(frame, calibration(), scan.camera_to_ap1, None).unwrap();
        let scope = renderer.evaluation().test_materializations(false);
        let build = |repairs: Vec<Repair>, cancelled: bool| {
            let mut input = clean.processing_fixture();
            input.repairs = repairs;
            pollster::block_on(ViewSource::acquire_originals_fixture(
                &device,
                &queue,
                &SubmissionGate::default(),
                renderer.clone(),
                kernels.clone(),
                &scan,
                None,
                vec![SourceRequest::for_processing(frame, &input).unwrap()],
                &scope,
                || cancelled,
            ))
            .map(|values| values.map(|mut values| values.remove(0)))
        };
        let base = build(Vec::new(), false).unwrap().unwrap();
        let baseline = read(&device, &queue, &base.pixels);
        let one = build(vec![first.clone()], false).unwrap().unwrap();
        let one_pixels = read(&device, &queue, &one.pixels);
        assert_ne!(
            one_pixels, baseline,
            "the first repair must affect actual sampled values"
        );
        assert!(
            build(vec![first.clone(), second.clone()], true)
                .unwrap()
                .is_none()
        );
        assert_eq!(read(&device, &queue, &one.pixels), one_pixels);
        let both = build(vec![first.clone(), second.clone()], false)
            .unwrap()
            .unwrap();
        let reversed = build(vec![second.clone(), first], false).unwrap().unwrap();
        assert_eq!(
            read(&device, &queue, &both.pixels),
            read(&device, &queue, &reversed.pixels)
        );
        let without_first = build(vec![second.clone()], false).unwrap().unwrap();
        let scalar =
            crate::processing::InputPipeline::new(calibration(), scan.camera_to_ap1).unwrap();
        let second_values = crate::dust_repair::evaluate(&second, scan.area, |point| {
            Ok(scalar
                .apply(scan.pixel(point[0], point[1])?)?
                .rgb
                .as_vec3()
                .to_array())
        })
        .unwrap();
        let removal = read(&device, &queue, &without_first.pixels);
        let mut effective_second = 0;
        let mut restored_first = 0;
        for (i, pixel) in removal.iter().enumerate() {
            let footprint = frame.footprint([i as u32 % 31, i as u32 / 31]).unwrap();
            let influenced = footprint
                .pixels(scan.area)
                .unwrap()
                .any(|(p, _)| second.support.contains(p));
            if !influenced {
                assert_eq!(pixel.map(f32::to_bits), baseline[i].map(f32::to_bits));
                if one_pixels[i].map(f32::to_bits) != baseline[i].map(f32::to_bits) {
                    restored_first += 1;
                }
            } else {
                let expected =
                    footprint
                        .pixels(scan.area)
                        .unwrap()
                        .fold(DVec3::ZERO, |sum, (p, w)| {
                            let base = scalar.apply(scan.pixel(p[0], p[1]).unwrap()).unwrap().rgb;
                            sum + DVec3::from_array(
                                second_values
                                    .pixel(p, base.as_vec3().to_array())
                                    .map(f64::from),
                            ) * w
                        })
                        / footprint.area();
                for c in 0..3 {
                    assert!(
                        (f64::from(pixel[c]) - expected[c]).abs()
                            <= 2e-6 + 2e-5 * expected[c].abs()
                    );
                }
                if (0..3).any(|c| (pixel[c] - baseline[i][c]).abs() > 2e-6) {
                    effective_second += 1;
                }
            }
            assert_eq!(pixel[3], baseline[i][3]);
        }
        assert!(
            effective_second > 0,
            "retaining the second repair must differ from removing both"
        );
        assert!(
            restored_first > 0,
            "removing the first repair must restore its effective, independent samples"
        );
        let mut shrunk = second;
        shrunk.support = SupportMask::new([135, 46], [1, 1], vec![true]).unwrap();
        let updated = build(vec![shrunk.clone()], false).unwrap().unwrap();
        let actual = read(&device, &queue, &updated.pixels);
        let evaluated = crate::dust_repair::evaluate(&shrunk, scan.area, |point| {
            Ok(scalar
                .apply(scan.pixel(point[0], point[1])?)?
                .rgb
                .as_vec3()
                .to_array())
        })
        .unwrap();
        let mut touched = 0;
        for (i, pixel) in actual.iter().enumerate() {
            let footprint = frame.footprint([i as u32 % 31, i as u32 / 31]).unwrap();
            let contains = footprint
                .pixels(scan.area)
                .unwrap()
                .any(|(p, _)| shrunk.support.contains(p));
            if !contains {
                assert_eq!(pixel.map(f32::to_bits), baseline[i].map(f32::to_bits));
            } else {
                touched += 1;
                let expected =
                    footprint
                        .pixels(scan.area)
                        .unwrap()
                        .fold(DVec3::ZERO, |sum, (p, w)| {
                            let base = scalar.apply(scan.pixel(p[0], p[1]).unwrap()).unwrap().rgb;
                            sum + DVec3::from_array(
                                evaluated.pixel(p, base.as_vec3().to_array()).map(f64::from),
                            ) * w
                        })
                        / footprint.area();
                for c in 0..3 {
                    assert!(
                        (f64::from(pixel[c]) - expected[c]).abs()
                            <= 2e-6 + 2e-5 * expected[c].abs()
                    );
                }
            }
            assert_eq!(pixel[3], baseline[i][3]);
        }
        assert!(touched > 0);
        let reset = build(Vec::new(), false).unwrap().unwrap();
        assert!(
            reset.same_pixels_as(&base),
            "the held clean graph value remains reusable"
        );
        assert_eq!(read(&device, &queue, &base.pixels), baseline);
        assert_eq!(read(&device, &queue, &one.pixels), one_pixels);
    });
}

#[cfg(feature = "ocio-reference")]
#[test]
fn original_reference_repairs_are_pre_grade_and_leave_other_rendered_pixels_bit_exact() {
    pollster::block_on(async {
        let (device, queue, renderer, _) = context().await;
        let scan = scan();
        let (patches, evaluated) = repaired(&scan);
        let roll = RollSettings {
            exposure: 0.8,
            wb: Balance {
                warmth: 0.06,
                tint: -0.04,
            },
        };
        let photo = AdjustmentLayerSettings {
            exposure: 0.3,
            contrast: 1.15,
            saturation: 0.8,
            ..AdjustmentLayerSettings::default()
        };
        let mut input = processing_input(&device, calibration(), roll, photo, &scan);
        let frame = frame(&scan);
        let request = crate::refinement::ViewRequest {
            area: scan.area,
            output: frame.output,
            frame: Some(frame),
        };
        let generation = AtomicU64::new(1);
        let render = |input| {
            crate::refinement::refine(
                &renderer,
                &device,
                (&queue, &SubmissionGate::default()),
                (&scan, None),
                input,
                OutputTarget::Srgb,
                (&generation, 1),
                request,
            )
            .unwrap()
            .unwrap()
        };
        let before = image(&device, &queue, &render(input.clone()));
        input.replacements = patches;
        let actual = image(&device, &queue, &render(input));
        let post = wb_matrix(photo.wb).unwrap()
            * wb_matrix(roll.wb).unwrap()
            * (roll.exposure + photo.exposure).exp2();
        let grade = crate::photographic::Grade::new(photo).unwrap();
        let cpu = crate::rendering::CpuRenderer::new(OutputTarget::Srgb).unwrap();
        let values = evaluated
            .pixels()
            .map(|(_, v)| {
                grade
                    .apply(post * DVec3::from_array(v.map(f64::from)))
                    .unwrap()
            })
            .collect::<Vec<_>>();
        let expected = cpu.render(&values).unwrap();
        for (i, ((point, _), expected)) in evaluated.pixels().zip(expected).enumerate() {
            let at = (point[1] - scan.area.y) * scan.area.width + point[0] - scan.area.x;
            for c in 0..3 {
                assert!(
                    (actual[at as usize][c] - expected[c]).abs() < 0.001,
                    "patch {i}/{c}"
                );
            }
        }
        for (i, (a, b)) in actual.iter().zip(before).enumerate() {
            let point = [
                scan.area.x + (i as u32) % scan.area.width,
                scan.area.y + (i as u32) / scan.area.width,
            ];
            if !evaluated.support().contains(point) {
                assert_eq!(a.map(f32::to_bits), b.map(f32::to_bits));
            }
        }
    });
}
