use super::*;
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

struct Pixels {
    domain: Domain,
    values: Arc<Vec<[f32; 4]>>,
    native: bool,
    reads: Arc<AtomicUsize>,
}
impl Pixels {
    fn values(&self, region: CellRegion) -> Vec<[f32; 4]> {
        let b = self.domain.finite().unwrap();
        (region.y..region.bottom())
            .flat_map(|y| (region.x..region.right()).map(move |x| self.values[b.index([x, y])]))
            .collect()
    }
}
impl Source for Pixels {
    fn domain(&self) -> Domain {
        self.domain
    }
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(self.values.clone()))
    }
    fn storage_format(&self) -> ImageFormat {
        if self.native {
            ImageFormat::Rgba16Uint
        } else {
            ImageFormat::Rgba32Float
        }
    }
    fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(self.values(region))
    }
    fn read_codes(&self, region: CellRegion) -> Result<Vec<[u16; 4]>, Error> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(self
            .values(region)
            .into_iter()
            .map(|p| p.map(|v| v as u16))
            .collect())
    }
}
fn fixture(native: bool) -> (Arc<ImageNode>, Arc<AtomicUsize>, Vec<[f32; 4]>) {
    let values: Vec<_> = (0..32)
        .map(|i| {
            if native {
                [i as f32 * 1000., 65535., 129., 0.]
            } else {
                [i as f32 - 3.5, 65536.375, 0.125, 0.]
            }
        })
        .collect();
    let reads = Arc::new(AtomicUsize::new(0));
    (
        source(Arc::new(Pixels {
            domain: Domain {
                bounds: Some([11, 17, 8, 4].into()),
                grid: Grid::axis_aligned([0.; 2], [1.; 2]),
                meaning: Meaning::CameraCodes,
            },
            values: Arc::new(values.clone()),
            native,
            reads: reads.clone(),
        })),
        reads,
        values,
    )
}
fn requested(context: &Context, root: Arc<ImageNode>) -> Evaluation {
    let d = root.domain;
    context
        .request(
            root,
            Region {
                bounds: d.finite().unwrap(),
                grid: d.grid,
            },
            Budget {
                tile: [8, 4],
                scratch_bytes: 1 << 20,
            },
        )
        .unwrap()
}

#[test]
fn static_input_allowance_is_bounded_independently_of_optional_retention() {
    assert_eq!(
        allowance(Some(48 << 30), crate::sys::GpuMemory::Unified),
        512 << 20
    );
    assert_eq!(
        allowance(Some(4 << 30), crate::sys::GpuMemory::Unified),
        128 << 20
    );
    assert_eq!(
        allowance(
            Some(48 << 30),
            crate::sys::GpuMemory::Discrete {
                dedicated_bytes: Some(8 << 30)
            }
        ),
        512 << 20
    );
    assert_eq!(allowance(None, crate::sys::GpuMemory::Unknown), 256 << 20);
}

#[test]
fn input_parts_cover_the_real_domain_without_holes_or_oversized_allocations() {
    let limits = wgpu::Limits::default();
    let bounds: CellRegion = [12, 12, 8640, 5760].into();
    let regions = parts(bounds, &limits).unwrap();
    assert!(regions.len() <= super::super::materialized::MAX_PARTS);
    assert_eq!(
        regions
            .iter()
            .map(|r| u64::from(r.width) * u64::from(r.height) * 8)
            .sum::<u64>(),
        398_131_200
    );
    for (i, r) in regions.iter().enumerate() {
        assert!(bounds.contains_region(*r));
        assert!(
            r.width <= limits.max_texture_dimension_2d
                && r.height <= limits.max_texture_dimension_2d
        );
        for next in &regions[i + 1..] {
            assert!(
                r.right() <= next.x
                    || next.right() <= r.x
                    || r.bottom() <= next.y
                    || next.bottom() <= r.y
            );
        }
    }
    assert!(
        parts(
            crate::gpu::evaluation::CellRegion::new(
                [0, 0],
                [limits.max_texture_dimension_2d * 5, 1]
            ),
            &limits
        )
        .is_none()
    );
}

