use super::*;

#[test]
fn immutable_table_and_uniform_arguments_stay_private_in_fused_and_stored_pixel_bodies() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let plate = Plate::new();
    let region = plate.domain.finite().unwrap();
    let body = "struct Parameters{offset:vec4f} fn pixel(p:vec2i,position:vec2f,input:vec4f,parameters:Parameters)->vec4f{let row=table_0(0u);return vec4f(input.rgb*f32(row.x)+f32(row.y)+parameters.offset.x,input.w);}";
    let layer = |input: Value, gain: u32, bias: u32, offset: f32| {
        pointwise(
            input,
            Meaning::Graded,
            Operation::new(
                "table affine witness v1",
                (gain, bias, offset),
                body,
                "pixel",
                Access::Pointwise,
            )
            .with_table(vec![[gain, bias, 0, 0]])
            .with_uniform("Parameters", &[offset, 0., 0., 0.]),
        )
        .unwrap()
    };
    let value = layer(
        layer(source(Arc::new(plate.clone())), 2, 1, 3.5),
        3,
        2,
        -1.25,
    )
    .request(region)
    .unwrap();
    for policy in [Policy::Composed, Policy::Materialized] {
        let (pixels, _) = run(&device, &queue, &value, policy);
        assert_eq!(pixels.len(), (region.width * region.height) as usize);
        for (i, pixel) in pixels.iter().enumerate() {
            let base = plate.pixel([
                region.x + (i % region.width as usize) as i32,
                region.y + (i / region.width as usize) as i32,
            ]);
            for c in 0..3 {
                assert_eq!(pixel[c], 6. * base[c] + 14.25);
            }
            assert_eq!(pixel[3].to_bits(), base[3].to_bits());
        }
    }
}

