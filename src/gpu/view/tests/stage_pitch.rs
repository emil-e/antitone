use super::*;

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
        let stages = crate::pixel_pipeline::PixelPipeline {
            stages: vec![crate::pixel_pipeline::PixelStage::new(
                crate::image_mtf::ImageMtfParameters { sigma: 2. },
            )],
        };
        let plan = crate::acquisition::Plan::new(crate::acquisition::Request {
            output: frame,
            stages: &stages,
            sampling: crate::acquisition::Sampling::NeutralFootprints,
        })
        .unwrap();
        let raster = crate::gpu::raster::Raster::planned(&plan);
        assert_eq!(raster.pitch(), pitch.map(|v| v as f32));
        assert_eq!(plan.output_offset(), Some(radius));
        let prepared = crate::gpu::image_mtf::Parameters::new(raster, 2.);
        assert_eq!(&prepared.radii[..2], &radius);
    }
}

#[test]
fn rotated_unit_pitch_gaussian_obeys_declared_radius_at_the_acquisition_boundary() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let frame = rotated_frame([1., 1.]);
        let input = ProcessingInput::from(Arc::new(
            ProcessingInput::new(
                &device,
                InputSettings::Digital,
                Default::default(),
                AdjustmentLayerSnapshot::new(vec![]).unwrap(),
                DMat3::IDENTITY,
                frame.composition.area(),
            )
            .unwrap(),
        ))
        .with_film_emulation(crate::film_emulation::FilmEmulationSettings {
            enabled: true,
            size: 0.1,
            image_sigma: 0.1,
            amount: 0.,
            colour: 0.5,
        })
        .with_frame_size(frame.composition.crop().size);
        let spec = ViewSourceSpec::new(17, frame, InputSettings::Digital, DMat3::IDENTITY, None)
            .unwrap()
            .with_pipeline(input.pixel_pipeline().unwrap())
            .unwrap();
        assert_eq!(spec.plan.output_offset(), Some([8, 8]));
        let size = spec.input_frame().unwrap().output;
        // A signed/HDR stripe exactly eight cells before the requested left edge.
        // A spurious ninth tap clamps to it instead of reading the absent real neighbour.
        let values: Vec<[f32; 4]> = (0..size[0] * size[1])
            .map(|i| [if i % size[0] == 0 { 16. } else { -0.03 }, 0.18, 1.8, 8.])
            .collect();
        let source = Arc::new(ViewSource {
            spec,
            renderer,
            kernels,
            pixels: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("rotated finite-dependency oracle"),
                contents: bytemuck::cast_slice(&values),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            }),
        });
        let view = RenderedView::new(&device, source, &input, OutputTarget::Srgb, None).unwrap();
        view.ensure_submitted(
            &device,
            &queue,
            &crate::gpu::SubmissionGate::default(),
            &egui::Context::default(),
        );
        let pixels = read(&device, &queue, &view.graded.as_ref().unwrap().pixels);
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
