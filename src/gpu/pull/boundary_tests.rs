use super::tests::{fixture, make_plan};
use super::*;
use crate::geometry::SourceRegion as Rect;

#[test]
fn comparison_helpers_reject_nan_metadata_and_truncated_output() {
    let expected = [glam::DVec3::ONE];
    assert_eq!(
        super::tests::checked_error(&expected, &[[1., 1., 1., 0.]]),
        Ok(0.)
    );
    for invalid in [
        [f32::NAN, 1., 1., 0.],
        [1., 1., 1., f32::NAN],
        [f32::INFINITY, 1., 1., 0.],
    ] {
        assert!(super::tests::checked_error(&expected, &[invalid]).is_err());
    }
    assert!(super::tests::checked_error(&expected, &[]).is_err());
    assert!(super::tests::checked_error(&[glam::DVec3::splat(f64::NAN)], &[[1.; 4]]).is_err());
}

#[test]
fn scratch_pool_obeys_a_smaller_subsequent_budget() {
    let (device, queue, _renderer) = crate::gpu::test_support::render_context();
    let node = source(Arc::new(fixture()));
    let large = make_plan(node.clone(), [41, 35]);
    let mut executor = execute::Executor::default();
    let gate = crate::gpu::SubmissionGate::default();
    let result = executor
        .run(&device, &queue, &gate, &large, || false)
        .unwrap()
        .unwrap();
    assert!(executor.pool_bytes() > 16);
    drop(result);
    let region = Rect {
        width: 1,
        height: 1,
        ..node.domain.bounds
    };
    let small = Plan::new(
        node,
        region,
        Budget {
            tile: [1, 1],
            scratch_bytes: 16,
        },
    )
    .unwrap();
    let result = executor
        .run(&device, &queue, &gate, &small, || false)
        .unwrap()
        .unwrap();
    assert_eq!(
        executor.pool_bytes(),
        16,
        "retained capacity, not only planned size"
    );
    assert_eq!(result.stats.scratch_bytes, 16);
}

#[test]
fn dispatch_grid_limits_are_admitted_without_source_acquisition() {
    struct NeverRead;
    impl Source for NeverRead {
        fn domain(&self) -> Domain {
            Domain {
                bounds: [0, 0, 2048, 2048].into(),
                grid: Grid::axis_aligned([0.; 2], [1.; 2]),
                meaning: Meaning::Neutral,
            }
        }
        fn read(&self, _: Rect) -> Result<Vec<[f32; 4]>, Error> {
            panic!("admission must precede acquisition")
        }
    }
    let node = pointwise(
        source(Arc::new(NeverRead)),
        Meaning::Neutral,
        Function {
            code: "fn identity(p: vec4f, xy: vec2f) -> vec4f { return p; }".into(),
            symbol: "identity",
            bindings: vec![],
        },
    );
    let plan = Plan::new(
        node.clone(),
        node.domain.bounds,
        Budget {
            tile: [2048, 2048],
            scratch_bytes: 256 << 20,
        },
    )
    .unwrap();
    let limits = wgpu::Limits {
        max_compute_workgroups_per_dimension: 1,
        ..Default::default()
    };
    let error = execute::Executor::admit(&limits, &plan).unwrap_err();
    assert!(error.to_string().contains("dispatch"));
}

#[test]
fn operation_constants_must_fit_their_gpu_representation() {
    let (device, _queue, _renderer) = crate::gpu::test_support::render_context();
    let camera = source(Arc::new(fixture()));
    assert!(
        interpret(
            &device,
            camera.clone(),
            glam::DMat3::from_diagonal(glam::DVec3::splat(1e100))
        )
        .is_err()
    );
    let graded = pointwise(
        camera,
        Meaning::Graded,
        Function {
            code: "fn identity(p: vec4f, xy: vec2f)->vec4f {return p;}".into(),
            symbol: "identity",
            bindings: vec![],
        },
    );
    assert!(gaussian(graded.clone(), 1e100).is_err());
    assert!(gaussian(graded, 1e20).is_err());
}

