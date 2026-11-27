use super::*;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

struct Samples {
    reads: AtomicU64,
    fail_strips: bool,
}

impl Source for Samples {
    fn domain(&self) -> Domain {
        Domain {
            bounds: Some(CellRegion::new([0, 0], [2048, 1024])),
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        }
    }

    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("resident run test source"))
    }

    fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        if self.fail_strips && region.width == 2048 {
            return Err(Error::GpuOutOfMemory("test texture strip failure".into()));
        }
        let cells = u64::from(region.width) * u64::from(region.height);
        self.reads.fetch_add(cells, Relaxed);
        Ok(vec![[9.25, -1.5, 80000.5, 1.]; cells as usize])
    }
}

fn fixture(fail_strips: bool) -> (Arc<Samples>, Value, Domain) {
    let source = Arc::new(Samples {
        reads: 0.into(),
        fail_strips,
    });
    let domain = Domain {
        bounds: Some(CellRegion::new([0; 2], [32, 16])),
        grid: Grid::axis_aligned([0.; 2], [64.; 2]),
    };
    let root =
        crate::reconstruction::reconstruct_complete(value::source(source.clone()), domain).unwrap();
    (source, root, domain)
}

fn evaluate(context: &Context, root: &Value, bounds: CellRegion, tile: [u32; 2]) -> Evaluation {
    context
        .request(
            root.clone(),
            Region {
                bounds,
                grid: root.domain().grid,
            },
            Tiling { tile },
        )
        .unwrap()
}

#[test]
fn pending_deliveries_share_one_lossless_source_upload_for_the_run() {
    let (device, queue, _) = crate::photographic::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let (source, root, domain) = fixture(false);
    let run = EvaluationRun::default();
    let full = domain.finite().unwrap();
    let pending = [full, CellRegion::new([0; 2], [8, 8])].map(|bounds| {
        let ImageDelivery::Pending(job) = context
            .request_images(vec![evaluate(&context, &root, bounds, [32, 16])])
            .unwrap()
        else {
            panic!("without an upload the reduction should require worker production");
        };
        (bounds, job)
    });
    for (bounds, job) in pending {
        let [image] = pollster::block_on(job.finish(&queue, &run, || false))
            .unwrap()
            .try_into()
            .ok()
            .unwrap();
        let pixels = crate::evaluation::read_buffer(&device, &queue, &image.pixels);
        assert_eq!(
            pixels,
            vec![[9.25, -1.5, 80000.5, 1.]; bounds.width as usize * bounds.height as usize]
        );
        assert_eq!(source.reads.load(Relaxed), 2048 * 1024);
    }
    let Some(lease) = run.0.get().and_then(Option::as_ref) else {
        panic!("the broad request must have leased the developed source");
    };
    let image = Arc::downgrade(&lease.image);
    drop(run);
    assert!(
        image.upgrade().is_none(),
        "the lease must not enter optional retention"
    );
}

#[test]
fn traversal_admits_from_total_demand_and_falls_back_for_narrow_or_failed_uploads() {
    let (device, queue, _) = crate::photographic::test_support::render_context();
    let tile = [4, 4];
    for (width, fail_strips, expect_resident) in
        [(32, false, true), (8, false, false), (32, true, false)]
    {
        let (source, root, _) = fixture(fail_strips);
        let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
        let bounds = CellRegion::new([0; 2], [width, 16]);
        let traversal = context
            .traversal(&queue, evaluate(&context, &root, bounds, tile), || Ok(()))
            .unwrap();
        let plan = traversal.plan(CellRegion::new([0; 2], tile)).unwrap();
        let pending_source = plan
            .tiles
            .iter()
            .flat_map(|tile| &tile.fragment.work)
            .any(|work| matches!(work, Work::Upload { .. } | Work::Produce { .. }));
        assert_eq!(pending_source, !expect_resident);
        assert_eq!(
            source.reads.load(Relaxed),
            if expect_resident { 2048 * 1024 } else { 0 }
        );
    }
}

#[test]
fn traversal_does_not_upload_through_an_already_stored_input() {
    let (device, queue, _) = crate::photographic::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let (source, prefix, domain) = fixture(false);
    let bounds = domain.finite().unwrap();
    let tile = [4, 4];
    let scope = context.materializations();
    let _stored = pollster::block_on(
        context
            .materialize(
                &queue,
                &scope,
                evaluate(&context, &prefix, bounds, tile),
                ImageFormat::Rgba32Float,
                ImageBacking::Buffer,
            )
            .unwrap(),
    )
    .unwrap();
    let root = value::pointwise(
        prefix,
        value::Operation::new(
            "subsequent grade",
            1u32,
            "fn pixel(p: vec2i, g: vec2f, a: vec4f) -> vec4f { return a * 2.; }",
            "pixel",
            value::Access::Pointwise,
        ),
    )
    .unwrap();
    let reads = source.reads.load(Relaxed);
    assert!(
        reads > 0,
        "the stored prefix should need camera pixels to produce"
    );
    let traversal = context
        .traversal(&queue, evaluate(&context, &root, bounds, tile), || Ok(()))
        .unwrap();
    let _plan = traversal.plan(CellRegion::new([0; 2], tile)).unwrap();
    assert_eq!(source.reads.load(Relaxed), reads);
    assert!(traversal._run.0.get().is_some_and(Option::is_none));
}
