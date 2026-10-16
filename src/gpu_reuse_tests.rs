use super::*;
use crate::{
    color::Balance,
    geometry::{SourcePoint, SourceRegion},
    local_adjustments::{GradientMask, LocalAdjustment, PhotoGradeSnapshot, SpatialPipeline},
    processing::Response,
};
use std::time::Instant;

#[cfg(feature = "raw")]
#[path = "gpu_reuse_raw_tests.rs"]
mod raw_tests;

#[path = "gpu_stage_performance_tests.rs"]
mod stage_tests;

#[path = "gpu_log_prefix_tests.rs"]
mod log_prefix_tests;

#[cfg(all(feature = "ui", feature = "render"))]
#[path = "gpu_screen_performance_tests.rs"]
mod screen_tests;

const SIZE: [u32; 2] = [1024, 640];
const RECORD_BYTES: u64 = 40; // Nine scalar f32 words and one u32 flag, no vec3 padding.
const ENTRIES: [&str; 9] = [
    "build_response_codes",
    "lookup_main",
    "build_camera_prefix",
    "camera_prefix_main",
    "build_global_prefix",
    "global_prefix_main",
    "build_local_prefix_lookup",
    "build_log_prefix",
    "log_prefix_main",
];

use super::response::ResponseKey;

#[derive(Debug, PartialEq, Eq)]
struct PrefixKey {
    response: ResponseKey,
    // Image-owned revision must advance for source writes, correction and geometry changes.
    source_identity_revision: [u64; 2],
    matrix: [u64; 9],
    geometry: Vec<u8>,
    global: Option<Vec<u8>>,
}

impl PrefixKey {
    fn new(
        identity_revision: [u64; 2],
        matrix: DMat3,
        geometry: flat::FieldGeometry,
        parameters: ProcessingParameters,
        after_global: bool,
    ) -> Self {
        Self {
            response: ResponseKey::new(parameters),
            source_identity_revision: identity_revision,
            matrix: matrix.to_cols_array().map(f64::to_bits),
            geometry: bytemuck::bytes_of(&geometry).to_vec(),
            global: after_global.then(|| bytemuck::bytes_of(&parameters).to_vec()),
        }
    }
}

fn calibration() -> Calibration {
    Calibration {
        dmin: DVec3::new(0.19, 0.14, 0.12),
        dmax: DVec3::new(0.008, 0.004, 0.002),
        input_floor: 1. / 65535.,
    }
}

fn camera_matrix() -> DMat3 {
    DMat3::from_cols(
        DVec3::new(0.6062, 0.0161, -0.0031),
        DVec3::new(0.3802, 1.1927, -0.1490),
        DVec3::new(0.0136, -0.2088, 1.1521),
    )
}

fn zero_y_matrix() -> DMat3 {
    let rgb = Inversion::new(calibration(), RollSettings::new_grade())
        .unwrap()
        .apply(DVec3::ZERO)
        .unwrap()
        .rgb;
    let y = AP1_TO_XYZ.row(1);
    let cancel = (-y.x * rgb.x - y.y * rgb.y + 0.25 * y.z * rgb.z) / (y.y * rgb.z);
    DMat3::from_cols(DVec3::X, DVec3::Y, DVec3::new(0., cancel, -0.25))
}

fn area() -> SourceRegion {
    SourceRegion {
        x: 12,
        y: 12,
        width: 8192,
        height: 5472,
    }
}

fn geometry() -> SourceGeometry {
    let a = area();
    SourceGeometry::new([a.x, a.y, a.width, a.height], SIZE).unwrap()
}

