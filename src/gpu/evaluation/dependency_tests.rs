use super::*;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
struct Coordinates {
    tag: u32,
    reads: Arc<Mutex<Vec<CellRegion>>>,
}
impl Source for Coordinates {
    fn domain(&self) -> Domain {
        Domain {
            bounds: Some(CellRegion::new([0, 0], [1000, 20])),
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
            meaning: Meaning::Neutral,
        }
    }
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(self.tag))
    }
    fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, crate::Error> {
        self.reads.lock().unwrap().push(region);
        Ok((region.y..region.bottom())
            .flat_map(|y| {
                (region.x..region.right())
                    .map(move |x| [x as f32 + self.tag as f32, y as f32, 0., 0.])
            })
            .collect())
    }
}

struct Pair;
impl Operation for Pair {
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("weighted pair"))
    }
    fn reads(&self, output: CellRegion) -> Result<Vec<Read>, crate::Error> {
        Ok(vec![
            Read::new(0, output),
            Read::new(
                1,
                CellRegion::new(
                    [output.x + 400, output.y + 10],
                    [output.width, output.height],
                ),
            ),
        ])
    }
    fn calculation(
        self: std::sync::Arc<Self>,
        inputs: &[crate::gpu::evaluation::value::Value],
        domain: Domain,
        output: CellRegion,
        _: Budget,
    ) -> Result<Option<crate::gpu::evaluation::value::Calculation>, Error> {
        let inputs = crate::gpu::evaluation::value::Calculation::references(inputs);
        let reads = self.reads(output)?;
        let operands = reads
            .iter()
            .map(|read| inputs[read.input].request(read.region))
            .collect::<Result<Vec<_>, _>>()?;
        let code = r#"
fn pixel(p:vec2i,position:vec2f,a:vec4f)->vec4f {
    return a+2.0*input_1(p+vec2i(400,10));
}
"#;
        let operation =
            value::Operation::new("weighted pair", (), code, "pixel", value::Access::Pointwise)
                .with_input_access(vec![value::Access::Pointwise, value::Access::Neighborhood]);
        value::Calculation::new(domain, output, operands, operation).map(Some)
    }
}

fn assert_read_coverage(reads: &[CellRegion], expected: CellRegion) {
    assert!(
        reads.iter().all(|read| expected.contains_region(*read)),
        "source reads must not include undeclared intervening pixels"
    );
    for y in expected.y..expected.bottom() {
        for x in expected.x..expected.right() {
            let cell = CellRegion::new([x, y], [1, 1]);
            assert!(
                reads.iter().any(|read| read.contains_region(cell)),
                "every declared input cell must be supplied"
            );
        }
    }
}

#[test]
fn declared_disjoint_reads_keep_parent_order_and_do_not_acquire_the_intervening_pixels() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let a_reads = Arc::new(Mutex::new(Vec::new()));
    let b_reads = Arc::new(Mutex::new(Vec::new()));
    let a = source(Arc::new(Coordinates {
        tag: 0,
        reads: a_reads.clone(),
    }));
    let b = source(Arc::new(Coordinates {
        tag: 1000,
        reads: b_reads.clone(),
    }));
    let region = CellRegion::new([2, 3], [8, 4]);
    let domain = Domain {
        bounds: Some(region),
        ..a.domain()
    };
    let root = Value::from_operation(domain, [a.clone(), b.clone()], Pair);
    assert_eq!(
        root.regions_for(
            &a,
            region,
            Budget {
                tile: [8, 4],
                scratch_bytes: 1 << 20
            }
        )
        .unwrap(),
        vec![region]
    );
    assert_eq!(
        root.regions_for(
            &b,
            region,
            Budget {
                tile: [8, 4],
                scratch_bytes: 1 << 20
            }
        )
        .unwrap(),
        vec![CellRegion::new([402, 13], [8, 4])]
    );
    let swapped = Value::from_operation(domain, [b, a], Pair);
    assert!(
        !root.same_definition(&swapped),
        "ordered dependencies affect weighted calculations"
    );
    let request = renderer
        .evaluation()
        .request(
            root,
            Region {
                bounds: region,
                grid: domain.grid,
            },
            Budget {
                tile: [8, 4],
                scratch_bytes: 1 << 20,
            },
        )
        .unwrap();
    let mut pixels = Vec::new();
    renderer
        .evaluation()
        .read(
            &queue,
            &Default::default(),
            &request,
            || false,
            |_, block| {
                pixels.extend_from_slice(block);
                Ok(())
            },
        )
        .unwrap()
        .unwrap();
    assert_read_coverage(&a_reads.lock().unwrap(), region);
    assert_read_coverage(&b_reads.lock().unwrap(), CellRegion::new([402, 13], [8, 4]));
    assert_eq!(pixels.len(), 32);
    for (i, pixel) in pixels.iter().enumerate() {
        let x = 2 + i as u32 % 8;
        let y = 3 + i as u32 / 8;
        assert_eq!(*pixel, [(3 * x + 2800) as f32, (3 * y + 20) as f32, 0., 0.]);
    }
    drop(device);
}

