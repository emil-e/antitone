use crate::{
    acquisition::raster::{Raster, buffer, uniform},
    evaluation::read_buffer as read,
    photographic::test_support::render_context,
};
use wgpu::util::DeviceExt;

#[test]
fn image_mtf_shader_preserves_large_signed_hdr_constants() {
    let (device, queue, _) = render_context();
    pollster::block_on(async {
        let mtf = super::shader_fixture::Kernels::new(&device).await.unwrap();
        let value = 2_f32.powi(122);
        let values = vec![[value, -value, 0.0001, 8.]; 65];
        let source = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("admissible large signed HDR"),
            contents: bytemuck::cast_slice(&values),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let output = buffer(&device, [65, 1], "MTF numerical output").unwrap();
        for footprint in [1. / 16., 2. / 0.075, 64.] {
            let raster = Raster {
                dimensions: [65, 1, 8, 0],
                axis_x: [footprint, 0., footprint, 0.],
                axis_y: [0., 1., 1., 0.],
                ..bytemuck::Zeroable::zeroed()
            };
            let view = uniform(
                &device,
                &super::shader_fixture::Parameters::new(raster, 2.),
                "MTF numerical control",
            );
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &mtf.horizontal.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: output.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 10,
                        resource: view.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 11,
                        resource: source.as_entire_binding(),
                    },
                ],
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&mtf.horizontal);
                pass.set_bind_group(0, &group, &[]);
                pass.dispatch_workgroups(2, 1, 1);
            }
            queue.submit([encoder.finish()]);
            for pixel in read(&device, &queue, &output) {
                assert!(pixel.iter().all(|v| v.is_finite()));
                assert!((pixel[0] / value - 1.).abs() < 0.00003);
                assert!((pixel[1] / (-value) - 1.).abs() < 0.00003);
                assert!((pixel[2] / 0.0001 - 1.).abs() < 0.00003);
            }
        }
    });
}