fn photographic() -> PhotoSettings {
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

fn snapshot(global: PhotoSettings, count: usize, nonlinear: bool) -> PhotoGradeSnapshot {
    let locals = (0..count)
        .map(|i| LocalAdjustment {
            mask: if i % 2 == 0 {
                GradientMask::Linear {
                    start: SourcePoint([900., 800.]),
                    end: SourcePoint([7000., 4400.]),
                }
            } else {
                GradientMask::Radial {
                    center: SourcePoint([4100., 2750.]),
                    radii: [3500., 2300.],
                    angle: 0.3,
                    feather: 0.8,
                }
            },
            photo: if nonlinear {
                photographic()
            } else {
                PhotoSettings {
                    exposure: if i % 2 == 0 { 0.25 } else { -0.125 },
                    ..Default::default()
                }
            },
            enabled: true,
            invert: i % 3 == 0,
            strength: 0.6,
        })
        .collect();
    PhotoGradeSnapshot::new(global, locals).unwrap()
}

#[test]
fn experimental_reuse_keys_preserve_response_and_prefix_dependencies() {
    let c = calibration();
    let r = RollSettings::new_grade();
    let photo = PhotoSettings::default();
    let p = ProcessingParameters::positive(c, r, photo, camera_matrix()).unwrap();
    let field = || flat::FieldGeometry::uncorrected(Some(geometry()));
    let key = |p, global| PrefixKey::new([1, 0], camera_matrix(), field(), p, global);
    for edit in [
        PhotoSettings {
            wb: photographic().wb,
            ..photo
        },
        PhotoSettings {
            contrast: 1.1,
            ..photo
        },
        PhotoSettings {
            saturation: 1.1,
            ..photo
        },
    ] {
        let changed = ProcessingParameters::positive(c, r, edit, camera_matrix()).unwrap();
        assert_eq!(ResponseKey::new(p), ResponseKey::new(changed));
        assert_eq!(key(p, false), key(changed, false));
        assert_ne!(key(p, true), key(changed, true));
    }
    for (calibration, roll, photo) in [
        (
            c,
            r,
            PhotoSettings {
                exposure: 1.,
                ..photo
            },
        ),
        (c, RollSettings { exposure: 1., ..r }, photo),
        (c, RollSettings { gamma: 0.7, ..r }, photo),
        (
            c,
            RollSettings {
                response: Response::Original,
                ..r
            },
            photo,
        ),
        (
            Calibration {
                dmin: c.dmin * 1.01,
                ..c
            },
            r,
            photo,
        ),
        (
            Calibration {
                input_floor: c.input_floor * 2.,
                ..c
            },
            r,
            photo,
        ),
    ] {
        let changed =
            ProcessingParameters::positive(calibration, roll, photo, camera_matrix()).unwrap();
        assert_ne!(ResponseKey::new(p), ResponseKey::new(changed));
        assert_ne!(key(p, false), key(changed, false));
    }
    let matrix = DMat3::IDENTITY;
    let changed = ProcessingParameters::positive(c, r, photo, matrix).unwrap();
    assert_eq!(ResponseKey::new(p), ResponseKey::new(changed));
    assert_ne!(
        key(p, false),
        PrefixKey::new([1, 0], matrix, field(), changed, false)
    );
    for stamp in [[2, 0], [1, 1]] {
        assert_ne!(
            key(p, false),
            PrefixKey::new(stamp, camera_matrix(), field(), p, false)
        );
    }
    let tile = SourceGeometry::new([100, 200, 1024, 256], [1024, 256]).unwrap();
    assert_ne!(
        key(p, false),
        PrefixKey::new(
            [1, 0],
            camera_matrix(),
            flat::FieldGeometry::uncorrected(Some(tile)),
            p,
            false
        )
    );
}

struct Probe {
    pipelines: Vec<wgpu::ComputePipeline>,
    response: wgpu::Buffer,
    prefix: wgpu::Buffer,
    output: wgpu::Buffer,
}

impl Probe {
    async fn new(device: &wgpu::Device, pixels: usize) -> Self {
        let shader = [
            ProcessingKernel::shader(false).as_str(),
            include_str!("shaders/probe_reuse.wgsl"),
            include_str!("shaders/probe_log_prefix.wgsl"),
        ]
        .join("\n");
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("experimental exact response and compensated prefix reuse"),
            source: wgpu::ShaderSource::Wgsl(shader.into()),
        });
        let entries: Vec<_> = (0..10)
            .map(|binding| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: match binding {
                        0 | 4 | 6 => wgpu::BufferBindingType::Storage { read_only: true },
                        1 | 8 | 9 => wgpu::BufferBindingType::Storage { read_only: false },
                        _ => wgpu::BufferBindingType::Uniform,
                    },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            })
            .collect();
        let bindings = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &entries,
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&bindings)],
            immediate_size: 0,
        });
        let pipelines = ENTRIES
            .iter()
            .map(|entry| {
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: Some(&layout),
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                })
            })
            .collect();
        assert!(
            scope.pop().await.is_none(),
            "reuse probe shader validation failed"
        );
        let buffer = |label, size| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })
        };
        Self {
            pipelines,
            response: buffer(
                "experimental per-code response constants",
                65536 * RECORD_BYTES,
            ),
            prefix: buffer(
                "experimental one compensated image prefix",
                pixels as u64 * RECORD_BYTES,
            ),
            output: buffer("experimental AP1 output", pixels as u64 * 16),
        }
    }

    fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source: &ProcessingSource,
        grade: &GpuPhotoGrade,
        steps: &[usize],
    ) {
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::bytes_of(&grade.parameters),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let identity = flat::identity_nodes(device);
        let (nodes, field) = match &source.flat_field {
            Some(f) => (
                &f.nodes,
                flat::FieldGeometry::new(f, source.geometry.unwrap()),
            ),
            None => (&identity, flat::FieldGeometry::uncorrected(source.geometry)),
        };
        let geometry = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::bytes_of(&field),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let buffers = [
            &source.input,
            &self.output,
            &uniform,
            &source.matrix,
            nodes,
            &geometry,
            &grade.buffer,
            &grade.context,
            &self.response,
            &self.prefix,
        ];
        let entries: Vec<_> = buffers
            .iter()
            .enumerate()
            .map(|(binding, buffer)| wgpu::BindGroupEntry {
                binding: binding as u32,
                resource: buffer.as_entire_binding(),
            })
            .collect();
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.pipelines[0].get_bind_group_layout(0),
            entries: &entries,
        });
        for &step in steps {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(ENTRIES[step]),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipelines[step]);
            pass.set_bind_group(0, &group, &[]);
            let count = if step == 0 { 65536 } else { source.pixels };
            let groups = count.div_ceil(64);
            let x = groups.min(device.limits().max_compute_workgroups_per_dimension);
            pass.dispatch_workgroups(x, groups.div_ceil(x), 1);
        }
    }
}

