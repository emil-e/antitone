#![cfg(all(feature = "gpu", target_os = "macos"))]
use antitone::{
    gpu::{
        SubmissionGate,
        look::{Kernel, Lut},
    },
    looks::Lut3D,
};
use glam::DVec3;
use std::{
    sync::{Arc, mpsc},
    time::Duration,
};
use wgpu::util::DeviceExt;
fn decode(x: f64) -> f64 {
    if x <= 0.155251141552511 {
        (x - 0.0729055341958355) / 10.5402377416545
    } else {
        (17.52 * x - 9.72).exp2()
    }
}

fn encode(x: f64) -> f64 {
    if x <= 0.0078125 {
        10.5402377416545 * x + 0.0729055341958355
    } else {
        (x.log2() + 9.72) / 17.52
    }
}

#[test]
fn resource_admission_is_allocation_free_and_checks_storage_and_buffer_limits() {
    let look = Lut3D::new(2, [0.; 3], [1.; 3], vec![[0.4; 3]; 8]).unwrap();
    let mut limits = wgpu::Limits {
        max_storage_buffer_binding_size: 128,
        max_buffer_size: 128,
        ..Default::default()
    };
    Lut::require_limits(&limits, &look).unwrap();
    limits.max_storage_buffer_binding_size = 127;
    assert!(Lut::require_limits(&limits, &look).is_err());
    limits.max_storage_buffer_binding_size = 128;
    limits.max_buffer_size = 127;
    assert!(Lut::require_limits(&limits, &look).is_err());
}

#[test]
fn tetrahedral_hinges_cover_all_orders_ties_edges_clipping_and_nonfinite_rejection() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let mut nodes = Vec::new();
        // These corner products have tetrahedral interpolants min(r,g), min(g,b),
        // min(b,r). Trilinear interpolation instead returns the interior products.
        for b in 0..2 {
            for g in 0..2 {
                for r in 0..2 {
                    nodes.push([(r * g) as f32, (g * b) as f32, (b * r) as f32]);
                }
            }
        }
        let cpu = Arc::new(Lut3D::new(2, [-0.5; 3], [1.; 3], nodes).unwrap());
        let gpu = Lut::new(&device, &cpu).unwrap();
        let kernel = Kernel::new(&device).await.unwrap();
        let gate = SubmissionGate::default();
        for case in 0..6 {
            let mut input = Vec::new();
            for r in [-0.4, 0.2, 0.9] {
                for g in [-0.3, 0.3, 0.8] {
                    for b in [-0.2, 0.4, 0.7] {
                        input.push([decode(r) as f32, decode(g) as f32, decode(b) as f32, 8.]);
                    }
                }
            }
            for point in [
                [-0.5; 3],
                [1.; 3],
                [0.25; 3],
                [0.25, 0.25, 0.75],
                [0.25, 0.75, 0.25],
                [0.75, 0.25, 0.25],
                [-0.5, 0.25, 1.],
                [1., -0.5, 0.25],
            ] {
                let point = point.map(|value| decode(value) as f32);
                input.push([point[0], point[1], point[2], 8.]);
            }
            match case {
                1 => input[3] = [-0.06, 0.18, 36_000., 8.],
                2 => input[3] = [-f32::MAX, f32::MAX, 0.18, 8.],
                3 => input[3][0] = f32::NAN,
                4 => input[3][0] = f32::INFINITY,
                5 => input[3][0] = f32::NEG_INFINITY,
                _ => {}
            }
            let source = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::cast_slice(&input),
                usage: wgpu::BufferUsages::STORAGE,
            });
            let output = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: source.size(),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: source.size(),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let dispatch = kernel
                .bind(
                    &device,
                    &source,
                    &output,
                    gpu.clone(),
                    input.len() as u32,
                    false,
                )
                .unwrap();
            let mut encoder = device.create_command_encoder(&Default::default());
            kernel.encode(&mut encoder, &dispatch);
            encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, output.size());
            let submission = gate.submit(&queue, [encoder.finish()]);
            dispatch.check.begin(|| {});
            let (send, receive) = mpsc::channel();
            readback.map_async(wgpu::MapMode::Read, .., move |r| send.send(r).unwrap());
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(submission),
                    timeout: Some(Duration::from_secs(30)),
                })
                .unwrap();
            receive
                .recv_timeout(Duration::from_secs(30))
                .unwrap()
                .unwrap();
            let status = dispatch.check.result().unwrap();
            if case >= 3 {
                assert!(status.unwrap_err().to_string().contains("nonfinite"));
            } else {
                status.unwrap();
                let mapped = readback.get_mapped_range(..).unwrap();
                let values = bytemuck::cast_slice::<u8, [f32; 4]>(&mapped);
                for (actual, x) in values.iter().zip(&input) {
                    let scene = DVec3::new(x[0] as f64, x[1] as f64, x[2] as f64);
                    let position = (scene.map(encode).clamp(DVec3::splat(-0.5), DVec3::ONE)
                        + DVec3::splat(0.5))
                        / 1.5;
                    let expected = DVec3::new(
                        position.x.min(position.y),
                        position.y.min(position.z),
                        position.z.min(position.x),
                    )
                    .map(decode);
                    assert!((cpu.apply(scene).unwrap() - expected).abs().max_element() < 1e-10);
                    for c in 0..3 {
                        assert!(
                            (actual[c] as f64 - expected[c]).abs()
                                < 2e-5 + 2e-4 * expected[c].abs(),
                            "{x:?} {actual:?} {expected:?}"
                        );
                    }
                    assert_eq!(actual[3], 8.);
                }
            }
            readback.unmap();
        }
    });
}

#[test]
fn composed_admission_ignores_unused_texels_but_not_positive_overlap() {
    use antitone::geometry::TextureMapping;
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let kernel = Kernel::new(&device).await.unwrap();
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 2,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&[[0f32, 0., 0., -1.], [0.18, 0.18, 0.18, 1.]]),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(32),
                rows_per_image: Some(1),
            },
            texture.size(),
        );
        for (mapping, valid) in [
            (TextureMapping::IDENTITY, false),
            (
                TextureMapping {
                    origin: [0.5, 0.],
                    x_axis: [0.5, 0.],
                    y_axis: [0., 1.],
                },
                true,
            ),
            (
                TextureMapping {
                    origin: [1., 0.],
                    x_axis: [-0.5, 0.],
                    y_axis: [0., 1.],
                },
                true,
            ),
            (
                TextureMapping {
                    origin: [0.49, 0.],
                    x_axis: [0.51, 0.],
                    y_axis: [0., 1.],
                },
                false,
            ),
        ] {
            let mut encoder = device.create_command_encoder(&Default::default());
            let check = kernel
                .check_projected(&device, &mut encoder, &texture, mapping)
                .unwrap();
            let submission = queue.submit([encoder.finish()]);
            check.begin(|| {});
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(submission),
                    timeout: Some(Duration::from_secs(30)),
                })
                .unwrap();
            assert_eq!(check.result().unwrap().is_ok(), valid, "{mapping:?}");
        }
    });
}
