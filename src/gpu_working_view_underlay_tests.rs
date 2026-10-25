use super::*;
use crate::processing::{NegativeInput, Response};

fn source(
    device: &wgpu::Device,
    renderer: &Arc<crate::renderer::GpuRenderer>,
    kernels: &Arc<ViewKernels>,
    spec: ViewSourceSpec,
    pixels: &[[f32; 4]],
) -> Arc<ViewSource> {
    assert_eq!(
        pixels.len(),
        (spec.frame.output[0] * spec.frame.output[1]) as usize
    );
    Arc::new(ViewSource {
        spec,
        renderer: renderer.clone(),
        kernels: kernels.clone(),
        pixels: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("synthetic signed/HDR coverage chart"),
            contents: bytemuck::cast_slice(pixels),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        }),
    })
}

fn sample(pixels: &[[f32; 4]], size: [u32; 2], uv: [f64; 2]) -> [f64; 4] {
    if uv.into_iter().any(|v| !(0. ..1.).contains(&v)) {
        return [0.; 4];
    }
    let at = std::array::from_fn::<_, 2, _>(|c| uv[c] * f64::from(size[c]) - 0.5);
    let lo = at.map(|v| v.floor() as i32);
    let fraction = at.map(|v| v - v.floor());
    let mut sum = [0.; 4];
    for y in 0..2 {
        for x in 0..2 {
            let weight = (if x == 0 {
                1. - fraction[0]
            } else {
                fraction[0]
            }) * (if y == 0 {
                1. - fraction[1]
            } else {
                fraction[1]
            });
            let xy = [x, y];
            let point = std::array::from_fn::<_, 2, _>(|c| {
                (lo[c] + xy[c]).clamp(0, size[c] as i32 - 1) as u32
            });
            let value = pixels[(point[0] + point[1] * size[0]) as usize];
            for c in 0..3 {
                sum[c] += f64::from(value[c]) * f64::from(value[3]) * weight;
            }
            sum[3] += f64::from(value[3]) * weight;
        }
    }
    sum
}

fn mapped(map: TextureMapping, uv: [f64; 2]) -> [f64; 2] {
    std::array::from_fn(|c| {
        f64::from(map.origin[c])
            + uv[0] * f64::from(map.x_axis[c])
            + uv[1] * f64::from(map.y_axis[c])
    })
}

#[test]
fn physical_view_underlay_paired_acquisition_keeps_each_original_footprint_and_floor_flags() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let gate = SubmissionGate::default();
        for corrected in [false, true] {
            let mut scan = scan();
            let field = corrected.then(|| {
                Arc::new(
                    crate::flat_field::FlatField::from_nodes(
                        [scan.area.x, scan.area.y, scan.area.width, scan.area.height],
                        [2, 2],
                        vec![
                            DVec3::new(0.8, 0.9, 1.),
                            DVec3::new(1.2, 1.1, 1.05),
                            DVec3::new(1., 1.1, 0.9),
                            DVec3::new(1.1, 0.95, 1.2),
                        ],
                    )
                    .unwrap(),
                )
            });
            scan.set_flat_field(field.clone()).unwrap();
            let cal = field
                .as_ref()
                .map_or(Ok(calibration()), |f| {
                    f.processing_calibration(calibration())
                })
                .unwrap();
            let gpu = field
                .as_ref()
                .map(|f| Arc::new(GpuFlatField::new(&device, f.clone()).unwrap()));
            let composition = Composition::new(
                scan.area,
                Default::default(),
                crate::geometry::PhotoOrientation {
                    turns: crate::geometry::QuarterTurns::One,
                    ..Default::default()
                },
            )
            .unwrap();
            let regional = FrameRaster::new(
                composition,
                FramePoint([31., 43.]),
                FramePoint([140., 173.]),
                [35, 27],
            )
            .unwrap();
            let spec = ViewSourceSpec::new(7, regional, cal, scan.camera_to_ap1, field).unwrap();
            let mut underlay = spec.underlay().unwrap();
            underlay.frame.output = [19, 17];
            let specs = vec![spec, underlay];
            let paired = ViewSource::acquire_many(
                &device,
                &queue,
                &gate,
                renderer.clone(),
                kernels.clone(),
                &scan,
                gpu.clone(),
                specs.clone(),
                || false,
            )
            .unwrap()
            .unwrap();
            for (spec, actual) in specs.into_iter().zip(&paired) {
                let separate = ViewSource::acquire(
                    &device,
                    &queue,
                    &gate,
                    renderer.clone(),
                    kernels.clone(),
                    &scan,
                    gpu.clone(),
                    spec,
                    || false,
                )
                .unwrap()
                .unwrap();
                for (a, b) in read(&device, &queue, &actual.pixels).into_iter().zip(read(
                    &device,
                    &queue,
                    &separate.pixels,
                )) {
                    assert_eq!(a[3], b[3]);
                    for c in 0..3 {
                        assert!(
                            (a[c] - b[c]).abs() <= 2e-5 + 2e-4 * b[c].abs(),
                            "{a:?} {b:?}"
                        );
                    }
                }
            }
        }
    });
}

