use super::*;
use crate::{
    adjustment_layers::AdjustmentLayerSnapshot,
    halation::{HalationSettings, reference},
};
use wgpu::util::DeviceExt;

#[test]
fn independent_halation_acquires_real_neighbours_beyond_a_rotated_physical_view() {
    let (device, queue, renderer, kernels) = pollster::block_on(context());
    let area = [100, 80, 6000, 4000].into();
    let composition = Composition::with_crop(
        area,
        Default::default(),
        Default::default(),
        Some(crate::crop::Crop {
            center: [3000., 2000.],
            size: [200, 160],
            angle: 0.37,
        }),
    )
    .unwrap();
    let frame = FrameRaster::new(
        composition,
        FramePoint([21.5, 30.25]),
        FramePoint([117.5, 102.25]),
        [48, 36],
    )
    .unwrap();
    let halo = HalationSettings {
        enabled: true,
        size: 0.5,
        strength: 0.4,
    };
    let processing = ProcessingInput::new(
        &device,
        InputSettings::Digital,
        Default::default(),
        AdjustmentLayerSnapshot::new(vec![]).unwrap(),
        glam::DMat3::IDENTITY,
        area,
    )
    .unwrap()
    .with_halation(halo)
    .with_frame_size(composition.crop().size);
    let spec = ViewSourceSpec::new(
        17,
        frame,
        InputSettings::Digital,
        glam::DMat3::IDENTITY,
        None,
    )
    .unwrap()
    .with_pipeline(processing.pixel_pipeline().unwrap())
    .unwrap();
    let upstream = spec.input_frame().unwrap();
    let offset = spec.plan.output_offset().unwrap();
    let [width, height] = upstream.output;
    assert!(offset[0] > 0 && offset[1] > 0);
    let values: Vec<[f32; 4]> = (0..height)
        .flat_map(|y| (0..width).map(move |x| [x, y]))
        .map(|[x, _]| {
            if x < offset[0] {
                [8., 4., 2., 8.]
            } else {
                [0.18, -0.03, 0.01, 8.]
            }
        })
        .collect();
    let sigma = halo
        .parameters(composition.crop().size)
        .unwrap()
        .unwrap()
        .sigma;
    let pitch = spec.plan.pitch();
    let scale: [f64; 2] = std::array::from_fn(|i| {
        f64::from(spec.plan.output().output[i]) / f64::from(frame.output[i])
    });
    let expected = reference::apply(
        &values.iter().map(|p| p.map(f64::from)).collect::<Vec<_>>(),
        [width as usize, height as usize],
        pitch.map(|p| f64::from(sigma as f32 / p as f32)),
        halo.strength,
        8,
    );
    let source = Arc::new(ViewSource {
        spec,
        renderer,
        kernels,
        pixels: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("padded halation physical input"),
            contents: bytemuck::cast_slice(&values),
            usage: wgpu::BufferUsages::STORAGE,
        }),
    });
    let view = RenderedView::new(&device, source, &processing, OutputTarget::Srgb, None).unwrap();
    view.ensure_submitted(
        &device,
        &queue,
        &SubmissionGate::default(),
        &egui::Context::default(),
    );
    let actual =
        crate::gpu::evaluation::read_buffer(&device, &queue, &view.graded.as_ref().unwrap().pixels);
    assert_eq!(actual.len(), (frame.output[0] * frame.output[1]) as usize);
    for y in 0..frame.output[1] {
        for x in 0..frame.output[0] {
            let low = [
                f64::from(offset[0]) + f64::from(x) * scale[0],
                f64::from(offset[1]) + f64::from(y) * scale[1],
            ];
            let high: [f64; 2] = std::array::from_fn(|i| low[i] + scale[i]);
            let mut e = [0.; 4];
            for sy in low[1].floor() as u32..high[1].ceil() as u32 {
                for sx in low[0].floor() as u32..high[0].ceil() as u32 {
                    let weight = (high[0].min(f64::from(sx + 1)) - low[0].max(f64::from(sx)))
                        .max(0.)
                        * (high[1].min(f64::from(sy + 1)) - low[1].max(f64::from(sy))).max(0.)
                        / (scale[0] * scale[1]);
                    for c in 0..3 {
                        e[c] += weight * expected[(sy * width + sx) as usize][c];
                    }
                }
            }
            let p = actual[(y * frame.output[0] + x) as usize];
            for c in 0..3 {
                assert!((f64::from(p[c]) - e[c]).abs() < 2e-6);
            }
        }
    }
    assert!(
        actual[0][0] > 0.2,
        "bright light outside the visible frame contributes to its first cell"
    );
}
