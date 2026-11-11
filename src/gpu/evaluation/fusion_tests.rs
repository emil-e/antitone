use super::tests::{chain, fixture, max_error, scalar};
use super::*;
#[cfg(feature = "ocio-reference")]
use crate::rendering::OutputTarget;

#[cfg(feature = "ocio-reference")]
#[test]
fn fused_values_preserve_acquisition_and_pixels_with_lower_transient_storage() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let data = fixture();
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        for physical in [false, true] {
            let root = chain(
                &device,
                &renderer,
                Arc::new(data.clone()),
                physical,
                Some(target),
                0.9,
            );
            let budget = Budget {
                tile: [13, 11],
                scratch_bytes: 32 << 20,
            };
            let baseline = Plan::new(root.clone(), root.domain.finite().unwrap(), budget).unwrap();
            let composed = Plan::with_policy(
                root.clone(),
                root.domain.finite().unwrap(),
                budget,
                Policy::Composed,
            )
            .unwrap();
            assert!(composed.peak_scratch_bytes < baseline.peak_scratch_bytes);
            assert_eq!(baseline.tiles.len(), composed.tiles.len());
            for (before, after) in baseline.tiles.iter().zip(&composed.tiles) {
                let regions = |tile: &Tile| {
                    tile.fragment
                        .work
                        .iter()
                        .filter_map(|work| match work {
                            Work::Upload { output, .. } => Some(output.region),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                };
                assert_eq!(
                    regions(before),
                    regions(after),
                    "fusion must preserve acquisition dependencies"
                );
                assert!(after.fragment.work.len() < before.fragment.work.len());
            }
            let gate = crate::gpu::submission::SubmissionGate::default();
            let before = execute::Executor::default()
                .run(&device, &queue, &gate, &baseline, || false)
                .unwrap()
                .unwrap();
            let before = execute::read(&device, &queue, &before.pixels);
            let after = execute::Executor::default()
                .run(&device, &queue, &gate, &composed, || false)
                .unwrap()
                .unwrap();
            let after = execute::read(&device, &queue, &after.pixels);
            let expected = scalar(&data, physical, Some(target), 0.9);
            let error = max_error(&expected, &after);
            assert!(
                error < 2e-5,
                "independent oracle physical={physical} target={target:?} error={error}"
            );
            let expected_before = before
                .iter()
                .map(|p| glam::DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64))
                .collect::<Vec<_>>();
            assert!(
                max_error(&expected_before, &after) < 2e-5,
                "fallback must preserve the image"
            );
        }
    }
}

#[test]
fn sampling_an_effect_result_is_not_a_mandatory_image_allocation() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let data = fixture();
    let root = chain(&device, &renderer, Arc::new(data.clone()), false, None, 0.9);
    let plan = Plan::with_policy(
        root.clone(),
        root.domain.finite().unwrap(),
        Budget {
            tile: [13, 11],
            scratch_bytes: 32 << 20,
        },
        Policy::Composed,
    )
    .unwrap();
    let result = execute::Executor::default()
        .run(&device, &queue, &Default::default(), &plan, || false)
        .unwrap()
        .unwrap();
    let actual = execute::read(&device, &queue, &result.pixels);
    let expected = scalar(&data, false, None, 0.9);
    assert!(max_error(&expected, &actual) < 2e-5);
}