#[test]
fn physical_view_underlay_chart_fills_only_missing_detail_with_one_coherent_target_and_scopes() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let gate = SubmissionGate::default();
        let context = egui::Context::default();
        let scan = scan();
        let cal = calibration();
        let input = NegativeInput {
            calibration: cal,
            response: Response::film(),
            gamma: 0.75,
            balance: Balance::default(),
        };
        let roll = RollSettings::default();
        let mut full = frame(&scan);
        full.output = [16, 16];
        let detail_frame = FrameRaster::new(
            full.composition,
            full.min,
            FramePoint([full.max.0[0] / 2., full.max.0[1]]),
            [8, 16],
        )
        .unwrap();
        let spec = ViewSourceSpec::new(11, detail_frame, input, scan.camera_to_ap1, None).unwrap();
        let mut base_spec = spec.underlay().unwrap();
        base_spec.frame.output = [8, 8];
        let detail_pixels = (0..128)
            .map(|i| {
                if i % 11 == 0 {
                    [0., 0., 0., 8.]
                }
                // Valid black must not reveal the underlay.
                else if i % 7 == 0 {
                    [2., 0.2, 0.1, 0.]
                }
                // Coverage is independent of colour/floor flags.
                else {
                    [-0.01, 0.04 + (i % 8) as f32 * 0.12, 1.5, 13.]
                }
            })
            .collect::<Vec<_>>();
        let base_pixels = (0..64)
            .map(|i| {
                if i % 8 >= 6 && i / 8 <= 1 {
                    [0., 0., 0., 0.]
                } else {
                    [
                        0.2 + (i % 8) as f32 * 0.09,
                        0.1 + (i / 8) as f32 * 0.17,
                        (i % 2) as f32 * 0.7,
                        8.,
                    ]
                }
            })
            .collect::<Vec<_>>();
        let detail_source = source(&device, &renderer, &kernels, spec.clone(), &detail_pixels);
        let base_source = source(&device, &renderer, &kernels, base_spec, &base_pixels);
        let mut previous_detail = None;
        let mut previous_base = None;
        let mut worst = 0f64;
        for epoch in 0..3 {
            let photo = PhotoSettings {
                exposure: f64::from(epoch) * 0.5 - 0.5,
                contrast: 1.1,
                saturation: 1.2,
                wb: Balance {
                    warmth: 0.1,
                    tint: -0.05,
                },
                ..Default::default()
            };
            let processing = processing_input(input, roll, photo, &scan);
            let snapshot = PhotoGradeSnapshot::new(photo, vec![]).unwrap();
            for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
                let detail = RenderedView::new(
                    &device,
                    detail_source.clone(),
                    input,
                    roll,
                    snapshot.clone(),
                    &processing,
                    target,
                    previous_detail.as_ref(),
                )
                .unwrap();
                let base = RenderedView::new(
                    &device,
                    base_source.clone(),
                    input,
                    roll,
                    snapshot.clone(),
                    &processing,
                    target,
                    previous_base.as_ref(),
                )
                .unwrap();
                if let Some(old) = &previous_base
                    && (old.processing != detail.processing || old.target != target)
                {
                    assert!(
                        RenderedView::composite(&device, detail.clone(), old.clone(), full, None)
                            .is_err()
                    );
                }
                detail.ensure_submitted(&device, &queue, &gate, &context);
                base.ensure_submitted(&device, &queue, &gate, &context);
                let detail_values = image(&device, &queue, &detail.image);
                let base_values = image(&device, &queue, &base.image);
                let mut turned = FrameRaster::full(
                    Composition::new(
                        scan.area,
                        Default::default(),
                        crate::geometry::PhotoOrientation {
                            turns: crate::geometry::QuarterTurns::One,
                            ..Default::default()
                        },
                    )
                    .unwrap(),
                )
                .unwrap();
                turned.output = [16, 16];
                let crop = FrameRaster::new(
                    full.composition,
                    FramePoint([full.max.0[0] * 0.25, full.max.0[1] * 0.25]),
                    FramePoint([full.max.0[0], full.max.0[1] * 0.75]),
                    [24, 8],
                )
                .unwrap();
                for requested in [full, turned, crop] {
                    let composite = RenderedView::composite(
                        &device,
                        detail.clone(),
                        base.clone(),
                        requested,
                        None,
                    )
                    .unwrap();
                    assert!(Arc::ptr_eq(
                        &composite,
                        &RenderedView::composite(
                            &device,
                            detail.clone(),
                            base.clone(),
                            requested,
                            Some(&composite)
                        )
                        .unwrap()
                    ));
                    assert!(
                        RenderedView::composite(
                            &device,
                            composite.clone(),
                            base.clone(),
                            full,
                            None
                        )
                        .is_err(),
                        "never recursively resample a composite"
                    );
                    composite.ensure_submitted(&device, &queue, &gate, &context);
                    let actual = image(&device, &queue, &composite.image);
                    let dmap = detail.mapping(requested);
                    let bmap = base.mapping(requested);
                    for y in 0..requested.output[1] {
                        for x in 0..requested.output[0] {
                            let uv = [
                                (f64::from(x) + 0.5) / f64::from(requested.output[0]),
                                (f64::from(y) + 0.5) / f64::from(requested.output[1]),
                            ];
                            let d = sample(&detail_values, detail_frame.output, mapped(dmap, uv));
                            let b = sample(&base_values, [8, 8], mapped(bmap, uv));
                            let alpha = d[3] + b[3] * (1. - d[3]);
                            let value = actual[(x + y * requested.output[0]) as usize];
                            assert_eq!(value[3], if alpha > 0. { 1. } else { 0. });
                            for c in 0..3 {
                                let expected = if alpha > 0. {
                                    (d[c] + b[c] * (1. - d[3])) / alpha
                                } else {
                                    0.
                                };
                                let error = (f64::from(value[c]) - expected).abs();
                                worst = worst.max(error);
                                assert!(error <= 2e-5, "{value:?} vs {expected}, {x},{y}");
                            }
                        }
                    }
                    let gpu = Arc::new(crate::gpu_scopes::GpuScopes::new(&device).await.unwrap());
                    let settings = crate::scopes::Settings {
                        target,
                        ..Default::default()
                    };
                    let paint = crate::gpu_scopes::ScopePaint::rendered(
                        gpu.clone(),
                        composite.clone(),
                        requested,
                        settings,
                        gate.clone(),
                        context.clone(),
                    );
                    let mut encoder = device.create_command_encoder(&Default::default());
                    egui_wgpu::CallbackTrait::prepare(
                        &paint,
                        &device,
                        &queue,
                        &egui_wgpu::ScreenDescriptor {
                            size_in_pixels: requested.output,
                            pixels_per_point: 1.,
                        },
                        &mut encoder,
                        &mut Default::default(),
                    );
                    gate.submit(&queue, [encoder.finish()]);
                    let totals = read(&device, &queue, &gpu.totals);
                    let (_, columns, counts) =
                        crate::scopes::reference(requested.output, &actual, settings).unwrap();
                    assert_eq!(
                        gpu.status_rendered(&composite, requested, settings)
                            .unwrap()
                            .unwrap(),
                        counts
                    );
                    let words = totals
                        .iter()
                        .flatten()
                        .map(|v| v.to_bits())
                        .collect::<Vec<_>>();
                    assert_eq!(&words[4..], &columns);
                    assert_eq!(gpu.reduction_count(), 1);
                    if epoch == 0 && target == OutputTarget::Srgb && requested == full {
                        let mut held = composite.clone();
                        let mut latest = full;
                        for pan in 1..=64 {
                            latest = FrameRaster::new(
                                full.composition,
                                FramePoint([0., full.max.0[1] * f64::from(pan) / 256.]),
                                full.max,
                                full.output,
                            )
                            .unwrap();
                            let pending = held.in_flight();
                            let next = RenderedView::composite(
                                &device,
                                detail.clone(),
                                base.clone(),
                                latest,
                                Some(&held),
                            )
                            .unwrap();
                            if pending {
                                assert!(
                                    Arc::ptr_eq(&next, &held),
                                    "coalesce navigation rather than queue another display-sized target"
                                );
                            }
                            next.ensure_submitted(&device, &queue, &gate, &context);
                            held = next;
                        }
                        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
                        let newest = RenderedView::composite(
                            &device,
                            detail.clone(),
                            base.clone(),
                            latest,
                            Some(&held),
                        )
                        .unwrap();
                        assert_eq!(
                            newest.spec.frame, latest,
                            "completion reaches the latest requested navigation"
                        );
                        newest.ensure_submitted(&device, &queue, &gate, &context);
                        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
                    }
                }
                assert!(
                    Arc::ptr_eq(
                        &detail,
                        &RenderedView::composite(
                            &device,
                            detail.clone(),
                            base.clone(),
                            detail_frame,
                            None
                        )
                        .unwrap()
                    ),
                    "fixed-view detail bypasses composition"
                );
                previous_detail = Some(detail);
                previous_base = Some(base);
            }
        }
        assert_eq!(read(&device, &queue, &detail_source.pixels), detail_pixels);
        assert_eq!(read(&device, &queue, &base_source.pixels), base_pixels);
        eprintln!("underlay_chart maximum_linear_composition_error={worst}");
    });
}

