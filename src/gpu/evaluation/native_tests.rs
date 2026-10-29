use super::*;
use crate::{
    adjustment_layers::AdjustmentLayerSnapshot,
    geometry::Composition,
    gpu::{GpuGrade, GpuPipeline, ProcessingInput},
    processing::{InputSettings, RollSettings},
    rendering::OutputTarget,
};
use std::sync::atomic::AtomicU64;

struct Case {
    scan: Arc<crate::scan::Scan>,
    node: Arc<ImageNode>,
    input: ProcessingInput,
    demand: crate::refinement::ViewRequest,
}

fn case(
    device: &wgpu::Device,
    renderer: &crate::renderer::GpuRenderer,
    scan: Arc<crate::scan::Scan>,
    size: [u32; 2],
) -> Case {
    let area = scan.area;
    let composition = Composition::new(area, Default::default(), Default::default()).unwrap();
    let matrix = scan.positive_matrix(&InputSettings::Digital).unwrap();
    let roll = RollSettings {
        exposure: -0.3,
        ..Default::default()
    };
    let snapshot =
        AdjustmentLayerSnapshot::new(vec![crate::adjustment_layers::AdjustmentLayer::global(
            crate::adjustment_layers::AdjustmentLayerSettings {
                contrast: 1.2,
                saturation: 0.9,
                ..Default::default()
            },
        )])
        .unwrap();
    let film = crate::film_emulation::FilmEmulationSettings {
        enabled: true,
        amount: 0.,
        size: crate::film_emulation::MAX_SIZE,
        ..Default::default()
    };
    let sigma = film
        .parameters(composition.crop().size)
        .unwrap()
        .unwrap()
        .image_mtf_sigma;
    let input = ProcessingInput::from(Arc::new(
        GpuPipeline::new(
            device,
            InputSettings::Digital,
            roll,
            snapshot.clone(),
            matrix,
            area,
        )
        .unwrap(),
    ))
    .with_film_emulation(film)
    .with_frame_size(composition.crop().size)
    .with_film_seed(19);
    let mut node = interpret(device, source(scan.clone()), matrix).unwrap();
    node = grade(
        node,
        device,
        Arc::new(GpuGrade::new(device, roll, snapshot, area).unwrap()),
    )
    .unwrap();
    node = gaussian(node, sigma).unwrap();
    node = display(node, renderer, OutputTarget::Srgb).unwrap();
    node = reconstruct(node.clone(), super::tests::reduced(node.domain, size)).unwrap();
    Case {
        scan,
        node,
        input,
        demand: crate::refinement::ViewRequest {
            area,
            output: size,
            frame: None,
        },
    }
}

fn regional(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &Arc<crate::renderer::GpuRenderer>,
    gate: &crate::gpu::SubmissionGate,
    case: &Case,
) -> Arc<crate::viewer::GpuImage> {
    crate::refinement::refine(
        renderer,
        device,
        (queue, gate),
        (&case.scan, None),
        case.input.clone(),
        OutputTarget::Srgb,
        (&AtomicU64::new(1), 1),
        case.demand,
    )
    .unwrap()
    .unwrap()
}

#[test]
fn authored_native_source_and_real_grade_mtf_display_match_regional_execution() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let file = crate::dng_fixture::Fixture {
        side: 256,
        bits: 16,
        ramp: true,
        ..Default::default()
    }
    .write();
    let scan = Arc::new(crate::scan::Scan::open(file.path(), None).unwrap());
    let case = case(&device, &renderer, scan, [73, 59]);
    let gate = crate::gpu::SubmissionGate::default();
    let original = regional(&device, &queue, &renderer, &gate, &case);
    let original = crate::viewer::readback::image_pixels(&device, &queue, &original);
    assert!(original.iter().flatten().all(|v| v.is_finite()));
    let expected = original
        .iter()
        .map(|p| glam::DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64))
        .collect::<Vec<_>>();
    for policy in [Policy::Materialized, Policy::Composed] {
        let plan = Plan::with_policy(
            case.node.clone(),
            case.node.domain.bounds,
            Budget {
                tile: [29, 19],
                scratch_bytes: 32 << 20,
            },
            policy,
        )
        .unwrap();
        let result = execute::Executor::default()
            .run(&device, &queue, &gate, &plan, || false)
            .unwrap()
            .unwrap();
        let actual = execute::read(&device, &queue, &result.pixels);
        let error = super::tests::max_error(&expected, &actual);
        assert!(error < 2e-5, "checkpoint correspondence {error}");
    }
}

