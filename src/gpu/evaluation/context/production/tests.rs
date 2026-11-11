use super::*;
use std::sync::Mutex;

struct Candidate {
    invalid: bool,
    described: Arc<Mutex<Vec<CellRegion>>>,
}
impl Operation for Candidate {
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(("admission candidate", self.invalid)))
    }
    fn calculation(
        self: Arc<Self>,
        _: &[Value],
        domain: Domain,
        region: CellRegion,
        _: Budget,
    ) -> Result<Option<value::Calculation>, Error> {
        self.described.lock().unwrap().push(region);
        if self.invalid {
            return Err(Error::InvalidParameter(
                "evaluation resource exceeds GPU limits",
            ));
        }
        value::Calculation::new(
            domain,
            region,
            vec![],
            value::Operation::new(
                "constant candidate",
                (),
                "fn pixel(p:vec2i,at:vec2f)->vec4f{return vec4f(2.,-1.,0.25,0.);}",
                "pixel",
                value::Access::Pointwise,
            ),
        )
        .map(Some)
    }
}

#[test]
fn admission_refines_resource_failures_but_never_semantic_failures_with_similar_messages() {
    let (device, _, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let region: CellRegion = [0, 0, 8, 4].into();
    let domain = Domain {
        bounds: Some(region),
        grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        meaning: Meaning::Neutral,
    };
    let budget = Budget {
        tile: [8, 4],
        scratch_bytes: 64 << 10,
    };
    let limits = wgpu::Limits {
        max_buffer_size: 128,
        max_storage_buffer_binding_size: 128,
        ..device.limits()
    };
    for invalid in [false, true] {
        let described = Arc::new(Mutex::new(Vec::new()));
        let root = Value::from_operation(
            domain,
            [],
            Candidate {
                invalid,
                described: described.clone(),
            },
        );
        let evaluation = context
            .request(
                root,
                Region {
                    bounds: region,
                    grid: domain.grid,
                },
                budget,
            )
            .unwrap();
        let plans = admitted_plans(
            evaluation,
            region,
            budget,
            limits.clone(),
            (&|| false, &|| {}),
        )
        .unwrap()
        .collect::<Result<Vec<_>, _>>();
        let seen = described.lock().unwrap();
        assert!(!seen.is_empty());
        if invalid {
            assert!(matches!(
                plans,
                Err(Error::InvalidParameter(
                    "evaluation resource exceeds GPU limits"
                ))
            ));
            assert!(
                seen.iter().all(|r| *r == region),
                "semantic failure was subdivided: {seen:?}"
            );
        } else {
            let plans = plans.unwrap();
            assert!(
                plans.len() > 1,
                "the original512-byte output cannot fit a128-byte device binding"
            );
            assert!(seen.iter().any(|r| *r != region));
            let mut covered = std::collections::HashSet::new();
            for plan in plans {
                Executor::admit(&limits, &plan).unwrap();
                for y in plan.region.y..plan.region.bottom() {
                    for x in plan.region.x..plan.region.right() {
                        assert!(covered.insert([x, y]));
                    }
                }
            }
            assert_eq!(covered.len(), 32);
        }
    }
}

#[test]
fn executor_distinguishes_invalid_dispatch_and_buffer_capability_from_resource_limits() {
    let (device, _, _) = crate::gpu::test_support::render_context();
    let region: CellRegion = [0, 0, 1, 1].into();
    let domain = Domain {
        bounds: Some(region),
        grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        meaning: Meaning::Neutral,
    };
    let mut fragment = Fragment::default();
    let output = fragment.image(domain, region);
    fragment.compute(
        "unused code",
        "unused",
        vec![(0, Binding::Write(output.resource))],
        1,
    );
    let mut plan = Plan::from_work(fragment, output, 64).unwrap();
    Executor::admit(&device.limits(), &plan).unwrap();
    let no_dispatch = wgpu::Limits {
        max_compute_workgroups_per_dimension: 0,
        ..device.limits()
    };
    assert!(matches!(
        Executor::admit(&no_dispatch, &plan),
        Err(Error::WorkingAllowance(
            "evaluation dispatch exceeds GPU limits"
        ))
    ));
    let Work::Compute { groups, .. } = &mut plan.tiles[0].fragment.work[0] else {
        unreachable!()
    };
    groups[0] = 0;
    assert_eq!(
        Executor::admit(&device.limits(), &plan),
        Err(Error::InvalidParameter(
            "evaluation dispatch exceeds GPU limits"
        ))
    );
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 16,
        usage: wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let mut fragment = Fragment::default();
    let input = fragment.resource(Storage::ResidentBuffer(ImageValue::new(buffer)));
    let output = fragment.image(domain, region);
    fragment.compute(
        "unused code",
        "unused",
        vec![
            (0, Binding::Read(input)),
            (1, Binding::Write(output.resource)),
        ],
        1,
    );
    let plan = Plan::from_work(fragment, output, 64).unwrap();
    assert_eq!(
        Executor::admit(&device.limits(), &plan),
        Err(Error::InvalidParameter(
            "evaluation resource exceeds GPU limits"
        ))
    );
}
