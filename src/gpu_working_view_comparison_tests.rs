//! The complete-raster oracle must detect a corrupted final cell and nonfinite input.
use super::*;
#[test]
fn physical_view_comparison_includes_padded_final_workgroup_and_nonfinite_values() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let values = adversaries();
        assert!(!values.len().is_multiple_of(64));
        let area = SourceRegion {
            x: 11,
            y: 17,
            width: 63,
            height: 47,
        };
        let composition = Composition::new(area, Default::default(), Default::default()).unwrap();
        let frame = FrameRaster::new(
            composition,
            FramePoint([0., 0.]),
            FramePoint(composition.size().map(f64::from)),
            [values.len() as u32, 1],
        )
        .unwrap();
        let source = authored_source(&device, renderer, kernels, &values, frame);
        let view = render(
            &device,
            &queue,
            source.clone(),
            PhotoGradeSnapshot::new(PhotoSettings::default(), vec![]).unwrap(),
            OutputTarget::Srgb,
        );
        let upload = |values: &[[f32; 4]]| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::cast_slice(values),
                usage: wgpu::BufferUsages::STORAGE,
            })
        };
        let mut changed = values.clone();
        let last = changed.last_mut().unwrap();
        last[0] += 0.1;
        last[3] = 14.;
        let words = compare_rasters(
            &device,
            &queue,
            (&source.pixels, &view.image.linear),
            (&upload(&changed), &view.image.linear),
        );
        assert_eq!([words[2], words[3], words[5], words[7]], [1, 0, 0, 1]);
        assert!(f32::from_bits(words[1]) > 0.09);
        assert_eq!(f32::from_bits(words[4]), 0.);
        assert_eq!(f32::from_bits(words[6]), 0.);
        let mut changed = values;
        changed.last_mut().unwrap()[1] = f32::NAN;
        let words = compare_rasters(
            &device,
            &queue,
            (&source.pixels, &view.image.linear),
            (&upload(&changed), &view.image.linear),
        );
        assert_eq!(words[3], 1);
        assert_eq!(words[2], 0);
    });
}
