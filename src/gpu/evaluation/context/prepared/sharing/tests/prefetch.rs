use super::*;

struct FailingPlane(Plane);
impl Source for FailingPlane {
    fn domain(&self) -> Domain {
        self.0.domain()
    }
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("fallible acquisition plane"))
    }
    fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        if self.0.0.load(Ordering::Relaxed) > 0 {
            self.0.0.fetch_add(1, Ordering::Relaxed);
            Err(Error::InvalidParameter("fixture source failure"))
        } else {
            self.0.read(region)
        }
    }
}

#[test]
fn interrupted_acquisition_releases_completed_slices_without_publishing_the_consumer() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    for failure in [false, true] {
        let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
        let reads = Arc::new(AtomicUsize::new(0));
        let plane = Plane(reads.clone());
        let input = if failure {
            source(Arc::new(FailingPlane(plane)))
        } else {
            source(Arc::new(plane))
        };
        let operation = SlicedPlane::new(&context, reads.clone(), &[8; 8]);
        let root = Value::from_operation(input.domain(), [input], operation);
        let region = root.domain().finite().unwrap();
        let request = context
            .request(
                root.clone(),
                Region {
                    bounds: region,
                    grid: root.domain().grid,
                },
                Tiling { tile: [64, 64] },
            )
            .unwrap()
            .without_retention();
        let prepared = context.prepare_image(request).unwrap();
        assert!(
            prepared.dependencies.len() >= 2,
            "interrupt after one acquired slice has completed"
        );
        let acquired = prepared.dependencies[0]
            .evaluation
            .job
            .image()
            .output(prepared.dependencies[0].evaluation.job.region())
            .unwrap()
            .identity()
            .unwrap()
            .clone();
        let root_key = root.output(region).unwrap().identity().unwrap().clone();
        let result = context.produce_prepared(&queue, prepared, || {
            !failure && reads.load(Ordering::Relaxed) >= 2
        });
        if failure {
            assert!(matches!(
                result,
                Err(Error::InvalidParameter("fixture source failure"))
            ));
        } else {
            assert!(matches!(result, Err(Error::Cancelled)));
        }
        assert_eq!(reads.load(Ordering::Relaxed), 2);
        assert!(
            context
                .materializations()
                .ready::<ImageLayout>(&acquired)
                .is_empty(),
            "completed acquisition leases retire when the remaining request fails or cancels"
        );
        assert!(
            context
                .materializations()
                .ready::<ImageLayout>(&root_key)
                .is_empty()
        );
        assert_eq!(context.retained_bytes(), 0);
    }
}

#[test]
fn prefetched_units_do_not_bypass_complete_result_storage_limits() {
    let instance = crate::gpu::instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, _) = pollster::block_on(crate::gpu::submission::request_device(
        &adapter,
        &wgpu::DeviceDescriptor {
            required_limits: wgpu::Limits {
                max_storage_buffer_binding_size: 32 << 10,
                ..Default::default()
            },
            ..Default::default()
        },
    ))
    .unwrap();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let reads = Arc::new(AtomicUsize::new(0));
    let source = source(Arc::new(Plane(reads.clone())));
    let root = Value::from_operation(
        source.domain(),
        [source],
        SlicedPlane::new(&context, reads.clone(), &[8; 8]),
    );
    let region = root.domain().finite().unwrap();
    let budget = Tiling { tile: [64, 64] };
    super::super::super::super::super::unit::plan(&root, [0, 0, 8, 64].into(), budget)
        .unwrap()
        .admit_for(&device.limits())
        .expect("the individual unit fits the device");
    assert!(64 * 64 * 16 > device.limits().max_storage_buffer_binding_size);
    let request = context
        .request(
            root.clone(),
            Region {
                bounds: region,
                grid: root.domain().grid,
            },
            budget,
        )
        .unwrap();
    assert!(
        matches!(
            context.prepare_image(request),
            Err(Error::EvaluationLimit(_))
        ),
        "a prepared image still needs a complete destination, not only fitting units"
    );
    assert_eq!(reads.load(Ordering::Relaxed), 0);
}

struct SlicedPlane {
    slices: Vec<CellRegion>,
    subdivisions: Arc<AtomicUsize>,
    source_reads: Arc<AtomicUsize>,
    scope: Materializations,
    live_source_copies: Arc<std::sync::Mutex<Vec<usize>>>,
    observed_source: Arc<std::sync::Mutex<Option<CellRegion>>>,
}

impl SlicedPlane {
    fn new(context: &Context, source_reads: Arc<AtomicUsize>, widths: &[u32]) -> Self {
        let mut x = 0;
        let slices = widths
            .iter()
            .map(|width| {
                let region = CellRegion::new([x, 0], [*width, 64]);
                x += *width as i32;
                region
            })
            .collect();
        assert_eq!(x, 64);
        Self {
            slices,
            subdivisions: Arc::new(AtomicUsize::new(0)),
            source_reads,
            scope: context.materializations(),
            live_source_copies: Arc::new(std::sync::Mutex::new(Vec::new())),
            observed_source: Arc::new(std::sync::Mutex::new(None)),
        }
    }
}

