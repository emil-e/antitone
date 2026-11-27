use super::*;
use crate::evaluation::numeric_fixture::NumericKey;
use crate::gaussian::operation as gaussian;
use crate::reconstruction::reconstruct_tiled;
use std::sync::atomic::{AtomicUsize, Ordering};

mod acquisition;

struct Plane(Arc<AtomicUsize>);
impl Source for Plane {
    fn domain(&self) -> Domain {
        Domain {
            bounds: Some([0, 0, 64, 64].into()),
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        }
    }
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("shared regional plane"))
    }
    fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok((region.y..region.bottom())
            .flat_map(|y| {
                (region.x..region.right())
                    .map(move |x| [x as f32 / 8. - 3., y as f32 / 4. + 2., -2., 8.])
            })
            .collect())
    }
}

struct Combine;
impl Operation for Combine {
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("shared regional combination"))
    }
    fn reads(&self, output: CellRegion) -> Result<Vec<Read>, Error> {
        Ok(vec![Read::new(0, output), Read::new(1, output)])
    }
    fn calculation(
        self: Arc<Self>,
        inputs: &[Value],
        domain: Domain,
        output: CellRegion,
        _: Tiling,
    ) -> Result<Option<value::Calculation>, Error> {
        let inputs = value::Calculation::references(inputs);
        let operation = value::Operation::new(
            "shared regional combination",
            (),
            "fn pixel(p:vec2i,position:vec2f,a:vec4f,b:vec4f)->vec4f {return vec4f(a.r+b.g,a.g-b.r,a.b*b.b,a.w);}",
            "pixel",
            value::Access::Pointwise,
        );
        value::Calculation::new(
            domain,
            output,
            vec![inputs[0].request(output)?, inputs[1].request(output)?],
            operation,
        )
        .map(Some)
    }
}

fn computes(preparation: &Preparation) -> usize {
    match &preparation.work {
        PreparedWork::Gpu(plan) | PreparedWork::Produce(plan) => plan
            .tiles
            .iter()
            .flat_map(|tile| &tile.fragment.work)
            .filter(|work| matches!(work, Work::Compute { .. }))
            .count(),
        _ => 0,
    }
}

#[test]
fn overlapping_branch_demands_compute_one_shared_value_with_cold_or_available_inputs() {
    let (device, queue, _) = crate::photographic::test_support::render_context();
    for resident in [false, true] {
        let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
        let reads = Arc::new(AtomicUsize::new(0));
        let input = source(Arc::new(Plane(reads.clone())));
        let held = if resident {
            let evaluation = context
                .request(
                    input.clone(),
                    Region {
                        bounds: input.domain().finite().unwrap(),
                        grid: input.domain().grid,
                    },
                    Tiling { tile: [64, 64] },
                )
                .unwrap()
                .without_retention();
            Some(
                pollster::block_on(
                    context
                        .materialize(
                            &queue,
                            &context.materializations(),
                            evaluation,
                            ImageFormat::Rgba32Float,
                            ImageBacking::Buffer,
                        )
                        .unwrap(),
                )
                .unwrap(),
            )
        } else {
            None
        };
        let first = gaussian::blur(input, 1.).unwrap();
        let expanded = gaussian::blur(first.clone(), 1.5).unwrap();
        let root = Value::from_operation(first.domain(), [first, expanded], Combine);
        let region = CellRegion::new([24, 20], [8, 8]);
        let evaluation = context
            .request(
                root,
                Region {
                    bounds: region,
                    grid: Grid::axis_aligned([0.; 2], [1.; 2]),
                },
                Tiling { tile: [8, 8] },
            )
            .unwrap()
            .without_retention();
        let shared = context.prepare_image(evaluation).unwrap();
        assert_eq!(
            computes(&shared),
            5,
            "two separable blurs need four Gaussian passes and a combination"
        );
        assert_eq!(shared.can_record(), resident);
        let pixels = if resident {
            let recorded = shared.record(&context).unwrap();
            let mut encoder = device.create_command_encoder(&Default::default());
            recorded
                .commands
                .as_ref()
                .unwrap()
                .encode_result(&device, &mut encoder, &recorded.pixels)
                .unwrap();
            queue.submit([encoder.finish()]);
            recorded.commands.as_ref().unwrap().submitted();
            recorded.pixels.submitted(&context.materializations());
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            super::super::super::super::execute::read(&device, &queue, &recorded.pixels)
        } else {
            let image = pollster::block_on(
                context
                    .produce_images(&queue, vec![shared], EvaluationRun::default())
                    .unwrap()
                    .remove(0),
            )
            .unwrap();
            super::super::super::super::execute::read(&device, &queue, image.as_buffer().unwrap())
        };
        for (index, pixel) in pixels.iter().enumerate() {
            let x = region.x as f32 + (index % 8) as f32;
            let y = region.y as f32 + (index / 8) as f32;
            let expected = [
                x / 8. - 3. + y / 4. + 2.,
                y / 4. + 2. - (x / 8. - 3.),
                4.,
                8.,
            ];
            for (actual, expected) in pixel.iter().zip(expected) {
                assert!(
                    (*actual - expected).abs() < 2e-5,
                    "linear fields preserve their mean across both filters: {pixel:?} {expected}"
                );
            }
        }
        assert!(reads.load(Ordering::Relaxed) > 0);
        if resident {
            assert_eq!(reads.load(Ordering::Relaxed), 1);
        }
        assert_eq!(context.retained_bytes(), 0);
        drop(held);
    }
}

