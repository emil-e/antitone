use super::*;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};
use wgpu::util::DeviceExt;

struct ObserveKernel {
    kernel: Arc<dyn GpuKernel>,
    armed: Arc<AtomicBool>,
}
impl GpuKernel for ObserveKernel {
    fn lower(&self, fragment: &mut Fragment, slots: &[Resource]) -> Result<PlannedImage, Error> {
        let first = fragment.work.len();
        let result = self.kernel.lower(fragment, slots)?;
        if fragment.work[first..].iter().any(|work| {
            matches!(
                work,
                Work::Compute { code, .. } if code.contains("contribution(")
            )
        }) {
            self.armed.store(true, Ordering::SeqCst);
        }
        Ok(result)
    }
}

struct Constant(Arc<Mutex<Vec<CellRegion>>>);
impl Source for Constant {
    fn domain(&self) -> Domain {
        Domain {
            bounds: Some([0, 0, 128, 64].into()),
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
            meaning: Meaning::Neutral,
        }
    }
    fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        self.0.lock().unwrap().push(region);
        Ok(vec![
            [1.; 4];
            region.width as usize * region.height as usize
        ])
    }
}

#[test]
fn cancellation_after_a_contribution_prevents_final_output_writes_and_more_source_reads() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let reads = Arc::new(Mutex::new(Vec::new()));
    let region: CellRegion = [0, 0, 1, 1].into();
    let domain = Domain {
        bounds: Some(region),
        grid: Grid::axis_aligned([0.; 2], [128., 64.]),
        meaning: Meaning::Neutral,
    };
    let root = crate::gpu::evaluation::reconstruct_cells(
        source(Arc::new(Constant(reads.clone()))),
        domain,
        [0.; 2],
        [128., 64.],
    )
    .unwrap();
    let budget = Budget {
        tile: [1, 1],
        scratch_bytes: 64 << 10,
    };
    let unit = root
        .plan_unit(region, budget)
        .unwrap()
        .admit_for(budget, &device.limits())
        .unwrap();
    let UnitWork::Gpu(mut work) = unit.work else {
        panic!("GPU reconstruction fixture")
    };
    let armed = Arc::new(AtomicBool::new(false));
    let observe = armed.clone();
    work.phases = Box::new(work.phases.map(move |phase| {
        phase.map(|mut phase| {
            phase.kernel = Arc::new(ObserveKernel {
                kernel: phase.kernel,
                armed: observe.clone(),
            });
            phase
        })
    }));
    assert!(reads.lock().unwrap().is_empty());
    let pixels = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&[[1234f32; 4]]),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    });
    let stopped = Arc::new(AtomicBool::new(false));
    let gate = crate::gpu::submission::SubmissionGate::default();
    let (stop, accumulating) = (stopped.clone(), armed.clone());
    gate.observe_submissions(Arc::new(move || {
        if accumulating.load(Ordering::SeqCst) {
            stop.store(true, Ordering::SeqCst);
        }
    }));
    let scope = context.test_materializations(false);
    let working = HostWorkingSet::default();
    let cancelled = || stopped.load(Ordering::SeqCst);
    let worker = Worker {
        context: &context,
        queue: &queue,
        gate: &gate,
        scope: &scope,
        budget,
        working: &working,
        cancelled: &cancelled,
        progress: &|| {},
    };
    assert!(matches!(
        worker.gpu_unit(unit.inputs, region, work, &pixels),
        Err(Error::Cancelled)
    ));
    assert!(
        armed.load(Ordering::SeqCst) && stopped.load(Ordering::SeqCst),
        "cancel only after the real accumulation kernel is submitted"
    );
    assert_eq!(reads.lock().unwrap().len(), 1);
    assert_eq!(
        crate::gpu::evaluation::execute::read(&device, &queue, &pixels),
        vec![[1234.; 4]]
    );
}