#[test]
fn distinct_source_uploads_survive_nonoverlapping_encoded_lifetimes() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let a_reads = Arc::new(Mutex::new(Vec::new()));
    let b_reads = Arc::new(Mutex::new(Vec::new()));
    let a = source(Arc::new(Coordinates {
        tag: 0,
        reads: a_reads,
    }));
    let b = source(Arc::new(Coordinates {
        tag: 1000,
        reads: b_reads,
    }));
    let a = pointwise(
        a,
        Meaning::Neutral,
        Function::new(
            "fn triple(p: vec4f, position: vec2f) -> vec4f { return vec4f(3. * p.rgb, p.w); }",
            "triple",
        ),
    );
    let region = CellRegion::new([2, 3], [8, 4]);
    let domain = Domain {
        bounds: Some(region),
        ..a.domain()
    };
    let root = Value::from_operation(domain, [a, b], Pair);
    let request = renderer
        .evaluation()
        .request(
            root,
            Region {
                bounds: region,
                grid: domain.grid,
            },
            Budget {
                tile: [8, 4],
                scratch_bytes: 1 << 20,
            },
        )
        .unwrap();
    let mut pixels = Vec::new();
    renderer
        .evaluation()
        .read(
            &queue,
            &Default::default(),
            &request,
            || false,
            |_, block| {
                pixels.extend_from_slice(block);
                Ok(())
            },
        )
        .unwrap()
        .unwrap();
    assert_eq!(pixels.len(), 32);
    for (i, pixel) in pixels.iter().enumerate() {
        let x = 2 + i as u32 % 8;
        let y = 3 + i as u32 / 8;
        assert_eq!(*pixel, [(5 * x + 2800) as f32, (5 * y + 20) as f32, 0., 0.]);
    }
    drop(device);
}

#[test]
fn stage_planning_preserves_a_natural_unit_without_reading_or_running_it() {
    struct Whole {
        region: CellRegion,
    }
    struct UnexpectedKernel;
    impl CpuKernel for UnexpectedKernel {
        fn run(
            &self,
            _: &[CpuImage],
            _: CellRegion,
            _: &dyn Fn() -> bool,
        ) -> Result<Option<Vec<[f32; 4]>>, crate::Error> {
            panic!("describing a work unit must not run its kernel");
        }
    }
    impl Operation for Whole {
        fn reads(&self, _: CellRegion) -> Result<Vec<Read>, crate::Error> {
            Ok(vec![Read::new(0, self.region)])
        }
        fn work_units(&self, _: CellRegion, _: Budget) -> Result<WorkUnits, crate::Error> {
            Ok(Box::new(std::iter::once(self.region)))
        }
        fn calculation(
            self: Arc<Self>,
            inputs: &[crate::gpu::evaluation::value::Value],
            domain: Domain,
            _: CellRegion,
            _: Budget,
        ) -> Result<Option<crate::gpu::evaluation::value::Calculation>, crate::Error> {
            Ok(Some(value::Calculation::new(
                domain,
                self.region,
                vec![value::Calculation::input(inputs[0].request(self.region)?)],
                value::Operation::cpu(
                    "whole CPU dependency proof",
                    self.region,
                    Arc::new(UnexpectedKernel),
                ),
            )?))
        }
    }
    let reads = Arc::new(Mutex::new(Vec::new()));
    let input = source(Arc::new(Coordinates {
        tag: 0,
        reads: reads.clone(),
    }));
    let whole = CellRegion::new([2, 3], [8, 4]);
    let operation = Arc::new(Whole { region: whole });
    let requested = CellRegion::new([3, 4], [1, 1]);
    for tile in [[1, 1], [8, 4]] {
        let budget = Budget {
            tile,
            scratch_bytes: 1 << 20,
        };
        let units = operation
            .work_units(requested, budget)
            .unwrap()
            .collect::<Vec<_>>();
        assert_eq!(units, vec![whole]);
        let plan = operation
            .clone()
            .calculation(
                std::slice::from_ref(&input),
                input.domain(),
                units[0],
                budget,
            )
            .unwrap()
            .unwrap()
            .compile(units[0], budget)
            .unwrap();
        assert_eq!(plan.output, whole);
        let UnitWork::Cpu(plan) = plan.work else {
            panic!("expected CPU work")
        };
        assert_eq!(plan.reads.len(), 1);
        assert_eq!(plan.reads[0].region, whole);
        let node = Value::from_operation(
            Domain {
                bounds: Some(whole),
                ..input.domain()
            },
            [input.clone()],
            Whole { region: whole },
        );
        assert_eq!(
            node.regions_for(&input, requested, budget).unwrap(),
            vec![whole]
        );
        assert!(reads.lock().unwrap().is_empty());
    }
}

