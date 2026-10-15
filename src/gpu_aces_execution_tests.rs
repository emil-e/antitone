use super::*;
use crate::rendering::CpuRenderer;

#[derive(Clone, Copy, Debug)]
pub(super) enum Execution {
    FragmentConstants,
    FragmentStorageHues,
    ComputeConstants,
    ComputeStorageHues,
}
impl Execution {
    fn compute(self) -> bool {
        !matches!(self, Self::FragmentConstants | Self::FragmentStorageHues)
    }
    fn storage_hues(self) -> bool {
        !matches!(self, Self::FragmentConstants | Self::ComputeConstants)
    }
}

// Independent original/equivalent shells keep the pinned function body and values.
// Do not use the production adapter here: it is the implementation under test.
fn execution_shader(target: OutputTarget, execution: Execution) -> (String, Vec<f32>) {
    let pinned = match target {
        OutputTarget::Srgb => include_str!("../assets/ocio/srgb-ap1.glsl"),
        OutputTarget::DisplayP3 => include_str!("../assets/ocio/display-p3-ap1.glsl"),
    };
    let mut code = pinned.to_owned();
    for (binding, name) in [(1, "aces_reach_m_table_0"), (2, "aces_gamut_cusp_table_0")] {
        let declaration =
            format!("layout(set=0, binding = {binding}) uniform sampler2D {name}Sampler;");
        assert_eq!(code.matches(&declaration).count(), 1);
        code = code.replace(&declaration, &format!("layout(set=0, binding = {binding}) uniform texture2D {name};\n#define {name}Sampler sampler2D({name}, aces_sampler)"));
    }
    let declaration = code
        .lines()
        .find(|line| line.starts_with("const float aces_gamut_cusp_table_0_hues_array[363]"))
        .unwrap()
        .to_owned();
    let values = declaration
        .split_once("float[363](")
        .unwrap()
        .1
        .strip_suffix(");")
        .unwrap();
    let hues: Vec<f32> = values
        .split(',')
        .map(|s| s.trim().parse().unwrap())
        .collect();
    assert_eq!(hues.len(), 363);
    if execution.storage_hues() {
        code = code.replace(&declaration, "layout(set=0,binding=5,std430) readonly buffer Hues { float aces_gamut_cusp_table_0_hues_array[]; };");
    }
    if execution.compute() {
        let mut fetches = 0;
        code = code
            .lines()
            .map(|line| {
                if line.contains("= texture(") {
                    fetches += 1;
                    line.replace("texture(", "textureLod(")
                        .replace(")).", "),0.0).")
                } else {
                    line.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(fetches, 4); // Both production tables have exactly one mip, no filtering.
    }
    let header = "#version 450\nlayout(set=0,binding=3) uniform sampler aces_sampler;\n";
    let inputs = "layout(set=0,binding=0,std430) readonly buffer Inputs { vec4 pixels[]; };\nlayout(set=0,binding=4,std140) uniform Dimensions { uvec4 size; };\n";
    let shell = if execution.compute() {
        "layout(local_size_x=8,local_size_y=8) in;\nlayout(set=0,binding=6,rgba32f) uniform writeonly image2D results;\nvoid main() { uvec2 xy=gl_GlobalInvocationID.xy; if(any(greaterThanEqual(xy,size.xy))) return; imageStore(results,ivec2(xy),antitone_render(vec4(pixels[xy.x+xy.y*size.x].rgb,1.0))); }"
    } else {
        "layout(location=0) in vec2 uv;\nlayout(location=0) out vec4 result;\nvoid main() { uvec2 xy=min(uvec2(max(uv,vec2(0.0))*vec2(size.xy)),size.xy-uvec2(1)); result=antitone_render(vec4(pixels[xy.x+xy.y*size.x].rgb,1.0)); }"
    };
    (format!("{header}{code}\n{inputs}{shell}"), hues)
}

enum ExecutionPipeline {
    Fragment(wgpu::RenderPipeline),
    Compute(wgpu::ComputePipeline),
}
pub(super) struct Executable {
    pipeline: ExecutionPipeline,
    group: wgpu::BindGroup,
    pub(super) rendered: ScreenTarget,
}
impl Executable {
    pub(super) async fn new(
        device: &wgpu::Device,
        renderer: &GpuRenderer,
        input: &wgpu::Buffer,
        size: [u32; 2],
        target: OutputTarget,
        execution: Execution,
    ) -> Self {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let (code, hues) = execution_shader(target, execution);
        let visibility = if execution.compute() {
            wgpu::ShaderStages::COMPUTE
        } else {
            wgpu::ShaderStages::FRAGMENT
        };
        let resources = renderer.target(target);
        let buffer = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            count: None,
            ty: wgpu::BindingType::Buffer {
                ty,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
        };
        let mut entries = vec![
            buffer(0, wgpu::BufferBindingType::Storage { read_only: true }),
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility,
                count: None,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility,
                count: None,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
            },
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility,
                count: None,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
            },
            buffer(4, wgpu::BufferBindingType::Uniform),
        ];
        if execution.storage_hues() {
            entries.push(buffer(
                5,
                wgpu::BufferBindingType::Storage { read_only: true },
            ));
        }
        if execution.compute() {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: 6,
                visibility,
                count: None,
                ty: wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format: wgpu::TextureFormat::Rgba32Float,
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
            });
        }
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("equivalent ACES execution bindings"),
            entries: &entries,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pinned ACES equivalent execution"),
            source: wgpu::ShaderSource::Glsl {
                shader: code.into(),
                stage: if execution.compute() {
                    wgpu::naga::ShaderStage::Compute
                } else {
                    wgpu::naga::ShaderStage::Fragment
                },
                defines: Default::default(),
            },
        });
        let pipeline = if execution.compute() {
            ExecutionPipeline::Compute(device.create_compute_pipeline(
                &wgpu::ComputePipelineDescriptor {
                    label: Some("ACES compute shell"),
                    layout: Some(&pipeline_layout),
                    module: &module,
                    entry_point: Some("main"),
                    compilation_options: Default::default(),
                    cache: None,
                },
            ))
        } else {
            let vertex = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: None,
                source: wgpu::ShaderSource::Wgsl(crate::renderer::VERTEX.into()),
            });
            ExecutionPipeline::Fragment(device.create_render_pipeline(
                &wgpu::RenderPipelineDescriptor {
                    label: Some("ACES fragment execution reference"),
                    layout: Some(&pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &vertex,
                        entry_point: Some("main"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &module,
                        entry_point: Some("main"),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: wgpu::TextureFormat::Rgba32Float,
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    primitive: Default::default(),
                    depth_stencil: None,
                    multisample: Default::default(),
                    multiview_mask: None,
                    cache: None,
                },
            ))
        };
        let mut rendered = ScreenTarget::new(device, renderer, input, size, target);
        // Compute writes directly into the same precision/shape of target used by reconstruction.
        if execution.compute() {
            rendered.texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("ACES compute linear target RGB"),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba32Float,
                usage: wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
        }
        let dimensions = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&[size[0], size[1], 0, 0]),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let hue_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("unchanged pinned ACES cusp hue values"),
            contents: bytemuck::cast_slice(&hues),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let view = rendered.texture.create_view(&Default::default());
        let mut entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: input.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&resources.tables[0]),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&resources.tables[1]),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Sampler(&resources.sampler),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: dimensions.as_entire_binding(),
            },
        ];
        if execution.storage_hues() {
            entries.push(wgpu::BindGroupEntry {
                binding: 5,
                resource: hue_buffer.as_entire_binding(),
            });
        }
        if execution.compute() {
            entries.push(wgpu::BindGroupEntry {
                binding: 6,
                resource: wgpu::BindingResource::TextureView(&view),
            });
        }
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &entries,
        });
        assert!(scope.pop().await.is_none());
        Self {
            pipeline,
            group,
            rendered,
        }
    }
    fn encode(&self, encoder: &mut wgpu::CommandEncoder) {
        let group = &self.group;
        match &self.pipeline {
            ExecutionPipeline::Compute(pipeline) => {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, group, &[]);
                pass.dispatch_workgroups(
                    self.rendered.texture.width().div_ceil(8),
                    self.rendered.texture.height().div_ceil(8),
                    1,
                );
            }
            ExecutionPipeline::Fragment(pipeline) => {
                let view = self.rendered.texture.create_view(&Default::default());
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("ACES equivalent fragment execution"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, group, &[]);
                pass.draw(0..3, 0..1);
            }
        }
    }
}