struct Quadratic;
impl Source for Quadratic {
    fn domain(&self) -> Domain {
        Domain {
            bounds: Some([0, 0, 64, 64].into()),
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        }
    }
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("quadratic plan fixture"))
    }
    fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        Ok((region.y..region.bottom())
            .flat_map(|y| {
                (region.x..region.right())
                    .map(move |x| [(x * x) as f32 / 4096., (y * y) as f32 / 4096., -2., 8.])
            })
            .collect())
    }
}

struct SumBlurs;
impl Operation for SumBlurs {
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("sum three blur levels"))
    }
    fn reads(&self, region: CellRegion) -> Result<Vec<Read>, Error> {
        Ok((0..3).map(|index| Read::new(index, region)).collect())
    }
    fn calculation(
        self: Arc<Self>,
        inputs: &[Value],
        domain: Domain,
        region: CellRegion,
        _: Tiling,
    ) -> Result<Option<value::Calculation>, Error> {
        let inputs = value::Calculation::references(inputs)
            .iter()
            .map(|value| value.request(region))
            .collect::<Result<Vec<_>, _>>()?;
        value::Calculation::new(domain, region, inputs, value::Operation::new(
            "sum three blur levels", (),
            "fn pixel(p:vec2i,position:vec2f,a:vec4f,b:vec4f,c:vec4f)->vec4f {return vec4f(a.rgb+b.rgb+c.rgb,a.w);}",
            "pixel", value::Access::Pointwise,
        )).map(Some)
    }
}