#[test]
fn composed_pixel_functions_keep_private_resources_and_original_positions() {
    let (device, queue, _renderer) = crate::gpu::test_support::render_context();
    let data = fixture();
    let area = data.domain().finite().unwrap();
    let pixels = data.read(area).unwrap();
    let module = r#"
struct Controls { affine: vec4f }
@group(0) @binding(0) var<uniform> controls: Controls;
const POSITION_GAIN: f32 = 0.01;
fn apply(pixel: vec4f, position: vec2f) -> vec4f {
    return vec4f(pixel.rgb * controls.affine.x + controls.affine.y + position.x * POSITION_GAIN, pixel.w);
}
"#;
    let first = pointwise(
        source(Arc::new(data)),
        Meaning::Graded,
        Function::new(module, "apply").with_uniform(0, &[2.0f32, 1., 0., 0.]),
    );
    let root = pointwise(
        first,
        Meaning::Graded,
        Function::new(module, "apply").with_uniform(0, &[-0.5f32, 0.25, 0., 0.]),
    );
    let budget = Budget {
        tile: [13, 11],
        scratch_bytes: 32 << 20,
    };
    let plans = [Policy::Materialized, Policy::Composed]
        .map(|policy| Plan::with_policy(root.clone(), area, budget, policy).unwrap());
    assert_eq!(
        plans[1].tiles[0]
            .fragment
            .work
            .iter()
            .filter(|work| matches!(work, Work::Compute { .. }))
            .count(),
        1
    );
    for plan in plans {
        let result = execute::Executor::default()
            .run(&device, &queue, &Default::default(), &plan, || false)
            .unwrap()
            .unwrap();
        let actual = execute::read(&device, &queue, &result.pixels);
        for (i, (input, output)) in pixels.iter().zip(actual).enumerate() {
            let x = f64::from(area.x) + (i % area.width as usize) as f64 + 0.5;
            for c in 0..3 {
                // Apply the two authored affine functions analytically, including source position.
                let expected = -f64::from(input[c]) - 0.25 + x * 0.005;
                assert!(
                    (f64::from(output[c]) - expected).abs() < 1e-6,
                    "{i}/{c}: {output:?}"
                );
            }
            assert_eq!(output[3], input[3]);
        }
    }
}

#[test]
fn an_independently_authored_pixel_function_composes_with_sampling() {
    use super::CellRegion as Rect;

    struct Gain;

    impl Operation for Gain {
        fn reads(&self, r: Rect) -> Result<Vec<crate::gpu::evaluation::Read>, Error> {
            Ok(vec![crate::gpu::evaluation::Read::new(0, r)])
        }

        fn calculation(
            self: std::sync::Arc<Self>,
            inputs: &[crate::gpu::evaluation::value::Value],
            domain: Domain,
            output: CellRegion,
            _budget: crate::gpu::evaluation::Budget,
        ) -> Result<Option<crate::gpu::evaluation::value::Calculation>, Error> {
            let inputs = crate::gpu::evaluation::value::Calculation::references(inputs);
            let function = Function {
                code: r#"
fn independent_map(p: vec4f, position: vec2f) -> vec4f {
    return vec4f(p.rgb * 0.75, p.w);
}
"#
                .into(),
                symbol: "independent_map",
                bindings: vec![],
            };
            value::Calculation::new(
                domain,
                output,
                vec![inputs[0].request(output)?],
                value::Operation::shader_function(self.key(), function),
            )
            .map(Some)
        }
    }
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let data = fixture();
    let root = chain(&device, &renderer, Arc::new(data.clone()), false, None, 0.9);
    let input = root.unary_input().unwrap();
    let extra = Value::from_operation(input.domain, Some(input.clone()), Gain);
    let changed = Value::captured(root.domain, Some(extra), root.operation.clone());
    let budget = Budget {
        tile: [13, 11],
        scratch_bytes: 32 << 20,
    };
    let original = Plan::with_policy(
        root.clone(),
        root.domain.finite().unwrap(),
        budget,
        Policy::Composed,
    )
    .unwrap();
    let modified = Plan::with_policy(
        changed.clone(),
        changed.domain.finite().unwrap(),
        budget,
        Policy::Composed,
    )
    .unwrap();
    assert_eq!(
        original.tiles[0].fragment.work.len(),
        modified.tiles[0].fragment.work.len()
    );
    let result = execute::Executor::default()
        .run(&device, &queue, &Default::default(), &modified, || false)
        .unwrap()
        .unwrap();
    let pixels = execute::read(&device, &queue, &result.pixels);
    let expected = scalar(&data, false, None, 0.9)
        .into_iter()
        .map(|p| p * 0.75)
        .collect::<Vec<_>>();
    assert!(max_error(&expected, &pixels) < 2e-5);
}