pub(super) async fn context() -> (wgpu::Device, wgpu::Queue, Arc<GpuRenderer>) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::METAL,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = instance.request_adapter(&Default::default()).await.unwrap();
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            required_limits: crate::platform::device_limits(adapter.limits()),
            ..Default::default()
        })
        .await
        .unwrap();
    println!("aces_execution_adapter {:?}", adapter.get_info());
    let renderer = GpuRenderer::new(&device, &queue).await.unwrap();
    (device, queue, renderer)
}
pub(super) fn complete(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encode: impl FnOnce(&mut wgpu::CommandEncoder),
) -> Timing {
    let start = Instant::now();
    let mut encoder = device.create_command_encoder(&Default::default());
    encode(&mut encoder);
    let encode_ms = start.elapsed().as_secs_f64() * 1000.;
    let submission = queue.submit([encoder.finish()]);
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })
        .unwrap();
    Timing {
        encode_ms,
        completion_ms: start.elapsed().as_secs_f64() * 1000.,
    }
}
fn pixels(size: [u32; 2]) -> Vec<[f32; 4]> {
    let levels = [
        -1., -0.2, 0., 0.00001, 0.001, 0.01, 0.05, 0.18, 0.5, 1., 2., 8., 32., 1000.,
    ];
    (0..size[0] * size[1])
        .map(|i| {
            let i = i as usize;
            [
                levels[i % levels.len()],
                levels[(i / levels.len()) % levels.len()],
                levels[(i / (levels.len() * levels.len())) % levels.len()],
                0.,
            ]
        })
        .collect()
}
pub(super) fn require_agreement(actual: &[u32], expected: &[u32]) -> f32 {
    assert_eq!(actual.len(), expected.len());
    let mut maximum = 0_f32;
    for (&a, &b) in actual.iter().zip(expected) {
        let a = f32::from_bits(a);
        let b = f32::from_bits(b);
        assert!(a.is_finite() && b.is_finite());
        maximum = maximum.max((a - b).abs());
        assert!(
            (a - b).abs() < 2e-5,
            "ACES equivalent execution: {a} != {b}"
        );
    }
    maximum
}

