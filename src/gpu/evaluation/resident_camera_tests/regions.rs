use super::*;
use crate::gpu::flat::GpuFlatField;
use crate::gpu::photograph::PhotographGraph;
use crate::gpu::source::ProcessingSource;
use crate::gpu::source::SourceGeometry;
use crate::gpu::test_inputs::AuthoredPhotograph;

#[test]
fn cropped_resident_codes_keep_full_stride_and_integer_nearest_centre_correction() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let area: SourceRegion = [16_000_000, 15_000_000, 6001, 4003].into();
    let size = [7, 5];
    let geometry = SourceGeometry::new([area.x, area.y, area.width, area.height], size).unwrap();
    let field = Arc::new(
        crate::flat_field::FlatField::from_nodes(
            [area.x, area.y, area.width, area.height],
            [2, 2],
            vec![
                DVec3::new(1., 1.25, 0.75),
                DVec3::new(2., 1.25, 0.75),
                DVec3::new(1., 1.75, 1.),
                DVec3::new(2., 1.75, 1.),
            ],
        )
        .unwrap(),
    );
    let gpu_field = Arc::new(GpuFlatField::new(&device, field).unwrap());
    let codes: Vec<[u16; 4]> = (0..35)
        .map(|i| [10000 + i * 1300, 2000 + i * 500, 40000 - i * 700, 0])
        .collect();
    for continuous in [false, true] {
        let values: Vec<[f32; 4]> = codes
            .iter()
            .map(|p| {
                [
                    f32::from(p[0]) + if continuous { 50000.375 } else { 0. },
                    f32::from(p[1]),
                    f32::from(p[2]),
                    0.,
                ]
            })
            .collect();
        let source = if continuous {
            ProcessingSource::new_continuous(
                &device,
                &values,
                glam::DMat3::IDENTITY,
                Some(geometry),
            )
            .unwrap()
        } else {
            ProcessingSource::new(&device, &codes, glam::DMat3::IDENTITY, Some(geometry)).unwrap()
        };
        let input = AuthoredPhotograph::new(
            &device.limits(),
            InputSettings::Digital,
            RollSettings::default(),
            AdjustmentLayerSnapshot::new(vec![]).unwrap(),
            glam::DMat3::IDENTITY,
            area,
        )
        .unwrap();
        let input = input.with_flat_field(Some(gpu_field.clone()));
        let photo = PhotographGraph::from_resident_with_stages(
            &device.limits(),
            &source,
            &input,
            crate::gpu::raster::Raster::region(area, size),
            &Default::default(),
        )
        .unwrap();
        let region: SourceRegion = [2, 1, 3, 3].into();
        for policy in [Policy::Materialized, Policy::Composed] {
            let plan = Plan::with_policy(
                photo.unrepaired.clone(),
                region.try_into().unwrap(),
                Budget {
                    tile: [3, 3],
                    scratch_bytes: 4 << 20,
                },
                policy,
            )
            .unwrap();
            let recorded = renderer
                .evaluation
                .executor()
                .prepare(&device, plan)
                .unwrap();
            let pixels =
                crate::gpu::raster::buffer(&device, [3, 3], "cropped camera light").unwrap();
            let mut encoder = device.create_command_encoder(&Default::default());
            recorded
                .encode_into(&device, &mut encoder, &source.input, &pixels)
                .unwrap();
            queue.submit([encoder.finish()]);
            let actual = execute::read(&device, &queue, &pixels);
            for (i, pixel) in actual.iter().enumerate() {
                let x = region.x + i as u32 % 3;
                let y = region.y + i as u32 / 3;
                let px = ((2 * x + 1) * area.width / (2 * size[0])) as f64 + 0.5;
                let py = ((2 * y + 1) * area.height / (2 * size[1])) as f64 + 0.5;
                let gain = [
                    1. + px / f64::from(area.width),
                    1.25 + 0.5 * py / f64::from(area.height),
                    0.75 + 0.25 * py / f64::from(area.height),
                ];
                let original = values[(y * size[0] + x) as usize];
                for channel in 0..3 {
                    let expected = f64::from(original[channel]) / 65535. * gain[channel];
                    assert!(
                        (f64::from(pixel[channel]) - expected).abs() < 3e-6,
                        "continuous={continuous} pixel={i} channel={channel}: {pixel:?} expected={expected}"
                    );
                }
                assert_eq!(pixel[3], 0.);
            }
        }
    }
}
