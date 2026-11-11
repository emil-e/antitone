use super::*;

#[test]
fn value_identity_uses_semantic_precision_but_not_requested_coverage() {
    let plate = Plate::new();
    let region = plate.domain.finite().unwrap();
    let input = source(Arc::new(plate));
    let gain = 1.5f64;
    let nearby = gain.next_up();
    assert_eq!(gain as f32, nearby as f32);
    let a = affine(input.clone(), gain, 0.).request(region).unwrap();
    let b = affine(input.clone(), nearby, 0.).request(region).unwrap();
    assert_ne!(
        a.identity(),
        b.identity(),
        "uniform packing must not erase mathematical parameter identity"
    );
    let small = affine(input, gain, 0.)
        .request(CellRegion::new([0, 6], [3, 2]))
        .unwrap();
    assert_eq!(a.identity(), small.identity());
    assert_ne!(a.coverage(), small.coverage());
}

#[test]
fn anonymous_output_descriptions_survive_compiler_and_value_retirement() {
    let domain = Plate::new().domain;
    let region = domain.finite().unwrap();
    let input = super::super::super::resident::borrowed_input(domain);
    let input_owner = input.downgrade();
    let group = gaussian(input.clone(), 0.75).unwrap();
    let group_owner = group.downgrade();
    let equivalent = gaussian(input.clone(), 0.75).unwrap();
    let different_input =
        gaussian(super::super::super::resident::borrowed_input(domain), 0.75).unwrap();
    let different_sigma = gaussian(input.clone(), 0.75f64.next_up()).unwrap();
    assert_eq!(0.75f64 as f32, 0.75f64.next_up() as f32);
    let (output, created) = testing::observe(0, || group.output(region).unwrap());
    assert_eq!(created, 0);
    assert!(
        output.identity().is_none(),
        "an unknown physical argument cannot gain a retained identity"
    );
    let value = group.request(region).unwrap();
    let lowered = value.output(region).unwrap();
    assert!(
        output.same_value(&lowered),
        "lowering must preserve the complete anonymous definition"
    );
    assert!(output.same_value(&equivalent.output(region).unwrap()));
    assert!(
        !output.same_value(&different_input.output(region).unwrap()),
        "equal layouts do not identify different argument captures"
    );
    assert!(
        !output.same_value(&different_sigma.output(region).unwrap()),
        "packed uniform equality cannot erase mathematical arguments"
    );
    let compiled = value
        .lower(region, budget(region))
        .unwrap()
        .compile(region, budget(region))
        .unwrap();
    drop(compiled);
    drop((
        group,
        equivalent,
        different_input,
        different_sigma,
        input,
        value,
    ));
    assert!(group_owner.upgrade().is_none());
    assert!(
        input_owner.upgrade().is_none(),
        "output descriptions must not retain retired calculation definitions"
    );
    assert!(output.same_value(&lowered));
}

#[test]
fn pointwise_inputs_must_supply_the_declared_output_coordinates() {
    let plate = Plate::new();
    let region = plate.domain.finite().unwrap();
    let domain = plate.domain;
    let input = source(Arc::new(plate))
        .request(CellRegion::new([0, 6], [3, 2]))
        .unwrap();
    let op = Operation::new(
        "identity v1",
        (),
        "fn pixel(p: vec2i, position: vec2f, input: vec4f) -> vec4f { return input; }",
        "pixel",
        Access::Pointwise,
    );
    assert!(matches!(
        Value::new(domain, region, vec![input.clone()], op.clone()),
        Err(Error::InvalidParameter(
            "pointwise input cannot supply the output coordinates"
        ))
    ));
    let shifted = Domain {
        grid: Grid::axis_aligned([99., 0.], domain.grid.pitch),
        ..domain
    };
    assert!(matches!(
        Value::new(shifted, input.coverage(), vec![input], op),
        Err(Error::InvalidParameter(
            "pointwise input cannot supply the output coordinates"
        ))
    ));
}

#[test]
fn shader_validation_is_deferred_and_pixel_bodies_keep_their_access_and_dispatch_contract() {
    let plate = Plate::new();
    let reads = plate.reads.clone();
    let area = plate.domain.finite().unwrap();
    let input = source(Arc::new(plate));
    for body in [
        "fn pixel(p: vec2i, position: vec2f, input: vec4f) -> vec4f { return input_0(p + vec2i(1,0)); }",
        "@group(0) @binding(0) var<storage,read> hidden: array<vec4f>; fn pixel(p:vec2i,position:vec2f,input:vec4f)->vec4f{return hidden[0];}",
        "fn pixel(p:vec2i,position:vec2f,input:vec4f)->vec4f{return input;} @compute @workgroup_size(1) fn private_dispatch(){}",
    ] {
        let node = pointwise(
            input.clone(),
            Meaning::Graded,
            Operation::new(
                "invalid authored body",
                (),
                body,
                "pixel",
                Access::Pointwise,
            ),
        )
        .unwrap();
        let value = node.request(area).unwrap();
        assert_eq!(
            reads.load(Ordering::Relaxed),
            0,
            "planning is not shader validation or source acquisition"
        );
        let result = Plan::with_policy(value, area, budget(area), Policy::Composed);
        assert!(
            result.is_err(),
            "invalid input access/storage escaped the pixel-body contract"
        );
        assert_eq!(reads.load(Ordering::Relaxed), 0);
    }
}

