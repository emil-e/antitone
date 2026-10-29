use super::*;
use crate::geometry::SourceRegion;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Pixels {
    domain: Domain,
    reads: Arc<AtomicUsize>,
}
impl Source for Pixels {
    fn domain(&self) -> Domain {
        self.domain
    }
    fn read(&self, r: SourceRegion) -> Result<Vec<[f32; 4]>, Error> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        Ok(vec![[-0.25, 0.4, 2., 3.]; (r.width * r.height) as usize])
    }
}
#[derive(Debug, PartialEq)]
struct FieldKey {
    region: SourceRegion,
    grid: Grid,
    seed: u32,
}
struct Field {
    seed: u32,
}
impl Operation for Field {
    fn input(&self, r: SourceRegion) -> Result<Option<SourceRegion>, Error> {
        Ok(Some(r))
    }
    fn lower(
        &self,
        f: &mut Fragment,
        input: Option<PlannedImage>,
        output: PlannedImage,
    ) -> Result<(), Error> {
        let input = input.unwrap();
        let field = f.resource(Storage::Retained {
            key: RetainedKey::new(FieldKey {
                region: output.region,
                grid: output.domain.grid,
                seed: self.seed,
            }),
            bytes: u64::from(output.cells()) * 16,
        });
        let r = output.region;
        let geometry = f.uniform(&[r.x, r.y, r.width, self.seed]);
        f.compute("@group(0) @binding(0) var<storage,read_write> field:array<vec4f>; @group(0) @binding(1) var<uniform> g:vec4u; @compute @workgroup_size(64) fn initialize(@builtin(global_invocation_id) id:vec3u){if id.x>=arrayLength(&field){return;} let x=g.x+id.x%g.z;let y=g.y+id.x/g.z;let v=1.0+f32((x+3u*y+g.w)%5u)*0.0625;field[id.x]=vec4f(v);}","initialize",vec![(0,Binding::Write(field)),(1,geometry)],output.cells());
        if let Work::Compute { initialize, .. } = f.work.last_mut().unwrap() {
            *initialize = Some(field);
        }
        f.compute("@group(0) @binding(0) var<storage,read> p:array<vec4f>; @group(0) @binding(1) var<storage,read> field:array<vec4f>; @group(0) @binding(2) var<storage,read_write> out:array<vec4f>; @compute @workgroup_size(64) fn apply(@builtin(global_invocation_id) id:vec3u){if id.x<arrayLength(&out){out[id.x]=vec4f(p[id.x].rgb*field[id.x].x,p[id.x].w);}}","apply",vec![(0,Binding::Read(input.resource)),(1,Binding::Read(field)),(2,Binding::Write(output.resource))],output.cells());
        Ok(())
    }
}
fn domain() -> Domain {
    Domain {
        bounds: [11, 17, 9, 7].into(),
        grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        meaning: Meaning::Graded,
    }
}
fn root(reads: Arc<AtomicUsize>) -> Arc<ImageNode> {
    let d = domain();
    ImageNode::new(
        d,
        Some(operators::source(Arc::new(Pixels { domain: d, reads }))),
        Field { seed: 7 },
    )
}
fn request(context: &Context, image: Arc<ImageNode>, bounds: SourceRegion) -> Evaluation {
    context
        .request(
            image.clone(),
            Region {
                bounds,
                grid: image.domain.grid,
            },
            Budget {
                tile: [bounds.width, bounds.height],
                scratch_bytes: 1 << 20,
            },
        )
        .unwrap()
}
fn expected(region: SourceRegion) -> Vec<[f32; 4]> {
    (region.y..region.y + region.height)
        .flat_map(|y| {
            (region.x..region.x + region.width).map(move |x| {
                let v = 1. + ((x + 3 * y + 7) % 5) as f32 * 0.0625;
                [-0.25 * v, 0.4 * v, 2. * v, 3.]
            })
        })
        .collect()
}