#[test]
fn equivalent_aces_execution_preserves_signed_hdr_grid_and_both_targets() {
    pollster::block_on(async {
        let (device, queue, renderer) = context().await;
        let mut pixels = pixels([128, 32]);
        // Dense hue/exposure sweeps exercise dynamic cusp indexing, wrap and HDR branches.
        for h in 0..1440 {
            let angle = f64::from(h) * std::f64::consts::TAU / 1440.;
            let rgb = DVec3::new(
                0.5 + 0.6 * angle.cos(),
                0.5 + 0.6 * (angle + std::f64::consts::TAU / 3.).cos(),
                0.5 + 0.6 * (angle - std::f64::consts::TAU / 3.).cos(),
            );
            for ev in [-12, -6, -2, 0, 2, 6, 12] {
                let p = (rgb * 2_f64.powi(ev)).as_vec3();
                pixels.push([p.x, p.y, p.z, 0.]);
            }
        }
        let size = [128, (pixels.len() as u32).div_ceil(128)];
        pixels.resize((size[0] * size[1]) as usize, [0.; 4]);
        let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&pixels),
            usage: wgpu::BufferUsages::STORAGE,
        });
        for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
            let reference = Executable::new(
                &device,
                &renderer,
                &input,
                size,
                target,
                Execution::FragmentConstants,
            )
            .await;
            complete(&device, &queue, |encoder| reference.encode(encoder));
            let expected = reference.rendered.words(&device, &queue);
            let production = ScreenTarget::new(&device, &renderer, &input, size, target);
            complete(&device, &queue, |encoder| {
                production.encode(&renderer, encoder)
            });
            let actual = production.words(&device, &queue);
            assert_eq!(
                actual, expected,
                "production resource adaptation changes pinned output for {target:?}"
            );
            let cpu = CpuRenderer::new(target)
                .unwrap()
                .render(
                    &pixels
                        .iter()
                        .map(|p| DVec3::new(f64::from(p[0]), f64::from(p[1]), f64::from(p[2])))
                        .collect::<Vec<_>>(),
                )
                .unwrap();
            let cpu: Vec<u32> = cpu
                .iter()
                .flat_map(|p| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits(), 1_f32.to_bits()])
                .collect();
            require_agreement(&expected, &cpu);
            for execution in [
                Execution::FragmentStorageHues,
                Execution::ComputeConstants,
                Execution::ComputeStorageHues,
            ] {
                let variant =
                    Executable::new(&device, &renderer, &input, size, target, execution).await;
                complete(&device, &queue, |encoder| variant.encode(encoder));
                let actual = variant.rendered.words(&device, &queue);
                let error = require_agreement(&actual, &expected);
                require_agreement(&actual, &cpu);
                println!(
                    "aces_execution_validation target={target:?} execution={execution:?} max_error_vs_fragment={error} exact_words={}",
                    actual == expected
                );
            }
        }
    });
}

