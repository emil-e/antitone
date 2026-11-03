use super::super::batch::{SINK_HEADROOM_BYTES, SOURCE_TILE};
use super::*;

#[test]
fn maximum_masked_source_unit_fits_the_fixed_sink_headroom() {
    let domain = Domain {
        bounds: Some(Rect::new([0; 2], [SOURCE_TILE[0], SOURCE_TILE[1] + 1])),
        grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        meaning: Meaning::Graded,
    };
    let mapping = Mapping {
        start: [256., 0.],
        axes: [[256., 64.5], [-256., 64.5]],
    };
    let root =
        reconstruct_complete(raster_input(domain), output(mapping, [0, 0, 1, 1].into())).unwrap();
    let operation = (root.operation.as_ref() as &dyn std::any::Any)
        .downcast_ref::<Reconstruction>()
        .unwrap();
    let part = Rect::new([0; 2], SOURCE_TILE);
    let mut fragment = Fragment::default();
    let source = PlannedImage {
        domain,
        region: part,
        resource: fragment.resource(Storage::Input {
            slot: 0,
            bytes: u64::from(SOURCE_TILE[0]) * u64::from(SOURCE_TILE[1]) * 16,
            writable: false,
        }),
    };
    let sample = Sampled::raster(&mut fragment, source);
    let result = PlannedImage {
        domain: root.domain,
        region: root.domain.finite().unwrap(),
        resource: fragment.resource(Storage::Input {
            slot: 1,
            bytes: 16,
            writable: true,
        }),
    };
    let flags = fragment.resource(Storage::Input {
        slot: 2,
        bytes: 4,
        writable: true,
    });
    operation
        .accumulate(&mut fragment, sample, (part, result.region), result, flags)
        .unwrap();
    assert!(fragment.resources.iter().any(|storage| {
        matches!(storage, Storage::Data(bytes)
            if bytes.len() as u64 == (u64::from(SOURCE_TILE[0]) * u64::from(SOURCE_TILE[1])).div_ceil(32) * 4)
    }), "control: exercise a full-sized source support mask");
    let plan = Plan::from_work(fragment, result, SINK_HEADROOM_BYTES).unwrap();
    assert!(plan.peak_scratch_bytes <= SINK_HEADROOM_BYTES);
    Executor::admit(&wgpu::Limits::default(), &plan).unwrap();
}
