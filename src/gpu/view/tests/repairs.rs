use super::*;
use crate::adjustment_layers::AdjustmentLayerSettings;
use crate::{
    adjustment_layers::AdjustmentLayerSnapshot,
    color::{Balance, wb_matrix},
    dust_repair::{Repair, RepairId, RepairOrigin, SupportMask},
    gpu::repairs::{Patch, Snapshot},
    processing::{NegativeInput, Response},
};

fn repaired(
    scan: &Scan,
    device: &wgpu::Device,
) -> (Snapshot, Arc<crate::dust_repair::EvaluatedRepair>) {
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
        Snapshot(vec![Patch::new(device, evaluated.clone()).unwrap()]),
        evaluated,
    )
}

fn render_repaired(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    (source, scan): (Arc<ViewSource>, &Scan),
    snapshot: &Snapshot,
    (roll, photo): (RollSettings, AdjustmentLayerSettings),
    target: OutputTarget,
    previous: Option<&Arc<RenderedView>>,
) -> Arc<RenderedView> {
    let source = ViewSource::reconstruct_repairs(
        device,
        queue,
        &SubmissionGate::default(),
        scan,
        None,
        source,
        snapshot.clone(),
        previous
            .and_then(|p| p.graded.as_ref())
            .map(|g| g.source.clone()),
        || false,
    )
    .unwrap()
    .unwrap();
    let mut input: ProcessingInput = crate::adjustment_layers::AdjustmentLayerSnapshot::new(vec![
        crate::adjustment_layers::AdjustmentLayer::global(photo),
    ])
    .and_then(|snapshot| {
        crate::gpu::GpuPipeline::new(
            device,
            calibration(),
            roll,
            snapshot,
            source.spec.matrix,
            source.spec.area,
        )
    })
    .map(Arc::new)
    .unwrap()
    .into();
    input.repairs = snapshot.clone();
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
    repairs: &Snapshot,
) -> Arc<ViewSource> {
    let area = source.spec.area;
    assert_eq!(source.spec.frame().output, [area.width, area.height]);
    let working = float_buffer(device, source.spec.frame().output).unwrap();
    let pixels = float_buffer(device, frame.output).unwrap();
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: area.width,
            height: area.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(&source.pixels, 0, &working, 0, working.size());
    source
        .renderer
        .repairs
        .neutral(
            device,
            &mut encoder,
            &working,
            repairs,
            SourceGeometry::new(
                [area.x, area.y, area.width, area.height],
                [area.width, area.height],
            )
            .unwrap(),
        )
        .unwrap();
    source
        .kernels
        .acquisition
        .copy(
            &source.renderer.evaluation,
            &mut encoder,
            &working,
            &texture,
        )
        .unwrap();
    let bounds = frame.bounds().unwrap();
    source
        .kernels
        .acquisition
        .filter(
            &source.renderer.evaluation,
            &mut encoder,
            &texture,
            &pixels,
            frame,
            [
                area.x as i32 - bounds.x as i32,
                area.y as i32 - bounds.y as i32,
            ],
            ([0, 0], frame.output),
        )
        .unwrap();
    let submission = queue.submit([encoder.finish()]);
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })
        .unwrap();
    let mut spec = source.spec.clone().with_repairs(repairs.clone());
    spec = spec.with_frame(frame).unwrap();
    Arc::new(ViewSource {
        spec,
        pixels,
        renderer: source.renderer.clone(),
        kernels: source.kernels.clone(),
    })
}