#[test]
fn one_pointwise_body_can_consume_multiple_identified_inputs() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let plate = Plate::new();
    let area = plate.domain.finite().unwrap();
    let domain = plate.domain;
    let left = source(Arc::new(plate.clone())).request(area).unwrap();
    let right = affine(source(Arc::new(plate.clone())), 2., 1.)
        .request(area)
        .unwrap();
    let body = "fn pixel(p:vec2i,position:vec2f,base:vec4f,other:vec4f)->vec4f { return vec4f(base.rgb*0.25 + other.rgb*0.75,base.w); }";
    let value = Value::new(
        domain,
        area,
        vec![left, right],
        Operation::new("blend v1", (), body, "pixel", Access::Pointwise),
    )
    .unwrap();
    for policy in [Policy::Composed, Policy::Materialized] {
        let (actual, _) = run(&device, &queue, &value, policy);
        assert_eq!(actual.len(), (area.width * area.height) as usize);
        for (i, out) in actual.iter().enumerate() {
            let p = [
                area.x + (i % area.width as usize) as i32,
                area.y + (i / area.width as usize) as i32,
            ];
            let base = plate.pixel(p);
            let x = domain.point([p[0] as f64 + 0.5, p[1] as f64 + 0.5])[0];
            for c in 0..3 {
                let expected = 1.75 * f64::from(base[c]) + 0.75 + 0.0075 * x;
                assert!(
                    (f64::from(out[c]) - expected).abs() < 2e-5,
                    "{p:?}: {out:?}, expected {expected}"
                );
            }
            assert_eq!(out[3].to_bits(), base[3].to_bits());
        }
    }
}

#[test]
fn unnamed_calculations_keep_actual_capture_equivalence_without_shader_byte_identity() {
    let plate = Plate::new();
    let input = source(Arc::new(plate.clone()));
    let function = Function::new(
        "fn unchanged(p:vec4f,position:vec2f)->vec4f{return p;}",
        "unchanged",
    );
    let operation = Operation::shader_function(None, function.clone());
    let root = pointwise(input.clone(), Meaning::Graded, operation.clone()).unwrap();
    let lowered = root.request(root.domain().finite().unwrap()).unwrap();
    assert!(root.identity().is_none() && lowered.identity().is_none());
    assert!(root.same_definition(&lowered));
    assert!(
        root.same_definition(
            &pointwise(input.clone(), Meaning::Graded, operation.clone()).unwrap()
        )
    );
    assert!(
        !root.same_definition(
            &pointwise(
                input,
                Meaning::Graded,
                Operation::shader_function(None, function)
            )
            .unwrap()
        ),
        "equal shader bytes are not a semantic identity"
    );
    assert!(
        !root.same_definition(
            &pointwise(source(Arc::new(Plate::new())), Meaning::Graded, operation).unwrap()
        ),
        "shared mathematical bodies must still compare their ordered upstream values"
    );
    assert_eq!(plate.reads.load(Ordering::Relaxed), 0);
}

#[test]
fn requested_source_and_argument_values_preserve_capabilities_without_inventing_pixel_identities() {
    let plate = Plate::new();
    let image = source(Arc::new(plate.clone()));
    let region = CellRegion::new([-5, 3], [4, 3]);
    let value = image.request(region).unwrap();
    assert_eq!(value.identity(), image.identity());
    assert!(value.source().is_some());
    assert!(value.inputs().is_empty());
    assert!(image.same_definition(&value));
    let weak = image.downgrade();
    drop(image);
    let upgraded = weak
        .upgrade()
        .expect("requested source retains its immutable definition");
    assert!(upgraded.same_capture(&value));
    drop((upgraded, value));
    assert!(weak.upgrade().is_none());
    assert_eq!(plate.reads.load(Ordering::Relaxed), 0);

    let argument = borrowed_input(plate.domain);
    let first = argument.request(region).unwrap();
    let second = argument.request([-3, 5, 2, 1].into()).unwrap();
    assert!(first.identity().is_none() && second.identity().is_none());
    assert!(first.inputs().is_empty());
    assert!(first.same_definition(&second));
    assert!(first.same_definition(&argument));
    assert!(
        !first.same_definition(&borrowed_input(plate.domain)),
        "equal layouts cannot identify independent unknown external buffers"
    );
    let weak = first.downgrade();
    drop(argument);
    let upgraded = weak.upgrade().unwrap();
    assert_eq!(
        upgraded.coverage(),
        region,
        "weak handles preserve their demand metadata"
    );
    assert!(upgraded.same_capture(&second));
    drop((upgraded, first, second));
    assert!(
        weak.upgrade().is_none(),
        "weak demand metadata cannot retain the definition"
    );
}