#[test]
fn evaluation_preserves_the_requested_grid_and_cancels_before_source_acquisition() {
    let (device, queue, _renderer) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let reads = Arc::new(AtomicUsize::new(0));
    let image = root(reads.clone());
    let mut region = Region {
        bounds: [12, 18, 5, 4].into(),
        grid: image.domain.grid,
    };
    let evaluation = request(&context, image.clone(), region.bounds);
    assert!(
        context
            .evaluate(
                &queue,
                &Default::default(),
                &evaluation,
                || true,
                |_, _, _| panic!("cancelled request reached its sink")
            )
            .unwrap()
            .is_none()
    );
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    region.grid.axes[0][0] = 2.;
    assert!(
        context
            .request(
                image,
                region,
                Budget {
                    tile: [5, 4],
                    scratch_bytes: 1 << 20
                }
            )
            .is_err()
    );
    let mut actual = None;
    let stats = context
        .evaluate(
            &queue,
            &Default::default(),
            &evaluation,
            || false,
            |encoder, image, buffer| {
                let output = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("context result"),
                    size: u64::from(image.cells()) * 16,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                });
                encoder.copy_buffer_to_buffer(buffer, 0, &output, 0, output.size());
                actual = Some(output);
                Ok(())
            },
        )
        .unwrap()
        .unwrap();
    assert_eq!(stats.initializations, 1);
    assert_eq!(
        execute::read(&device, &queue, &actual.unwrap()),
        expected([12, 18, 5, 4].into())
    );
    assert_eq!(context.retained_bytes(), 0);
}

#[test]
fn generated_retention_is_bounded_independently_of_scratch_and_cache_off_preserves_pixels() {
    let (device, queue, _renderer) = crate::gpu::test_support::render_context();
    let region = SourceRegion::from([12, 18, 5, 4]);
    let field_bytes = u64::from(region.width * region.height) * 16;
    for limit in [0, field_bytes - 1, field_bytes] {
        let context = Context::new(
            &device,
            CachePolicy {
                generated_bytes: limit,
            },
        );
        let evaluation = request(&context, root(Arc::new(AtomicUsize::new(0))), region);
        let plan = evaluation.plans().next().unwrap().unwrap();
        let mut executor = context.executor();
        for iteration in 0..2 {
            let result = executor
                .run(&device, &queue, &Default::default(), &plan, || false)
                .unwrap()
                .unwrap();
            assert_eq!(
                execute::read(&device, &queue, &result.pixels),
                expected(region)
            );
            assert_eq!(
                result.stats.initializations,
                usize::from(iteration == 0 || limit < field_bytes)
            );
            assert!(
                result.stats.scratch_bytes > 0,
                "cache-off still admits necessary working storage"
            );
            assert_eq!(
                context.retained_bytes(),
                if limit >= field_bytes { field_bytes } else { 0 }
            );
        }
        drop(executor);
        assert_eq!(
            context.retained_bytes(),
            0,
            "dropping work releases its optional budget ownership"
        );
    }
}

#[test]
fn record_only_cache_off_regenerates_and_budgeted_preparations_share_only_admitted_values() {
    use wgpu::util::DeviceExt;
    let (device, queue, _renderer) = crate::gpu::test_support::render_context();
    let d = domain();
    let bytes = u64::from(d.bounds.width * d.bounds.height) * 16;
    let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&vec![[-0.25f32, 0.4, 2., 3.]; (bytes / 16) as usize]),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: bytes,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    for limit in [0, bytes] {
        let context = Context::new(
            &device,
            CachePolicy {
                generated_bytes: limit,
            },
        );
        let image = ImageNode::new(d, Some(resident::borrowed_input(d)), Field { seed: 7 });
        let first = context
            .record(request(&context, image.clone(), d.bounds), vec![])
            .unwrap();
        let reserved = first.cached::<FieldKey>().cloned();
        assert_eq!(reserved.is_some(), limit > 0);
        if let Some(field) = &reserved {
            assert!(
                !field.ready(),
                "prepared but unsubmitted data is not initialized"
            );
            assert_eq!(field.pixels.size(), context.retained_bytes());
        }
        let second = context
            .record(
                request(&context, image.clone(), d.bounds),
                first.resources(),
            )
            .unwrap();
        if let Some(field) = &reserved {
            assert!(Arc::ptr_eq(field, second.cached::<FieldKey>().unwrap()));
        }
        let competing = context
            .record(request(&context, image.clone(), d.bounds), vec![])
            .unwrap();
        assert!(
            competing.cached::<FieldKey>().is_none(),
            "simultaneously owned optional values cannot exceed the context budget"
        );
        assert!(context.retained_bytes() <= limit);
        for prepared in [&first, &second, &competing] {
            for _ in 0..2 {
                let mut encoder = device.create_command_encoder(&Default::default());
                prepared
                    .encode_into(&device, &mut encoder, &input, &output)
                    .unwrap();
                queue.submit([encoder.finish()]);
                prepared.submitted();
                assert_eq!(execute::read(&device, &queue, &output), expected(d.bounds));
            }
        }
        if let Some(field) = &reserved {
            assert!(field.ready());
        }
        let foreign_context = Context::new(&device, CachePolicy { generated_bytes: 0 });
        let foreign = foreign_context
            .record(
                request(&foreign_context, image, d.bounds),
                first.resources(),
            )
            .unwrap();
        assert!(
            foreign.cached::<FieldKey>().is_none(),
            "a previous preparation cannot bypass a different context's disabled cache"
        );
        assert_eq!(foreign_context.retained_bytes(), 0);
        let changed = ImageNode::new(d, Some(resident::borrowed_input(d)), Field { seed: 8 });
        let replaced = context
            .record(request(&context, changed, d.bounds), first.resources())
            .unwrap();
        assert!(
            replaced.resources().is_empty(),
            "an active replacement must not pin its obsolete variant when the retention budget is exhausted"
        );
        drop((first, second, competing, reserved));
        assert_eq!(context.retained_bytes(), 0);
    }
}