#[test]
fn sparse_repair_projection_retains_signed_hdr_and_repair_values_extend_range_preflight() {
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
        let patches = Snapshot(vec![Patch::new(&device, evaluated.clone()).unwrap()]);
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
        let mut input: ProcessingInput = Arc::new(
            GpuPipeline::new(
                &device,
                negative,
                roll,
                AdjustmentLayerSnapshot::new(vec![
                    crate::adjustment_layers::AdjustmentLayer::global(Default::default()),
                ])
                .unwrap(),
                source.spec.matrix,
                area,
            )
            .unwrap(),
        )
        .into();
        input.repairs = patches;
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
        let roll = RollSettings {
            exposure: 16.,
            ..roll
        };
        let photo = AdjustmentLayerSettings {
            exposure: 16.,
            ..Default::default()
        };
        crate::adjustment_layers::AdjustmentLayerSnapshot::new(vec![
            crate::adjustment_layers::AdjustmentLayer::global(photo),
        ])
        .and_then(|snapshot| {
            crate::gpu::GpuPipeline::new(
                &device,
                calibration(),
                roll,
                snapshot,
                source.spec.matrix,
                source.spec.area,
            )
        })
        .map(Arc::new)
        .unwrap();
        assert!(
            GpuPipeline::validate_repaired(
                &device,
                calibration(),
                roll,
                &AdjustmentLayerSnapshot::new(vec![
                    crate::adjustment_layers::AdjustmentLayer::global(photo)
                ])
                .unwrap(),
                source.spec.matrix,
                area,
                DVec3::splat(1e30)
            )
            .is_err()
        );
    });
}

#[test]
fn neutral_repairs_filter_before_grading_preserve_known_pixels_and_reuse_gpu_content() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let scan = scan();
        let (patches, evaluated) = repaired(&scan, &device);
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
            let spec =
                ViewSourceSpec::new(1, frame, calibration(), scan.camera_to_ap1, None).unwrap();
            let source = ViewSource::acquire(
                &device,
                &queue,
                &SubmissionGate::default(),
                renderer.clone(),
                kernels.clone(),
                &scan,
                None,
                spec,
                || false,
            )
            .unwrap()
            .unwrap();
            let baseline = read(&device, &queue, &source.pixels);
            let first = render_repaired(
                &device,
                &queue,
                (source.clone(), &scan),
                &patches,
                (RollSettings::default(), AdjustmentLayerSettings::default()),
                OutputTarget::Srgb,
                None,
            );
            let acquired = &first.graded.as_ref().unwrap().source;
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
                (source.clone(), &scan),
                &patches,
                (roll, photo),
                OutputTarget::Srgb,
                Some(&first),
            );
            let same_acquired = &graded.graded.as_ref().unwrap().source;
            assert!(Arc::ptr_eq(acquired, same_acquired));
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
                (source.clone(), &scan),
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
                (source.clone(), &scan),
                &Snapshot::default(),
                (roll, photo),
                OutputTarget::DisplayP3,
                Some(&p3),
            );
            assert!(Arc::ptr_eq(
                &cleared.graded.as_ref().unwrap().source,
                &source
            ));
            assert_eq!(read(&device, &queue, &source.pixels), baseline);
        }
    });
}

