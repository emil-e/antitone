use super::*;
use crate::gpu::evaluation::{Domain, Grid, ImageNode, Source};
use std::sync::Mutex;

#[test]
fn streamed_dense_repairs_include_simultaneous_host_uploads_in_export_payload_bounds() {
    struct Observed {
        source: Arc<dyn Source>,
        reads: Arc<Mutex<Vec<evaluation::CellRegion>>>,
    }
    impl Source for Observed {
        fn domain(&self) -> Domain {
            self.source.domain()
        }
        fn read(&self, region: evaluation::CellRegion) -> Result<Vec<[f32; 4]>, Error> {
            self.reads.lock().unwrap().push(region);
            self.source.read(region)
        }
    }

    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let scan = crate::scan::Scan::open(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng"),
        None,
    )
    .unwrap();
    let repair = crate::dust_repair::Repair {
        id: crate::dust_repair::RepairId(1),
        enabled: true,
        origin: crate::dust_repair::RepairOrigin::Manual,
        support: crate::dust_repair::SupportMask::new([32, 48], [64, 96], vec![true; 64 * 96])
            .unwrap(),
        offset: [128, 0],
    };
    let evaluated = Arc::new(
        crate::dust_repair::evaluate(&repair, scan.area, |_| Ok([0.25, 0.5, 0.75])).unwrap(),
    );
    let mut input = ProcessingInput::new(
        &device.limits(),
        crate::processing::InputSettings::Digital,
        Default::default(),
        crate::adjustment_layers::AdjustmentLayerSnapshot::new(Vec::new()).unwrap(),
        scan.positive_matrix(&crate::processing::InputSettings::Digital)
            .unwrap(),
        scan.area,
    )
    .unwrap();
    input.repairs = crate::gpu::repairs::Snapshot::logical(vec![evaluated]).unwrap();
    let stages = crate::pixel_pipeline::PixelPipeline {
        stages: vec![crate::pixel_pipeline::PixelStage::new(
            crate::image_mtf::ImageMtfParameters { sigma: 4. },
        )],
    };
    let reads = Arc::new(Mutex::new(Vec::new()));
    let camera = evaluation::source(Arc::new(Observed {
        source: evaluation::camera_source(&scan).unwrap().source().unwrap(),
        reads: reads.clone(),
    }));
    let photo = evaluation::Photograph::new_with_stages(
        &device.limits(),
        camera,
        None,
        &input,
        input.input_program().matrix,
        &stages,
    )
    .unwrap();
    let composition = Composition::new(scan.area, Default::default(), Default::default()).unwrap();
    let domain = Domain {
        bounds: Some(evaluation::CellRegion::new([0; 2], composition.size())),
        grid: Grid {
            coverage: 8,
            ..Grid::from_frame(FrameRaster::full(composition).unwrap())
        },
        meaning: photo.processed.domain().meaning,
    };
    let root: Arc<ImageNode> = evaluation::reconstruct_complete(photo.processed, domain).unwrap();
    let extent = [32; 2];
    let context = renderer.evaluation();
    // A scaled allowance forces the same original-sample graph through worker production
    // without a large RAW or a test-only execution mode in the export adapter.
    let request_for = |scratch_bytes| {
        context.request(
            root.clone(),
            evaluation::Region {
                bounds: evaluation::CellRegion::new([48, 64], extent),
                grid: domain.grid,
            },
            evaluation::Budget {
                tile: extent,
                scratch_bytes,
            },
        )
    };
    let resident = request_for(2 << 20).unwrap();
    assert!(
        resident.plans().next().unwrap().unwrap().tiles[0]
            .fragment
            .work
            .iter()
            .all(|work| !matches!(work, evaluation::Work::Produce { .. }))
    );
    let mut reference = Vec::new();
    context
        .read(
            &queue,
            &Default::default(),
            &resident,
            || false,
            |_, values| {
                reference.extend_from_slice(values);
                Ok(())
            },
        )
        .unwrap()
        .unwrap();
    reads.lock().unwrap().clear();
    let request = request_for(128 << 10).unwrap();
    let plan = request.plans().next().unwrap().unwrap();
    assert!(
        matches!(
            plan.tiles[0].fragment.work[0],
            evaluation::Work::Produce { .. }
        ),
        "fixture must require production: peak={}",
        plan.peak_scratch_bytes,
    );
    let mut actual = Vec::new();
    let stats = context
        .read(
            &queue,
            &Default::default(),
            &request,
            || false,
            |_, values| {
                actual.extend_from_slice(values);
                Ok(())
            },
        )
        .unwrap()
        .unwrap();
    assert_eq!(actual.len(), 32 * 32);
    assert_eq!(actual.len(), reference.len());
    for (streamed, resident) in actual.iter().zip(&reference) {
        for (a, b) in streamed.iter().zip(resident) {
            assert!(
                (a - b).abs() < 2e-6,
                "streamed={streamed:?}, resident={resident:?}"
            );
        }
    }
    let reads = reads.lock().unwrap();
    assert!(
        reads.len() > 1,
        "exercise bounded repair uploads, not the resident outer plan"
    );
    let mut host_lower_bound = 0;
    let mut largest_source = 0;
    for region in reads.iter() {
        let cells = u64::from(region.width) * u64::from(region.height);
        let width = (region.right().min(96) - region.x.max(32)).max(0) as u64;
        let height = (region.bottom().min(144) - region.y.max(48)).max(0) as u64;
        // One f32 RGBA source vector and one 32-byte sparse record per selected repair cell.
        host_lower_bound = host_lower_bound.max(cells * 16 + width * height * 32);
        largest_source = largest_source.max(cells * 16);
    }
    assert!(
        host_lower_bound > largest_source,
        "repair payload must be distinguishable from source uploads"
    );
    assert!(stats.host_upload_bytes >= host_lower_bound, "{stats:?}");
    let destination_payload = u64::from(TILE[0]) * u64::from(TILE[1]) * 16 + 32 * 32 * 42;
    assert!(
        unit_payload_bound(&stats, extent, false)
            >= stats.scratch_bytes + host_lower_bound + destination_payload
    );
}
