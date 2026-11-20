use super::*;
use crate::gpu::evaluation::numeric_fixture::NumericKey;
use std::sync::{Mutex, atomic::AtomicBool};

struct Pixels {
    domain: Domain,
    values: Arc<Vec<[f32; 4]>>,
    native: bool,
    reads: Arc<AtomicUsize>,
}
impl Pixels {
    fn values(&self, region: Rect) -> Vec<[f32; 4]> {
        let bounds = self.domain.finite().unwrap();
        (region.y..region.bottom())
            .flat_map(|y| {
                (region.x..region.right()).map(move |x| self.values[bounds.index([x, y])])
            })
            .collect()
    }
}
impl Source for Pixels {
    fn domain(&self) -> Domain {
        self.domain
    }
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(NumericKey(self.values.clone())))
    }
    fn storage_format(&self) -> ImageFormat {
        if self.native {
            ImageFormat::Rgba16Uint
        } else {
            ImageFormat::Rgba32Float
        }
    }
    fn read(&self, region: Rect) -> Result<Vec<[f32; 4]>, Error> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(self.values(region))
    }
    fn read_codes(&self, region: Rect) -> Result<Vec<[u16; 4]>, Error> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(self
            .values(region)
            .into_iter()
            .map(|p| p.map(|v| v as u16))
            .collect())
    }
}
fn pixels(native: bool) -> (Value, Arc<AtomicUsize>, Vec<[f32; 4]>) {
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
                meaning: Meaning::Arguments,
            },
            values: Arc::new(values.clone()),
            native,
            reads: reads.clone(),
        })),
        reads,
        values,
    )
}
fn uploaded(
    context: &Context,
    queue: &wgpu::Queue,
    scope: &Materializations,
    source: Value,
) -> Image<ImageLayout> {
    let format = source.source().unwrap().storage_format();
    let region = source.domain().finite().unwrap();
    pollster::block_on(
        context
            .materialize(
                queue,
                &Default::default(),
                scope,
                request(context, source, region),
                format,
                ImageBacking::Texture,
            )
            .unwrap(),
    )
    .unwrap()
}

#[test]
fn live_source_materializations_preserve_native_and_continuous_pixels_without_optional_retention() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    for native in [false, true] {
        let (source, reads, expected) = pixels(native);
        let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
        let scope = context.materializations();
        let owner = uploaded(&context, &queue, &scope, source.clone());
        let count = reads.load(Ordering::SeqCst);
        let mut domain = source.domain();
        domain.grid.coverage = 8;
        let root = reconstruct(source, domain).unwrap();
        let evaluate = |store: &Materializations| {
            pollster::block_on(
                context
                    .materialize(
                        &queue,
                        &Default::default(),
                        store,
                        request(&context, root.clone(), domain.finite().unwrap()),
                        ImageFormat::Rgba32Float,
                        ImageBacking::Buffer,
                    )
                    .unwrap(),
            )
            .unwrap()
        };
        let expected: Vec<_> = expected
            .into_iter()
            .map(|mut p| {
                p[3] = 8.;
                p
            })
            .collect();
        let image = evaluate(&scope);
        assert_eq!(
            execute::read(&device, &queue, image.as_buffer().unwrap()),
            expected
        );
        assert_eq!(reads.load(Ordering::SeqCst), count);
        let cold = context.test_materializations(false);
        let control = evaluate(&cold);
        assert!(reads.load(Ordering::SeqCst) > count);
        assert_eq!(
            execute::read(&device, &queue, control.as_buffer().unwrap()),
            expected
        );
        assert_eq!(context.retained_bytes(), 0);
        let weak = Arc::downgrade(&owner.image);
        drop((owner, image, control));
        assert!(weak.upgrade().is_none());
    }
}

#[test]
fn cancelled_upload_keeps_its_producer_through_queued_copy_cleanup() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let (source, _, _) = pixels(true);
    let producer = source.source().unwrap();
    let weak = Arc::downgrade(&producer);
    drop(producer);
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let scope = context.materializations();
    let gate = crate::gpu::submission::SubmissionGate::default();
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
    let region = source.domain().finite().unwrap();
    let mut future = Box::pin(
        context
            .materialize(
                &queue,
                &gate,
                &scope,
                request(&context, source, region),
                ImageFormat::Rgba16Uint,
                ImageBacking::Texture,
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
    drop(future);
    scope.clear();
    assert!(weak.upgrade().is_some());
    release.send(()).unwrap();
    // A final empty submission drains uploads even after cancellation.
    release.send(()).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while weak.upgrade().is_some() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert_eq!(context.retained_bytes(), 0);
}

#[test]
fn submitted_consumers_keep_materialized_inputs_after_owner_and_waiter_retire() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let (source, _, _) = pixels(true);
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let scope = context.materializations();
    let owner = uploaded(&context, &queue, &scope, source.clone());
    let weak = Arc::downgrade(&owner.image);
    let domain = Domain {
        bounds: Some([0, 0, 4, 2].into()),
        grid: Grid {
            origin: [11., 17.],
            axes: [[2., 0.], [0., 2.]],
            pitch: [2., 2.],
            coverage: 8,
        },
        meaning: source.domain().meaning,
    };
    let root = reconstruct(source, domain).unwrap();
    let gate = crate::gpu::submission::SubmissionGate::default();
    let (started, starts) = std::sync::mpsc::channel();
    let (release, resume) = std::sync::mpsc::channel();
    let resume = Mutex::new(resume);
    let observed = Arc::new(AtomicBool::new(false));
    let first = observed.clone();
    gate.observe_submissions(Arc::new(move || {
        if !first.swap(true, Ordering::SeqCst) {
            started.send(()).unwrap();
            resume
                .lock()
                .unwrap()
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap();
        }
    }));
    let mut future = Box::pin(
        context
            .materialize(
                &queue,
                &gate,
                &scope,
                request(&context, root, domain.finite().unwrap()),
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
    drop((owner, future));
    scope.clear();
    assert!(weak.upgrade().is_some());
    release.send(()).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while weak.upgrade().is_some() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert!(observed.load(Ordering::SeqCst));
    assert_eq!(context.retained_bytes(), 0);
}
