//! Production physical-view colour, checked against original precise arithmetic and scalar masks.
use super::*;
#[path = "gpu_working_view_comparison_tests.rs"]
mod comparison;
#[path = "gpu_working_view_oracle_tests.rs"]
mod oracles;
#[path = "gpu_shared_colour_tests.rs"]
mod shared_colour;
use crate::{
    geometry::{FramePoint, SourcePoint, SourceRegion},
    local_adjustments::{GradientMask, LocalAdjustment},
};

fn precise_kernels(device: &wgpu::Device, base: &ViewKernels) -> Arc<ViewKernels> {
    let locals = include_str!("shaders/photographic_precise.wgsl");
    let local = &locals[locals.find("fn local_grade(").unwrap()..];
    let replace = |text: String, old: &str, new: &str| {
        assert_eq!(text.matches(old).count(), 1, "precise oracle seam: {old}");
        text.replacen(old, new, 1)
    };
    let local = replace(local.to_owned(), "fn local_grade(", "fn reference_locals(");
    let local = replace(
        local,
        "let source = source_pixel(index);",
        "let source = view_point(index);",
    );
    let local = replace(
        local,
        r#"if any(source < local_context.area.xy)
        || any(source >= local_context.area.xy + local_context.area.zw) {
        return input;
    }"#,
        r#"if !precise_le(number(f32(local_context.area.x)), source[0])
        || !precise_le(number(f32(local_context.area.y)), source[1])
        || precise_le(number(f32(local_context.area.x + local_context.area.z)), source[0])
        || precise_le(number(f32(local_context.area.y + local_context.area.w)), source[1]) {
        return input;
    }"#,
    );
    let local = replace(
        local,
        "local_coverage(stage, source)",
        "local_coverage_point(stage, source)",
    );
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("original precise colour oracle"),
        source: wgpu::ShaderSource::Wgsl(
            [
                ProcessingKernel::shader(),
                include_str!("shaders/view_geometry.wgsl").to_owned(),
                local,
                include_str!("shaders/test_view_reference.wgsl").to_owned(),
            ]
            .join("\n")
            .into(),
        ),
    });
    let grade = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: None,
        layout: None,
        module: &module,
        entry_point: Some("reference_main"),
        compilation_options: Default::default(),
        cache: None,
    });
    Arc::new(ViewKernels {
        grade,
        inspection: base.inspection.clone(),
        filter: base.filter.clone(),
        proxy_coverage: base.proxy_coverage.clone(),
        copy: base.copy.clone(),
        compose: base.compose.clone(),
    })
}

fn source_with_kernel(source: &Arc<ViewSource>, kernels: Arc<ViewKernels>) -> Arc<ViewSource> {
    Arc::new(ViewSource {
        spec: source.spec.clone(),
        pixels: source.pixels.clone(),
        renderer: source.renderer.clone(),
        kernels,
    })
}

pub(super) fn authored_source(
    device: &wgpu::Device,
    renderer: Arc<crate::renderer::GpuRenderer>,
    kernels: Arc<ViewKernels>,
    values: &[[f32; 4]],
    frame: FrameRaster,
) -> Arc<ViewSource> {
    let matrix = DMat3::from_cols(
        DVec3::new(0.6062, 0.0161, -0.0031),
        DVec3::new(0.3802, 1.1927, -0.1490),
        DVec3::new(0.0136, -0.2088, 1.1521),
    );
    let spec = ViewSourceSpec::new(
        1,
        frame,
        calibration(),
        RollSettings {
            gamma: 0.65,
            ..RollSettings::new_grade()
        },
        matrix,
        None,
    )
    .unwrap();
    Arc::new(ViewSource {
        spec,
        renderer,
        kernels,
        pixels: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("identical signed/HDR acquired AP1, including provenance"),
            contents: bytemuck::cast_slice(values),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        }),
    })
}

fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    source: Arc<ViewSource>,
    snapshot: PhotoGradeSnapshot,
    target: OutputTarget,
) -> Arc<RenderedView> {
    render_with_basis(
        device,
        queue,
        source,
        snapshot,
        target,
        calibration(),
        RollSettings {
            gamma: 0.65,
            ..RollSettings::new_grade()
        },
    )
}