#[test]
fn repaired_region_updates_compose_independently_restore_removed_support_and_cancel_atomically() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let scan = scan();
        let (first, _) = repaired(&scan, &device);
        let scalar =
            crate::processing::InputPipeline::new(calibration(), scan.camera_to_ap1).unwrap();
        let second_repair = Repair {
            id: RepairId(2),
            enabled: true,
            origin: RepairOrigin::Manual,
            offset: [-60, 60],
            support: SupportMask::new([134, 45], [3, 3], vec![true; 9]).unwrap(),
        };
        let evaluate = |repair: &Repair| {
            Arc::new(
                crate::dust_repair::evaluate(repair, scan.area, |p| {
                    Ok(scalar
                        .apply(scan.pixel(p[0], p[1])?)?
                        .rgb
                        .as_vec3()
                        .to_array())
                })
                .unwrap(),
            )
        };
        let second = Patch::new(&device, evaluate(&second_repair)).unwrap();
        let both = Snapshot(vec![first.0[0].clone(), second.clone()]);
        let mut frame = frame(&scan);
        frame.output = [31, 23];
        let base = ViewSource::acquire(
            &device,
            &queue,
            &SubmissionGate::default(),
            renderer,
            kernels,
            &scan,
            None,
            ViewSourceSpec::new(1, frame, calibration(), scan.camera_to_ap1, None).unwrap(),
            || false,
        )
        .unwrap()
        .unwrap();
        let baseline = read(&device, &queue, &base.pixels);
        let build = |snapshot: Snapshot, previous: Option<Arc<ViewSource>>| {
            ViewSource::reconstruct_repairs(
                &device,
                &queue,
                &SubmissionGate::default(),
                &scan,
                None,
                base.clone(),
                snapshot,
                previous,
                || false,
            )
            .unwrap()
            .unwrap()
        };
        let one = build(first.clone(), None);
        let one_pixels = read(&device, &queue, &one.pixels);
        assert!(
            ViewSource::reconstruct_repairs(
                &device,
                &queue,
                &SubmissionGate::default(),
                &scan,
                None,
                one.clone(),
                both.clone(),
                None,
                || false
            )
            .is_err()
        );
        assert!(
            ViewSource::reconstruct_repairs(
                &device,
                &queue,
                &SubmissionGate::default(),
                &scan,
                None,
                base.clone(),
                both.clone(),
                Some(one.clone()),
                || true
            )
            .unwrap()
            .is_none()
        );
        assert_eq!(read(&device, &queue, &one.pixels), one_pixels);
        let combined = build(both.clone(), Some(one));
        let reversed = build(Snapshot(vec![second.clone(), first.0[0].clone()]), None);
        let forward = build(both, None);
        assert_eq!(
            read(&device, &queue, &forward.pixels),
            read(&device, &queue, &reversed.pixels)
        );
        for (a, b) in read(&device, &queue, &combined.pixels).iter().zip(read(
            &device,
            &queue,
            &forward.pixels,
        )) {
            assert_eq!(a[3], b[3]);
            for c in 0..3 {
                assert!((a[c] - b[c]).abs() <= 2e-6 + 2e-5 * b[c].abs());
            }
        }
        let without_first = build(Snapshot(vec![second.clone()]), Some(combined));
        let mut shrunk = second_repair;
        shrunk.support = SupportMask::new([135, 46], [1, 1], vec![true]).unwrap();
        let shrunk = Patch::new(&device, evaluate(&shrunk)).unwrap();
        let updated = build(Snapshot(vec![shrunk.clone()]), Some(without_first));
        let fresh = build(Snapshot(vec![shrunk.clone()]), None);
        let expected = read(&device, &queue, &fresh.pixels);
        let actual = read(&device, &queue, &updated.pixels);
        for (i, pixel) in actual.iter().enumerate() {
            let footprint = frame.footprint([(i as u32) % 31, (i as u32) / 31]).unwrap();
            let repaired = footprint
                .pixels(scan.area)
                .unwrap()
                .any(|(p, _)| shrunk.evaluated.support().contains(p));
            if !repaired {
                assert_eq!(pixel.map(f32::to_bits), baseline[i].map(f32::to_bits));
            } else {
                for c in 0..3 {
                    assert!(
                        (pixel[c] - expected[i][c]).abs() <= 2e-6 + 2e-5 * expected[i][c].abs()
                    );
                }
            }
        }
        let reset = build(Snapshot::default(), Some(updated));
        assert!(Arc::ptr_eq(&reset, &base));
        assert_eq!(read(&device, &queue, &base.pixels), baseline);
    });
}

#[cfg(feature = "ocio-reference")]
#[test]
fn original_reference_repairs_are_pre_grade_and_leave_other_rendered_pixels_bit_exact() {
    pollster::block_on(async {
        let (device, queue, renderer, _) = context().await;
        let scan = scan();
        let (patches, evaluated) = repaired(&scan, &device);
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
        input.repairs = patches;
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
