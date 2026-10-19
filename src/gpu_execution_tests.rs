use super::*;
use crate::{
    color::Balance,
    geometry::{SourcePoint, SourceRegion},
    local_adjustments::{GradientMask, LocalAdjustment, PhotoGradeSnapshot},
};
use std::time::Instant;

struct Variant {
    name: String,
    kernel: ProcessingKernel,
    global_only: bool,
}

async fn variant(
    device: &wgpu::Device,
    baseline: &ProcessingKernel,
    workgroup: u32,
    global_only: bool,
    shader: &str,
) -> Variant {
    let name = format!(
        "wg{workgroup}{}",
        if global_only { "-global-only" } else { "" }
    );
    let shader = if global_only {
        assert_eq!(shader.matches("local_grade(global.rgb, index)").count(), 1);
        shader.replace("local_grade(global.rgb, index)", "global.rgb")
    } else {
        shader.to_owned()
    };
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(&name),
        source: wgpu::ShaderSource::Wgsl(shader.into()),
    });
    // Keep identical resource bindings even when a specialized entry does not use locals.
    let entries: Vec<_> = (0..8)
        .map(|binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: match binding {
                    0 | 4 | 6 => wgpu::BufferBindingType::Storage { read_only: true },
                    1 => wgpu::BufferBindingType::Storage { read_only: false },
                    _ => wgpu::BufferBindingType::Uniform,
                },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        })
        .collect();
    let bindings = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(&name),
        entries: &entries,
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(&name),
        bind_group_layouts: &[Some(&bindings)],
        immediate_size: 0,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(&name),
        layout: Some(&layout),
        module: &module,
        entry_point: Some("main"),
        compilation_options: wgpu::PipelineCompilationOptions {
            constants: &[("WORKGROUP_SIZE", f64::from(workgroup))],
            ..Default::default()
        },
        cache: None,
    });
    if let Some(error) = scope.pop().await {
        panic!("{name} shader validation: {error}");
    }
    Variant {
        name,
        global_only,
        kernel: ProcessingKernel {
            pipeline,
            output_binding: 1,
            output_stride: 16,
            workgroup_size: workgroup,
            identity: baseline.identity.clone(),
            empty_locals: baseline.empty_locals.clone(),
            empty_local_context: baseline.empty_local_context.clone(),
        },
    }
}

