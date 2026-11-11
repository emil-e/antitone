use super::*;

struct DamagedPlate(Arc<Mutex<Vec<CellRegion>>>);
impl Source for DamagedPlate {
    fn domain(&self) -> Domain {
        Plate.domain()
    }
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("damaged regional ramp"))
    }
    fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        self.0.lock().unwrap().push(region);
        let mut pixels = Plate.read(region)?;
        for pixel in &mut pixels {
            pixel[3] = 0.;
        }
        for point in [[41, 41], [361, 41]] {
            if region.x <= point[0]
                && point[0] < region.right()
                && region.y <= point[1]
                && point[1] < region.bottom()
            {
                pixels[region.index(point)] = [0.; 4];
            }
        }
        Ok(pixels)
    }
}
struct Grade;
impl Operation for Grade {
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("regional retention grade"))
    }
    fn reads(&self, region: CellRegion) -> Result<Vec<Read>, Error> {
        Ok(vec![Read::new(0, region)])
    }
    fn calculation(
        self: std::sync::Arc<Self>,
        inputs: &[crate::gpu::evaluation::value::Value],
        domain: Domain,
        output: CellRegion,
        _budget: crate::gpu::evaluation::Budget,
    ) -> Result<Option<crate::gpu::evaluation::value::Calculation>, crate::Error> {
        let inputs = crate::gpu::evaluation::value::Calculation::references(inputs);
        let function = Function::new(
            "fn grade(p:vec4f,at:vec2f)->vec4f{return vec4f(p.rgb*2.,p.w);}",
            "grade",
        );
        value::Calculation::new(
            domain,
            output,
            vec![inputs[0].request(output)?],
            value::Operation::shader_function(self.key(), function),
        )
        .map(Some)
    }
}

#[test]
fn complete_frames_and_regional_values_reuse_the_same_optional_cache() {
    regional_retention(4 << 20, true);
}

#[test]
fn a_small_optional_cache_bounds_retention_without_changing_repaired_pixels() {
    regional_retention(768 << 10, false);
}