#[test]
fn nested_shared_reads_compile_dependency_first_in_either_operand_order() {
    let (device, queue, _) = crate::photographic::test_support::render_context();
    let a = gaussian::blur(source(Arc::new(Quadratic)), 1.).unwrap();
    let b = gaussian::blur(a.clone(), 1.).unwrap();
    let c = gaussian::blur(b.clone(), 1.).unwrap();
    let region: CellRegion = [24, 20, 8, 8].into();
    let weight = |x: i32| (-0.5 * f64::from(x * x)).exp();
    let variance = (-4..=4).map(|x| weight(x) * f64::from(x * x)).sum::<f64>()
        / (-4..=4).map(weight).sum::<f64>();
    for inputs in [
        [b.clone(), c.clone(), a.clone()],
        [a.clone(), b.clone(), c.clone()],
    ] {
        let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
        let root = Value::from_operation(a.domain(), inputs, SumBlurs);
        let evaluation = context
            .request(
                root,
                Region {
                    bounds: region,
                    grid: a.domain().grid,
                },
                Tiling { tile: [8, 8] },
            )
            .unwrap();
        let prepared = context.prepare_image(evaluation).unwrap();
        assert_eq!(
            computes(&prepared),
            7,
            "three distinct blurs, two passes each, and the sum"
        );
        let image = context
            .produce_prepared(&queue, prepared, || false)
            .unwrap();
        let pixels =
            super::super::super::super::execute::read(&device, &queue, image.as_buffer().unwrap());
        for (index, pixel) in pixels.iter().enumerate() {
            let x = region.x + (index % 8) as i32;
            let y = region.y + (index / 8) as i32;
            // Each symmetric normalized blur adds its variance to a quadratic.
            let expected = [x, y].map(|v| (3. * f64::from(v * v) + 6. * variance) / 4096.);
            for (actual, expected) in pixel[..2].iter().zip(expected) {
                assert!((f64::from(*actual) - expected).abs() < 2e-5);
            }
            assert_eq!(pixel[2..], [-6., 8.]);
        }
    }
}

#[test]
fn mtf_and_halation_share_the_upstream_blur_without_changing_signed_light() {
    let (device, queue, _) = crate::photographic::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let input = source(Arc::new(Plane(Arc::new(AtomicUsize::new(0)))));
    let mtf = gaussian::blur(input, 1.).unwrap();
    let root = crate::photographic::stages::halation::HalationParameters {
        sigma: 1.5,
        strength: 0.5,
    }
    .compose(mtf)
    .unwrap();
    let region = CellRegion::new([24, 20], [8, 8]);
    let evaluation = context
        .request(
            root,
            Region {
                bounds: region,
                grid: Grid::axis_aligned([0.; 2], [1.; 2]),
            },
            Tiling { tile: [8, 8] },
        )
        .unwrap()
        .without_retention();
    let shared = context.prepare_image(evaluation).unwrap();
    assert_eq!(
        computes(&shared),
        5,
        "four Gaussian passes and signed recombination"
    );
    let image = pollster::block_on(
        context
            .produce_images(&queue, vec![shared], EvaluationRun::default())
            .unwrap()
            .remove(0),
    )
    .unwrap();
    let pixels =
        super::super::super::super::execute::read(&device, &queue, image.as_buffer().unwrap());
    let weight = |offset: i32| (-0.5 * (f64::from(offset) / 1.5).powi(2)).exp();
    let normalization = (-6..=6).map(weight).sum::<f64>();
    for (index, pixel) in pixels.iter().enumerate() {
        let x = region.x + (index % 8) as i32;
        let y = region.y + (index / 8) as i32;
        let red = f64::from(x) / 8. - 3.;
        let positive = (-6..=6)
            .map(|offset| (f64::from(x + offset) / 8. - 3.).max(0.) * weight(offset))
            .sum::<f64>()
            / normalization;
        let expected = [
            red + (positive - red.max(0.)) / 3.,
            f64::from(y) / 4. + 2.,
            -2.,
            8.,
        ];
        for (actual, expected) in pixel.iter().zip(expected) {
            assert!(
                (f64::from(*actual) - expected).abs() < 2e-5,
                "signed input plus positive-light redistribution: {pixel:?} {expected}"
            );
        }
    }
}

