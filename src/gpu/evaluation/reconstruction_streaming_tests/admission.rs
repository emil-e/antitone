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
    let result = root.plan_unit(region, Tiling { tile: [65_536; 2] });
    assert!(matches!(
        result,
        Err(Error::InvalidParameter(
            "evaluation raster exceeds GPU cell addressing"
        ))
    ));
    assert!(reads.lock().unwrap().is_empty());
}

#[test]
fn large_reduction_uses_representable_contributions_on_portable_bindings() {
    let input = raster_input(Domain {
        bounds: Some(Rect::new([0; 2], [4096; 2])),
        grid: Grid::axis_aligned([0.; 2], [1.; 2]),
    });
    let mapping = Mapping {
        start: [0.; 2],
        axes: [[16., 0.], [0., 16.]],
    };
    let region = Rect::new([0; 2], [256; 2]);
    let root = reconstruct_mapped(input, output(mapping, region), mapping).unwrap();
    let tiling = Tiling { tile: [256; 2] };
    let limits = wgpu::Limits::default();
    assert!(
        matches!(
            root.plan_unit(region, tiling).unwrap().admit_for(&limits),
            Err(Error::EvaluationLimit(_))
        ),
        "control: unrestricted coalescing requires a 256 MiB binding"
    );
    let unit = crate::gpu::evaluation::unit::plan_with_limits(&root, region, tiling, Some(&limits))
        .unwrap()
        .admit_for(&limits)
        .unwrap();
    let UnitWork::Gpu(work) = unit.work else {
        panic!("GPU reconstruction expected")
    };
    let phases = work.phases.collect::<Result<Vec<_>, _>>().unwrap();
    assert!(phases.len() > 1);
    assert!(
        phases
            .iter()
            .flat_map(|phase| &phase.reads)
            .all(
                |read| u64::from(read.region.width) * u64::from(read.region.height) * 16
                    <= limits.max_storage_buffer_binding_size
            )
    );
}

#[test]
fn maximum_source_support_mask_fits_portable_device_bindings() {
    let domain = Domain {
        bounds: Some(Rect::new([0; 2], [SOURCE_TILE[0], SOURCE_TILE[1] + 1])),
        grid: Grid::axis_aligned([0.; 2], [1.; 2]),
    };
    let mapping = Mapping {
        start: [256., 0.],
        axes: [[256., 64.5], [-256., 64.5]],
    };
    let root =
        reconstruct_complete(raster_input(domain), output(mapping, [0, 0, 1, 1].into())).unwrap();
    let region = root.domain().finite().unwrap();
    let plan = Plan::new(root, region, Tiling { tile: [1, 1] }).unwrap();
    let bytes = (u64::from(SOURCE_TILE[0]) * u64::from(SOURCE_TILE[1])).div_ceil(32) * 4;
    assert!(
        plan.tiles
            .iter()
            .any(|tile| tile.fragment.resources.iter().any(|storage| {
                matches!(storage, Storage::Data(data) if data.len() as u64 == bytes)
            })),
        "control: exercise a full-sized source support mask"
    );
    Executor::admit(&wgpu::Limits::default(), &plan).unwrap();
}