fn regional_retention(capacity: u64, keeps_regions: bool) {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(
        &device,
        CachePolicy {
            generated_bytes: capacity,
        },
    );
    let scope = context.materializations();
    scope.set_test_pressure(crate::sys::MemoryPressure::Normal);
    let reads = Arc::new(Mutex::new(Vec::new()));
    let make = |repairs: &[Repair]| {
        let base = source(Arc::new(DamagedPlate(reads.clone())));
        [[256, 128], [128, 64]].map(|size| {
            let domain = Domain {
                bounds: Some(CellRegion::new([0, 0], size)),
                grid: Grid::axis_aligned([0.; 2], [512. / size[0] as f64, 256. / size[1] as f64]),
                meaning: Meaning::Neutral,
            };
            reconstruct(compose(base.clone(), repairs).unwrap(), domain).unwrap()
        })
    };
    let materialize = |roots: &[Value]| {
        let requests = roots
            .iter()
            .map(|root| query(&context, root.clone()))
            .collect();
        pollster::block_on(futures_util::future::try_join_all(
            context
                .materialize_many(&queue, &Default::default(), &scope, requests)
                .unwrap(),
        ))
        .unwrap()
    };
    let pixels = |root: Value| {
        let bounds = root.domain().finite().unwrap();
        let mut output = vec![[0.; 4]; bounds.width as usize * bounds.height as usize];
        context
            .read(
                &queue,
                &Default::default(),
                &query(&context, root),
                || false,
                |block, values| {
                    for row in 0..block.height {
                        let from = row as usize * block.width as usize;
                        let to = bounds.index([block.x, block.y + row as i32]);
                        output[to..to + block.width as usize]
                            .copy_from_slice(&values[from..from + block.width as usize]);
                    }
                    Ok(())
                },
            )
            .unwrap()
            .unwrap();
        output
    };
    let original = make(&[repair(40, 32)]);
    let footprint: u64 = original
        .iter()
        .map(|root| {
            let r = root.domain().finite().unwrap();
            u64::from(r.width) * u64::from(r.height) * 16
        })
        .sum();
    assert!(footprint < context.cache_policy().generated_bytes);
    assert_eq!(
        2 * footprint < capacity,
        keeps_regions,
        "the cases must distinguish room for complete frames and regional values from a competing working set"
    );
    let neutral = materialize(&original);
    let graded = original.map(|root| Value::from_operation(root.domain(), [root], Grade));
    let finished = materialize(&graded);
    for image in &finished {
        crate::gpu::evaluation::BufferImage::try_from(image.clone())
            .unwrap()
            .submitted(&scope);
    }
    let before = pixels(graded[0].clone());
    let index = 20 * 256 + 180;
    let expected = [2. * (1. + 360.5 / 512.), 2. * (-0.5 - 40.5 / 256.), 0.5, 0.];
    assert!(
        (before[index][0] - expected[0]).abs() > 0.1,
        "the second defect must visibly remain before accepting its repair"
    );
    assert!(
        reads
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.x < 256 && r.right() <= 256),
        "cold detail acquisition must actually read the left region"
    );
    reads.lock().unwrap().clear();
    let changed = make(&[repair(40, 32), repair(360, 32)]);
    let new_neutral = materialize(&changed);
    let graded = changed.map(|root| Value::from_operation(root.domain(), [root], Grade));
    let new_finished = materialize(&graded);
    for image in &new_finished {
        crate::gpu::evaluation::BufferImage::try_from(image.clone())
            .unwrap()
            .submitted(&scope);
    }
    let check_plate = |image: &[[f32; 4]]| {
        assert_eq!(image.len(), 256 * 128);
        for y in 0..128 {
            for x in 0..256 {
                // Each output averages a 2x2 block of the authored linear plate.
                let expected = [
                    2. * (1. + (2. * x as f32 + 0.5) / 512.),
                    2. * (-0.5 - (2. * y as f32 + 0.5) / 256.),
                    0.5,
                    0.,
                ];
                for (actual, expected) in image[y * 256 + x].into_iter().zip(expected) {
                    assert!(
                        (actual - expected).abs() < 2e-5,
                        "repaired plate differs at [{x}, {y}]: {:?}",
                        image[y * 256 + x]
                    );
                }
            }
        }
    };
    check_plate(&pixels(graded[0].clone()));
    let acquired = reads.lock().unwrap();
    assert!(
        !acquired.is_empty(),
        "the changed repair and underlay must actually execute"
    );
    if keeps_regions {
        assert!(
            acquired.iter().all(|r| !(r.x < 256 && r.right() <= 256)),
            "unchanged detail must be consumed from its retained GPU region, not recomputed: {acquired:?}"
        );
    }
    assert!(
        context.retained_bytes() > 0,
        "ordinary requests must select optional ownership for completed calculations"
    );
    assert!(context.retained_bytes() <= capacity);
    drop(acquired);
    let weak = Arc::downgrade(&new_finished[0].image);
    let key = new_finished[0].identity.clone();
    drop((neutral, finished, new_neutral, new_finished));
    if keeps_regions {
        assert!(
            weak.upgrade().is_some(),
            "the cache must retain a complete processed frame after its view departs"
        );
        assert!(scope.find_ready::<ImageLayout>(&key, |_| true).is_some());
    }
    scope.set_retention_enabled(false);
    assert!(
        weak.upgrade().is_none(),
        "no frame owner may hide outside the common optional cache"
    );
    assert_eq!(context.retained_bytes(), 0);
    reads.lock().unwrap().clear();
    check_plate(&pixels(graded[0].clone()));
    assert!(
        !reads.lock().unwrap().is_empty(),
        "evicted values must genuinely recompute from source samples"
    );
    assert_eq!(context.retained_bytes(), 0);
}
