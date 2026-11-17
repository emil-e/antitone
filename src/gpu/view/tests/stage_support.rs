use super::*;
use crate::gpu::submission::SubmissionGate;
use crate::gpu::test_inputs::AuthoredPhotograph;
use crate::gpu::view::request::SourceRequest;

#[test]
fn viewport_and_accepted_crop_edges_do_not_become_filter_boundaries() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let area = [100, 80, 6000, 6000].into();
        let full = Composition::new(area, Default::default(), Default::default()).unwrap();
        let cropped = Composition::with_crop(
            area,
            Default::default(),
            Default::default(),
            Some(crate::crop::Crop {
                center: [3008., 3000.],
                size: [4000, 4000],
                angle: 0.,
            }),
        )
        .unwrap();
        let small = FrameRaster::new(
            cropped,
            FramePoint([0., 0.]),
            FramePoint([64., 64.]),
            [64, 64],
        )
        .unwrap();
        let wide = FrameRaster::new(
            full,
            FramePoint([892., 920.]),
            FramePoint([988., 984.]),
            [96, 64],
        )
        .unwrap();
        let snapshot = AdjustmentLayerSnapshot::new(vec![]).unwrap();
        let grade = Arc::new(
            AuthoredPhotograph::new(
                &device.limits(),
                InputSettings::Digital,
                Default::default(),
                snapshot.clone(),
                DMat3::IDENTITY,
                area,
            )
            .unwrap(),
        );
        // Resolve sigma directly to isolate boundary handling from crop-relative recipe assembly.
        let input = AuthoredPhotograph::from(grade).with_definition({
            let mtf = crate::image_mtf::ImageMtfParameters { sigma: 2. };
            let pitch = [Some(mtf.sigma / 0.6)]
                .into_iter()
                .flatten()
                .reduce(f64::min);
            crate::gpu::photograph::assembly::Assembly::authored(move |mut image, output| {
                image = mtf.compose(image)?;
                if let Some(output) = output {
                    image = output.compose(image)?;
                }
                Ok(image)
            })
            .with_view_pitch(pitch)
        });
        let make = |frame: FrameRaster, padded: bool| {
            let spec =
                SourceRequest::new(frame, InputSettings::Digital, DMat3::IDENTITY, None).unwrap();
            let spec = if padded {
                spec.with_definition(input.definition()).unwrap()
            } else {
                spec
            };
            let sampled = spec.input_frame().unwrap();
            let mut values: Vec<[f32; 4]> = Vec::new();
            for y in 0..sampled.output[1] {
                for x in 0..sampled.output[0] {
                    let p = sampled.source_point([x as f64 + 0.5, y as f64 + 0.5]).0;
                    values.push([if p[0] >= 1008. { 0.4 } else { -0.03 }, 0.18, 0.05, 8.]);
                }
            }
            let source = Arc::new(ViewImage::from_neutral_samples(
                spec,
                renderer.clone(),
                kernels.clone(),
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: None,
                    contents: bytemuck::cast_slice(&values),
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                }),
            ));
            let view = RenderedView::new(&device, source, &input, OutputTarget::Srgb, None)?;
            view.ensure_submitted(
                &device,
                &queue,
                &SubmissionGate::default(),
                &egui::Context::default(),
            );
            Ok::<_, Error>(read(
                &device,
                &queue,
                &view.product.as_ref().unwrap().pixels,
            ))
        };
        let a = make(small, true).unwrap();
        let b = make(wide, true).unwrap();
        assert!(
            matches!(make(small, false), Err(Error::InvalidParameter(_))),
            "missing neighbours must reject processing rather than turn the viewport into a filter edge"
        );
        let radius = 8;
        let mass: f64 = (-radius..=radius)
            .map(|i| (-0.5 * (i * i) as f64 / 4.).exp())
            .sum();
        for y in 0..64 {
            for x in 0..64 {
                let expected: f64 = (-radius..=radius)
                    .map(|i| {
                        let sample = if 1008 + x as i32 + i >= 1008 {
                            0.4_f32
                        } else {
                            -0.03_f32
                        };
                        (-0.5 * (i * i) as f64 / 4.).exp() * sample as f64 / mass
                    })
                    .sum();
                assert!(
                    (a[y * 64 + x][0] as f64 - expected).abs() < 0.000002,
                    "independent Gaussian at crop/viewport edge x={x}"
                );
                for c in 0..4 {
                    assert!(
                        (a[y * 64 + x][c] - b[y * 96 + x + 16][c]).abs() < 0.000002,
                        "shared source pixel differs after pan/crop, x={x} y={y} c={c}"
                    );
                }
            }
        }
    });
}