#[test]
fn reconstruction_uses_actual_constant_edges_and_handles_nonunit_near_integer_grids() {
    #[derive(Clone)]
    struct Constant(Domain);
    impl Source for Constant {
        fn domain(&self) -> Domain {
            self.0
        }
        fn read(&self, r: Rect) -> Result<Vec<[f32; 4]>, Error> {
            assert!(self.0.bounds.contains_region(r));
            Ok(vec![[1., -0.25, 3., 0.]; (r.width * r.height) as usize])
        }
    }
    let (device, queue, _renderer) = crate::gpu::test_support::render_context();
    for (origin, pitch) in [([0., 0.], [1., 1.]), ([11.1, 17.3], [0.3, 0.7])] {
        let source_domain = Domain {
            bounds: [0, 0, 7, 5].into(),
            grid: Grid::axis_aligned(origin, pitch),
            meaning: Meaning::Neutral,
        };
        for offset in [-20., -0.5, 0., 6.5, 20.] {
            let output = Domain {
                bounds: [0, 0, 7, 5].into(),
                grid: Grid::axis_aligned(
                    [origin[0] + offset * pitch[0], origin[1] - 0.5 * pitch[1]],
                    pitch,
                ),
                meaning: Meaning::Neutral,
            };
            let root = reconstruct(source(Arc::new(Constant(source_domain))), output).unwrap();
            let plan = make_plan(root, [3, 2]);
            let result = execute::Executor::default()
                .run(&device, &queue, &Default::default(), &plan, || false)
                .unwrap()
                .unwrap();
            let actual = execute::read(&device, &queue, &result.pixels);
            let expected = vec![glam::DVec3::new(1., -0.25, 3.); 35];
            assert!(
                super::tests::max_error(&expected, &actual) < 1e-6,
                "offset={offset} pitch={pitch:?}"
            );
        }
    }
}

#[test]
fn conservative_reconstruction_demand_preserves_nonconstant_nonunit_grid_samples() {
    struct Grid(Domain);
    impl Source for Grid {
        fn domain(&self) -> Domain {
            self.0
        }
        fn read(&self, region: Rect) -> Result<Vec<[f32; 4]>, Error> {
            assert!(self.0.bounds.contains_region(region));
            Ok((region.y..region.y + region.height)
                .flat_map(|y| {
                    (region.x..region.x + region.width).map(move |x| {
                        [
                            (x % 7) as f32 * 0.3 - 0.2,
                            (y % 5) as f32 * 0.4,
                            (x + y) as f32 * 0.02,
                            0.,
                        ]
                    })
                })
                .collect())
        }
    }
    let (device, queue, _renderer) = crate::gpu::test_support::render_context();
    let domain = Domain {
        bounds: [11, 17, 17, 13].into(),
        grid: super::Grid::axis_aligned([11.1, 17.3], [0.3, 0.7]),
        meaning: Meaning::Neutral,
    };
    let data = Grid(domain);
    let samples = data
        .read(domain.bounds)
        .unwrap()
        .iter()
        .map(|p| glam::DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64))
        .collect::<Vec<_>>();
    for shift in [-1e-12, 0., 1e-12] {
        let mut output = super::tests::reduced(domain, [11, 9]);
        output.grid.origin[0] += shift;
        output.grid.origin[1] += shift;
        let expected = super::tests::reconstruct_scalar(&samples, domain, output);
        let root = reconstruct(source(Arc::new(Grid(domain))), output).unwrap();
        let plan = make_plan(root, [4, 3]);
        let result = execute::Executor::default()
            .run(&device, &queue, &Default::default(), &plan, || false)
            .unwrap()
            .unwrap();
        let actual = execute::read(&device, &queue, &result.pixels);
        assert!(
            super::tests::max_error(&expected, &actual) < 2e-5,
            "near-integer shift {shift}"
        );
    }
}