#[test]
fn captured_library_resources_link_privately_and_unused_exports_do_not_reach_execution() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let plate = Plate::new();
    let region = plate.domain.finite().unwrap();
    let layer = |input: Value, gain: f32, bias: f32| {
        let code = "@group(0) @binding(0) var<uniform> gain:vec4f; @group(0) @binding(1) var<storage,read> bias:array<vec4f>; @group(0) @binding(7) var<storage,read> unused:array<vec4f>; fn unused_library_witness()->vec4f{return unused[0];} fn pixel(cell:vec2i,position:vec2f,value:vec4f)->vec4f {return vec4f(value.rgb*gain.x+bias[0].x,value.w);}";
        let bindings = Function::new(code, "pixel")
            .with_uniform(0, &[gain, 0., 0., 0.])
            .with_storage(1, bytemuck::cast_slice(&[[bias, 0., 0., 0.]]).to_vec())
            .bindings;
        pointwise(
            input,
            Meaning::Graded,
            Operation::new(
                "captured affine library witness",
                (gain, bias),
                code,
                "pixel",
                Access::Pointwise,
            )
            .with_bindings(bindings),
        )
        .unwrap()
    };
    let root = layer(layer(source(Arc::new(plate.clone())), 2., 1.5), 3., -0.5);
    let value = root.request(region).unwrap();
    for policy in [Policy::Composed, Policy::Materialized] {
        let plan = Plan::with_policy(value.clone(), region, budget(region), policy).unwrap();
        let codes = plan
            .tiles
            .iter()
            .flat_map(|tile| &tile.fragment.work)
            .filter_map(|work| {
                if let Work::Compute { code, .. } = work {
                    Some(code)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert!(!codes.is_empty());
        assert!(
            codes
                .iter()
                .all(|code| !code.contains("unused_library_witness")),
            "unreachable library exports must not be carried through every stage"
        );
        let (actual, _) = run(&device, &queue, &value, policy);
        assert_eq!(actual.len(), (region.width * region.height) as usize);
        for (i, pixel) in actual.iter().enumerate() {
            let p = [
                region.x + (i % region.width as usize) as i32,
                region.y + (i / region.width as usize) as i32,
            ];
            let expected = plate.pixel(p);
            for c in 0..3 {
                assert_eq!(pixel[c], 6. * expected[c] + 4.);
            }
            assert_eq!(pixel[3].to_bits(), expected[3].to_bits());
        }
    }
}

#[test]
fn cached_shader_libraries_do_not_capture_arguments_or_requested_coverage() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let plate = Plate::new();
    let code = "@group(0) @binding(0) var<uniform> gain:vec4f; @group(0) @binding(1) var<storage,read> bias:array<vec4f>; fn pixel(cell:vec2i,position:vec2f,value:vec4f)->vec4f{return vec4f(value.rgb*gain.x+bias[0].x,value.w);}";
    for (index, (gain, bias, region)) in [
        (2., 1.5, CellRegion::new([-4, 4], [7, 5])),
        (3., -0.5, CellRegion::new([1, 9], [8, 6])),
        (-0.5, 0.25, CellRegion::new([-4, 4], [7, 5])),
    ]
    .into_iter()
    .enumerate()
    {
        let (evaluation, descriptions) =
            super::super::super::sampling::library_testing::observe(|| {
                let bindings = Function::new(code, "pixel")
                    .with_uniform(0, &[gain, 0., 0., 0.])
                    .with_storage(1, bytemuck::cast_slice(&[[bias, 0., 0., 0.]]).to_vec())
                    .bindings;
                let root = pointwise(
                    source(Arc::new(plate.clone())),
                    Meaning::Graded,
                    Operation::new(
                        "program cache bound arguments witness",
                        (gain, bias),
                        code,
                        "pixel",
                        Access::Pointwise,
                    )
                    .with_bindings(bindings),
                )
                .unwrap();
                let _ = root.request(region).unwrap();
                context
                    .request(
                        root,
                        Region {
                            bounds: region,
                            grid: plate.domain.grid,
                        },
                        budget(region),
                    )
                    .unwrap()
            });
        assert_eq!(
            descriptions,
            (0, 0),
            "capturing a request must not parse or validate a shader"
        );
        let mut delivered = 0;
        let (_, compilation) = super::super::super::sampling::library_testing::observe(|| {
            assert!(
                context
                    .read(
                        &queue,
                        &Default::default(),
                        &evaluation,
                        || false,
                        |block, pixels| {
                            delivered += pixels.len();
                            for (index, pixel) in pixels.iter().enumerate() {
                                let p = [
                                    block.x + (index % block.width as usize) as i32,
                                    block.y + (index / block.width as usize) as i32,
                                ];
                                let original = plate.pixel(p);
                                for channel in 0..3 {
                                    assert_eq!(
                                        pixel[channel],
                                        original[channel] * gain + bias,
                                        "{p:?}"
                                    );
                                }
                                assert_eq!(pixel[3].to_bits(), original[3].to_bits());
                            }
                            Ok(())
                        }
                    )
                    .unwrap()
                    .is_some()
            );
        });
        assert_eq!(delivered, (region.width * region.height) as usize);
        if index == 0 {
            assert!(compilation.0 > 0 && compilation.1 > 0);
        } else {
            assert_eq!(
                compilation,
                (0, 0),
                "new arguments and coverage reuse only the program, not its bindings or pixels"
            );
        }
    }
    let region = plate.domain.finite().unwrap();
    let incomplete = pointwise(
        source(Arc::new(plate.clone())),
        Meaning::Graded,
        Operation::new(
            "incomplete cached capture witness",
            (),
            code,
            "pixel",
            Access::Pointwise,
        )
        .with_bindings(
            Function::new(code, "pixel")
                .with_uniform(0, &[2., 0., 0., 0.])
                .bindings,
        ),
    )
    .unwrap();
    let evaluation = context
        .request(
            incomplete,
            Region {
                bounds: region,
                grid: plate.domain.grid,
            },
            budget(region),
        )
        .unwrap();
    let before = plate.reads.load(Ordering::Relaxed);
    let (result, compilation) = super::super::super::sampling::library_testing::observe(|| {
        evaluation.plans().next().unwrap()
    });
    assert!(
        matches!(
            result,
            Err(Error::InvalidParameter(
                "pixel bodies cannot declare storage or dispatch entry points"
            ))
        ),
        "a warm shader library must still validate this operation's captures"
    );
    assert_eq!(compilation, (0, 0));
    assert_eq!(
        plate.reads.load(Ordering::Relaxed),
        before,
        "invalid captures must reject before source acquisition"
    );
}

#[test]
fn record_results_and_mismatched_input_access_reject_at_the_image_request_boundary() {
    let plate = Plate::new();
    let domain = plate.domain;
    let region = domain.finite().unwrap();
    let input = source(Arc::new(plate)).request(region).unwrap();
    let operation=Operation::new("record boundary witness",(),"fn pixel(p:vec2i,at:vec2f,input:vec4f)->array<vec4f,2>{return array<vec4f,2>(input,vec4f(0.0));}","pixel",Access::Pointwise).with_shape(Shape::Pair);
    assert!(matches!(
        Value::new(
            domain,
            region,
            vec![input.clone()],
            operation
                .clone()
                .with_input_access(vec![Access::Pointwise, Access::Pointwise])
        ),
        Err(Error::InvalidParameter(
            "pixel input access differs from its inputs"
        ))
    ));
    let record = Value::new(domain, region, vec![input], operation).unwrap();
    let (device, _, _) = crate::gpu::test_support::render_context();
    assert!(
        matches!(
            Context::new(&device, CachePolicy { generated_bytes: 0 }).request(
                record,
                Region {
                    bounds: region,
                    grid: domain.grid
                },
                budget(region)
            ),
            Err(Error::InvalidParameter(
                "record values cannot be requested as photographic images"
            ))
        ),
        "private multi-field records must not be misallocated as RGBA images"
    );
}