fn performance_input(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &GpuRenderer,
    size: [u32; 2],
) -> wgpu::Buffer {
    #[cfg(feature = "raw")]
    if std::env::var_os("ANTITONE_SCREEN_RAW").is_some() {
        let (scan, calibration) = crate::raw_test_support::train_raw::load();
        let raster = InputRaster::raw(&scan, calibration, size);
        let mut source = ProcessingSource::new(device, &raster.pixels, raster.matrix).unwrap();
        let r = raster.region;
        source
            .set_geometry(SourceGeometry::new([r.x, r.y, r.width, r.height], size).unwrap())
            .unwrap();
        let grade = GpuPhotoGrade::new(
            device,
            calibration,
            RollSettings::new_grade(),
            edited("global-photographic", 0),
            raster.matrix,
            raster.area,
        )
        .unwrap();
        let input = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("RAW-derived authored nonlinear Global AP1"),
            size: u64::from(size[0]) * u64::from(size[1]) * 16,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        complete(device, queue, |encoder| {
            renderer
                .kernel
                .encode_grade(device, encoder, &source, &input, &grade)
                .unwrap()
        });
        println!(
            "aces_execution_input kind=sony_original_centre_global gamma=0.60 correction=none"
        );
        return input;
    }
    let _ = (queue, renderer);
    let pixels = pixels(size);
    println!("aces_execution_input kind=authored_signed_hdr_grid");
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("signed HDR AP1 ACES fixture"),
        contents: bytemuck::cast_slice(&pixels),
        usage: wgpu::BufferUsages::STORAGE,
    })
}

#[test]
#[ignore = "screen-sized equivalent ACES execution, not production or physical latency"]
fn aces_execution_cost() {
    pollster::block_on(async {
        let (device, queue, renderer) = context().await;
        let size = match std::env::var("ANTITONE_SCREEN_PERF_SIZE").as_deref() {
            Ok("4k") => [3840, 2160],
            Ok("1080p") | Err(std::env::VarError::NotPresent) => [1920, 1080],
            other => panic!("invalid screen size: {other:?}"),
        };
        let input = performance_input(&device, &queue, &renderer, size);

        for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
            let reference = Executable::new(
                &device,
                &renderer,
                &input,
                size,
                target,
                Execution::FragmentConstants,
            )
            .await;
            complete(&device, &queue, |encoder| reference.encode(encoder));
            let expected = reference.rendered.words(&device, &queue);
            let production = ScreenTarget::new(&device, &renderer, &input, size, target);
            complete(&device, &queue, |encoder| {
                production.encode(&renderer, encoder)
            });
            assert_eq!(
                production.words(&device, &queue),
                expected,
                "production full raster differs from pinned constants"
            );
            let mut baseline = Vec::new();
            let mut adapted = Vec::new();
            for i in 0..5 {
                for use_production in if i % 2 == 0 {
                    [false, true]
                } else {
                    [true, false]
                } {
                    let timing = complete(&device, &queue, |encoder| {
                        if use_production {
                            production.encode(&renderer, encoder);
                        } else {
                            reference.encode(encoder);
                        }
                    });
                    if use_production {
                        adapted.push(timing.completion_ms);
                    } else {
                        baseline.push(timing.completion_ms);
                    }
                }
            }
            println!(
                "aces_production_cost size={size:?} target={target:?} original_constants_ms={:.3} production_ms={:.3} exact_words=true timing=encode_submit_completion excludes=compile_upload_readback_reconstruction_scopes_presentation",
                median(&baseline),
                median(&adapted)
            );
            for execution in [
                Execution::FragmentStorageHues,
                Execution::ComputeConstants,
                Execution::ComputeStorageHues,
            ] {
                let variant =
                    Executable::new(&device, &renderer, &input, size, target, execution).await;
                complete(&device, &queue, |encoder| variant.encode(encoder));
                let actual = variant.rendered.words(&device, &queue);
                let error = require_agreement(&actual, &expected);
                let exact_words = actual == expected;
                drop(actual);
                let mut baseline = Vec::new();
                let mut candidate = Vec::new();
                let mut encode = Vec::new();
                for i in 0..5 {
                    for reuse in if i % 2 == 0 {
                        [false, true]
                    } else {
                        [true, false]
                    } {
                        let timing = complete(&device, &queue, |encoder| {
                            if reuse {
                                variant.encode(encoder)
                            } else {
                                reference.encode(encoder)
                            }
                        });
                        if reuse {
                            candidate.push(timing.completion_ms);
                            encode.push(timing.encode_ms);
                        } else {
                            baseline.push(timing.completion_ms);
                        }
                    }
                }
                println!(
                    "aces_execution_cost size={size:?} target={target:?} execution={execution:?} fragment_ms={:.3} candidate_ms={:.3} candidate_encode_ms={:.3} max_error={error} exact_words={exact_words} metal_resource_bytes={} timing=encode_submit_completion excludes=compile_upload_readback_reconstruction_scopes_presentation",
                    median(&baseline),
                    median(&candidate),
                    median(&encode),
                    allocated_resource_bytes(&device).unwrap()
                );
            }
        }
    });
}
