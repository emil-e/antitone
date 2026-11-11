use super::*;
use crate::{
    gpu::{
        lut3d::{Kernel, Lut},
        submission::SubmissionGate,
    },
    rendering::{CpuRenderer, OutputTarget},
};
use std::{sync::mpsc, time::Duration};
use wgpu::util::DeviceExt;

fn check(input: Vec<DVec3>) {
    assert!(!input.is_empty() && input.iter().all(|p| p.is_finite()));
    let input: Vec<_> = input.into_iter().map(|p| p.as_vec3().as_dvec3()).collect();
    pollster::block_on(async {
        let instance = crate::gpu::instance();
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        for paper in Paper::all() {
            let model = Model::new(
                Selection {
                    negative: None,
                    paper,
                },
                &AtomicBool::new(false),
            )
            .unwrap();
            let asset = model
                .bake(
                    crate::looks::authoring::DEFAULT_GRID,
                    &AtomicBool::new(false),
                )
                .unwrap();
            let gpu = Lut::new(&device, asset.lut()).unwrap();
            let kernel = Kernel::new(&device).await.unwrap();
            let pixels: Vec<_> = input
                .iter()
                .map(|p| [p.x as f32, p.y as f32, p.z as f32, 8.])
                .collect();
            let source = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Colour media signed/HDR sampling controls"),
                contents: bytemuck::cast_slice(&pixels),
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
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let dispatch = kernel
                .bind(&device, &source, &output, gpu, input.len() as u32, false)
                .unwrap();
            let mut encoder = device.create_command_encoder(&Default::default());
            kernel.encode(&mut encoder, &dispatch);
            encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, output.size());
            let submission = SubmissionGate::default().submit(&queue, [encoder.finish()]);
            dispatch.check.begin(|| {});
            let (send, receive) = mpsc::channel();
            readback.map_async(wgpu::MapMode::Read, .., move |r| send.send(r).unwrap());
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
            let mapped = readback.get_mapped_range(..).unwrap();
            let actual: Vec<_> = bytemuck::cast_slice::<u8, [f32; 4]>(&mapped)
                .iter()
                .map(|p| {
                    assert_eq!(p[3], 8.);
                    DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64)
                })
                .collect();
            drop(mapped);
            readback.unmap();
            let expected: Vec<_> = input
                .iter()
                .map(|&p| asset.lut().apply(p).unwrap())
                .collect();
            let renderer = CpuRenderer::new(OutputTarget::DisplayP3).unwrap();
            let actual = OutputTarget::DisplayP3
                .encode_u16(&renderer.render(&actual).unwrap())
                .unwrap();
            let expected = OutputTarget::DisplayP3
                .encode_u16(&renderer.render(&expected).unwrap())
                .unwrap();
            let maximum = actual
                .iter()
                .zip(&expected)
                .flat_map(|(a, e)| (0..3).map(move |c| a[c].abs_diff(e[c]) as f64 / 257.))
                .fold(0., f64::max);
            println!(
                "{} production{} {} GPU/scalar sites: maxRGB8 error {maximum}",
                input.len(),
                asset.lut().size(),
                paper.id()
            );
            // Computational agreement of the accepted numeric asset, not fidelity of
            // its sampled approximation to the continuous material or physical print truth.
            assert!(
                maximum < 0.02,
                "{} GPU/scalar sampling error {maximum}",
                paper.id()
            );
        }
    });
}

#[test]
fn every_media_production_lut_gpu_matches_scalar_for_signed_hdr_and_near_gray_colours() {
    let mut points = Vec::new();
    for r in [-0.05, 0., 0.005, 0.045, 0.18, 0.7, 4., 1024.] {
        for g in [-0.02, 0.003, 0.09, 0.18, 0.36, 1., 8.] {
            for b in [-0.03, 0., 0.0078125, 0.16, 0.18, 0.32, 2.] {
                points.push(DVec3::new(r, g, b));
            }
        }
    }
    points.extend([
        DVec3::new(-100., 1., 1024.),
        DVec3::splat(-0.02),
        DVec3::splat(0.18),
        DVec3::new(0.18001, 0.18, 0.17999),
        DVec3::new(0.0078115, 0.0078125, 0.0078135),
    ]);
    check(points);
}

#[test]
#[ignore = "Set ANTITONE_MEDIA_POINTS to captured float32 AP1 triples"]
fn every_media_captured_original_site_gpu_sampling() {
    let bytes =
        std::fs::read(std::env::var_os("ANTITONE_MEDIA_POINTS").expect("captured AP1 points"))
            .unwrap();
    assert!(!bytes.is_empty() && bytes.len().is_multiple_of(12));
    let input = bytes
        .as_chunks::<12>()
        .0
        .iter()
        .map(|p| {
            DVec3::from_array(std::array::from_fn(|c| {
                f32::from_le_bytes(p[c * 4..c * 4 + 4].try_into().unwrap()) as f64
            }))
        })
        .collect();
    check(input);
}
