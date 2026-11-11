use super::*;
mod boundaries;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use wgpu::util::DeviceExt;

fn value(x: i32, y: i32) -> [f32; 4] {
    [
        -(x as f32),
        2. * y as f32,
        8. + x as f32 + 16. * y as f32,
        f32::from_bits(0x3f00_0000 + x as u32 + y as u32),
    ]
}
fn field(region: CellRegion) -> Vec<[f32; 4]> {
    (region.y..region.bottom())
        .flat_map(|y| (region.x..region.right()).map(move |x| value(x, y)))
        .collect()
}
fn gpu(device: &wgpu::Device, values: &[[f32; 4]]) -> Arc<ImageValue> {
    ImageValue::new(
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("assembly source"),
            contents: bytemuck::cast_slice(values),
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
        }),
    )
}
fn domain() -> Domain {
    Domain {
        bounds: Some([0, 0, 8, 4].into()),
        grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        meaning: Meaning::Neutral,
    }
}
fn layout(region: CellRegion, backing: ImageBacking) -> ImageLayout {
    ImageLayout {
        domain: domain(),
        region,
        format: ImageFormat::Rgba32Float,
        backing,
    }
}
fn units() -> Vec<CellRegion> {
    (0..2)
        .flat_map(|y| (0..4).map(move |x| [x * 2, y * 2, 2, 2].into()))
        .rev()
        .collect()
}
struct Tiles(Arc<AtomicUsize>);
impl Operation for Tiles {
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("assembly coordinate field"))
    }
    fn work_units(&self, _: CellRegion, _: Budget) -> Result<WorkUnits, Error> {
        Ok(Box::new(units().into_iter()))
    }
    fn calculation(
        self: Arc<Self>,
        _inputs: &[crate::gpu::evaluation::value::Value],
        domain: Domain,
        region: CellRegion,
        _: Budget,
    ) -> Result<Option<crate::gpu::evaluation::value::Calculation>, Error> {
        Ok(Some(value::Calculation::new(
            domain,
            region,
            Vec::new(),
            value::Operation::cpu("assembly coordinate field", self.key(), self),
        )?))
    }
}
impl CpuKernel for Tiles {
    fn run(
        &self,
        _: &[CpuImage],
        output: CellRegion,
        _: &dyn Fn() -> bool,
    ) -> Result<Option<Vec<[f32; 4]>>, Error> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok(Some(field(output)))
    }
}

#[test]
fn cached_units_assemble_partial_nonlocal_regions_in_byte_bounded_groups() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    for scratch in [128, 160, 192, 512] {
        let context = Context::new(
            &device,
            CachePolicy {
                generated_bytes: 1 << 20,
            },
        );
        let scope = context.materializations();
        scope.set_test_pressure(crate::sys::MemoryPressure::Normal);
        let calls = Arc::new(AtomicUsize::new(0));
        let root = Value::from_operation(domain(), [], Tiles(calls.clone()));
        let region: CellRegion = [1, 1, 6, 2].into();
        let query = context
            .request(
                root,
                Region {
                    bounds: region,
                    grid: domain().grid,
                },
                Budget {
                    tile: [6, 2],
                    scratch_bytes: scratch,
                },
            )
            .unwrap()
            .without_retention();
        let gate = crate::gpu::submission::SubmissionGate::default();
        let submissions = Arc::new(AtomicUsize::new(0));
        let observed = submissions.clone();
        gate.observe_encoded_resources(Arc::new(move || {
            observed.fetch_add(1, Ordering::Relaxed);
        }));
        let run = || {
            pollster::block_on(
                context
                    .materialize(
                        &queue,
                        &gate,
                        &scope,
                        query.clone(),
                        ImageFormat::Rgba32Float,
                        ImageBacking::Buffer,
                    )
                    .unwrap(),
            )
            .unwrap()
        };
        let first = run();
        assert_eq!(
            calls.load(Ordering::Relaxed),
            8,
            "cold controls must actually calculate every natural unit"
        );
        let actual =
            super::super::super::super::execute::read(&device, &queue, first.as_buffer().unwrap());
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(&actual),
            bytemuck::cast_slice::<_, u8>(&field(region))
        );
        drop(first);
        submissions.store(0, Ordering::Relaxed);
        let second = run();
        assert_eq!(
            calls.load(Ordering::Relaxed),
            8,
            "warm assembly must consume completed units, not recalculate them"
        );
        let maximum_units = scratch as usize / 64;
        assert_eq!(
            submissions.load(Ordering::Relaxed),
            8usize.div_ceil(maximum_units),
            "copies must share submissions without exceeding the allowance for full unit payloads"
        );
        let actual =
            super::super::super::super::execute::read(&device, &queue, second.as_buffer().unwrap());
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(&actual),
            bytemuck::cast_slice::<_, u8>(&field(region)),
            "partial rows retain global coordinates, signed/HDR values and exact metadata bits"
        );
    }
}

