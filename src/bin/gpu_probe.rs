//! Checks OCIO-generated GLSL/LUTs on Metal against CPU references; no window is needed.
//! Requires fresh `.cache/probes` OCIO artifacts; writes adapted shaders there.
//! `--camera` additionally gates the complete model-4 RGB16 GPU path and floor flags.
//! Missing/stale requested camera artifacts fail; agreement is not photographic or monitor acceptance.
use std::borrow::Cow;
use std::sync::mpsc;
use std::time::Instant;

use wgpu::util::DeviceExt;

const VERTEX: &str = r#"
@vertex fn main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let positions = array<vec2f, 3>(vec2f(-1., -1.), vec2f(3., -1.), vec2f(-1., 3.));
    return vec4f(positions[i], 0., 1.);
}
"#;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    let camera = match arguments.as_slice() {
        [] => false,
        [flag] if flag == "--camera" => true,
        _ => return Err("Usage: antitone-gpu-probe [--camera]".into()),
    };
    pollster::block_on(run(camera))
}

async fn run(camera: bool) -> Result<(), Box<dyn std::error::Error>> {
    let instance = antitone::gpu::instance();
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions::default())
        .await?;
    println!("adapter={:?}", adapter.get_info());
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor::default())
        .await?;
    let input_bytes = std::fs::read(".cache/probes/aces-inputs-ap1.f32")?;
    if input_bytes.is_empty() || input_bytes.len() % 16 != 0 {
        return Err("invalid OCIO input artifact".into());
    }
    let pixels = (input_bytes.len() / 16) as u32;
    let width = pixels.min(2048);
    let height = pixels.div_ceil(width);
    let inputs = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("ACES reference inputs"),
        contents: &input_bytes,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    });
    if camera {
        let (working, reference) = camera_processing(&device, &queue).await?;
        if !input_bytes.ends_with(&reference) {
            return Err("Regenerate OCIO artifacts after creating the camera fixture".into());
        }
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(
            &working,
            0,
            &inputs,
            (input_bytes.len() - reference.len()) as u64,
            reference.len() as u64,
        );
        queue.submit([encoder.finish()]);
    }
    let vertex = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("probe fullscreen triangle"),
        source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(VERTEX)),
    });
    for target in ["srgb", "display-p3"] {
        let started = Instant::now();
        let mut generated = std::fs::read_to_string(format!(".cache/probes/{target}-ap1.glsl"))?;
        for (binding, name) in [(1, "aces_reach_m_table_0"), (2, "aces_gamut_cusp_table_0")] {
            let declaration =
                format!("layout(set=0, binding = {binding}) uniform sampler2D {name}Sampler;");
            if !generated.contains(&declaration) {
                return Err("unexpected generated OCIO resource declaration".into());
            }
            generated = generated.replace(
                &declaration,
                &format!(
                    "layout(set=0, binding = {binding}) uniform texture2D {name};\n#define {name}Sampler sampler2D({name}, aces_sampler)"
                ),
            );
        }
        let fragment_source = format!(
            "#version 450\nlayout(set=0, binding=3) uniform sampler aces_sampler;\n{generated}\nlayout(set=0, binding=0, std430) readonly buffer Inputs {{ vec4 pixels[]; }};\nlayout(location=0) out vec4 result;\nvoid main() {{ int index = int(gl_FragCoord.x) + int(gl_FragCoord.y) * {width}; result = index < {pixels} ? antitone_render(vec4(pixels[index].rgb, 1.0)) : vec4(0.0); }}\n"
        );
        std::fs::write(
            format!(".cache/probes/{target}-ap1-adapted.frag"),
            &fragment_source,
        )?;
        let error_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("non-filtering OCIO f32 resources"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    count: None,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    count: None,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    count: None,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    count: None,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("OCIO probe pipeline"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let fragment = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(target),
            source: wgpu::ShaderSource::Glsl {
                shader: Cow::Owned(fragment_source),
                stage: wgpu::naga::ShaderStage::Fragment,
                defines: Default::default(),
            },
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(target),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &vertex,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &fragment,
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
        });
        if let Some(error) = error_scope.pop().await {
            return Err(error.to_string().into());
        }
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor::default());
        let mut luts = Vec::new();
        for index in 0..2 {
            let bytes = std::fs::read(format!(".cache/probes/{target}-ap1-lut-{index}.f32"))?;
            let channels = if index == 0 { 1 } else { 3 };
            if bytes.len() != 363 * channels * 4 {
                return Err("unexpected generated OCIO table size".into());
            }
            let rgba: Vec<u8> = bytes
                .chunks_exact(channels * 4)
                .flat_map(|pixel| {
                    let mut expanded = [0u8; 16];
                    expanded[..channels * 4].copy_from_slice(pixel);
                    expanded[12..].copy_from_slice(&1.0_f32.to_le_bytes());
                    expanded
                })
                .collect();
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("OCIO f32 LUT"),
                size: wgpu::Extent3d {
                    width: 363,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba32Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
                texture.as_image_copy(),
                &rgba,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(363 * 16),
                    rows_per_image: Some(1),
                },
                texture.size(),
            );
            luts.push(texture.create_view(&Default::default()));
        }
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("OCIO resources"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: inputs.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&luts[0]),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&luts[1]),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let output = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ACES linear output"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let row_bytes = (width * 16).div_ceil(256) * 256;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ACES readback"),
            size: u64::from(row_bytes) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let view = output.create_view(&Default::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(target),
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
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        encoder.copy_texture_to_buffer(
            output.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row_bytes),
                    rows_per_image: Some(height),
                },
            },
            output.size(),
        );
        queue.submit([encoder.finish()]);
        let (sender, receiver) = mpsc::channel();
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                sender.send(result).unwrap();
            });
        device.poll(wgpu::PollType::wait_indefinitely())?;
        receiver.recv()??;
        let mapped = readback.slice(..).get_mapped_range()?;
        let expected = std::fs::read(format!(".cache/probes/{target}-ap1-reference.f32"))?;
        if expected.len() != input_bytes.len() {
            return Err("invalid OCIO reference artifact length".into());
        }
        let mut max_error = 0.0_f32;
        for (index, reference) in expected.as_chunks::<4>().0.iter().enumerate() {
            let pixel = index / 4;
            let offset = (pixel / width as usize) * row_bytes as usize
                + (pixel % width as usize) * 16
                + (index % 4) * 4;
            let actual = f32::from_le_bytes(mapped[offset..offset + 4].try_into()?);
            let reference = f32::from_le_bytes(*reference);
            let error = (actual - reference).abs();
            max_error = max_error.max(error);
            if !actual.is_finite()
                || !reference.is_finite()
                || error > 2e-5 + 2e-4 * reference.abs()
            {
                return Err(format!(
                    "{target} CPU/GPU mismatch component={index} actual={actual} expected={reference} error={error}"
                )
                .into());
            }
        }
        println!(
            "input_space=AP1 target={target} pixels={pixels} CPU_GPU_max_abs_error={max_error:.8e} compile_render_readback_seconds={:.3}",
            started.elapsed().as_secs_f64()
        );
    }
    Ok(())
}

