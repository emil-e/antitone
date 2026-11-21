use super::*;
use crate::gpu::evaluation::numeric_fixture::NumericKey;
use std::sync::atomic::{AtomicUsize, Ordering};

mod prefetch;

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
    let own = match &preparation.work {
        PreparedWork::Gpu(plan) | PreparedWork::Produce(plan) => plan
            .tiles
            .iter()
            .flat_map(|tile| &tile.fragment.work)
            .filter(|work| matches!(work, Work::Compute { .. }))
            .count(),
        _ => 0,
    };
    own + preparation.dependencies.iter().map(computes).sum::<usize>()
}

#[test]
fn overlapping_branch_demands_compute_one_shared_value_with_cold_or_available_inputs() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
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
        let first = value::gaussian(input, 1.).unwrap();
        let expanded = value::gaussian(first.clone(), 1.5).unwrap();
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
        let ordinary = context.prepare_base(evaluation.clone()).unwrap();
        let shared = context.prepare_image(evaluation).unwrap();
        assert_eq!(shared.dependencies.len(), 1);
        assert_eq!(
            computes(&ordinary) - computes(&shared),
            2,
            "two separable blurs need four Gaussian passes, not six"
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
                    .produce_images(&queue, vec![shared])
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

#[test]
fn mtf_and_halation_share_the_upstream_blur_without_changing_signed_light() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let input = source(Arc::new(Plane(Arc::new(AtomicUsize::new(0)))));
    let mtf = value::gaussian(input, 1.).unwrap();
    let root = halation(
        mtf,
        crate::halation::HalationParameters {
            sigma: 1.5,
            strength: 0.5,
        },
    )
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
    let ordinary = context.prepare_base(evaluation.clone()).unwrap();
    let shared = context.prepare_image(evaluation).unwrap();
    assert_eq!(shared.dependencies.len(), 1);
    assert_eq!(computes(&ordinary) - computes(&shared), 2);
    assert_eq!(
        computes(&shared),
        5,
        "four Gaussian passes and signed recombination"
    );
    let image = pollster::block_on(
        context
            .produce_images(&queue, vec![shared])
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
    let (device, queue, _) = crate::gpu::test_support::render_context();
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
    let second = value::gaussian(first.clone(), 1.5).unwrap();
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
    assert!(
        !work.dependencies.is_empty(),
        "the shared value must actually be prepared"
    );
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
fn abandoned_recording_does_not_publish_shared_dependency_pixels() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
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
    let first = value::gaussian(input, 1.).unwrap();
    let second = value::gaussian(first.clone(), 1.5).unwrap();
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
    let dependency = &prepared.dependencies[0].evaluation;
    let identity = dependency
        .job
        .image()
        .output(dependency.job.region())
        .unwrap()
        .identity()
        .unwrap()
        .clone();
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
    recorded.commands.as_ref().unwrap().submitted();
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    assert!(
        context
            .materializations()
            .find_ready::<ImageLayout>(&identity, |_| true)
            .is_some()
    );
}

#[test]
fn missing_projected_inputs_are_fulfilled_before_consumers_and_remain_reusable() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let reads = Arc::new(AtomicUsize::new(0));
    let input = source(Arc::new(Plane(reads.clone())));
    let domain = Domain {
        bounds: Some([0, 0, 16, 16].into()),
        grid: Grid {
            coverage: 8,
            ..Grid::axis_aligned([0.; 2], [2.; 2])
        },
    };
    let projected = reconstruct_tiled(input, domain).unwrap();
    let scale = |factor: f32| {
        value::pointwise(projected.clone(), value::Operation::new(
        "projected plane gain", NumericKey(factor),
        "struct Parameters { factor:vec4f } fn pixel(p:vec2i,position:vec2f,a:vec4f,v:Parameters)->vec4f {return vec4f(a.rgb*v.factor.x,a.w);}",
        "pixel", value::Access::Pointwise,
    ).with_uniform("Parameters", &[factor, 0., 0., 0.])).unwrap()
    };
    let request = |root| {
        context
            .request(
                root,
                Region {
                    bounds: domain.finite().unwrap(),
                    grid: domain.grid,
                },
                Tiling { tile: [16, 16] },
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
    assert_eq!(work.dependencies.len(), 1);
    assert!(!work.can_record());
    let PreparedWork::Gpu(plan) = &work.work else {
        panic!("the consumer is prepared against its explicit projected input");
    };
    assert!(
        !plan
            .tiles
            .iter()
            .flat_map(|tile| &tile.fragment.work)
            .any(|work| matches!(work, Work::Upload { .. } | Work::Produce { .. }))
    );
    let image = pollster::block_on(
        context
            .produce_images(&queue, vec![work])
            .unwrap()
            .remove(0),
    )
    .unwrap();
    let pixels =
        super::super::super::super::execute::read(&device, &queue, image.as_buffer().unwrap());
    for (index, pixel) in pixels.iter().enumerate() {
        let x = (index % 16) as f32 * 2. + 0.5;
        let y = (index / 16) as f32 * 2. + 0.5;
        let expected = [2. * (x / 8. - 3.), 2. * (y / 4. + 2.), -4., 8.];
        for (actual, expected) in pixel.iter().zip(expected) {
            assert!(
                (*actual - expected).abs() < 2e-5,
                "area-integrated plane: {pixel:?}"
            );
        }
    }
    let before = reads.load(Ordering::Relaxed);
    assert!(before > 0);
    let next = context.prepare_image(request(scale(3.))).unwrap();
    assert!(
        next.dependencies.is_empty(),
        "a completed projection does not become another source job"
    );
    assert!(next.can_record());
    next.record(&context).unwrap();
    assert_eq!(reads.load(Ordering::Relaxed), before);
    assert_eq!(context.retained_bytes(), 0);
}

#[test]
fn regional_sharing_does_not_fill_holes_between_requested_rectangles() {
    let a = CellRegion::new([0, 0], [4, 4]);
    assert_eq!(joined(a, [1, 1, 2, 2].into()), Some(a));
    assert_eq!(joined(a, [4, 0, 4, 4].into()), Some([0, 0, 8, 4].into()));
    assert_eq!(joined(a, [2, 2, 4, 4].into()), None);
    assert_eq!(joined(a, [8, 0, 4, 4].into()), None);
}