#[test]
fn mixed_copy_inputs_keep_physical_owners_until_completion_without_optional_retention() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    for scratch in [160, 192] {
        let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
        let scope = context.materializations();
        let gate = crate::gpu::submission::SubmissionGate::default();
        let working = HostWorkingSet::default();
        let region = [0, 0, 8, 4].into();
        let destination = gpu(&device, &[[99.; 4]; 32]);
        let visible = Arc::new(std::sync::Mutex::new(Vec::new()));
        let observed = visible.clone();
        let read_device = device.clone();
        let read_queue = queue.clone();
        let read_pixels = destination.as_buffer().unwrap().clone();
        // The observer runs after submit and outside the gate. A separate readback
        // observes the pixels actually copied by each group, including host writes.
        gate.observe_submissions(Arc::new(move || {
            let pixels =
                super::super::super::super::execute::read(&read_device, &read_queue, &read_pixels);
            observed
                .lock()
                .unwrap()
                .push(pixels.iter().filter(|pixel| **pixel != [99.; 4]).count());
        }));
        let production = Production {
            worker: Worker {
                context: &context,
                queue: &queue,
                gate: &gate,
                scope: &scope,
                budget: Budget {
                    tile: [8, 4],
                    scratch_bytes: scratch,
                },
                working: &working,
                cancelled: &|| false,
                progress: &|| {},
            },
            layout: layout(region, ImageBacking::Buffer),
            pixels: destination.as_buffer().unwrap(),
        };
        let mut copies = Copies::default();
        let mut weak = Vec::new();
        for (index, output) in units().into_iter().enumerate() {
            let values = field(output);
            let image = if index % 2 == 0 {
                gpu(&device, &values)
            } else {
                ImageValue::host(values)
            };
            weak.push(Arc::downgrade(&image));
            copies.copy(&production, image, output, output).unwrap();
        }
        let capacity = scratch as usize / 64;
        assert_eq!(copies.counts.submissions, (8 - 1) / capacity);
        assert!(
            weak[..6].iter().all(|value| value.upgrade().is_none()),
            "completed groups release their physical inputs"
        );
        assert!(
            weak[6..].iter().all(|value| value.upgrade().is_some()),
            "pending copies own inputs even with no cache or external owner"
        );
        assert_eq!(context.retained_bytes(), 0);
        copies.flush(&production).unwrap();
        assert!(weak.iter().all(|value| value.upgrade().is_none()));
        let mut previous = 0;
        for &pixels in visible.lock().unwrap().iter() {
            assert!(
                (pixels - previous) * 16 <= scratch as usize,
                "a submitted group exceeded the payload allowance: {previous}->{pixels} pixels under {scratch} bytes"
            );
            previous = pixels;
        }
        assert_eq!(previous, 32, "every input unit must reach the GPU output");
        let actual = super::super::super::super::execute::read(
            &device,
            &queue,
            destination.as_buffer().unwrap(),
        );
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(&actual),
            bytemuck::cast_slice::<_, u8>(&field(region))
        );
    }
}