async fn camera_processing(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> Result<(wgpu::Buffer, Vec<u8>), Box<dyn std::error::Error>> {
    use antitone::gpu::{ProcessingKernel, ProcessingParameters, ProcessingSource};
    let scans = std::fs::read(".cache/probes/camera-source-model5.rgb16")?;
    let matrix = std::fs::read(".cache/probes/camera-to-ap1-model5.f64")?;
    let reference = std::fs::read(".cache/probes/camera-working-ap1-model5.f32")?;
    let flags = std::fs::read(".cache/probes/camera-floor-flags-model5.u8")?;
    let parameters = std::fs::read(".cache/probes/camera-processing-model5.bin")?;
    if scans.is_empty()
        || scans.len() % 8 != 0
        || matrix.len() != 72
        || reference.len() != scans.len() * 2
        || flags.len() != scans.len() / 8
        || parameters.len() != std::mem::size_of::<ProcessingParameters>()
    {
        return Err("invalid camera-processing fixture".into());
    }
    let parameters: ProcessingParameters = bytemuck::pod_read_unaligned(&parameters);
    let matrix = glam::DMat3::from_cols_array(&std::array::from_fn(|c| {
        f64::from_le_bytes(matrix[c * 8..c * 8 + 8].try_into().unwrap())
    }));
    let codes: Vec<[u16; 4]> = scans
        .as_chunks::<8>()
        .0
        .iter()
        .map(|p| {
            std::array::from_fn(|c| u16::from_le_bytes(p[c * 2..c * 2 + 2].try_into().unwrap()))
        })
        .collect();
    let input = ProcessingSource::new(device, &codes, matrix)?;
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("real-image AP1 working RGB"),
        size: reference.len() as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("processing readback"),
        size: output.size(),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let kernel = ProcessingKernel::new(device).await?;
    let started = Instant::now();
    let mut encoder = device.create_command_encoder(&Default::default());
    kernel.encode(device, &mut encoder, &input, &output, parameters);
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, output.size());
    queue.submit([encoder.finish()]);
    let (sender, receiver) = mpsc::channel();
    readback.slice(..).map_async(wgpu::MapMode::Read, move |r| {
        sender.send(r).unwrap();
    });
    device.poll(wgpu::PollType::wait_indefinitely())?;
    receiver.recv()??;
    let mapped = readback.slice(..).get_mapped_range()?;
    let mut max_error = 0.0f32;
    for (index, expected) in reference.as_chunks::<4>().0.iter().enumerate() {
        let actual = f32::from_le_bytes(mapped[index * 4..index * 4 + 4].try_into()?);
        let expected = if index % 4 == 3 {
            f32::from(flags[index / 4])
        } else {
            f32::from_le_bytes(*expected)
        };
        let error = (actual - expected).abs();
        if !actual.is_finite() || !expected.is_finite() || error > 2e-5 + 2e-4 * expected.abs() {
            return Err(format!(
                "camera processing mismatch component={index} actual={actual} expected={expected}"
            )
            .into());
        }
        if index % 4 != 3 {
            max_error = max_error.max(error);
        }
    }
    println!(
        "camera_processing_pixels={} CPU_f64_GPU_f32_max_abs_error={max_error:.8e} submit_readback_seconds={:.6}",
        scans.len() / 8,
        started.elapsed().as_secs_f64()
    );
    drop(mapped);
    readback.unmap();
    Ok((output, reference))
}
