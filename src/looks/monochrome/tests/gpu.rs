use super::*;
use crate::gpu::{
    SubmissionGate,
    lut3d::{Kernel, Lut},
};
use std::{sync::mpsc, time::Duration};
use wgpu::util::DeviceExt;

#[test]
fn all_papers_and_negative_pairs_survive_the_production_cube_and_authoring_view() {
    pollster::block_on(async {
        let adapter = crate::gpu::instance()
            .request_adapter(&Default::default())
            .await
            .unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let reference = crate::looks::reference::ReferenceTransforms::new(&device, &queue)
            .await
            .unwrap();
        let gate = SubmissionGate::default();
        let f = catalogue::fixtures();
        let mut failures = Vec::new();
        for case in &f.cases {
            let selection = case.selection();
            let asset = Model::new(selection)
                .bake(
                    crate::looks::authoring::DEFAULT_GRID,
                    &AtomicBool::new(false),
                )
                .unwrap();
            let gpu = Lut::new(&device, asset.lut()).unwrap();
            let kernel = Kernel::new(&device).await.unwrap();
            let min = crate::photographic::decode(crate::looks::authoring::DOMAIN_MIN);
            let max = crate::photographic::decode(crate::looks::authoring::domain_max());
            // Out-of-window clipping has a different contract from native material
            // evaluation. The sampler's independent analytic tests cover that rule;
            // this comparison uses source probes inside its declared input window.
            let cases = f
                .inputs
                .iter()
                .zip(&case.reference_rgb)
                .filter(|(rgb, _)| rgb.min_element() >= min && rgb.max_element() <= max)
                .collect::<Vec<_>>();
            assert!(cases.iter().any(|(rgb, _)| rgb.min_element() < 0.));
            assert!(cases.iter().any(|(rgb, _)| rgb.max_element() > 1.));
            let input = cases
                .iter()
                .map(|(rgb, _)| [rgb.x as f32, rgb.y as f32, rgb.z as f32, 1.])
                .collect::<Vec<_>>();
            let source = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("paper source probes"),
                contents: bytemuck::cast_slice(&input),
                usage: wgpu::BufferUsages::STORAGE,
            });
            let output = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: source.size(),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: source.size(),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let dispatch = kernel
                .bind(&device, &source, &output, gpu, input.len() as u32, false)
                .unwrap();
            let mut encoder = device.create_command_encoder(&Default::default());
            kernel.encode(&mut encoder, &dispatch);
            encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, output.size());
            let submission = gate.submit(&queue, [encoder.finish()]);
            dispatch.check.begin(|| {});
            let (send, receive) = mpsc::channel();
            readback.map_async(wgpu::MapMode::Read, .., move |result| {
                send.send(result).unwrap()
            });
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(submission),
                    timeout: Some(Duration::from_secs(30)),
                })
                .unwrap();
            receive
                .recv_timeout(Duration::from_secs(30))
                .unwrap()
                .unwrap();
            dispatch.check.result().unwrap().unwrap();
            let scene = {
                let mapped = readback.get_mapped_range(..).unwrap();
                bytemuck::cast_slice::<u8, [f32; 4]>(&mapped).to_vec()
            };
            readback.unmap();
            let viewed = reference
                .apply(&device, &queue, &gate, false, &scene)
                .unwrap();
            assert_eq!(viewed.len(), cases.len());
            let maximum = viewed
                .iter()
                .zip(&cases)
                .map(|(actual, (_, expected))| {
                    (DVec3::new(actual[0] as f64, actual[1] as f64, actual[2] as f64) - **expected)
                        .abs()
                        .max_element()
                })
                .fold(0., f64::max);
            println!(
                "{} source probes through 129³/GPU tetrahedral/reference DRT: maxRGB {maximum}",
                cases.len()
            );
            if maximum >= 0.001 {
                let mut errors = viewed
                    .iter()
                    .zip(&cases)
                    .map(|(actual, (input, expected))| {
                        let actual =
                            DVec3::new(actual[0] as f64, actual[1] as f64, actual[2] as f64);
                        (
                            (actual - **expected).abs().max_element(),
                            **input,
                            actual,
                            **expected,
                        )
                    })
                    .collect::<Vec<_>>();
                errors.sort_by(|a, b| b.0.total_cmp(&a.0));
                println!("{} largest errors: {:?}", selection.name(), &errors[..3]);
                failures.push((selection.name(), maximum));
            }
        }
        assert!(
            failures.is_empty(),
            "final-cube/view error budget exceeded: {failures:?}"
        );
    });
}
