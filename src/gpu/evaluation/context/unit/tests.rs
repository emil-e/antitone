use super::*;
mod join;
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

#[test]
fn regional_callable_candidates_do_not_bypass_reusable_unit_boundaries() {
    struct Unread;
    impl Source for Unread {
        fn domain(&self) -> Domain {
            Domain {
                bounds: Some([0, 0, 256, 128].into()),
                grid: Grid::axis_aligned([0.; 2], [1.; 2]),
                meaning: Meaning::Neutral,
            }
        }
        fn identity(&self) -> Option<RetainedKey> {
            Some(RetainedKey::new("regional callable boundary witness"))
        }
        fn read(&self, _: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
            panic!("planning must not acquire source pixels")
        }
    }
    let input = source(Arc::new(Unread));
    let domain = input.domain();
    let support = crate::dust_repair::SupportMask::new(
        [19, 19],
        [3, 3],
        vec![false, false, false, false, true, false, false, false, false],
    )
    .unwrap();
    let repair = crate::dust_repair::Repair {
        id: crate::dust_repair::RepairId(1),
        enabled: true,
        origin: crate::dust_repair::RepairOrigin::Manual,
        support,
        offset: [32, 32],
    };
    let composition = crate::gpu::repairs::definition::compose(input, &[repair]).unwrap();
    let root = crate::gpu::evaluation::reconstruct_tiled(composition, domain).unwrap();
    let full = domain.finite().unwrap();
    let unit = CellRegion::new([0, 0], [128, 128]);
    let budget = Budget {
        tile: [256, 128],
        scratch_bytes: 2 << 20,
    };
    assert!(
        !super::super::super::unit::can_inline(&root, full, budget).unwrap(),
        "a whole-frame callable must not replace independently reusable regional units"
    );
    assert!(
        super::super::super::unit::can_inline(&root, unit, budget).unwrap(),
        "individual regional units must retain the ordinary callable implementation"
    );
    let plan = Plan::with_policy(root.clone(), full, budget, Policy::Composed).unwrap();
    assert!(
        plan.tiles
            .iter()
            .flat_map(|tile| &tile.fragment.work)
            .any(|work| matches!(work, Work::Produce { image, .. } if image.same_capture(&root))),
        "the requested photograph must actually use bounded regional assembly"
    );
}

#[test]
fn self_production_uses_declared_outputs_not_logical_or_lowered_node_pointers() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let input = source(Arc::new(ObservedRamp(events.clone())));
    let make = |gain: f32| {
        value::pointwise(
            input.clone(),
            Meaning::Neutral,
            value::Operation::new(
                "self-production coordinates",
                gain,
                "struct Parameters { gain:vec4f }\nfn pixel(p:vec2i, position:vec2f, input:vec4f, parameters:Parameters)->vec4f { return vec4f(input.rgb*parameters.gain.x,input.w); }",
                "pixel",
                value::Access::Pointwise,
            ).with_uniform("Parameters", &[gain,0.,0.,0.]),
        )
        .unwrap()
    };
    let root = make(2.);
    let region = root.domain().finite().unwrap();
    let calculation = make(2.).request(region).unwrap();
    assert!(
        !root.same_capture(&calculation),
        "the control must use a distinct capture of equivalent mathematics"
    );
    let mut fragment = Fragment::default();
    let output = fragment.image(root.domain(), region);
    fragment.work.push(Work::Produce {
        image: calculation,
        output,
    });
    let plan = Plan::from_work(fragment, output, 32 << 10).unwrap();
    assert!(
        plan.requires_production(&root).unwrap(),
        "an equivalent self-demand must not be delivered as an independent dependency"
    );
    assert!(
        !plan.requires_production(&make(3.)).unwrap(),
        "another calculation on the same layout is not this pending output"
    );
    assert!(events.lock().unwrap().is_empty());
}