#[test]
fn active_sources_preserve_native_and_continuous_values_with_no_optional_retention() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    for native in [false, true] {
        let (source, reads, expected) = fixture(native);
        let mut context = Context::new(&device, CachePolicy { generated_bytes: 0 });
        context.active_inputs = cache::Allowance::new(1 << 20);
        let scope = context.test_materializations(false);
        let gate = crate::gpu::submission::SubmissionGate::default();
        let active = pollster::block_on(context.prepare_input(
            &queue,
            &gate,
            &scope,
            source.clone(),
            || false,
        ))
        .unwrap()
        .unwrap();
        assert!(active.matches(&source));
        let count = reads.load(Ordering::SeqCst);
        let mut domain = source.domain;
        domain.grid.coverage = 8;
        let root = reconstruct(source.clone(), domain).unwrap();
        let image = pollster::block_on(
            context
                .materialize(
                    &queue,
                    &gate,
                    &scope,
                    requested(&context, root.clone()),
                    ImageFormat::Rgba32Float,
                    ImageBacking::Buffer,
                )
                .unwrap(),
        )
        .unwrap();
        let expected: Vec<_> = expected
            .into_iter()
            .map(|mut p| {
                p[3] = 8.;
                p
            })
            .collect();
        assert_eq!(
            execute::read(&device, &queue, image.as_buffer().unwrap()),
            expected
        );
        assert_eq!(
            reads.load(Ordering::SeqCst),
            count,
            "the captured source input supplies execution"
        );
        assert_eq!(context.retained_bytes(), 0);
        let cold = context.test_materializations(false);
        let streamed = pollster::block_on(
            context
                .materialize(
                    &queue,
                    &gate,
                    &cold,
                    requested(&context, root),
                    ImageFormat::Rgba32Float,
                    ImageBacking::Buffer,
                )
                .unwrap(),
        )
        .unwrap();
        assert!(
            reads.load(Ordering::SeqCst) > count,
            "control: the unsupplied scope really reads originals"
        );
        assert_eq!(
            execute::read(&device, &queue, streamed.as_buffer().unwrap()),
            expected
        );
        assert_eq!(
            context.active_input_bytes(),
            32 * if native { 8 } else { 16 }
        );
        drop((image, streamed, active));
        assert_eq!(context.active_input_bytes(), 0);
    }
}

#[test]
fn an_exhausted_input_allowance_falls_back_without_source_io_or_optional_budget_exceptions() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let (source, reads, _) = fixture(true);
    let mut context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    context.active_inputs = cache::Allowance::new(255);
    let scope = context.test_materializations(false);
    assert!(
        pollster::block_on(context.prepare_input(
            &queue,
            &Default::default(),
            &scope,
            source,
            || false
        ))
        .unwrap()
        .is_none()
    );
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    assert_eq!(context.active_input_bytes(), 0);
    assert!(scope.is_empty());
}

#[test]
fn cancelling_an_upload_keeps_its_reservation_until_queued_copy_cleanup() {
    use std::sync::atomic::AtomicBool;
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let (source, _, _) = fixture(true);
    let mut context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    context.active_inputs = cache::Allowance::new(256);
    let scope = context.test_materializations(false);
    let gate = crate::gpu::submission::SubmissionGate::default();
    let (started, starts) = std::sync::mpsc::channel();
    let (release, resume) = std::sync::mpsc::channel();
    let resume = Mutex::new(resume);
    let count = AtomicUsize::new(0);
    gate.observe_submissions(Arc::new(move || {
        if count.fetch_add(1, Ordering::SeqCst) == 0 {
            started.send(()).unwrap();
            resume
                .lock()
                .unwrap()
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap();
        }
    }));
    let stop = AtomicBool::new(false);
    let mut future = Box::pin(context.prepare_input(&queue, &gate, &scope, source, || {
        stop.load(Ordering::SeqCst)
    }));
    pollster::block_on(std::future::poll_fn(|cx| {
        assert!(std::future::Future::poll(future.as_mut(), cx).is_pending());
        std::task::Poll::Ready(())
    }));
    starts
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    stop.store(true, Ordering::SeqCst);
    assert!(pollster::block_on(future.as_mut()).unwrap().is_none());
    drop(future);
    scope.clear();
    assert_eq!(context.active_input_bytes(), 256);
    release.send(()).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while context.active_input_bytes() != 0 {
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert_eq!(context.retained_bytes(), 0);
}

#[test]
fn submitted_work_keeps_admission_after_owner_and_waiter_retirement() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let (source, _, _) = fixture(true);
    let mut context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    context.active_inputs = cache::Allowance::new(256);
    let scope = context.test_materializations(false);
    let gate = crate::gpu::submission::SubmissionGate::default();
    let active =
        pollster::block_on(context.prepare_input(&queue, &gate, &scope, source.clone(), || false))
            .unwrap()
            .unwrap();
    let output = Domain {
        bounds: Some([0, 0, 4, 2].into()),
        grid: Grid {
            origin: [11., 17.],
            axes: [[2., 0.], [0., 2.]],
            pitch: [2., 2.],
            coverage: 8,
        },
        meaning: Meaning::CameraCodes,
    };
    let root = reconstruct(source.clone(), output).unwrap();
    let (started, starts) = std::sync::mpsc::channel();
    let (release, resume) = std::sync::mpsc::channel();
    let resume = Mutex::new(resume);
    gate.observe_submissions(Arc::new(move || {
        started.send(()).unwrap();
        resume
            .lock()
            .unwrap()
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
    }));
    let mut future = Box::pin(
        context
            .materialize(
                &queue,
                &gate,
                &scope,
                requested(&context, root),
                ImageFormat::Rgba32Float,
                ImageBacking::Buffer,
            )
            .unwrap(),
    );
    pollster::block_on(std::future::poll_fn(|cx| {
        assert!(std::future::Future::poll(future.as_mut(), cx).is_pending());
        std::task::Poll::Ready(())
    }));
    starts
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    drop((active, future));
    scope.clear();
    assert_eq!(context.active_input_bytes(), 256);
    assert!(
        pollster::block_on(context.prepare_input(
            &queue,
            &Default::default(),
            &scope,
            source,
            || false
        ))
        .unwrap()
        .is_none()
    );
    release.send(()).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while context.active_input_bytes() != 0 {
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
}