#[test]
fn physical_view_underlay_constant_signed_hdr_charts_have_no_tone_or_colour_seam() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let gate = SubmissionGate::default();
        let context = egui::Context::default();
        let scan = scan();
        let cal = calibration();
        let input = NegativeInput {
            calibration: cal,
            response: Response::film(),
            gamma: 0.75,
            balance: Balance::default(),
        };
        let roll = RollSettings::default();
        let mut full = frame(&scan);
        full.output = [64, 64];
        let regional = FrameRaster::new(
            full.composition,
            full.min,
            FramePoint([full.max.0[0] / 2., full.max.0[1]]),
            [8, 16],
        )
        .unwrap();
        let spec = ViewSourceSpec::new(13, regional, input, scan.camera_to_ap1, None).unwrap();
        let mut base_spec = spec.underlay().unwrap();
        base_spec.frame.output = [8, 8];
        let photo = PhotoSettings {
            exposure: 2.,
            contrast: 0.7,
            saturation: 1.4,
            wb: Balance {
                warmth: -0.1,
                tint: 0.1,
            },
            ..Default::default()
        };
        let processing = processing_input(input, roll, photo, &scan);
        let mut worst = 0f32;
        for rgb in [[0.; 3], [0.18; 3], [-0.01, 1.5, 0.04]] {
            let value = [rgb[0], rgb[1], rgb[2], 8.];
            let detail_source = source(&device, &renderer, &kernels, spec.clone(), &[value; 128]);
            let base_source = source(
                &device,
                &renderer,
                &kernels,
                base_spec.clone(),
                &[value; 64],
            );
            for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
                let make = |s| {
                    RenderedView::new(
                        &device,
                        s,
                        input,
                        roll,
                        PhotoGradeSnapshot::new(photo, vec![]).unwrap(),
                        &processing,
                        target,
                        None,
                    )
                    .unwrap()
                };
                let detail = make(detail_source.clone());
                let base = make(base_source.clone());
                let composite =
                    RenderedView::composite(&device, detail.clone(), base, full, None).unwrap();
                composite.ensure_submitted(&device, &queue, &gate, &context);
                let expected = image(&device, &queue, &detail.image)[0];
                for pixel in image(&device, &queue, &composite.image) {
                    assert_eq!(pixel[3], 1.);
                    for c in 0..3 {
                        let error = (pixel[c] - expected[c]).abs();
                        worst = worst.max(error);
                        assert!(error <= 2e-5);
                    }
                }
            }
        }
        eprintln!("underlay_flat maximum_linear_seam_error={worst}");
    });
}

