use super::tests::{fixture, make_plan, make_plan_with_policy};
use super::*;
use crate::gpu::photograph::test_support::{matrix, settings};
use crate::{gpu::evaluation::CellRegion as Rect, gpu::grading::Grading};

#[test]
fn independent_code_reference_can_be_composed_and_replaced_without_planner_changes() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let data = fixture();
    let neutral = interpret(&device, source(Arc::new(data.clone())), matrix()).unwrap();
    let (roll, snapshot) = settings(data.domain().finite().unwrap());
    let graded = grade(
        neutral,
        &device,
        Arc::new(
            Grading::new(
                &device.limits(),
                roll,
                snapshot,
                data.domain().finite().unwrap().source().unwrap(),
            )
            .unwrap(),
        ),
    )
    .unwrap();
    let original = make_plan_with_policy(graded.clone(), [27, 23], Policy::Composed);
    let mut executor = execute::Executor::default();
    let base = executor
        .run(&device, &queue, &Default::default(), &original, || false)
        .unwrap()
        .unwrap();
    let base = execute::read(&device, &queue, &base.pixels);
    for gain in [0.75f32, 1.25] {
        let function = Function {
            code: format!(
                r#"
fn experiment_gain(p: vec4f, position: vec2f) -> vec4f {{
    return vec4f(p.rgb * {gain}, p.w);
}}
"#
            )
            .into(),
            symbol: "experiment_gain",
            bindings: vec![],
        };
        let candidate = make_plan_with_policy(
            value::pointwise(
                graded.clone(),
                Meaning::Graded,
                value::Operation::pointwise_function("gain witness", gain, function),
            )
            .unwrap(),
            [27, 23],
            Policy::Composed,
        );
        assert_eq!(
            candidate.tiles[0].fragment.work.len(),
            original.tiles[0].fragment.work.len(),
            "new compatible shader functions do not create another pass"
        );
        let result = executor
            .run(&device, &queue, &Default::default(), &candidate, || false)
            .unwrap()
            .unwrap();
        let actual = execute::read(&device, &queue, &result.pixels);
        let expected = base
            .iter()
            .map(|p| glam::DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64) * f64::from(gain))
            .collect::<Vec<_>>();
        assert!(super::tests::max_error(&expected, &actual) < 2e-5);
        assert!(base.iter().zip(&actual).all(|(a, b)| a[3] == b[3]));
    }
    drop(renderer);
}

#[test]
fn pure_admission_rejects_insufficient_budget_and_invalid_demands() {
    let data = fixture();
    let root = source(Arc::new(data));
    assert!(
        Plan::new(
            root.clone(),
            root.domain.finite().unwrap(),
            Budget {
                tile: [16, 16],
                scratch_bytes: 1
            }
        )
        .is_err()
    );
    assert!(
        Plan::new(
            root.clone(),
            root.domain.finite().unwrap(),
            Budget {
                tile: [0, 16],
                scratch_bytes: 1 << 20
            }
        )
        .is_err()
    );
    let exterior = Rect {
        x: 0,
        y: 0,
        width: 16,
        height: 16,
    };
    assert!(
        Plan::new(
            root,
            exterior,
            Budget {
                tile: [16, 16],
                scratch_bytes: 1 << 20
            }
        )
        .is_err()
    );
}

#[test]
fn cancellation_at_submission_does_not_read_or_submit_another_source_tile() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    struct Counted {
        source: super::tests::Authored,
        reads: Arc<AtomicUsize>,
    }

    impl Source for Counted {
        fn domain(&self) -> Domain {
            self.source.domain()
        }

        fn read(&self, region: Rect) -> Result<Vec<[f32; 4]>, Error> {
            self.reads.fetch_add(1, Ordering::Relaxed);
            self.source.read(region)
        }
    }
    let (device, queue, _renderer) = crate::gpu::test_support::render_context();
    let reads = Arc::new(AtomicUsize::new(0));
    let node = source(Arc::new(Counted {
        source: fixture(),
        reads: reads.clone(),
    }));
    let plan = make_plan(node, [16, 16]);
    let gate = crate::gpu::submission::SubmissionGate::default();
    let stop = Arc::new(AtomicBool::new(false));
    let (changed, submissions) = (stop.clone(), Arc::new(AtomicUsize::new(0)));
    let observed = submissions.clone();
    gate.observe_submissions(Arc::new(move || {
        observed.fetch_add(1, Ordering::Relaxed);
        changed.store(true, Ordering::Relaxed);
    }));
    let mut executor = execute::Executor::default();
    assert!(
        executor
            .run(&device, &queue, &gate, &plan, || stop
                .load(Ordering::Relaxed))
            .unwrap()
            .is_none()
    );
    assert_eq!(reads.load(Ordering::Relaxed), 1);
    assert_eq!(submissions.load(Ordering::Relaxed), 1);
    assert!(
        executor
            .run(&device, &queue, &gate, &plan, || true)
            .unwrap()
            .is_none()
    );
    assert_eq!(reads.load(Ordering::Relaxed), 1);
}

#[test]
fn fragment_scheduling_rejects_undeclared_production_before_any_gpu_work() {
    let domain = fixture().domain();
    let region = domain.finite().unwrap();
    let mut fragment = Fragment::default();
    let output = fragment.image(domain, region);
    let uninitialized = fragment.image(domain, region);
    fragment.compute(
        "unused shader code",
        "main",
        vec![
            (0, Binding::Read(uninitialized.resource)),
            (1, Binding::Write(output.resource)),
        ],
        output.cells(),
    );
    let error = Plan::from_work(fragment, output, 1 << 20).err().unwrap();
    assert!(error.to_string().contains("uninitialized"));
}