fn render_with_basis(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    source: Arc<ViewSource>,
    snapshot: PhotoGradeSnapshot,
    target: OutputTarget,
    cal: Calibration,
    roll: RollSettings,
) -> Arc<RenderedView> {
    let grade = Arc::new(
        GpuPhotoGrade::new(
            device,
            cal,
            roll,
            snapshot.clone(),
            source.spec.matrix,
            source.spec.area,
        )
        .unwrap(),
    );
    let input = ProcessingInput::from(grade);
    let view =
        RenderedView::new(device, source, cal, roll, snapshot, &input, target, None).unwrap();
    view.ensure_submitted(
        device,
        queue,
        &SubmissionGate::default(),
        &egui::Context::default(),
    );
    view
}

fn adversaries() -> Vec<[f32; 4]> {
    let mut v = Vec::new();
    for x in [
        -1e10,
        -16.,
        -1.,
        -0.01,
        -1e-12,
        -1e-25,
        0.,
        1e-25,
        1e-12,
        0.0078124995,
        0.0078125,
        0.007812501,
        0.18,
        1.,
        16.,
        1e10,
    ] {
        v.push([x, x, x, 11.]);
        v.push([x, x * 0.5, -x * 0.25, 13.]);
    }
    let w = crate::color::AP1_TO_XYZ.row(1);
    let g = (-w.x / w.y * 0.25) as f32;
    for n in -12..=12 {
        v.push([
            0.25,
            f32::from_bits(g.to_bits().checked_add_signed(n).unwrap()),
            0.,
            15.,
        ]);
    }
    for r in [0.1234567, 0.25, 1., 100.] {
        for b in [-0.008, 0.004, 0.125] {
            let g = (-(w.x * r + w.z * b) / w.y) as f32;
            for n in -2..=2 {
                v.push([
                    r as f32,
                    f32::from_bits(g.to_bits().checked_add_signed(n).unwrap()),
                    b as f32,
                    13.,
                ]);
            }
        }
    }
    v
}

fn photo() -> PhotoSettings {
    PhotoSettings {
        exposure: 0.25,
        wb: Balance {
            warmth: 0.005,
            tint: -0.003,
        },
        contrast: 1.01,
        contrast_colour_mix: 0.25,
        saturation: 1.02,
        saturation_density: 0.4,
        ..Default::default()
    }
}

fn local_pair(area: SourceRegion) -> Vec<LocalAdjustment> {
    let x = area.x as f64;
    let y = area.y as f64;
    let w = area.width as f64;
    let h = area.height as f64;
    vec![
        LocalAdjustment {
            mask: GradientMask::Linear {
                start: SourcePoint([x + w * 0.15, y + h * 0.2]),
                end: SourcePoint([x + w * 0.85, y + h * 0.8]),
            },
            photo: photo(),
            enabled: true,
            invert: true,
            strength: 0.6,
        },
        LocalAdjustment {
            mask: GradientMask::Radial {
                center: SourcePoint([x + w * 0.5, y + h * 0.5]),
                radii: [w * 0.4, h * 0.4],
                angle: 0.3,
                feather: 0.8,
            },
            photo: PhotoSettings {
                exposure: 0.3125,
                ..photo()
            },
            enabled: true,
            invert: false,
            strength: 0.6,
        },
    ]
}