#[derive(Clone, Copy, Debug)]
enum Event {
    Rejected(CellRegion),
    Accepted(CellRegion),
    Read(CellRegion),
}
struct ObservedRamp(Arc<Mutex<Vec<Event>>>);
impl Source for ObservedRamp {
    fn domain(&self) -> Domain {
        Domain {
            bounds: Some(CellRegion::new([-3, 5], [7, 5])),
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
            meaning: Meaning::Neutral,
        }
    }
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("description retry ramp"))
    }
    fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        self.0.lock().unwrap().push(Event::Read(region));
        Ok((region.y..region.bottom())
            .flat_map(|y| {
                (region.x..region.right()).map(move |x| [x as f32, y as f32, -0.25, 0.75])
            })
            .collect())
    }
}
struct Candidate {
    max_cells: u64,
    gain: f32,
    invalid: bool,
    events: Arc<Mutex<Vec<Event>>>,
}
impl Operation for Candidate {
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new((
            "retry scaled coordinates",
            self.gain.to_bits(),
            self.invalid,
        )))
    }
    fn calculation(
        self: Arc<Self>,
        inputs: &[crate::gpu::evaluation::value::Value],
        domain: Domain,
        region: CellRegion,
        _: Budget,
    ) -> Result<Option<crate::gpu::evaluation::value::Calculation>, Error> {
        if self.invalid || u64::from(region.width) * u64::from(region.height) > self.max_cells {
            self.events.lock().unwrap().push(Event::Rejected(region));
            return Err(if self.invalid {
                Error::InvalidParameter("invalid calculation witness")
            } else {
                Error::WorkingAllowance("candidate description is too large")
            });
        }
        self.events.lock().unwrap().push(Event::Accepted(region));
        let input = value::Calculation::input(inputs[0].request(region)?);
        let operation=value::Operation::new("retry scaled coordinates",self.gain,
            "struct Parameters { gain:vec4f }\nfn pixel(p:vec2i, position:vec2f, value:vec4f, parameters:Parameters)->vec4f { return vec4f(value.rgb*parameters.gain.x,value.w); }",
            "pixel",value::Access::Pointwise).with_uniform("Parameters",&[self.gain,0.,0.,0.]);
        value::Calculation::new(domain, region, vec![input], operation).map(Some)
    }
}
fn root(events: &Arc<Mutex<Vec<Event>>>, max_cells: u64, gain: f32, invalid: bool) -> Value {
    let input = source(Arc::new(ObservedRamp(events.clone())));
    Value::from_operation(
        input.domain(),
        [input],
        Candidate {
            max_cells,
            gain,
            invalid,
            events: events.clone(),
        },
    )
}
fn query(context: &Context, root: Value) -> Evaluation {
    let domain = root.domain();
    context
        .request(
            root,
            Region {
                bounds: domain.finite().unwrap(),
                grid: domain.grid,
            },
            Budget {
                tile: [7, 5],
                scratch_bytes: 32 << 10,
            },
        )
        .unwrap()
}
fn check_pixels(device: &wgpu::Device, queue: &wgpu::Queue, image: &Image<ImageLayout>, gain: f32) {
    let pixels = super::super::super::execute::read(device, queue, image.as_buffer().unwrap());
    let region = image.descriptor.region;
    assert_eq!(pixels.len(), 35);
    for (index, pixel) in pixels.iter().enumerate() {
        let x = region.x + (index % region.width as usize) as i32;
        let y = region.y + (index / region.width as usize) as i32;
        assert_eq!(
            *pixel,
            [x as f32 * gain, y as f32 * gain, -0.25 * gain, 0.75]
        );
    }
}

