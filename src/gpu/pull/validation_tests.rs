use super::*;

#[test]
fn display_does_not_hide_invalid_working_pixels_in_either_physical_policy() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    for policy in [Policy::Materialized, Policy::Composed] {
        for body in [
            "return p;",
            "let high=p.rgb*1e30;return vec4f(high*high,p.w);",
            "return vec4f(p.rgb,-2.0);",
        ] {
            let source = source(Arc::new(super::tests::fixture()));
            let input = pointwise(
                source,
                Meaning::Neutral,
                Function {
                    code: format!("fn working_pixel(p:vec4f,position:vec2f)->vec4f {{{body}}}")
                        .into(),
                    symbol: "working_pixel",
                    bindings: vec![],
                },
            );
            let root = display(input, &renderer, crate::rendering::OutputTarget::Srgb).unwrap();
            let plan = Plan::with_policy(
                root.clone(),
                root.domain.bounds,
                Budget {
                    tile: [41, 35],
                    scratch_bytes: 8 << 20,
                },
                policy,
            )
            .unwrap();
            let result = Executor::default()
                .run(&device, &queue, &Default::default(), &plan, || false)
                .unwrap()
                .unwrap();
            let pixels = execute::read(&device, &queue, &result.pixels);
            assert!(pixels.iter().flatten().all(|v| v.is_finite()));
            if body == "return p;" {
                assert!(
                    pixels.iter().all(|p| p[3] >= 0.),
                    "valid input is a control for failure propagation"
                );
                assert!(pixels.iter().any(|p| p[0] > 0.));
            } else {
                assert!(
                    pixels.iter().all(|p| p[3] < 0.),
                    "display clipping must not turn an invalid working value into valid output"
                );
            }
        }
    }
}