fn domain_cases(area: SourceRegion) -> Vec<(String, PhotoGradeSnapshot)> {
    let mut cases = Vec::new();
    let simple = |p| PhotoGradeSnapshot::new(p, vec![]).unwrap();
    for contrast in [0.4, 0.8, 1. - 1e-8, 1., 1. + 1e-8, 1.25, 1.6] {
        for pivot in [0.01, 0.18, 1.] {
            for colour in [0., 0.5, 1.] {
                cases.push((
                    format!("contrast{contrast}-pivot{pivot}-colour{colour}"),
                    simple(PhotoSettings {
                        contrast,
                        pivot,
                        contrast_colour_mix: colour,
                        saturation: 1.3,
                        saturation_density: 0.4,
                        ..Default::default()
                    }),
                ));
            }
        }
    }
    for contrast in [0.4, 0.5, 0.8] {
        let qp = (0.155251141552511 - contrast * 0.0729055341958355) / (1. - contrast);
        let pivot = f64::exp2(qp * 17.52 - 9.72);
        for dp in [-1e-7, 0., 1e-7] {
            cases.push((
                format!("toe-{contrast}-{dp}"),
                simple(PhotoSettings {
                    contrast,
                    pivot: pivot + dp,
                    contrast_colour_mix: 1.,
                    ..Default::default()
                }),
            ));
        }
    }
    for ds in [-1., -0.3, 1e-8, 0.3, 1.] {
        for density in [0., 0.5, 1.] {
            cases.push((
                format!("saturation-{ds}-{density}"),
                simple(PhotoSettings {
                    saturation: 1. + ds,
                    saturation_density: density,
                    ..Default::default()
                }),
            ));
        }
    }
    cases.push((
        "global-photographic".into(),
        simple(PhotoSettings {
            contrast: 1.010625,
            ..photo()
        }),
    ));
    cases.push((
        "local-last".into(),
        PhotoGradeSnapshot::new(PhotoSettings::default(), local_pair(area)).unwrap(),
    ));
    let full = |photo, strength| LocalAdjustment {
        mask: GradientMask::Linear {
            start: SourcePoint([-100., -100.]),
            end: SourcePoint([-99., -100.]),
        },
        photo,
        enabled: true,
        invert: false,
        strength,
    };
    let high = PhotoSettings {
        exposure: 16.,
        contrast: 0.4,
        pivot: 0.03,
        contrast_colour_mix: 1.,
        ..Default::default()
    };
    cases.push(("high-exposure-low-contrast".into(), simple(high)));
    cases.push((
        "high-exposure-then-local-recover".into(),
        PhotoGradeSnapshot::new(
            high,
            vec![full(
                PhotoSettings {
                    exposure: -16.,
                    ..Default::default()
                },
                1.,
            )],
        )
        .unwrap(),
    ));
    cases.push((
        "alternating-wb".into(),
        PhotoGradeSnapshot::new(
            PhotoSettings::default(),
            vec![
                full(
                    PhotoSettings {
                        wb: Balance {
                            warmth: 0.8,
                            tint: -0.4,
                        },
                        ..Default::default()
                    },
                    0.7,
                ),
                full(
                    PhotoSettings {
                        wb: Balance {
                            warmth: -0.8,
                            tint: 0.4,
                        },
                        ..Default::default()
                    },
                    0.7,
                ),
            ],
        )
        .unwrap(),
    ));
    for count in [3, 7] {
        let locals = [-16., 16.]
            .into_iter()
            .flat_map(|exposure| {
                std::iter::repeat_n(
                    full(
                        PhotoSettings {
                            exposure,
                            ..Default::default()
                        },
                        0.25,
                    ),
                    count,
                )
            })
            .collect();
        cases.push((
            format!("attenuate-recover-{count}"),
            PhotoGradeSnapshot::new(PhotoSettings::default(), locals).unwrap(),
        ));
    }
    let mut tiny = vec![
        full(
            PhotoSettings {
                contrast: 1.6,
                pivot: 0.03,
                contrast_colour_mix: 0.5,
                saturation: 1.3,
                ..Default::default()
            },
            1e-8
        );
        4
    ];
    tiny.push(full(
        PhotoSettings {
            exposure: 16.,
            ..Default::default()
        },
        0.25,
    ));
    cases.push((
        "tiny-weight-then-amplify".into(),
        PhotoGradeSnapshot::new(PhotoSettings::default(), tiny).unwrap(),
    ));
    assert_eq!(cases.len(), 95);
    cases
}

fn compare(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    a: &RenderedView,
    b: &RenderedView,
) -> [u32; 8] {
    assert_eq!(a.target, b.target);
    compare_rasters(
        device,
        queue,
        (&a.graded.as_ref().unwrap().pixels, &a.image.linear),
        (&b.graded.as_ref().unwrap().pixels, &b.image.linear),
    )
}