pub(super) fn read_words(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    buffer: &wgpu::Buffer,
) -> Vec<u32> {
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: buffer.size(),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(buffer, 0, &read, 0, buffer.size());
    queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    read.map_async(wgpu::MapMode::Read, .., move |r| tx.send(r).unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    rx.recv().unwrap().unwrap();
    let mapped = read.get_mapped_range(..).unwrap();
    bytemuck::cast_slice(&mapped).to_vec()
}

fn median(values: &[f64]) -> f64 {
    let mut values = values.to_vec();
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

pub(super) fn exact(case: &str, candidate: &str, expected: &[u32], actual: &[u32]) {
    assert_eq!(actual.len(), expected.len());
    for (i, (a, b)) in actual.iter().zip(expected).enumerate() {
        assert_eq!(
            a,
            b,
            "{case} {candidate} pixel={} channel={} actual={} reference={}",
            i / 4,
            i % 4,
            f32::from_bits(*a),
            f32::from_bits(*b)
        );
    }
}

#[test]
#[ignore = "exact RGB16 lookup and compensated prefix A/B experiment; no production cache or interaction acceptance"]
fn exact_response_and_prefix_reuse() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        println!(
            "reuse_probe adapter={:?} size={SIZE:?} samples=9 timing=encode_submit_gpu_completion excludes=upload_readback_rendering_reconstruction",
            adapter.get_info()
        );
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let baseline = ProcessingKernel::new_fused(&device, &queue).await.unwrap();
        let pixels: Vec<_> = (0u32..SIZE[0] * SIZE[1])
            .map(|i| {
                [
                    i.wrapping_mul(1871) as u16,
                    i.wrapping_mul(7919) as u16,
                    i.wrapping_mul(3413) as u16,
                    42,
                ]
            })
            .collect();
        let probe = Probe::new(&device, pixels.len()).await;
        println!(
            "reuse_probe source_bytes={} output_bytes={} response_bytes={} one_prefix_bytes={} prefix_bytes_per_pixel={RECORD_BYTES}",
            pixels.len() * 8,
            probe.output.size(),
            probe.response.size(),
            probe.prefix.size()
        );
        for (case, matrix, roll, global, count, nonlinear) in [
            (
                "global-identity-matrix",
                DMat3::IDENTITY,
                RollSettings::new_grade(),
                PhotoSettings::default(),
                0,
                false,
            ),
            (
                "global-camera",
                camera_matrix(),
                RollSettings::new_grade(),
                PhotoSettings::default(),
                0,
                false,
            ),
            (
                "two-exposure",
                camera_matrix(),
                RollSettings::new_grade(),
                PhotoSettings::default(),
                2,
                false,
            ),
            (
                "two-photographic",
                camera_matrix(),
                RollSettings::new_grade(),
                PhotoSettings::default(),
                2,
                true,
            ),
            (
                "global-photographic",
                camera_matrix(),
                RollSettings::new_grade(),
                photographic(),
                0,
                false,
            ),
            (
                "classic",
                camera_matrix(),
                RollSettings::default(),
                PhotoSettings::default(),
                0,
                false,
            ),
            (
                "shoulder",
                camera_matrix(),
                RollSettings {
                    response: Response::Film {
                        toe: 3.,
                        shoulder: 0.01,
                    },
                    gamma: 1.2,
                    ..RollSettings::new_grade()
                },
                PhotoSettings::default(),
                0,
                false,
            ),
            (
                "amplified-zero-y-ratio-contrast",
                zero_y_matrix(),
                RollSettings::new_grade(),
                PhotoSettings {
                    exposure: 16.,
                    contrast: 0.4,
                    pivot: 0.03,
                    contrast_colour_mix: 1.,
                    ..Default::default()
                },
                0,
                false,
            ),
        ] {
            let c = calibration();
            let snapshot = snapshot(global, count, nonlinear);
            let grade =
                GpuPhotoGrade::new(&device, c, roll, snapshot.clone(), matrix, area()).unwrap();
            let mut source = ProcessingSource::new(&device, &pixels, matrix).unwrap();
            source.set_geometry(geometry()).unwrap();
            assert_eq!(source.byte_len(), pixels.len() as u64 * 8);
            let run = |steps: Option<&[usize]>| {
                let start = Instant::now();
                let mut encoder = device.create_command_encoder(&Default::default());
                if let Some(steps) = steps {
                    probe.encode(&device, &mut encoder, &source, &grade, steps);
                } else {
                    baseline
                        .encode_grade(&device, &mut encoder, &source, &probe.output, &grade)
                        .unwrap();
                }
                let submission = queue.submit([encoder.finish()]);
                device
                    .poll(wgpu::PollType::Wait {
                        submission_index: Some(submission),
                        timeout: None,
                    })
                    .unwrap();
                start.elapsed().as_secs_f64() * 1000.
            };
            run(None);
            let expected = read_words(&device, &queue, &probe.output);
            let scalar = SpatialPipeline::new(c, roll, snapshot, matrix).unwrap();
            for i in (0..pixels.len()).step_by(257) {
                let scan =
                    DVec3::from_array(std::array::from_fn(|ch| f64::from(pixels[i][ch]) / 65535.));
                let reference = scalar
                    .apply(scan, geometry().pixel(i as u32).unwrap())
                    .unwrap();
                for ch in 0..3 {
                    let actual = f64::from(f32::from_bits(expected[4 * i + ch]));
                    assert!(
                        (actual - reference.rgb[ch]).abs() <= 2e-5 + 2e-4 * reference.rgb[ch].abs(),
                        "{case} scalar pixel={i} channel={ch}: {actual} != {}",
                        reference.rgb[ch]
                    );
                }
                let flags = (0..3).fold(0, |a, ch| a | (u32::from(reference.floored[ch]) << ch));
                assert_eq!(f32::from_bits(expected[4 * i + 3]), flags as f32);
            }
            let table_ms = run(Some(&[0]));
            let words = read_words(&device, &queue, &probe.response);
            let inversion = Inversion::new(c, roll).unwrap();
            let gain = exposure_multiplier(roll.exposure).unwrap()
                * exposure_multiplier(global.exposure).unwrap();
            for code in 0..65536 {
                let reference = inversion.apply(DVec3::splat(code as f64 / 65535.)).unwrap();
                for ch in 0..3 {
                    let actual: f64 = (0..3)
                        .map(|word| f64::from(f32::from_bits(words[10 * code + 3 * ch + word])))
                        .sum();
                    let reference = reference.rgb[ch] * gain;
                    assert!(
                        actual.is_finite()
                            && (actual - reference).abs() <= 2e-5 + 2e-4 * reference.abs(),
                        "{case} native code={code} channel={ch}"
                    );
                }
                let flags = (0..3).fold(0, |a, ch| a | (u32::from(reference.floored[ch]) << ch));
                assert_eq!(words[10 * code + 9], flags);
            }
            println!(
                "reuse_probe case={case} initial_table_build_ms={table_ms:.3} scalar_samples={} native_code_checks=196608",
                pixels.len().div_ceil(257)
            );
            for (name, build, warm) in [
                ("lookup", 0, 1),
                ("camera-prefix", 2, 3),
                ("global-prefix", 4, 5),
            ] {
                run(Some(&[build, warm]));
                exact(
                    case,
                    name,
                    &expected,
                    &read_words(&device, &queue, &probe.output),
                );
                run(Some(&[warm]));
                exact(
                    case,
                    name,
                    &expected,
                    &read_words(&device, &queue, &probe.output),
                );
                let mut warm_times = Vec::new();
                let mut cold_times = Vec::new();
                let mut warm_ratios = Vec::new();
                let mut cold_ratios = Vec::new();
                let mut anchors = Vec::new();
                for sample in 0..9 {
                    let before = run(None);
                    let (cold, warm_time) = if sample % 2 == 0 {
                        (run(Some(&[build, warm])), run(Some(&[warm])))
                    } else {
                        let warm_time = run(Some(&[warm]));
                        (run(Some(&[build, warm])), warm_time)
                    };
                    let after = run(None);
                    let anchor = (before + after) / 2.;
                    anchors.extend([before, after]);
                    warm_times.push(warm_time);
                    cold_times.push(cold);
                    warm_ratios.push(warm_time / anchor);
                    cold_ratios.push(cold / anchor);
                }
                println!(
                    "reuse_probe case={case} candidate={name} baseline_ms={:.3} warm_ms={:.3} warm_ratio={:.4} cold_ms={:.3} cold_ratio={:.4} exact_pixels={}",
                    median(&anchors),
                    median(&warm_times),
                    median(&warm_ratios),
                    median(&cold_times),
                    median(&cold_ratios),
                    pixels.len()
                );
            }
        }
    });
}

