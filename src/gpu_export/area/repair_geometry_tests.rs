use super::*;
use crate::processing::Calibration;
use crate::{
    adjustment_layers::AdjustmentLayerSettings,
    dust_repair::{Repair, RepairId, RepairOrigin, SupportMask},
};
use glam::DMat3;

#[test]
fn common_original_repairs_cross_units_at_large_integer_origins_and_preserve_untouched_samples() {
    let (device, queue, renderer) = super::super::tests::context();
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
    let logical = crate::gpu::repairs::Snapshot::logical(vec![evaluated.clone()]).unwrap();
    let photo = AdjustmentLayerSettings::default();
    let grade = Arc::new(
        GpuPipeline::new(
            &device,
            Calibration {
                dmin: DVec3::splat(0.8),
                dmax: DVec3::splat(0.01),
                input_floor: 1. / 65535.,
            },
            RollSettings::default(),
            AdjustmentLayerSnapshot::new(vec![crate::adjustment_layers::AdjustmentLayer::global(
                photo,
            )])
            .unwrap(),
            DMat3::IDENTITY,
            area,
        )
        .unwrap(),
    );
    let scalar = crate::photographic::Grade::new(photo).unwrap();
    let kernel = &renderer.kernel;
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
    let mut repaired_count = 0;
    for origin in [[0, 0], [128, 0]] {
        let mut unit = Unit::new(composition, origin, [128; 2]).unwrap();
        unit.mapping.space[0] = 1;
        let b = unit.bounds;
        let size = [b.width, b.height];
        let mut source = ProcessingSource::new_continuous(
            &device,
            &vec![[20000., 25000., 30000., 0.]; (size[0] * size[1]) as usize],
            DMat3::IDENTITY,
        )
        .unwrap();
        let geometry = SourceGeometry::original_tile([b.x, b.y, b.width, b.height], size).unwrap();
        source.set_flat_field(None, geometry).unwrap();
        let working = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(size[0]) * u64::from(size[1]) * 16,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let render = |repaired| {
            let mut encoder = device.create_command_encoder(&Default::default());
            kernel
                .encode_grade(&device, &mut encoder, &source, &working, &grade)
                .unwrap();
            if repaired {
                let mut input = crate::gpu::ProcessingInput::from(grade.clone());
                input.repairs = logical.clone();
                renderer
                    .repairs
                    .original(&device, &mut encoder, kernel, &working, &input, geometry)
                    .unwrap();
            }
            reconstruction
                .encode(&device, &mut encoder, &unit, &texture, &working)
                .unwrap();
            reconstruction
                .read(&device, gate.submit(&queue, [encoder.finish()]), [128; 2])
                .unwrap()
        };
        let plain = render(false);
        let actual = render(true);
        for (index, (plain, actual)) in plain.iter().zip(actual).enumerate() {
            let point = [
                start + origin[0] + index as u32 % 128,
                start + origin[1] + index as u32 / 128,
            ];
            if evaluated.support().contains(point) {
                repaired_count += 1;
                let expected = scalar
                    .apply(DVec3::from_array(
                        evaluated.pixel(point, [0.; 3]).map(f64::from),
                    ))
                    .unwrap();
                for c in 0..3 {
                    assert!(
                        (f64::from(actual[c]) - expected[c]).abs()
                            <= 2e-5 + 2e-4 * expected[c].abs(),
                        "{point:?} channel={c}"
                    );
                }
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