fn compare_rasters(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    (ab, at): (&wgpu::Buffer, &wgpu::Texture),
    (bb, bt): (&wgpu::Buffer, &wgpu::Texture),
) -> [u32; 8] {
    assert_eq!(ab.size(), bb.size());
    assert_eq!(at.size(), bt.size());
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("complete-raster unchanged numerical gates"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shaders/probe_float32_compare.wgsl").into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: None,
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 32,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let av = at.create_view(&Default::default());
    let bv = bt.create_view(&Default::default());
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: ab.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: bb.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&av),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(&bv),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: output.as_entire_binding(),
            },
        ],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        let count = (ab.size() / 16).div_ceil(64) as u32;
        let x = count.min(device.limits().max_compute_workgroups_per_dimension);
        pass.dispatch_workgroups(x, count.div_ceil(x), 1);
    }
    queue.submit([encoder.finish()]);
    read(device, queue, &output)
        .iter()
        .flatten()
        .map(|v| v.to_bits())
        .collect::<Vec<_>>()
        .try_into()
        .unwrap()
}

fn assert_comparison(case: &str, words: [u32; 8]) {
    println!(
        "physical_view_colour case={case} ap1_units={} linear_max={} encoded_max={} failures={:?}",
        f32::from_bits(words[0]),
        f32::from_bits(words[4]),
        f32::from_bits(words[6]),
        [words[2], words[3], words[5], words[7]]
    );
    assert_eq!(
        [words[2], words[3], words[5], words[7]],
        [0; 4],
        "{case}: {words:?}"
    );
}

#[test]
fn physical_view_colour_identity_and_recovery_preserve_complete_raster_and_flags() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let precise = precise_kernels(&device, &kernels);
        let values = adversaries();
        let area = SourceRegion {
            x: 11,
            y: 17,
            width: 63,
            height: 47,
        };
        let composition = Composition::new(area, Default::default(), Default::default()).unwrap();
        let frame = FrameRaster::new(
            composition,
            FramePoint([0., 0.]),
            FramePoint(composition.size().map(f64::from)),
            [values.len() as u32, 1],
        )
        .unwrap();
        let source = authored_source(&device, renderer, kernels, &values, frame);
        for photo in [
            PhotoSettings::default(),
            PhotoSettings {
                contrast: 0.4,
                pivot: 0.03,
                contrast_colour_mix: 1.,
                ..Default::default()
            },
            PhotoSettings {
                exposure: 16.,
                contrast: 0.4,
                pivot: 0.03,
                contrast_colour_mix: 1.,
                ..Default::default()
            },
        ] {
            let snap = PhotoGradeSnapshot::new(photo, vec![]).unwrap();
            let a = render(
                &device,
                &queue,
                source_with_kernel(&source, precise.clone()),
                snap.clone(),
                OutputTarget::Srgb,
            );
            let b = render(&device, &queue, source.clone(), snap, OutputTarget::Srgb);
            assert_comparison("identity-or-recovery", compare(&device, &queue, &a, &b));
            if photo == PhotoSettings::default() {
                assert_eq!(
                    read(&device, &queue, &b.graded.as_ref().unwrap().pixels),
                    values
                );
            }
        }
        assert_eq!(read(&device, &queue, &source.pixels), values);
    });
}

#[test]
#[ignore = "95 signed/HDR production-vs-original-precision colour cases; not photographic sampling acceptance"]
fn physical_view_colour_domain() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let precise = precise_kernels(&device, &kernels);
        let values = adversaries();
        let area = SourceRegion {
            x: 11,
            y: 17,
            width: 63,
            height: 47,
        };
        let composition = Composition::new(area, Default::default(), Default::default()).unwrap();
        let frame = FrameRaster::new(
            composition,
            FramePoint([0., 0.]),
            FramePoint(composition.size().map(f64::from)),
            [values.len() as u32, 1],
        )
        .unwrap();
        let source = authored_source(&device, renderer, kernels, &values, frame);
        for (case, snap) in domain_cases(area) {
            let a = render(
                &device,
                &queue,
                source_with_kernel(&source, precise.clone()),
                snap.clone(),
                OutputTarget::Srgb,
            );
            let b = render(&device, &queue, source.clone(), snap, OutputTarget::Srgb);
            assert_comparison(&case, compare(&device, &queue, &a, &b));
        }
    });
}