#[test]
#[ignore = "isolated authored native-source comparison of evaluation versus checkpointed execution"]
fn native_evaluation_resource_comparison() {
    use std::sync::atomic::{AtomicU64, Ordering};
    let mode = std::env::var("ANTITONE_EVALUATION_CASE").unwrap_or_else(|_| "evaluation".into());
    assert!(mode == "evaluation" || mode == "composed" || mode == "regional");
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let file = crate::dng_fixture::Fixture {
        side: 768,
        bits: 16,
        ramp: true,
        ..Default::default()
    }
    .write();
    let scan = Arc::new(crate::scan::Scan::open(file.path(), None).unwrap());
    let case = case(&device, &renderer, scan, [147, 113]);
    let gate = crate::gpu::SubmissionGate::default();
    let peak = Arc::new(AtomicU64::new(0));
    let submissions = Arc::new(AtomicU64::new(0));
    let submitted = submissions.clone();
    gate.observe_submissions(Arc::new(move || {
        submitted.fetch_add(1, Ordering::Relaxed);
    }));
    let (observed, observed_device) = (peak.clone(), device.clone());
    gate.observe_encoded_resources(Arc::new(move || {
        observed.fetch_max(
            crate::gpu::allocated_resource_bytes(&observed_device).unwrap_or(0) as u64,
            Ordering::Relaxed,
        );
    }));
    let mut executor = execute::Executor::default();
    let plan = if mode != "regional" {
        let start = std::time::Instant::now();
        let policy = if mode == "composed" {
            Policy::Composed
        } else {
            Policy::Materialized
        };
        let plan = Plan::with_policy(
            case.node.clone(),
            case.node.domain.bounds,
            Budget {
                tile: [96, 19],
                scratch_bytes: 32 << 20,
            },
            policy,
        )
        .unwrap();
        println!(
            "evaluation_plan seconds={:.9} tiles={} scheduled_scratch={}",
            start.elapsed().as_secs_f64(),
            plan.tiles.len(),
            plan.peak_scratch_bytes
        );
        let start = std::time::Instant::now();
        executor.compile(&device, &plan).unwrap();
        println!(
            "evaluation_compile seconds={:.9}",
            start.elapsed().as_secs_f64()
        );
        Some(plan)
    } else {
        None
    };
    println!(
        "evaluation_initial implementation={mode} device={:?}",
        crate::gpu::allocated_resource_bytes(&device)
    );
    for run in 0..6 {
        peak.store(0, Ordering::Relaxed);
        submissions.store(0, Ordering::Relaxed);
        let start = std::time::Instant::now();
        if let Some(plan) = &plan {
            let result = executor
                .run(&device, &queue, &gate, plan, || false)
                .unwrap()
                .unwrap();
            println!(
                "evaluation_compare implementation={mode} run={run} seconds={:.9} retained={:?} encoded_peak={} stats={:?}",
                start.elapsed().as_secs_f64(),
                crate::gpu::allocated_resource_bytes(&device),
                peak.load(Ordering::Relaxed),
                result.stats
            );
            if run == 0
                && let Some(path) = std::env::var_os("ANTITONE_EVALUATION_OUTPUT")
            {
                let pixels = execute::read(&device, &queue, &result.pixels);
                std::fs::write(path, bytemuck::cast_slice(&pixels)).unwrap();
            }
        } else {
            let result = regional(&device, &queue, &renderer, &gate, &case);
            println!(
                "evaluation_compare implementation={mode} run={run} seconds={:.9} retained={:?} encoded_peak={}",
                start.elapsed().as_secs_f64(),
                crate::gpu::allocated_resource_bytes(&device),
                peak.load(Ordering::Relaxed)
            );
            if run == 0
                && let Some(path) = std::env::var_os("ANTITONE_EVALUATION_OUTPUT")
            {
                let pixels = crate::viewer::readback::image_pixels(&device, &queue, &result);
                std::fs::write(path, bytemuck::cast_slice(&pixels)).unwrap();
            }
            drop(result);
        }
        println!(
            "evaluation_submissions implementation={mode} run={run} count={}",
            submissions.load(Ordering::Relaxed)
        );
    }
    drop(executor);
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(queue.submit([])),
            timeout: Some(std::time::Duration::from_secs(10)),
        })
        .unwrap();
    println!(
        "evaluation_retired implementation={mode} device={:?}",
        crate::gpu::allocated_resource_bytes(&device)
    );
}
