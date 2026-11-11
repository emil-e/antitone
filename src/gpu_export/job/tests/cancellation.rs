use super::*;
use crate::gpu::evaluation::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Default)]
struct Observation {
    entered: AtomicBool,
    stopped: AtomicBool,
    returned: AtomicBool,
}
struct ObserveKernel {
    kernel: Arc<dyn CpuKernel>,
    observation: Arc<Observation>,
}
impl CpuKernel for ObserveKernel {
    fn run(
        &self,
        inputs: &[CpuImage],
        output: CellRegion,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Option<Vec<[f32; 4]>>, Error> {
        assert!(!inputs.is_empty() && inputs.iter().all(|input| !input.pixels.is_empty()));
        self.observation.entered.store(true, Ordering::Release);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !cancelled() {
            assert!(
                std::time::Instant::now() < deadline,
                "the export owner did not retire its healer wait"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let result = self.kernel.run(inputs, output, cancelled);
        self.observation
            .stopped
            .store(matches!(&result, Ok(None)), Ordering::Release);
        self.observation.returned.store(true, Ordering::Release);
        result
    }
}

// Decorate the actual healer rather than substituting a cancellation-only algorithm. The
// decorated operation is unnamed so an earlier retained solve cannot hide this observation.
fn observe(image: &Value, observation: &Arc<Observation>) -> Value {
    fn value(
        image: &crate::gpu::evaluation::value::Value,
        observation: &Arc<Observation>,
    ) -> crate::gpu::evaluation::value::Value {
        let inputs = image
            .inputs()
            .iter()
            .map(|input| value(input, observation))
            .collect::<Vec<_>>();
        image
            .map_cpu_kernel(inputs.clone(), |kernel| {
                Arc::new(ObserveKernel {
                    kernel,
                    observation: observation.clone(),
                })
            })
            .unwrap_or_else(|| {
                crate::gpu::evaluation::value::Value::captured(
                    image.domain(),
                    inputs,
                    image.operation().clone(),
                )
            })
    }
    value(image, observation)
}

#[test]
fn export_cancellation_reaches_the_real_healer_after_its_inputs_arrive_and_preserves_the_destination()
 {
    let (device, queue, renderer) = super::super::super::tests::context();
    let scan = Scan::open(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng"),
        None,
    )
    .unwrap();
    let (mut project, id) = fixture(&scan);
    let area = scan.area;
    project
        .source_mut(id)
        .unwrap()
        .repairs
        .insert(
            SupportMask::new(
                [area.x + area.width / 2, area.y + area.height / 2],
                [3, 3],
                vec![true; 9],
            )
            .unwrap(),
            RepairOrigin::Manual,
            [-32, 0],
        )
        .unwrap();
    let recipe = project.image_recipe(id).unwrap();
    let photograph = crate::gpu::photograph::PhotographGraph::from_recipe(
        &device.limits(),
        camera_source(&scan).unwrap(),
        None,
        &recipe,
    )
    .unwrap();
    let observation = Arc::new(Observation::default());
    let definition = observe(&photograph.neutral, &observation);
    assert!(
        !observation.entered.load(Ordering::Acquire),
        "description must not start the observed healer"
    );
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("result.exr");
    std::fs::write(&path, b"keep existing output").unwrap();
    let result = crate::gpu_export::write_checked(
        &renderer,
        &device,
        (&queue, &Default::default()),
        crate::gpu_export::Request {
            definition,
            composition: recipe.composition,
            format: Format::Exr(crate::export::ExrPrecision::Full),
        },
        &path,
        Overwrite::Confirmed,
        || {
            if observation.entered.load(Ordering::Acquire) {
                Err(Error::Cancelled)
            } else {
                Ok(())
            }
        },
    );
    assert!(
        matches!(result, Err(Error::Cancelled)),
        "unexpected outcome: {result:?}"
    );
    assert!(
        observation.entered.load(Ordering::Acquire),
        "must cancel after actual healing inputs arrive"
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !observation.returned.load(Ordering::Acquire) {
        assert!(
            std::time::Instant::now() < deadline,
            "the retired healer did not finish cleanup"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(
        observation.stopped.load(Ordering::Acquire),
        "the healer itself must acknowledge cancellation"
    );
    assert!(
        observation.returned.load(Ordering::Acquire),
        "the retired kernel completes physical cleanup"
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"keep existing output");
    assert_eq!(
        std::fs::read_dir(directory.path()).unwrap().count(),
        1,
        "no temporary output survives cancellation"
    );
}
