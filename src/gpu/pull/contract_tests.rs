use super::tests::{fixture, make_plan, matrix, settings};
use super::*;
use crate::{geometry::SourceRegion as Rect, gpu::GpuGrade};

#[test]
fn independent_code_reference_can_be_composed_and_replaced_without_planner_changes() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let data = fixture();
    let neutral = interpret(&device, source(Arc::new(data.clone())), matrix()).unwrap();
    let (roll, snapshot) = settings(data.domain().bounds);
    let graded = grade(
        neutral,
        &device,
        Arc::new(GpuGrade::new(&device, roll, snapshot, data.domain().bounds).unwrap()),
    )
    .unwrap();
    let original = make_plan(graded.clone(), [27, 23]);
    let mut executor = execute::Executor::default();
    let base = executor
        .run(&device, &queue, &Default::default(), &original, || false)
        .unwrap()
        .unwrap();
    let base = execute::read(&device, &queue, &base.pixels);
    for gain in [0.75f32, 1.25] {
        let function = Function {
            code: format!("fn experiment_gain(p: vec4f, position: vec2f) -> vec4f {{ return vec4f(p.rgb * {gain}, p.w); }}").into(),
            symbol: "experiment_gain",
            bindings: vec![],
        };
        let candidate = make_plan(
            pointwise(graded.clone(), Meaning::Graded, function),
            [27, 23],
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
            root.domain.bounds,
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
            root.domain.bounds,
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
    let gate = crate::gpu::SubmissionGate::default();
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
    struct Broken;
    impl Operation for Broken {
        fn input(&self, r: Rect) -> Result<Option<Rect>, Error> {
            Ok(Some(r))
        }
        fn lower(
            &self,
            f: &mut Fragment,
            _input: Option<Image>,
            output: Image,
        ) -> Result<(), Error> {
            let uninitialized = f.image(output.domain, output.region);
            f.compute(
                "unused shader code",
                "main",
                vec![
                    (0, Binding::Read(uninitialized.resource)),
                    (1, Binding::Write(output.resource)),
                ],
                output.cells(),
            );
            Ok(())
        }
    }
    let node = source(Arc::new(fixture()));
    let root = Layer::new(node.domain, Some(node), Broken);
    let error = Plan::new(
        root.clone(),
        root.domain.bounds,
        Budget {
            tile: [16, 16],
            scratch_bytes: 1 << 20,
        },
    )
    .err()
    .unwrap();
    assert!(error.to_string().contains("uninitialized"));
}
