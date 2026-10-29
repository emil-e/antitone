use super::*;
#[test]
fn look_failures_survive_reconstruction_only_for_positive_weight_samples() {
    use crate::gpu::look::Kernel;
    use std::{sync::Arc, time::Duration};
    pollster::block_on(async {
        let instance = crate::gpu::instance();
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let pipelines = Arc::new(AveragePipelines::new(&device).await.unwrap());
        let check = Kernel::new(&device).await.unwrap();
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
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            );
            let mut encoder = device.create_command_encoder(&Default::default());
            average
                .accumulate(&device, &mut encoder, &source, [0, 0])
                .unwrap();
            average.finish(&device, &mut encoder, &output).unwrap();
            let validation = check.check_rendered(&device, &mut encoder, &output);
            let submission = queue.submit([encoder.finish()]);
            validation.begin(|| {});
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(submission),
                    timeout: Some(Duration::from_secs(10)),
                })
                .unwrap();
            let result = validation.result().unwrap();
            assert_eq!(result.is_ok(), valid);
            if !valid {
                assert!(result.unwrap_err().to_string().contains("nonfinite"));
            }
        }
    });
}
