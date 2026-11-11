use super::*;
mod reconstruction;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Coordinates {
    domain: Domain,
    named: bool,
    cells: Arc<AtomicUsize>,
}
impl Source for Coordinates {
    fn domain(&self) -> Domain {
        self.domain
    }
    fn identity(&self) -> Option<RetainedKey> {
        self.named
            .then(|| RetainedKey::new(("phase alias coordinates", self.domain)))
    }
    fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        self.cells
            .fetch_add((region.width * region.height) as usize, Ordering::Relaxed);
        Ok((region.y..region.bottom())
            .flat_map(|y| {
                (region.x..region.right())
                    .map(move |x| [(x + 2 * y) as f32, (y - 4) as f32, (x - y) as f32, 0.125])
            })
            .collect())
    }
}

#[test]
fn repeated_named_and_anonymous_phase_reads_share_their_actual_captured_input() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let region = CellRegion::new([-2, 5], [4, 4]);
    let domain = Domain {
        bounds: Some(region),
        grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        meaning: Meaning::Neutral,
    };
    for named in [false, true] {
        let cells = Arc::new(AtomicUsize::new(0));
        let source = source(Arc::new(Coordinates {
            domain,
            named,
            cells: cells.clone(),
        }))
        .request(region)
        .unwrap();
        let operands = vec![source.clone(), source];
        let sum = Value::new(domain, region, operands.clone(), value::Operation::new(
            "repeated source weighted sum", (),
            "fn pixel(p:vec2i,position:vec2f,a:vec4f,b:vec4f)->vec4f { return vec4f(a.rgb+2.0*b.rgb,a.w); }",
            "pixel", value::Access::Pointwise,
        )).unwrap();
        let UnitWork::Gpu(mut compiled) = sum
            .plan_unit(
                region,
                Budget {
                    tile: [4, 4],
                    scratch_bytes: 1 << 20,
                },
            )
            .unwrap()
            .work
        else {
            panic!("GPU fixture");
        };
        let kernel = compiled.phases.next().unwrap().unwrap().kernel;
        let allowance = Budget {
            tile: [4, 4],
            scratch_bytes: 700,
        };
        let unit = WorkUnit {
            inputs: operands,
            domain,
            output: region,
            work: UnitWork::Gpu(GpuUnit {
                state: vec![],
                phases: Box::new((0..2).map(move |_| {
                    Ok(GpuPhase {
                        reads: vec![Read::new(0, region), Read::new(1, region)],
                        inputs: vec![UnitValue::Read(0), UnitValue::Read(1), UnitValue::Output],
                        kernel: kernel.clone(),
                    })
                })),
            }),
        }
        .admit(allowance)
        .unwrap();
        assert_eq!(cells.load(Ordering::Relaxed), 0);
        let UnitWork::Gpu(work) = unit.work else {
            unreachable!()
        };
        let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
        let scope = context.test_materializations(false);
        let gate = crate::gpu::submission::SubmissionGate::default();
        let working = HostWorkingSet::default();
        let worker = Worker {
            context: &context,
            queue: &queue,
            gate: &gate,
            scope: &scope,
            budget: allowance,
            working: &working,
            cancelled: &|| false,
            progress: &|| {},
        };
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("repeated phase result"),
            size: 4 * 4 * 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        worker.gpu_unit(unit.inputs, region, work, &output).unwrap();
        let pixels = crate::gpu::evaluation::execute::read(&device, &queue, &output);
        assert_eq!(pixels.len(), 16);
        for (i, pixel) in pixels.iter().enumerate() {
            let x = region.x + (i % region.width as usize) as i32;
            let y = region.y + (i / region.width as usize) as i32;
            assert_eq!(
                *pixel,
                [
                    (3 * (x + 2 * y)) as f32,
                    (3 * (y - 4)) as f32,
                    (3 * (x - y)) as f32,
                    0.125
                ]
            );
        }
        let acquired = cells.load(Ordering::Relaxed);
        assert!(
            acquired > 0 && acquired <= 2 * pixels.len(),
            "at most one physical input per phase: named={named}, {acquired}"
        );
    }
}