#[test]
fn cancellation_drops_unsubmitted_copies_and_finishes_committed_host_or_gpu_transfers() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    for host in [false, true] {
        for committed in [false, true] {
            let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
            let scope = context.materializations();
            let gate = crate::gpu::submission::SubmissionGate::default();
            let submissions = Arc::new(AtomicUsize::new(0));
            let working = HostWorkingSet::default();
            let stop = Arc::new(AtomicBool::new(false));
            let cancelled = || stop.load(Ordering::Relaxed);
            let region = [0, 0, 2, 2].into();
            let destination = gpu(&device, &[[99.; 4]; 4]);
            let production = Production {
                worker: Worker {
                    context: &context,
                    queue: &queue,
                    gate: &gate,
                    scope: &scope,
                    budget: Budget {
                        tile: [2, 2],
                        scratch_bytes: 128,
                    },
                    working: &working,
                    cancelled: &cancelled,
                    progress: &|| {},
                },
                layout: layout(region, ImageBacking::Buffer),
                pixels: destination.as_buffer().unwrap(),
            };
            let source = if host {
                ImageValue::host(field(region))
            } else {
                gpu(&device, &field(region))
            };
            let weak = Arc::downgrade(&source);
            let observed = weak.clone();
            let observed_stop = stop.clone();
            let counted = submissions.clone();
            gate.observe_submissions(Arc::new(move || {
                counted.fetch_add(1, Ordering::Relaxed);
                assert!(
                    observed.upgrade().is_some(),
                    "submitted transfers retain their actual input owner"
                );
                observed_stop.store(true, Ordering::Relaxed);
            }));
            let mut copies = Copies::default();
            copies.copy(&production, source, region, region).unwrap();
            stop.store(!committed, Ordering::Relaxed);
            assert!(matches!(copies.flush(&production), Err(Error::Cancelled)));
            assert_eq!(submissions.load(Ordering::Relaxed), usize::from(committed));
            assert!(weak.upgrade().is_none());
            // A later independent submission must not apply abandoned host writes.
            // Committed work, in contrast, must finish despite owner cancellation.
            let actual = super::super::super::super::execute::read(
                &device,
                &queue,
                destination.as_buffer().unwrap(),
            );
            assert_eq!(
                actual,
                if committed {
                    field(region)
                } else {
                    vec![[99.; 4]; 4]
                }
            );
        }
    }
}

#[test]
fn an_assembly_failure_does_not_cancel_an_independently_joined_unit() {
    use crate::gpu::evaluation::materializations::ImageFuture;
    use std::future::Future;
    let (device, queue, _) = crate::gpu::test_support::render_context();
    for failure in [Error::Cancelled, Error::Gpu("copy failed".into())] {
        let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
        let scope = context.materializations();
        let gate = crate::gpu::submission::SubmissionGate::default();
        let working = HostWorkingSet::default();
        let stop = AtomicBool::new(false);
        let cancelled = || stop.load(Ordering::Relaxed);
        let worker = Worker {
            context: &context,
            queue: &queue,
            gate: &gate,
            scope: &scope,
            budget: Budget {
                tile: [2, 2],
                scratch_bytes: 128,
            },
            working: &working,
            cancelled: &cancelled,
            progress: &|| {},
        };
        let calls = Arc::new(AtomicUsize::new(0));
        let root = Value::from_operation(domain(), [], Tiles(calls.clone()));
        let region = [0, 0, 2, 2].into();
        let unit = root.plan_unit(region, worker.budget).unwrap();
        let mut joined = None::<std::pin::Pin<Box<ImageFuture<ImageLayout>>>>;
        let result = worker.unit_image_before(&root, unit, &mut WorkCounts::default(), || {
            let mut consumer = Box::pin(scope.obtain(
                root.identity().unwrap().clone(),
                Arc::new(layout(region, ImageBacking::Host)),
                64,
                || async { panic!("joined work must not be produced twice") },
            ));
            let mut task = std::task::Context::from_waker(futures_util::task::noop_waker_ref());
            assert!(
                consumer.as_mut().poll(&mut task).is_pending(),
                "independent consumer must genuinely join the inline producer before failure"
            );
            joined = Some(consumer);
            stop.store(matches!(failure, Error::Cancelled), Ordering::Relaxed);
            Err(failure.clone())
        });
        assert!(
            matches!(result, Err(Error::Cancelled) | Err(Error::Gpu(_))),
            "assembly still reports its own failure"
        );
        let image = pollster::block_on(joined.unwrap()).unwrap();
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(image.as_host().unwrap(), field(region));
    }
}
