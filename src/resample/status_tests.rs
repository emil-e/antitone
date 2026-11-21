use super::*;
#[test]
fn rejection_flags_survive_reconstruction_only_for_positive_weight_samples() {
    use std::{sync::Arc, time::Duration};
    pollster::block_on(async {
        let instance = crate::gpu::instance();
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = crate::gpu::submission::request_device(&adapter, &Default::default())
            .await
            .unwrap();
        let pipelines = Arc::new(AveragePipelines::new(&device).await.unwrap());
        let texture = |width, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba32Float,
                usage,
                view_formats: &[],
            })
        };
        let source = texture(
            2,
            wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &source,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&[[0f32, 0., 0., -2.], [0.18, 0.18, 0.18, 1.]]),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(32),
                rows_per_image: Some(1),
            },
            source.size(),
        );
        for (affine, valid) in [
            (None, false),
            (Some([[1., 0.], [1., 0.], [0., 1.]]), true),
            (Some([[0.5, 0.], [1., 0.], [0., 1.]]), false),
        ] {
            let average =
                AreaAverage::create(&device, pipelines.clone(), [2, 1], [1, 1], affine).unwrap();
            let output = texture(
                1,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            );
            let mut encoder = device.create_command_encoder(&Default::default());
            average
                .accumulate(&device, &mut encoder, &source, [0, 0])
                .unwrap();
            average.finish(&device, &mut encoder, &output).unwrap();
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 16,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_texture_to_buffer(
                output.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: Default::default(),
                },
                output.size(),
            );
            let submission = queue.submit([encoder.finish()]);
            let (send, receive) = std::sync::mpsc::channel();
            buffer.map_async(wgpu::MapMode::Read, .., move |result| {
                send.send(result).unwrap()
            });
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(submission),
                    timeout: Some(Duration::from_secs(10)),
                })
                .unwrap();
            receive
                .recv_timeout(Duration::from_secs(10))
                .unwrap()
                .unwrap();
            let mapped = buffer.get_mapped_range(..).unwrap();
            let pixel: &[f32; 4] = bytemuck::from_bytes(&mapped);
            assert_eq!(pixel[3] >= 0., valid);
        }
    });
}