#[test]
fn dependency_queries_do_not_lower_gpu_work_or_read_pixels() {
    struct Deferred;
    impl Operation for Deferred {
        fn calculation(
            self: Arc<Self>,
            inputs: &[value::Value],
            domain: Domain,
            output: CellRegion,
            _: Budget,
        ) -> Result<Option<value::Calculation>, Error> {
            Ok(Some(value::Calculation::new(
                domain,
                output,
                vec![value::Calculation::input(inputs[0].request(output)?)],
                value::Operation::new(
                    "uncompiled dependency witness",
                    (),
                    "deliberately invalid WGSL: dependency inspection must not parse this",
                    "unused",
                    value::Access::Neighborhood,
                ),
            )?))
        }
    }
    let observed = Arc::new(Mutex::new(Vec::new()));
    let source = source(Arc::new(Coordinates {
        tag: 0,
        reads: observed.clone(),
    }));
    let region = CellRegion::new([2, 3], [8, 4]);
    let node = Value::from_operation(
        Domain {
            bounds: Some(region),
            ..source.domain()
        },
        [source.clone()],
        Deferred,
    );
    assert_eq!(
        node.regions_for(
            &source,
            region,
            Budget {
                tile: [8, 4],
                scratch_bytes: 1 << 20
            }
        )
        .unwrap(),
        vec![region]
    );
    assert!(observed.lock().unwrap().is_empty());
}

#[test]
fn declared_gpu_phase_lowers_against_its_resident_input_bindings() {
    use wgpu::util::DeviceExt;

    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let source_a = Arc::new(Coordinates {
        tag: 0,
        reads: Arc::new(Mutex::new(Vec::new())),
    });
    let source_b = Arc::new(Coordinates {
        tag: 1000,
        reads: Arc::new(Mutex::new(Vec::new())),
    });
    let region = CellRegion::new([2, 3], [8, 4]);
    let domain = Domain {
        bounds: Some(region),
        ..source_a.domain()
    };
    let budget = Budget {
        tile: [8, 4],
        scratch_bytes: 1 << 20,
    };
    let authored = Value::from_operation(
        domain,
        [source(source_a.clone()), source(source_b.clone())],
        Pair,
    );
    let unit = authored.plan_unit(region, budget).unwrap();
    let UnitWork::Gpu(mut work) = unit.work else {
        panic!("expected GPU work")
    };
    let phase = work.phases.next().unwrap().unwrap();
    assert!(work.phases.next().is_none());
    let mut fragment = Fragment::default();
    let mut buffers = Vec::new();
    let mut slots = Vec::new();
    for (slot, (read, input)) in phase.reads.iter().zip([source_a, source_b]).enumerate() {
        let mut data = input.read(read.region).unwrap();
        // Contrasting metadata also verifies full-vector arithmetic on the supplied bindings.
        for pixel in &mut data {
            pixel[3] = 7. + slot as f32 * 6.;
        }
        buffers.push(
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("declared phase input"),
                contents: bytemuck::cast_slice(&data),
                usage: wgpu::BufferUsages::STORAGE,
            }),
        );
        slots.push(fragment.resource(Storage::Input {
            slot,
            bytes: data.len() as u64 * 16,
            writable: false,
        }));
    }
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("declared phase output"),
        size: 32 * 16,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    slots.push(fragment.resource(Storage::Input {
        slot: 2,
        bytes: output.size(),
        writable: true,
    }));
    let result = phase.kernel.lower(&mut fragment, &slots).unwrap();
    let recorded = renderer.evaluation().record_work(fragment, result).unwrap();
    let mut encoder = device.create_command_encoder(&Default::default());
    recorded
        .encode_work(&device, &mut encoder, &[&buffers[0], &buffers[1], &output])
        .unwrap();
    renderer
        .evaluation()
        .submit_with(
            &queue,
            &Default::default(),
            encoder,
            || false,
            || recorded.submitted(),
        )
        .unwrap()
        .unwrap();
    let pixels = read_buffer(&device, &queue, &output);
    assert_eq!(pixels.len(), 32);
    for (i, pixel) in pixels.iter().enumerate() {
        let x = 2 + i as u32 % 8;
        let y = 3 + i as u32 / 8;
        assert_eq!(
            *pixel,
            [(3 * x + 2800) as f32, (3 * y + 20) as f32, 0., 33.]
        );
    }
}