#[test]
fn retention_reservations_are_atomic_and_do_not_overflow() {
    let retention = cache::Retention::new(32);
    let admitted = std::sync::Barrier::new(9);
    let release = std::sync::Barrier::new(9);
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let retention = &retention;
            let admitted = &admitted;
            let release = &release;
            scope.spawn(move || {
                let lease = retention.acquire(16);
                admitted.wait();
                release.wait();
                drop(lease);
            });
        }
        admitted.wait();
        assert_eq!(retention.bytes(), 32);
        release.wait();
    });
    assert_eq!(retention.bytes(), 0);
    let huge = cache::Retention::new(u64::MAX);
    let lease = huge.acquire(u64::MAX).unwrap();
    assert!(huge.acquire(1).is_none());
    drop(lease);
    assert_eq!(huge.bytes(), 0);
}

#[test]
fn output_grid_carries_oriented_source_coordinates_and_declared_pitch() {
    use crate::geometry::{Composition, FramePoint, PhotoOrientation, QuarterTurns};
    let composition = Composition::new(
        [11, 17, 9, 7].into(),
        Default::default(),
        PhotoOrientation {
            turns: QuarterTurns::One,
            flips: Default::default(),
        },
    )
    .unwrap();
    let frame = crate::frame_raster::FrameRaster::new(
        composition,
        FramePoint([1., 2.]),
        FramePoint([6., 8.]),
        [5, 3],
    )
    .unwrap();
    let grid = Grid::from_frame(frame);
    assert_eq!(grid.origin, [13., 23.]);
    assert_eq!(grid.axes, [[0., -1.], [2., 0.]]);
    assert_eq!(grid.pitch, [1., 2.]);
    assert_eq!(grid.point([1.5, 1.5]), [16., 21.5]);
}

#[test]
fn evaluation_arms_its_readback_sink_before_the_completion_wait() {
    let (device, queue, _renderer) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let bounds = SourceRegion::from([12, 18, 5, 4]);
    let evaluation = request(&context, root(Arc::new(AtomicUsize::new(0))), bounds);
    let output = std::cell::RefCell::new(None);
    let (sender, receiver) = std::sync::mpsc::channel();
    let completed = context
        .evaluate_with(
            &queue,
            &Default::default(),
            &evaluation,
            || false,
            Sink {
                encode: |encoder: &mut wgpu::CommandEncoder,
                         image: PlannedImage,
                         pixels: &wgpu::Buffer| {
                    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("bounded context readback"),
                        size: u64::from(image.cells()) * 16,
                        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                        mapped_at_creation: false,
                    });
                    encoder.copy_buffer_to_buffer(pixels, 0, &buffer, 0, buffer.size());
                    *output.borrow_mut() = Some(buffer);
                    Ok(())
                },
                submitted: || {
                    let sender = sender.clone();
                    output.borrow().as_ref().unwrap().slice(..).map_async(
                        wgpu::MapMode::Read,
                        move |result| {
                            sender.send(result).unwrap();
                        },
                    );
                },
            },
        )
        .unwrap();
    assert!(completed.is_some());
    receiver
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap()
        .unwrap();
    let output = output.into_inner().unwrap();
    let mapped = output.slice(..).get_mapped_range().unwrap();
    assert_eq!(
        bytemuck::cast_slice::<u8, [f32; 4]>(&mapped),
        expected(bounds)
    );
}
