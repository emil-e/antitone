use super::*;

struct TiledPlane {
    reads: Arc<AtomicUsize>,
    width: u32,
    fail: bool,
}
impl Source for TiledPlane {
    fn domain(&self) -> Domain {
        Plane(self.reads.clone()).domain()
    }
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(("tiled test acquisition", self.fail)))
    }
    fn acquisition_tile(&self) -> Option<[u32; 2]> {
        Some([self.width, 64])
    }
    fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        assert_eq!(region.width, self.width);
        assert_eq!(region.x % self.width as i32, 0);
        if self.fail && self.reads.load(Ordering::Relaxed) > 0 {
            self.reads.fetch_add(1, Ordering::Relaxed);
            return Err(Error::InvalidParameter("fixture source failure"));
        }
        Plane(self.reads.clone()).read(region)
    }
}

struct Strips;
impl Operation for Strips {
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("source strip gain"))
    }
    fn reads(&self, region: CellRegion) -> Result<Vec<Read>, Error> {
        Ok(vec![Read::new(0, region)])
    }
    fn work_units(&self, region: CellRegion, _: Tiling) -> Result<WorkUnits, Error> {
        tiled_units(region, [8, 64])
    }
    fn calculation(
        self: Arc<Self>,
        inputs: &[Value],
        domain: Domain,
        output: CellRegion,
        _: Tiling,
    ) -> Result<Option<value::Calculation>, Error> {
        assert_eq!([output.width, output.height], [8, 64]);
        let inputs = value::Calculation::references(inputs);
        value::Calculation::new(
            domain,
            output,
            vec![inputs[0].request(output)?],
            value::Operation::new(
                "source strip gain",
                (),
                "fn pixel(p:vec2i,position:vec2f,a:vec4f)->vec4f {return vec4f(a.rgb*2.0,a.w);}",
                "pixel",
                value::Access::Pointwise,
            ),
        )
        .map(Some)
    }
}

fn image_request(context: &Context, root: Value) -> Evaluation {
    context
        .request(
            root.clone(),
            Region {
                bounds: root.domain().finite().unwrap(),
                grid: root.domain().grid,
            },
            Tiling { tile: [64, 64] },
        )
        .unwrap()
        .without_retention()
}

#[test]
fn interrupted_acquisition_releases_tiles_without_publishing_the_consumer() {
    let (device, queue, _) = crate::photographic::test_support::render_context();
    for fail in [false, true] {
        let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
        let reads = Arc::new(AtomicUsize::new(0));
        let input = source(Arc::new(TiledPlane {
            reads: reads.clone(),
            width: 8,
            fail,
        }));
        let source_id = input.identity().unwrap().clone();
        let root = Value::from_operation(input.domain(), [input], Strips);
        let output_id = root.identity().unwrap().clone();
        let prepared = context
            .prepare_image(image_request(&context, root))
            .unwrap();
        assert!(!prepared.can_record());
        assert_eq!(reads.load(Ordering::Relaxed), 0);
        let result = context.produce_prepared(&queue, prepared, || {
            !fail && reads.load(Ordering::Relaxed) >= 2
        });
        if fail {
            assert!(matches!(
                result,
                Err(Error::InvalidParameter("fixture source failure"))
            ));
        } else {
            assert!(matches!(result, Err(Error::Cancelled)));
        }
        assert_eq!(reads.load(Ordering::Relaxed), 2);
        let scope = context.materializations();
        assert!(scope.ready::<ImageLayout>(&source_id).is_empty());
        assert!(scope.ready::<ImageLayout>(&output_id).is_empty());
        assert_eq!(context.retained_bytes(), 0);
    }
}

#[test]
fn source_tiles_do_not_bypass_complete_result_storage_limits() {
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
    let input = source(Arc::new(TiledPlane {
        reads: reads.clone(),
        width: 8,
        fail: false,
    }));
    let root = Value::from_operation(input.domain(), [input], Strips);
    let request = image_request(&context, root.clone());
    super::super::super::super::super::unit::plan(
        &root,
        [0, 0, 8, 64].into(),
        request.job.tiling(),
    )
    .unwrap()
    .admit_for(&device.limits())
    .expect("the individual unit fits the device");
    assert!(matches!(
        context.prepare_image(request),
        Err(Error::EvaluationLimit(_))
    ));
    assert_eq!(reads.load(Ordering::Relaxed), 0);
}

#[test]
fn units_share_active_source_tiles_after_submissions_finish_without_retention() {
    let (device, queue, _) = crate::photographic::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let scope = context.materializations();
    let reads = Arc::new(AtomicUsize::new(0));
    let input = source(Arc::new(TiledPlane {
        reads: reads.clone(),
        width: 32,
        fail: false,
    }));
    let identity = input.identity().unwrap().clone();
    let request = image_request(
        &context,
        Value::from_operation(input.domain(), [input], Strips),
    );
    let image = {
        let working = RunWorkingSet::default();
        let worker = Worker {
            context: &context,
            queue: &queue,
            scope: &scope,
            working: &working,
            cancelled: &|| false,
            progress: &|| working.finish_gpu().unwrap(),
        };
        let image = worker
            .materialize_image(
                &request,
                context.prepared_layout(&request).unwrap(),
                &mut WorkCounts::default(),
            )
            .unwrap();
        working.finish_gpu().unwrap();
        assert_eq!(reads.load(Ordering::Relaxed), 2);
        assert_eq!(scope.ready::<ImageLayout>(&identity).len(), 2);
        image
    };
    assert!(scope.ready::<ImageLayout>(&identity).is_empty());
    assert_eq!(context.retained_bytes(), 0);
    let pixels = super::super::super::super::super::execute::read(
        &device,
        &queue,
        image.as_buffer().unwrap(),
    );
    assert_eq!(pixels[63], [9.75, 4., -4., 8.]);
}

#[test]
fn source_tiles_cover_smaller_consumer_units_without_duplicate_storage() {
    let (device, queue, _) = crate::photographic::test_support::render_context();
    let context = Context::new(
        &device,
        CachePolicy {
            generated_bytes: 1 << 20,
        },
    );
    let scope = context.materializations();
    scope.set_retention_enabled(true);
    scope.set_test_pressure(crate::sys::MemoryPressure::Normal);
    let reads = Arc::new(AtomicUsize::new(0));
    let input = source(Arc::new(TiledPlane {
        reads: reads.clone(),
        width: 32,
        fail: false,
    }));
    let source_id = input.identity().unwrap().clone();
    let root = Value::from_operation(input.domain(), [input], Strips);
    let prepared = context
        .prepare_image(image_request(&context, root))
        .unwrap();
    let image = context
        .produce_prepared(&queue, prepared, || false)
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
    assert_eq!(reads.load(Ordering::Relaxed), 2);
    let mut tiles = scope
        .ready::<ImageLayout>(&source_id)
        .into_iter()
        .map(|image| image.descriptor.region)
        .collect::<Vec<_>>();
    tiles.sort_by_key(|region| region.x);
    assert_eq!(tiles, vec![[0, 0, 32, 64].into(), [32, 0, 32, 64].into()]);
    context.set_retention_limit(0);
    assert!(scope.ready::<ImageLayout>(&source_id).is_empty());
    assert_eq!(context.retained_bytes(), 0);
}