#[test]
fn physical_view_underlay_in_flight_callbacks_do_not_retain_support_or_renderer_owners() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let gate = SubmissionGate::default();
        let context = egui::Context::default();
        let scan = scan();
        let cal = calibration();
        let roll = RollSettings::default();
        let mut full = frame(&scan);
        full.output = [64, 64];
        let regional = FrameRaster::new(
            full.composition,
            full.min,
            FramePoint([full.max.0[0] / 2., full.max.0[1]]),
            [8, 16],
        )
        .unwrap();
        let spec = ViewSourceSpec::new(17, regional, cal, scan.camera_to_ap1, None).unwrap();
        let mut base_spec = spec.underlay().unwrap();
        base_spec.frame.output = [8, 8];
        let detail_source = source(
            &device,
            &renderer,
            &kernels,
            spec,
            &[[0.18, 0.18, 0.18, 8.]; 128],
        );
        let base_source = source(
            &device,
            &renderer,
            &kernels,
            base_spec,
            &[[0.18, 0.18, 0.18, 8.]; 64],
        );
        let owners = [Arc::downgrade(&detail_source), Arc::downgrade(&base_source)];
        let renderer_owner = Arc::downgrade(&renderer);
        let photo = PhotoSettings::default();
        let input = processing_input(cal, roll, photo, &scan);
        let make = |s| {
            RenderedView::new(
                &device,
                s,
                cal,
                roll,
                PhotoGradeSnapshot::new(photo, vec![]).unwrap(),
                &input,
                OutputTarget::Srgb,
                None,
            )
            .unwrap()
        };
        let detail = make(detail_source.clone());
        let base = make(base_source.clone());
        let composite = RenderedView::composite(&device, detail, base, full, None).unwrap();
        composite.ensure_submitted(&device, &queue, &gate, &context);
        drop(composite);
        drop(detail_source);
        drop(base_source);
        drop(renderer);
        assert!(owners.iter().all(|w| w.upgrade().is_none()));
        assert!(renderer_owner.upgrade().is_none());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        assert!(!gate.pending());
    });
}