#[test]
fn shared_values_remain_recordable_without_a_speculative_working_reserve() {
    let (device, queue, _) = crate::photographic::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let input = source(Arc::new(Plane(Arc::new(AtomicUsize::new(0)))));
    let source_request = context
        .request(
            input.clone(),
            Region {
                bounds: input.domain().finite().unwrap(),
                grid: input.domain().grid,
            },
            Tiling { tile: [64, 64] },
        )
        .unwrap()
        .without_retention();
    let _input = pollster::block_on(
        context
            .materialize(
                &queue,
                &context.materializations(),
                source_request,
                ImageFormat::Rgba32Float,
                ImageBacking::Buffer,
            )
            .unwrap(),
    )
    .unwrap();
    let first = value::pointwise(
        input,
        value::Operation::new(
            "shared pointwise plane",
            (),
            "fn pixel(p:vec2i,position:vec2f,a:vec4f)->vec4f {return vec4f(a.rgb*2.0,a.w);}",
            "pixel",
            value::Access::Pointwise,
        ),
    )
    .unwrap();
    let second = gaussian::blur(first.clone(), 1.5).unwrap();
    let root = Value::from_operation(first.domain(), [first, second], Combine);
    let region = CellRegion::new([24, 20], [8, 8]);
    let request = context
        .request(
            root.clone(),
            Region {
                bounds: region,
                grid: root.domain().grid,
            },
            Tiling { tile: [8, 8] },
        )
        .unwrap()
        .without_retention();
    let work = context.prepare_image(request).unwrap();
    assert!(work.can_record());

    let recorded = work.record(&context).unwrap();
    let mut encoder = device.create_command_encoder(&Default::default());
    recorded
        .commands
        .as_ref()
        .unwrap()
        .encode_result(&device, &mut encoder, &recorded.pixels)
        .unwrap();
    queue.submit([encoder.finish()]);
    recorded.commands.as_ref().unwrap().submitted();
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let pixels = super::super::super::super::execute::read(&device, &queue, &recorded.pixels);
    for (index, pixel) in pixels.iter().enumerate() {
        let x = region.x as f32 + (index % 8) as f32;
        let y = region.y as f32 + (index / 8) as f32;
        let expected = [
            2. * (x / 8. - 3. + y / 4. + 2.),
            2. * (y / 4. + 2. - (x / 8. - 3.)),
            16.,
            8.,
        ];
        for (actual, expected) in pixel.iter().zip(expected) {
            assert!((*actual - expected).abs() < 2e-5);
        }
    }
}

#[test]
fn recorded_dependencies_enter_the_cache_only_after_submission() {
    let (device, queue, _) = crate::photographic::test_support::render_context();
    let context = Context::new(
        &device,
        CachePolicy {
            generated_bytes: 1 << 20,
        },
    );
    context.materializations().set_retention_enabled(true);
    let input = source(Arc::new(Plane(Arc::new(AtomicUsize::new(0)))));
    let source_request = context
        .request(
            input.clone(),
            Region {
                bounds: input.domain().finite().unwrap(),
                grid: input.domain().grid,
            },
            Tiling { tile: [64, 64] },
        )
        .unwrap()
        .without_retention();
    let _input = pollster::block_on(
        context
            .materialize(
                &queue,
                &context.materializations(),
                source_request,
                ImageFormat::Rgba32Float,
                ImageBacking::Buffer,
            )
            .unwrap(),
    )
    .unwrap();
    let first = gaussian::blur(input, 1.).unwrap().for_reuse();
    let identity = first.identity().unwrap().clone();
    let second = gaussian::blur(first.clone(), 1.5).unwrap();
    let root = Value::from_operation(first.domain(), [first, second], Combine);
    let region = CellRegion::new([24, 20], [8, 8]);
    let request = context
        .request(
            root,
            Region {
                bounds: region,
                grid: Grid::axis_aligned([0.; 2], [1.; 2]),
            },
            Tiling { tile: [8, 8] },
        )
        .unwrap()
        .without_retention();
    let prepared = context.prepare_image(request.clone()).unwrap();
    let recorded = prepared.record(&context).unwrap();
    assert!(
        context
            .materializations()
            .find_ready::<ImageLayout>(&identity, |_| true)
            .is_none()
    );
    let mut abandoned = device.create_command_encoder(&Default::default());
    recorded
        .commands
        .as_ref()
        .unwrap()
        .encode_result(&device, &mut abandoned, &recorded.pixels)
        .unwrap();
    drop((abandoned, recorded));
    assert!(
        context
            .materializations()
            .find_ready::<ImageLayout>(&identity, |_| true)
            .is_none()
    );
    let recorded = context
        .prepare_image(request)
        .unwrap()
        .record(&context)
        .unwrap();
    let mut encoder = device.create_command_encoder(&Default::default());
    recorded
        .commands
        .as_ref()
        .unwrap()
        .encode_result(&device, &mut encoder, &recorded.pixels)
        .unwrap();
    queue.submit([encoder.finish()]);
    recorded.commands.unwrap().submitted();
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    assert!(
        context
            .materializations()
            .find_ready::<ImageLayout>(&identity, |_| true)
            .is_some()
    );
}