#[test]
#[ignore = "fresh downstream edits, corrected continuous samples and inspection bypass in the isolated reuse probe"]
fn reuse_edits_and_bypasses() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let baseline = ProcessingKernel::new_fused(&device, &queue).await.unwrap();
        let mut pixels: Vec<_> = (0u32..SIZE[0] * SIZE[1])
            .map(|i| {
                [
                    i.wrapping_mul(1871) as u16,
                    i.wrapping_mul(7919) as u16,
                    i.wrapping_mul(3413) as u16,
                    123,
                ]
            })
            .collect();
        let probe = Probe::new(&device, pixels.len()).await;
        let matrix = camera_matrix();
        let mut source = ProcessingSource::new(&device, &pixels, matrix).unwrap();
        source.set_geometry(geometry()).unwrap();
        let base = GpuPhotoGrade::new(
            &device,
            calibration(),
            RollSettings::new_grade(),
            snapshot(PhotoSettings::default(), 2, true),
            matrix,
            area(),
        )
        .unwrap();
        let run = |source: &ProcessingSource, grade: &GpuPhotoGrade, steps: Option<&[usize]>| {
            let start = Instant::now();
            let mut encoder = device.create_command_encoder(&Default::default());
            if let Some(steps) = steps {
                probe.encode(&device, &mut encoder, source, grade, steps);
            } else {
                baseline
                    .encode_grade(&device, &mut encoder, source, &probe.output, grade)
                    .unwrap();
            }
            let submission = queue.submit([encoder.finish()]);
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(submission),
                    timeout: None,
                })
                .unwrap();
            start.elapsed().as_secs_f64() * 1000.
        };
        for (name, build, warm) in [
            ("lookup", 0, 1),
            ("camera-prefix", 2, 3),
            ("global-prefix", 4, 5),
        ] {
            run(&source, &base, Some(&[build]));
            let mut times = Vec::new();
            let mut ratios = Vec::new();
            let mut anchors = Vec::new();
            for edit in 0..9 {
                let global = if warm == 5 {
                    PhotoSettings::default()
                } else {
                    PhotoSettings {
                        wb: Balance {
                            warmth: 0.002 + f64::from(edit) * 0.0002,
                            tint: -0.001,
                        },
                        contrast: 1.0005 + f64::from(edit) * 0.0001,
                        ..Default::default()
                    }
                };
                let mut locals = base.snapshot().locals().to_vec();
                locals[0].photo.saturation = 1.01 + f64::from(edit) * 0.001;
                locals[0].strength = 0.55 + f64::from(edit) * 0.01;
                if edit % 2 == 1 {
                    locals.swap(0, 1);
                }
                let changed = PhotoGradeSnapshot::new(global, locals).unwrap();
                assert_ne!(&changed, base.snapshot());
                let grade = GpuPhotoGrade::new(
                    &device,
                    calibration(),
                    RollSettings::new_grade(),
                    changed,
                    matrix,
                    area(),
                )
                .unwrap();
                assert_eq!(
                    ResponseKey::new(base.parameters),
                    ResponseKey::new(grade.parameters)
                );
                let key = |p| {
                    PrefixKey::new(
                        [1, 0],
                        matrix,
                        flat::FieldGeometry::uncorrected(Some(geometry())),
                        p,
                        warm == 5,
                    )
                };
                assert_eq!(key(base.parameters), key(grade.parameters));
                let before = run(&source, &grade, None);
                let expected = read_words(&device, &queue, &probe.output);
                let time = run(&source, &grade, Some(&[warm]));
                exact(
                    "fresh-downstream-edit",
                    name,
                    &expected,
                    &read_words(&device, &queue, &probe.output),
                );
                let after = run(&source, &grade, None);
                times.push(time);
                ratios.push(time / ((before + after) / 2.));
                anchors.extend([before, after]);
            }
            println!(
                "reuse_edits candidate={name} fresh_grades=9 source_size={SIZE:?} baseline_ms={:.3} warm_ms={:.3} paired_ratio={:.4} exact_pixels_per_edit={}",
                median(&anchors),
                median(&times),
                median(&ratios),
                pixels.len()
            );
        }
        // Continuous correction and changed source content rebuild image prefixes, not code tables.
        let a = area();
        let field = Arc::new(
            crate::flat_field::FlatField::from_nodes(
                [a.x, a.y, a.width, a.height],
                [2, 2],
                vec![
                    DVec3::new(1.2, 0.9, 1.1),
                    DVec3::new(0.95, 1.3, 1.05),
                    DVec3::new(1.1, 1.05, 0.88),
                    DVec3::new(1.25, 0.98, 1.15),
                ],
            )
            .unwrap(),
        );
        pixels[1] = [65535, 0, 12452, 0];
        source.write(&queue, &pixels).unwrap();
        source
            .set_flat_field(
                Some(Arc::new(GpuFlatField::new(&device, field.clone()).unwrap())),
                geometry(),
            )
            .unwrap();
        let corrected_calibration = field.processing_calibration(calibration()).unwrap();
        let corrected_snapshot = snapshot(PhotoSettings::default(), 2, true);
        let mut grade = GpuPhotoGrade::new(
            &device,
            corrected_calibration,
            RollSettings::new_grade(),
            corrected_snapshot.clone(),
            matrix,
            area(),
        )
        .unwrap();
        run(&source, &grade, None);
        let expected = read_words(&device, &queue, &probe.output);
        let scalar = SpatialPipeline::new(
            corrected_calibration,
            RollSettings::new_grade(),
            corrected_snapshot,
            matrix,
        )
        .unwrap();
        for i in (0..pixels.len()).step_by(1021) {
            let point = geometry().pixel(i as u32).unwrap();
            let scan = DVec3::from_array(std::array::from_fn(|c| f64::from(pixels[i][c]) / 65535.))
                * field.gain(point.map(|v| f64::from(v) + 0.5)).unwrap();
            let reference = scalar.apply(scan, point).unwrap();
            for c in 0..3 {
                let actual = f64::from(f32::from_bits(expected[4 * i + c]));
                assert!((actual - reference.rgb[c]).abs() <= 2e-5 + 2e-4 * reference.rgb[c].abs());
            }
            let flags = (0..3).fold(0, |a, c| a | (u32::from(reference.floored[c]) << c));
            assert_eq!(f32::from_bits(expected[4 * i + 3]), flags as f32);
        }
        for (name, steps) in [
            ("corrected-lookup-original-GPU-path", &[1][..]),
            ("corrected-camera-prefix", &[2, 3][..]),
            ("corrected-global-prefix", &[4, 5][..]),
        ] {
            run(&source, &grade, Some(steps));
            exact(
                "corrected-source-write",
                name,
                &expected,
                &read_words(&device, &queue, &probe.output),
            );
        }
        // Leave positive caches and active local bindings in place; inspection must ignore them.
        grade.parameters = ProcessingParameters::source_inspection(2.).unwrap();
        let mut encoder = device.create_command_encoder(&Default::default());
        baseline.encode(
            &device,
            &mut encoder,
            &source,
            &probe.output,
            grade.parameters,
        );
        queue.submit([encoder.finish()]);
        let expected = read_words(&device, &queue, &probe.output);
        for (name, step) in [("lookup", 1), ("camera-prefix", 3), ("global-prefix", 5)] {
            run(&source, &grade, Some(&[step]));
            exact(
                "inspection-ignores-positive-caches",
                name,
                &expected,
                &read_words(&device, &queue, &probe.output),
            );
        }
        println!(
            "reuse_edits corrected_and_inspection_exact_pixels={} source_write_and_correction_rebuild_prefixes=true",
            pixels.len()
        );
    });
}
