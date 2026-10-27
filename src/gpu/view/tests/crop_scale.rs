use super::*;

#[test]
fn a_tighter_crop_reduces_the_source_gaussian_instead_of_magnifying_uncropped_blur() {
    pollster::block_on(async {
        let (device, queue, _, kernels) = context().await;
        let settings = crate::film_emulation::FilmEmulationSettings {
            enabled: true,
            amount: 0.,
            size: 0.040,
            colour: 0.5,
        };
        let values: Vec<[f32; 4]> = (0..65)
            .map(|x| [if x >= 32 { 0.4 } else { -0.03 }, 0.18, 1.5, 0.])
            .collect();
        let raster = crate::gpu::raster::Raster::native([100, 80, 65, 1].into());
        let mut results = Vec::new();
        // Independent expected sigmas: 0.040% of shorter crop side 6000 / 3000.
        for (frame_size, sigma) in [([8000, 6000], 2.4_f64), ([4000, 3000], 1.2)] {
            let pixels = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("crop-scale step control"),
                contents: bytemuck::cast_slice(&values),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            });
            let plan =
                crate::pixel_pipeline::PixelPipeline::film(settings, frame_size, 17).unwrap();
            let prepared =
                crate::gpu::pixel_pipeline::Prepared::new(&device, &plan, raster, None).unwrap();
            let mut encoder = device.create_command_encoder(&Default::default());
            kernels
                .stages
                .encode(&device, &mut encoder, &prepared, &pixels)
                .unwrap();
            queue.submit([encoder.finish()]);
            let output = read(&device, &queue, &pixels);
            let radius = (4. * sigma).ceil() as i32;
            let mass: f64 = (-radius..=radius)
                .map(|i| (-0.5 * f64::from(i * i) / sigma.powi(2)).exp())
                .sum();
            for (x, pixel) in output.iter().enumerate().take(41).skip(24) {
                let expected: f64 = (-radius..=radius)
                    .map(|i| {
                        let value = if x as i32 + i >= 32 {
                            0.4_f32
                        } else {
                            -0.03_f32
                        };
                        f64::from(value) * (-0.5 * f64::from(i * i) / sigma.powi(2)).exp() / mass
                    })
                    .sum();
                assert!(
                    (f64::from(pixel[0]) - expected).abs() < 0.000002,
                    "crop {frame_size:?}, independent Gaussian x={x}"
                );
            }
            results.push(output);
        }
        assert!(
            results[1][33][0] > results[0][33][0] + 0.025,
            "the crop must retain a sharper edge, not the uncropped frame's blur"
        );
    });
}
