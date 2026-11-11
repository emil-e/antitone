use super::super::super::phases::{PHASE_HEADROOM, SOURCE_TILE};
use super::*;

#[test]
fn oversized_gpu_unit_addressing_rejects_during_description_without_source_io() {
    let (input, reads) = fixture([1, 1], vec![[1.; 4]]);
    let mapping = Mapping {
        start: [0.; 2],
        axes: [[1., 0.], [0., 1.]],
    };
    let region = Rect::new([0, 0], [65_536; 2]);
    let root = reconstruct_mapped(input, output(mapping, region), mapping).unwrap();
    let result = root.plan_unit(
        region,
        Budget {
            tile: [65_536; 2],
            scratch_bytes: u64::MAX,
        },
    );
    assert!(matches!(
        result,
        Err(Error::InvalidParameter(
            "evaluation raster exceeds GPU cell addressing"
        ))
    ));
    assert!(reads.lock().unwrap().is_empty());
}

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
    let plan = Plan::from_work(fragment, result, PHASE_HEADROOM).unwrap();
    assert!(plan.peak_scratch_bytes <= PHASE_HEADROOM);
    Executor::admit(&wgpu::Limits::default(), &plan).unwrap();
}