impl Operation for SlicedPlane {
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("source-sliced plane gain"))
    }

    fn reads(&self, output: CellRegion) -> Result<Vec<Read>, Error> {
        Ok(self
            .slices
            .iter()
            .copied()
            .filter(|region| {
                region.x < output.right()
                    && output.x < region.right()
                    && region.y < output.bottom()
                    && output.y < region.bottom()
            })
            .map(|region| Read::source_slice(0, region))
            .collect())
    }

    fn work_units(&self, output: CellRegion, _: Tiling) -> Result<WorkUnits, Error> {
        tiled_units(output, [8, 64])
    }

    fn calculation(
        self: Arc<Self>,
        inputs: &[Value],
        domain: Domain,
        output: CellRegion,
        _: Tiling,
    ) -> Result<Option<value::Calculation>, Error> {
        if output.width < 8 || output.height < 64 {
            self.subdivisions.fetch_add(1, Ordering::Relaxed);
        }
        if self.source_reads.load(Ordering::Relaxed) > 0 {
            let region = self.observed_source.lock().unwrap().unwrap();
            let first = inputs[0].output(region)?;
            let copies = self
                .scope
                .ready::<ImageLayout>(first.identity().unwrap())
                .iter()
                .filter(|image| image.descriptor.region == first.region)
                .count();
            self.live_source_copies.lock().unwrap().push(copies);
        }
        let inputs = value::Calculation::references(inputs);
        value::Calculation::new(
            domain,
            output,
            vec![inputs[0].request(output)?],
            value::Operation::new(
                "source-sliced plane gain",
                (),
                "fn pixel(p:vec2i,position:vec2f,a:vec4f)->vec4f {return vec4f(a.rgb*2.0,a.w);}",
                "pixel",
                value::Access::Pointwise,
            ),
        )
        .map(Some)
    }
}

#[test]
fn prefetch_keeps_useful_slices_without_subdividing_the_consumer_to_fit_them() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    for (retention, widths) in [
        (0, vec![8; 8]),
        (1 << 20, vec![8; 8]),
        (0, vec![8, 8, 8, 8, 32]),
    ] {
        let context = Context::new(
            &device,
            CachePolicy {
                generated_bytes: retention,
            },
        );
        context
            .materializations()
            .set_test_pressure(crate::sys::MemoryPressure::Normal);
        let reads = Arc::new(AtomicUsize::new(0));
        let source = source(Arc::new(Plane(reads.clone())));
        let operation = SlicedPlane::new(&context, reads.clone(), &widths);
        let subdivisions = operation.subdivisions.clone();
        let live_source_copies = operation.live_source_copies.clone();
        let observed_source = operation.observed_source.clone();
        let root = Value::from_operation(source.domain(), [source], operation);
        let region = root.domain().finite().unwrap();
        let budget = Tiling { tile: [64, 64] };
        let request = context
            .request(
                root.clone(),
                Region {
                    bounds: region,
                    grid: root.domain().grid,
                },
                budget,
            )
            .unwrap()
            .without_retention();
        let prepared = context.prepare_image(request).unwrap();
        assert_eq!(
            reads.load(Ordering::Relaxed),
            0,
            "preparation performs no source IO"
        );
        assert_eq!(
            prepared.dependencies.len(),
            widths.len(),
            "valid declared slices need no artificial prefetch reserve"
        );
        for dependency in &prepared.dependencies {
            let region = dependency.evaluation.job.region();
            assert!(region.width > 0 && dependency.destination.is_some());
        }
        assert_eq!(
            subdivisions.load(Ordering::Relaxed),
            0,
            "prefetch admission must not search for smaller consumer units"
        );
        *observed_source.lock().unwrap() = Some(prepared.dependencies[0].evaluation.job.region());
        let image = pollster::block_on(
            context
                .produce_images(&queue, vec![prepared])
                .unwrap()
                .remove(0),
        )
        .unwrap();
        let pixels = super::super::super::super::super::execute::read(
            &device,
            &queue,
            image.as_buffer().unwrap(),
        );
        for (index, pixel) in pixels.iter().enumerate() {
            let x = (index % 64) as f32;
            let y = (index / 64) as f32;
            assert_eq!(*pixel, [2. * (x / 8. - 3.), 2. * (y / 4. + 2.), -4., 8.]);
        }
        assert!(reads.load(Ordering::Relaxed) > 0);
        if retention == 0 {
            assert_eq!(context.retained_bytes(), 0);
            let copies = live_source_copies.lock().unwrap();
            assert!(
                !copies.is_empty(),
                "observe source storage while the consumer is planned"
            );
            assert!(
                copies.iter().all(|count| *count == 1),
                "only the acquired slice, not its completed producer's temporary copy, remains live: {copies:?}"
            );
        }
    }
}