#[test]
fn physical_view_underlay_texture_composition_does_not_inherit_storage_buffer_size_limits() {
    pollster::block_on(async {
        let instance = crate::gpu::instance();
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let limits = wgpu::Limits {
            max_storage_buffer_binding_size: 4 * 1024 * 1024,
            ..Default::default()
        };
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                required_limits: limits,
                ..Default::default()
            })
            .await
            .unwrap();
        let renderer = crate::renderer::GpuRenderer::new(&device, &queue)
            .await
            .unwrap();
        let kernels = ViewKernels::new(&device).await.unwrap();
        let gate = SubmissionGate::default();
        let context = egui::Context::default();
        let scan = scan();
        let cal = calibration();
        let roll = RollSettings::default();
        let mut full = frame(&scan);
        full.output = [1024, 512];
        assert!(
            u64::from(full.output[0]) * u64::from(full.output[1]) * 16
                > device.limits().max_storage_buffer_binding_size
        );
        let regional = FrameRaster::new(
            full.composition,
            full.min,
            FramePoint([full.max.0[0] / 2., full.max.0[1]]),
            [8, 16],
        )
        .unwrap();
        let spec = ViewSourceSpec::new(19, regional, cal, scan.camera_to_ap1, None).unwrap();
        let mut base_spec = spec.underlay().unwrap();
        base_spec.frame.output = [8, 8];
        let detail_source = source(
            &device,
            &renderer,
            &kernels,
            spec,
            &[[0.18, 0.18, 0.18, 8.]; 128],
        );
        let base_source = source(
            &device,
            &renderer,
            &kernels,
            base_spec,
            &[[0.18, 0.18, 0.18, 8.]; 64],
        );
        let photo = PhotoSettings::default();
        let input = processing_input(cal, roll, photo, &scan);
        let make = |s| {
            RenderedView::new(
                &device,
                s,
                cal,
                roll,
                PhotoGradeSnapshot::new(photo, vec![]).unwrap(),
                &input,
                OutputTarget::Srgb,
                None,
            )
            .unwrap()
        };
        let composite =
            RenderedView::composite(&device, make(detail_source), make(base_source), full, None)
                .unwrap();
        composite.ensure_submitted(&device, &queue, &gate, &context);
        assert!(
            image(&device, &queue, &composite.image)
                .iter()
                .all(|p| p[3] == 1.)
        );
    });
}