fn median(values: &[f64]) -> f64 {
    let mut values = values.to_vec();
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn read_pixels(device: &wgpu::Device, queue: &wgpu::Queue, output: &wgpu::Buffer) -> Vec<u32> {
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("kernel variant equivalence readback"),
        size: output.size(),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(output, 0, &readback, 0, output.size());
    queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    readback.map_async(wgpu::MapMode::Read, .., move |r| tx.send(r).unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    rx.recv().unwrap().unwrap();
    let mapped = readback.get_mapped_range(..).unwrap();
    bytemuck::cast_slice::<u8, u32>(&mapped).to_vec()
}

#[test]
#[ignore = "paired fresh-dispatch kernel experiments with exact full-raster output comparison; not editor acceptance"]
fn paired_kernel_variants() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        println!(
            "kernel_variants adapter={:?} size=[1024,640] samples=9 timing=encode_submit_gpu_completion excludes=upload_rendering_reconstruction",
            adapter.get_info()
        );
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let shader = ProcessingKernel::shader();
        let shader = if let Some(path) = std::env::var_os("ANTITONE_KERNEL_CANDIDATE_WGSL") {
            println!(
                "kernel_variants candidate={}",
                std::path::Path::new(&path).display()
            );
            std::fs::read_to_string(path).unwrap()
        } else {
            shader
        };
        let baseline = ProcessingKernel::new(&device).await.unwrap();
        let baseline = if let Some(path) = std::env::var_os("ANTITONE_KERNEL_REFERENCE_WGSL") {
            println!(
                "kernel_variants reference={}",
                std::path::Path::new(&path).display()
            );
            let reference = std::fs::read_to_string(path).unwrap();
            variant(&device, &baseline, 64, false, &reference)
                .await
                .kernel
        } else {
            baseline
        };
        let mut variants = Vec::new();
        let workgroups: &[u32] = if std::env::var_os("ANTITONE_KERNEL_SWEEP").is_some() {
            &[8, 16, 32, 64, 128, 256]
        } else {
            &[32, 64]
        };
        for &workgroup in workgroups {
            variants.push(variant(&device, &baseline, workgroup, false, &shader).await);
        }
        variants.push(variant(&device, &baseline, 64, true, &shader).await);
        let area = SourceRegion {
            x: 12,
            y: 12,
            width: 8192,
            height: 5472,
        };
        let size = [1024, 640];
        let pixels: Vec<_> = (0_u32..size[0] * size[1])
            .map(|i| {
                [
                    i.wrapping_mul(1871) as u16,
                    i.wrapping_mul(7919) as u16,
                    i.wrapping_mul(3413) as u16,
                    0,
                ]
            })
            .collect();
        let mut source = ProcessingSource::new(&device, &pixels, DMat3::IDENTITY).unwrap();
        source
            .set_geometry(
                SourceGeometry::new([area.x, area.y, area.width, area.height], size).unwrap(),
            )
            .unwrap();
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("kernel variant working output"),
            size: pixels.len() as u64 * 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        assert_eq!(source.byte_len(), pixels.len() as u64 * 8);
        let calibration = Calibration {
            dmin: DVec3::new(0.19, 0.14, 0.12),
            dmax: DVec3::new(0.008, 0.004, 0.002),
            input_floor: 1. / 65535.,
        };
        let parameters = ProcessingParameters::positive(
            calibration,
            RollSettings::new_grade(),
            PhotoSettings::default(),
            DMat3::IDENTITY,
        )
        .unwrap();
        for (case, count, photographic, blend) in [
            ("global", 0, false, None),
            ("two-exposure", 2, false, None),
            ("two-photographic", 2, true, None),
            ("two-rgb-y", 2, true, Some(0.)),
            ("two-ratio-max", 2, true, Some(1.)),
            ("sixteen-exposure", 16, false, None),
        ] {
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
                    photo: if photographic {
                        PhotoSettings {
                            exposure: 0.25,
                            wb: Balance {
                                warmth: 0.005,
                                tint: -0.003,
                            },
                            contrast: 1.01,
                            contrast_colour_mix: blend.unwrap_or(0.25),
                            saturation: 1.02,
                            saturation_density: blend.unwrap_or(0.4),
                            ..Default::default()
                        }
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
            let grade = if count == 0 {
                None
            } else {
                Some(
                    GpuPhotoGrade::new(
                        &device,
                        calibration,
                        RollSettings::new_grade(),
                        PhotoGradeSnapshot::new(PhotoSettings::default(), locals).unwrap(),
                        DMat3::IDENTITY,
                        area,
                    )
                    .unwrap(),
                )
            };
            let run = |kernel: &ProcessingKernel| {
                let start = Instant::now();
                let mut encoder = device.create_command_encoder(&Default::default());
                if let Some(grade) = &grade {
                    kernel
                        .encode_grade(&device, &mut encoder, &source, &output, grade)
                        .unwrap();
                } else {
                    kernel.encode(&device, &mut encoder, &source, &output, parameters);
                }
                let submitted = queue.submit([encoder.finish()]);
                device
                    .poll(wgpu::PollType::Wait {
                        submission_index: Some(submitted),
                        timeout: None,
                    })
                    .unwrap();
                start.elapsed().as_secs_f64() * 1000.
            };
            run(&baseline);
            let expected = read_pixels(&device, &queue, &output);
            let active: Vec<_> = variants
                .iter()
                .filter(|v| !v.global_only || count == 0)
                .collect();
            for variant in &active {
                run(&variant.kernel);
                let actual = read_pixels(&device, &queue, &output);
                assert_eq!(actual.len(), expected.len());
                for (i, (actual, expected)) in actual.iter().zip(&expected).enumerate() {
                    assert_eq!(
                        actual,
                        expected,
                        "{} {case} pixel={} channel={}",
                        variant.name,
                        i / 4,
                        i % 4
                    );
                }
            }
            let mut measurements = vec![Vec::new(); active.len()];
            let mut ratios = measurements.clone();
            let mut anchors = Vec::new();
            for sample in 0..9 {
                let before = run(&baseline);
                let mut times = vec![0.; active.len()];
                for offset in 0..active.len() {
                    let index = (sample + offset) % active.len();
                    times[index] = run(&active[index].kernel);
                }
                let after = run(&baseline);
                let anchor = (before + after) / 2.;
                anchors.extend([before, after]);
                for i in 0..active.len() {
                    measurements[i].push(times[i]);
                    ratios[i].push(times[i] / anchor);
                }
            }
            println!(
                "kernel_variants case={case} baseline_median_ms={:.3} baseline_range_ms=[{:.3},{:.3}]",
                median(&anchors),
                anchors.iter().copied().fold(f64::INFINITY, f64::min),
                anchors.iter().copied().fold(0., f64::max)
            );
            for (i, variant) in active.iter().enumerate() {
                println!(
                    "kernel_variants case={case} variant={} median_ms={:.3} paired_ratio={:.4} exact_pixels={}",
                    variant.name,
                    median(&measurements[i]),
                    median(&ratios[i]),
                    pixels.len()
                );
            }
        }
    });
}
