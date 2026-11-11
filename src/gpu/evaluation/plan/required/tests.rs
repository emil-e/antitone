use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Anonymous;
impl Operation for Anonymous {}

struct Leaf(u32);
impl Operation for Leaf {
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(("required leaf", self.0)))
    }
    fn calculation(
        self: Arc<Self>,
        _inputs: &[crate::gpu::evaluation::value::Value],
        domain: Domain,
        output: CellRegion,
        _: Budget,
    ) -> Result<Option<crate::gpu::evaluation::value::Calculation>, Error> {
        Ok(Some(value::Calculation::new(
            domain,
            output,
            Vec::new(),
            value::Operation::cpu("required read-proof leaf", self.key(), self),
        )?))
    }
}
impl CpuKernel for Leaf {
    fn run(
        &self,
        _: &[CpuImage],
        _: CellRegion,
        _: &dyn Fn() -> bool,
    ) -> Result<Option<Vec<[f32; 4]>>, Error> {
        panic!("read proofs must never evaluate pixels")
    }
}
struct Reads {
    reads: Vec<Read>,
    calls: Arc<AtomicUsize>,
}
impl Operation for Reads {
    fn calculation(
        self: Arc<Self>,
        inputs: &[Value],
        domain: Domain,
        output: Rect,
        _: Budget,
    ) -> Result<Option<value::Calculation>, Error> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let operands = self
            .reads
            .iter()
            .map(|read| {
                inputs[read.input]
                    .request(read.region)
                    .map(value::Calculation::input)
            })
            .collect::<Result<_, _>>()?;
        Ok(Some(value::Calculation::new(
            domain,
            output,
            operands,
            value::Operation::cpu("required root read proof", (), Arc::new(ReadProof)),
        )?))
    }
}
struct ReadProof;
impl CpuKernel for ReadProof {
    fn run(
        &self,
        _: &[CpuImage],
        _: CellRegion,
        _: &dyn Fn() -> bool,
    ) -> Result<Option<Vec<[f32; 4]>>, Error> {
        panic!("read proofs must not execute pixels")
    }
}
fn domain() -> Domain {
    Domain {
        bounds: Some([0, 0, 16, 1].into()),
        grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        meaning: Meaning::Neutral,
    }
}
fn check(root: Value, targets: Vec<(Value, Rect)>) -> bool {
    let work = targets
        .into_iter()
        .enumerate()
        .map(|(i, (image, region))| Work::Produce {
            output: PlannedImage {
                domain: image.domain(),
                region,
                resource: Resource(i),
            },
            image: image.clone(),
        })
        .collect::<Vec<_>>();
    productions_are_required(
        &root,
        [0, 0, 1, 1].into(),
        &work,
        Planning {
            policy: Policy::Composed,
            scope: None,
            supplied: &[],
            scratch_bytes: 1 << 20,
            tile: [1, 1],
            limits: None,
        },
    )
    .unwrap()
}
#[test]
fn many_hoisted_units_share_one_declared_read_traversal() {
    let calls = Arc::new(AtomicUsize::new(0));
    let inputs = (0..128)
        .map(|i| Value::from_operation(domain(), [], Leaf(i)))
        .collect::<Vec<_>>();
    let targets = inputs
        .iter()
        .cloned()
        .map(|i| (i, [0, 0, 1, 1].into()))
        .collect();
    let reads = (0..128)
        .map(|i| Read::new(i, [0, 0, 1, 1].into()))
        .collect();
    let root = Value::from_operation(
        domain(),
        inputs,
        Reads {
            reads,
            calls: calls.clone(),
        },
    );
    assert!(check(root, targets));
    assert!(
        calls.load(Ordering::Relaxed) <= 2,
        "unit proof must not repeat the same root traversal for each producer"
    );
}
#[test]
fn every_hoisted_unit_must_be_required_not_only_the_first() {
    let a = Value::from_operation(domain(), [], Leaf(1));
    let b = Value::from_operation(domain(), [], Leaf(2));
    let root = Value::from_operation(
        domain(),
        [a.clone(), b.clone()],
        Reads {
            reads: vec![Read::new(0, [0, 0, 1, 1].into())],
            calls: Default::default(),
        },
    );
    assert!(!check(
        root,
        vec![(a, [0, 0, 1, 1].into()), (b, [0, 0, 1, 1].into())]
    ));
}
#[test]
fn repeated_parent_reads_keep_separate_regions_and_equivalent_definitions_match() {
    let actual = Value::from_operation(domain(), [], Leaf(1));
    let equivalent = Value::from_operation(domain(), [], Leaf(1));
    let root = Value::from_operation(
        domain(),
        [actual],
        Reads {
            reads: vec![
                Read::new(0, [0, 0, 1, 1].into()),
                Read::new(0, [9, 0, 1, 1].into()),
            ],
            calls: Default::default(),
        },
    );
    assert!(check(
        root,
        vec![
            (equivalent.clone(), [0, 0, 1, 1].into()),
            (equivalent, [9, 0, 1, 1].into())
        ]
    ));
}

#[test]
fn anonymous_equivalent_wrappers_keep_their_shared_argument_provenance() {
    let shared = Value::from_operation(domain(), [], Anonymous);
    let actual = Value::from_operation(domain(), [shared.clone()], Leaf(1));
    let equivalent = Value::from_operation(domain(), [shared], Leaf(1));
    assert!(actual.identity().is_none() && equivalent.identity().is_none());
    let root = Value::from_operation(
        domain(),
        [actual],
        Reads {
            reads: vec![Read::new(0, [0, 0, 1, 1].into())],
            calls: Default::default(),
        },
    );
    assert!(check(root, vec![(equivalent, [0, 0, 1, 1].into())]));
}

#[test]
fn an_exact_match_does_not_hide_a_required_equivalent_alias() {
    let actual = Value::from_operation(domain(), [], Leaf(1));
    let alias = Value::from_operation(domain(), [], Leaf(1));
    let root = Value::from_operation(
        domain(),
        [actual.clone(), alias.clone()],
        Reads {
            reads: vec![Read::new(0, [0, 0, 1, 1].into())],
            calls: Default::default(),
        },
    );
    assert!(check(
        root,
        vec![(actual, [0, 0, 1, 1].into()), (alias, [0, 0, 1, 1].into())]
    ));
}
