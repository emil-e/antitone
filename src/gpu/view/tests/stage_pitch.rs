use super::*;
use crate::gpu::test_inputs::AuthoredPhotograph;
use crate::photographic::graph::view_source::SourceRequest;

fn rotated_frame(pitch: [f64; 2]) -> FrameRaster {
    let composition = Composition::with_crop(
        [100, 80, 6000, 6000].into(),
        Default::default(),
        Default::default(),
        Some(crate::crop::Crop {
            center: [3100., 3080.],
            size: [2000, 2000],
            angle: 0.29,
        }),
    )
    .unwrap();
    FrameRaster::new(
        composition,
        FramePoint([0.25, 1.75]),
        FramePoint([0.25 + 64. * pitch[0], 1.75 + 64. * pitch[1]]),
        [64, 64],
    )
    .unwrap()
}

#[test]
fn rotated_rasters_preserve_declared_pitch_and_finite_dependencies_through_padding() {
    for (pitch, radius) in [
        ([1., 1.], [8, 8]),
        ([0.5, 0.5], [16, 16]),
        ([2., 2.], [4, 4]),
        ([1.5, 3.], [6, 3]),
    ] {
        let frame = rotated_frame(pitch);
        let stages = {
            let mtf = crate::photographic::stages::image_mtf::ImageMtfParameters { sigma: 2. };
            let pitch = [Some(mtf.sigma / 0.6)]
                .into_iter()
                .flatten()
                .reduce(f64::min);
            crate::photographic::graph::authoring::Assembly::authored(move |mut image, output| {
                image = mtf.compose(image)?;
                if let Some(output) = output {
                    image = output.compose(image)?;
                }
                Ok(image)
            })
            .with_view_pitch(pitch)
        };
        let plan = crate::acquisition::Plan::for_graph(
            crate::acquisition::Request {
                output: frame,

                sampling: crate::acquisition::Sampling::NeutralFootprints,
            },
            |input| stages.sampling_image(input),
        )
        .unwrap();
        let raster = crate::gpu::raster::Raster::planned(&plan);
        assert_eq!(raster.pitch(), pitch.map(|v| v as f32));
        assert_eq!(plan.output_offset(), Some(radius));
        let prepared =
            crate::photographic::stages::image_mtf::shader_fixture::Parameters::new(raster, 2.);
        assert_eq!(&prepared.radii[..2], &radius);
    }
}

#[test]
fn rotated_unit_pitch_gaussian_obeys_declared_radius_at_the_acquisition_boundary() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let frame = rotated_frame([1., 1.]);
        let input = AuthoredPhotograph::from(Arc::new(
            AuthoredPhotograph::new(
                &device.limits(),
                InputSettings::Digital,
                Default::default(),
                AdjustmentLayerSnapshot::new(vec![]).unwrap(),
                DMat3::IDENTITY,
                frame.composition.area(),
            )
            .unwrap(),
        ))
        .with_definition(
            crate::photographic::graph::authoring::Assembly::emulation(
                crate::photographic::stages::grain::GrainSettings {
                    enabled: true,
                    size: 0.1,
                    image_sigma: 0.1,
                    amount: 0.,
                    colour: 0.5,
                },
                Default::default(),
                frame.composition.crop().size,
                0,
            )
            .unwrap(),
        );
        let spec = SourceRequest::new(frame, InputSettings::Digital, DMat3::IDENTITY, None)
            .unwrap()
            .with_definition(input.definition())
            .unwrap();
        assert_eq!(spec.plan().output_offset(), Some([8, 8]));
        let size = spec.input_frame().unwrap().output;
        // A signed/HDR stripe exactly eight cells before the requested left edge.
        // A spurious ninth tap clamps to it instead of reading the absent real neighbour.
        let values: Vec<[f32; 4]> = (0..size[0] * size[1])
            .map(|i| [if i % size[0] == 0 { 16. } else { -0.03 }, 0.18, 1.8, 8.])
            .collect();
        let source = Arc::new(ViewImage::from_neutral_samples(
            spec,
            renderer,
            kernels,
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("rotated finite-dependency oracle"),
                contents: bytemuck::cast_slice(&values),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            }),
        ));
        let view = RenderedView::new(&device, source, &input, OutputTarget::Srgb, None).unwrap();
        view.ensure_submitted(&device, &queue, &egui::Context::default());
        let pixels = read(&device, &queue, &view.product.as_ref().unwrap().pixels);
        let weights: Vec<f64> = (-8..=8)
            .map(|x| (-0.5 * f64::from(x * x) / 4.).exp())
            .collect();
        let expected = -0.03 + (16. + 0.03) * weights[0] / weights.iter().sum::<f64>();
        for y in 0..64 {
            let actual = pixels[y * 64][0];
            assert!(
                (f64::from(actual) - expected).abs() < 2e-6,
                "row={y}: actual={actual}, finite-radius convolution={expected}"
            );
        }
    });
}
