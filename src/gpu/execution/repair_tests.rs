use super::*;
use crate::dust_repair::{Repair, RepairId, RepairOrigin, SupportMask};
use crate::gpu::{
    evaluation as e,
    repairs::{definition, fixtures},
    submission::SubmissionGate,
};
use std::sync::Arc;

#[test]
fn common_original_repairs_cross_units_at_large_integer_origins_and_preserve_untouched_samples() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let gate = SubmissionGate::default();
    let start = 1 << 25;
    let area: SourceRegion = [start, start, 256, 128].into();
    let composition = Composition::new(area, Default::default(), Default::default()).unwrap();
    let repair = Repair {
        id: RepairId(1),
        enabled: true,
        origin: RepairOrigin::Manual,
        support: SupportMask::new(
            [start + 126, start + 32],
            [3, 3],
            vec![true, false, true, false, true, false, true, false, true],
        )
        .unwrap(),
        offset: [-64, 0],
    };
    let evaluated =
        Arc::new(crate::dust_repair::evaluate(&repair, area, |_| Ok([-0.125, 2., 0.25])).unwrap());
    let base = fixtures::image(
        e::Domain {
            bounds: Some(area.try_into().unwrap()),
            grid: e::Grid::axis_aligned([0.; 2], [1.; 2]),
            meaning: e::Meaning::Neutral,
        },
        vec![[0.1, 0.2, 0.3, 0.]; area.width as usize * area.height as usize],
    )
    .unwrap();
    let repaired = definition::compose_images(
        base.clone(),
        fixtures::replacements(std::slice::from_ref(&evaluated)).unwrap(),
    )
    .unwrap();
    let reconstruction = Reconstructor::new(&device).unwrap();
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let scope = renderer.evaluation.test_materializations(false);
    let mut repaired_count = 0;
    for origin in [[0, 0], [128, 0]] {
        let mut unit = Unit::new(composition, origin, [128; 2]).unwrap();
        unit.mapping.space[0] = 1;
        let render = |root: e::Value| {
            let bounds = unit.bounds.try_into().unwrap();
            let request = renderer
                .evaluation
                .request(
                    root.clone(),
                    e::Region {
                        bounds,
                        grid: root.domain().grid,
                    },
                    e::Budget {
                        tile: [128; 2],
                        scratch_bytes: 4 << 20,
                    },
                )
                .unwrap();
            let value = pollster::block_on(
                renderer
                    .evaluation
                    .materialize(
                        &queue,
                        &gate,
                        &scope,
                        request,
                        e::ImageFormat::Rgba32Float,
                        e::ImageBacking::Buffer,
                    )
                    .unwrap(),
            )
            .unwrap();
            let working: e::BufferImage = value.try_into().unwrap();
            let mut encoder = device.create_command_encoder(&Default::default());
            reconstruction
                .encode(&device, &mut encoder, &unit, &texture, &working)
                .unwrap();
            reconstruction
                .read(&device, gate.submit(&queue, [encoder.finish()]), [128; 2])
                .unwrap()
        };
        let plain = render(base.clone());
        let actual = render(repaired.clone());
        for (index, (plain, actual)) in plain.iter().zip(actual).enumerate() {
            let point = [
                start + origin[0] + index as u32 % 128,
                start + origin[1] + index as u32 / 128,
            ];
            if evaluated.support().contains(point) {
                repaired_count += 1;
                let expected = evaluated.pixel(point, [0.; 3]);
                for c in 0..3 {
                    assert!(
                        (actual[c] - expected[c]).abs() <= 2e-5 + 2e-4 * expected[c].abs(),
                        "{point:?} channel={c} actual={actual:?} expected={expected:?}"
                    );
                }
                assert_eq!(actual[3], plain[3]);
            } else {
                assert_eq!(
                    plain.map(f32::to_bits),
                    actual.map(f32::to_bits),
                    "untouched original {point:?}"
                );
            }
        }
    }
    assert_eq!(
        repaired_count, 5,
        "selected cells must straddle the destination-unit seam"
    );
}