#[test]
fn candidate_descriptions_split_before_execution_and_completed_outputs_ignore_partitioning() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let scope = context.test_materializations(false);
    let gate = crate::gpu::submission::SubmissionGate::default();
    let events = Arc::new(Mutex::new(Vec::new()));
    let materialize = |max_cells, gain| {
        pollster::block_on(
            context
                .materialize(
                    &queue,
                    &gate,
                    &scope,
                    query(&context, root(&events, max_cells, gain, false)),
                    ImageFormat::Rgba32Float,
                    ImageBacking::Buffer,
                )
                .unwrap(),
        )
        .unwrap()
    };
    let first = materialize(15, 2.);
    check_pixels(&device, &queue, &first, 2.);
    {
        let events = events.lock().unwrap();
        let accepted: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                Event::Accepted(r) => Some(*r),
                _ => None,
            })
            .collect();
        assert!(
            events
                .iter()
                .any(|event| matches!(event, Event::Rejected(r) if r.width == 7 && r.height == 5)),
            "the control must reject the original candidate"
        );
        assert!(
            accepted.len() >= 3,
            "the output must really execute multiple smaller descriptions"
        );
        assert!(
            accepted
                .iter()
                .all(|r| u64::from(r.width) * u64::from(r.height) <= 15)
        );
        let first_read = events
            .iter()
            .position(|event| matches!(event, Event::Read(_)))
            .unwrap();
        assert!(
            events[..first_read]
                .iter()
                .any(|event| matches!(event, Event::Rejected(_)))
        );
        assert!(
            events[..first_read]
                .iter()
                .any(|event| matches!(event, Event::Accepted(_)))
        );
        for region in events.iter().filter_map(|event| match event {
            Event::Read(r) => Some(r),
            _ => None,
        }) {
            assert!(
                accepted
                    .iter()
                    .any(|candidate| candidate.contains_region(*region)),
                "only accepted descriptions may acquire pixels: {region:?}"
            );
        }
    }
    events.lock().unwrap().clear();
    let second = materialize(35, 2.);
    assert_eq!(first.identity, second.identity);
    assert!(
        Arc::ptr_eq(&first.image, &second.image),
        "a different partition allowance must reuse the actual completed value"
    );
    assert!(
        events.lock().unwrap().is_empty(),
        "reuse must not build descriptions or reacquire pixels"
    );
    check_pixels(&device, &queue, &second, 2.);
    let changed = materialize(35, -3.);
    assert_ne!(
        first.identity, changed.identity,
        "changed mathematics must not alias a completed output"
    );
    check_pixels(&device, &queue, &changed, -3.);
    assert_eq!(context.retained_bytes(), 0);
}

#[test]
fn irreducible_and_semantic_description_errors_do_not_execute() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let scope = context.test_materializations(false);
    let gate = crate::gpu::submission::SubmissionGate::default();
    let submissions = Arc::new(AtomicUsize::new(0));
    let observed = submissions.clone();
    gate.observe_submissions(Arc::new(move || {
        observed.fetch_add(1, Ordering::Relaxed);
    }));
    for invalid in [false, true] {
        let events = Arc::new(Mutex::new(Vec::new()));
        let root = root(&events, 0, 2., invalid);
        let layout = ImageLayout {
            domain: root.domain(),
            region: root.domain().finite().unwrap(),
            format: ImageFormat::Rgba32Float,
            backing: ImageBacking::Buffer,
        };
        let pixels = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: layout.bytes(),
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let working = HostWorkingSet::default();
        let production = Production {
            worker: Worker {
                context: &context,
                queue: &queue,
                gate: &gate,
                scope: &scope,
                budget: Budget {
                    tile: [7, 5],
                    scratch_bytes: 32 << 10,
                },
                working: &working,
                cancelled: &|| false,
                progress: &|| {},
            },
            layout,
            pixels: &pixels,
        };
        let error = production.produce_units(&root).unwrap_err();
        let events = events.lock().unwrap();
        if invalid {
            assert_eq!(
                error,
                Error::InvalidParameter("invalid calculation witness")
            );
            assert!(
                matches!(events.as_slice(), [Event::Rejected(region)] if *region == layout.region),
                "semantic errors must not trigger subdivision: {events:?}"
            );
        } else {
            assert_eq!(
                error,
                Error::WorkingAllowance("candidate description is too large")
            );
            assert!(
                matches!(events.last(), Some(Event::Rejected(region)) if region.width == 1 && region.height == 1)
            );
            assert!(
                events.len() > 1,
                "the control must attempt smaller descriptions before the irreducible error"
            );
        }
        assert!(
            events
                .iter()
                .all(|event| matches!(event, Event::Rejected(_))),
            "failed descriptions must not acquire or execute: {events:?}"
        );
        assert_eq!(submissions.load(Ordering::Relaxed), 0);
    }
}