fn scale_plane(input: Value, factor: f32) -> Value {
    let gain = value::Operation::new(
        "plane gain",
        NumericKey(factor),
        "struct Parameters { factor:vec4f }
         fn pixel(p:vec2i,position:vec2f,a:vec4f,v:Parameters)->vec4f {
             return vec4f(a.rgb*v.factor.x,a.w);
         }",
        "pixel",
        value::Access::Pointwise,
    )
    .with_uniform("Parameters", &[factor, 0., 0., 0.]);
    value::pointwise(input, gain).unwrap()
}

#[test]
fn reusable_boundary_can_record_and_only_publishes_after_submit() {
    let (device, queue, _) = crate::photographic::test_support::render_context();
    let context = Context::new(&device, CachePolicy::default());
    let scope = context.materializations();
    let reads = Arc::new(AtomicUsize::new(0));
    let source = source(Arc::new(Plane(reads.clone())));
    let region = [4, 8, 8, 8].into();
    let request = |image: Value| {
        let grid = image.domain().grid;
        context
            .request(
                image,
                Region {
                    bounds: region,
                    grid,
                },
                Tiling { tile: [8, 8] },
            )
            .unwrap()
    };
    let _source = pollster::block_on(
        context
            .materialize(
                &queue,
                &scope,
                request(source.clone()).without_retention(),
                ImageFormat::Rgba32Float,
                ImageBacking::Buffer,
            )
            .unwrap(),
    )
    .unwrap();
    let before = reads.load(Ordering::Relaxed);
    assert!(before > 0);

    let reusable = scale_plane(source, 2.).for_reuse();
    let output = reusable.output(region).unwrap();
    let key = output.identity().unwrap();
    let root = request(scale_plane(reusable, 3.)).without_retention();
    for submit in [false, true] {
        let prepared = context.prepare_image(root.clone()).unwrap();
        assert!(
            prepared.can_record(),
            "resident inputs can use the drawing path"
        );
        let recorded = prepared.record(&context).unwrap();
        let commands = recorded.commands.as_ref().unwrap();
        let mut encoder = device.create_command_encoder(&Default::default());
        commands
            .encode_result(&device, &mut encoder, &recorded.pixels)
            .unwrap();
        assert!(scope.ready::<ImageLayout>(key).is_empty());
        if submit {
            queue.submit([encoder.finish()]);
            commands.submitted();
            let pixels =
                super::super::super::super::execute::read(&device, &queue, &recorded.pixels);
            assert_eq!(pixels[0], [-15., 24., -12., 8.]);
            assert!(
                scope
                    .ready::<ImageLayout>(key)
                    .iter()
                    .any(|image| image.descriptor.region == region)
            );
        }
    }
    assert_eq!(reads.load(Ordering::Relaxed), before);
}

#[test]
fn worker_retains_a_declared_reusable_single_unit() {
    check_projection_reuse([16, 16], [2., 2.]);
}

#[test]
fn worker_assembles_a_declared_reusable_tiled_read() {
    check_projection_reuse([320, 16], [0.125, 2.]);
}

fn check_projection_reuse(extent: [u32; 2], pitch: [f64; 2]) {
    let (device, queue, _) = crate::photographic::test_support::render_context();
    let context = Context::new(
        &device,
        CachePolicy {
            generated_bytes: 1 << 20,
        },
    );
    let store = context.materializations();
    store.set_retention_enabled(true);
    let reads = Arc::new(AtomicUsize::new(0));
    let input = source(Arc::new(Plane(reads.clone())));
    let domain = Domain {
        bounds: Some(CellRegion::new([0, 0], extent)),
        grid: Grid {
            coverage: 8,
            ..Grid::axis_aligned([0.; 2], pitch)
        },
    };
    let projection = reconstruct_tiled(input, domain).unwrap();
    let projected = projection.clone().for_reuse();
    let region = domain.finite().unwrap();
    assert_eq!(
        projection.output(region).unwrap().identity(),
        projected.output(region).unwrap().identity()
    );
    let scale = |factor| scale_plane(projected.clone(), factor);
    let request = |root| {
        context
            .request(
                root,
                Region {
                    bounds: domain.finite().unwrap(),
                    grid: domain.grid,
                },
                Tiling { tile: extent },
            )
            .unwrap()
            .without_retention()
    };
    let work = context.prepare_image(request(scale(2.))).unwrap();
    assert_eq!(
        reads.load(Ordering::Relaxed),
        0,
        "preparation only describes source demand"
    );
    assert!(!work.can_record());

    let image = pollster::block_on(
        context
            .produce_images(&queue, vec![work], EvaluationRun::default())
            .unwrap()
            .remove(0),
    )
    .unwrap();
    let pixels =
        super::super::super::super::execute::read(&device, &queue, image.as_buffer().unwrap());
    let width = extent[0] as usize;
    for (index, pixel) in pixels.iter().enumerate() {
        let x = (index % width) as f32 * pitch[0] as f32;
        let x = if pitch[0] < 1. { x.floor() } else { x + 0.5 };
        let y = (index / width) as f32 * 2. + 0.5;
        let expected = [2. * (x / 8. - 3.), 2. * (y / 4. + 2.), -4., 8.];
        for (actual, expected) in pixel.iter().zip(expected) {
            assert!(
                (*actual - expected).abs() < 2e-5,
                "area-integrated plane: {pixel:?}"
            );
        }
    }
    let key = projected.output(domain.finite().unwrap()).unwrap();
    let retained = store
        .ready::<ImageLayout>(key.identity().unwrap())
        .into_iter()
        .find(|image| image.descriptor.region == domain.finite().unwrap())
        .expect("exact projection ready for the requested window");
    let projection = Arc::downgrade(&retained.image);
    drop(retained);

    let before = reads.load(Ordering::Relaxed);
    assert!(before > 0);
    let next = context.prepare_image(request(scale(3.))).unwrap();
    assert!(next.can_record());
    {
        let recorded = next.record(&context).unwrap();
        let cached = projection.upgrade().unwrap();
        assert!(
            recorded
                .commands
                .as_ref()
                .unwrap()
                .image_inputs()
                .any(|input| Arc::ptr_eq(input, &cached)),
            "the warm plan binds the exact previously assembled buffer"
        );
    }
    assert_eq!(reads.load(Ordering::Relaxed), before);

    store.evict_optional();
    assert_eq!(context.retained_bytes(), 0);
    assert!(projection.upgrade().is_none());
    assert_eq!(
        super::super::super::super::execute::read(&device, &queue, image.as_buffer().unwrap()),
        pixels,
        "the finished consumer image is still usable"
    );
    let miss = context.prepare_image(request(scale(4.))).unwrap();
    assert!(!miss.can_record());
    let rebuilt = context.produce_prepared(&queue, miss, || false).unwrap();
    let rebuilt =
        super::super::super::super::execute::read(&device, &queue, rebuilt.as_buffer().unwrap());
    assert_eq!(
        rebuilt,
        pixels
            .iter()
            .map(|p| [2. * p[0], 2. * p[1], 2. * p[2], p[3]])
            .collect::<Vec<_>>()
    );
    assert!(reads.load(Ordering::Relaxed) > before);
}
