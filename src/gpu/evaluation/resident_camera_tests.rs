use super::*;
use crate::{
    adjustment_layers::{
        AdjustmentLayer, AdjustmentLayerCoverage, AdjustmentLayerSettings, AdjustmentLayerSnapshot,
        AdjustmentLayerStack,
    },
    geometry::{SourcePoint, SourceRegion},
    gpu::{GpuFlatField, GpuPipeline, ProcessingInput, ProcessingSource, SourceGeometry},
    processing::{Calibration, InputPipeline, InputSettings, RollPipeline, RollSettings},
};
use glam::DVec3;

fn recorded(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &crate::renderer::GpuRenderer,
    root: Arc<ImageNode>,
    source: &ProcessingSource,
    policy: Policy,
) -> (Vec<[f32; 4]>, usize) {
    let plan = Plan::with_policy(
        root.clone(),
        root.domain.bounds,
        Budget {
            tile: [root.domain.bounds.width, root.domain.bounds.height],
            scratch_bytes: 4 << 20,
        },
        policy,
    )
    .unwrap();
    let computes = plan.tiles[0]
        .fragment
        .work
        .iter()
        .filter(|w| matches!(w, Work::Compute { .. }))
        .count();
    let prepared = renderer
        .evaluation
        .executor()
        .prepare(device, plan, vec![])
        .unwrap();
    let output = crate::gpu::raster::buffer(
        device,
        [root.domain.bounds.width, root.domain.bounds.height],
        "resident photographic boundary",
    )
    .unwrap();
    let mut encoder = device.create_command_encoder(&Default::default());
    prepared
        .encode_into(device, &mut encoder, &source.input, &output)
        .unwrap();
    queue.submit([encoder.finish()]);
    (execute::read(device, queue, &output), computes)
}

#[test]
fn packed_and_continuous_camera_cells_share_conversion_grade_and_repair_boundaries() {
    use crate::dust_repair::{Repair, RepairId, RepairOrigin, SupportMask};
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let size = [13, 11];
    let matrix = super::tests::matrix();
    for region in [
        SourceRegion::from([23, 37, 13, 11]),
        SourceRegion::from([23, 37, 31, 25]),
    ] {
        let geometry =
            SourceGeometry::new([region.x, region.y, region.width, region.height], size).unwrap();
        let native = geometry.size() == [region.width, region.height];
        let field = Arc::new(
            crate::flat_field::FlatField::from_nodes(
                [region.x, region.y, region.width, region.height],
                [2, 2],
                vec![
                    DVec3::new(1.2, 0.9, 1.1),
                    DVec3::new(0.95, 1.1, 0.85),
                    DVec3::new(1.1, 1.3, 0.95),
                    DVec3::new(0.9, 1.05, 1.2),
                ],
            )
            .unwrap(),
        );
        let gpu_field = Arc::new(GpuFlatField::new(&device, field.clone()).unwrap());
        let mut local = AdjustmentLayer::global(AdjustmentLayerSettings {
            exposure: 1.5,
            contrast: 1.1,
            ..Default::default()
        });
        local.mask = AdjustmentLayerCoverage::Radial {
            center: SourcePoint([
                f64::from(region.x) + 0.6 * f64::from(region.width),
                f64::from(region.y) + 0.55 * f64::from(region.height),
            ]),
            radii: [3.2, 4.1],
            angle: 0.37,
            feather: 0.4,
        };
        let stack = AdjustmentLayerSnapshot::new(vec![
            AdjustmentLayer::global(AdjustmentLayerSettings {
                saturation: 0.8,
                ..Default::default()
            }),
            local,
        ])
        .unwrap();
        let scalar_stack = AdjustmentLayerStack::new(stack.layers().to_vec()).unwrap();
        let roll = RollSettings {
            exposure: 0.15,
            ..Default::default()
        };
        let scalar_roll = RollPipeline::new(roll).unwrap();
        let repaired = native.then(|| {
            let repair = Repair {
                id: RepairId(1),
                enabled: true,
                origin: RepairOrigin::Manual,
                support: SupportMask::new([region.x + 4, region.y + 4], [3, 3], vec![true; 9])
                    .unwrap(),
                offset: [16, 0],
            };
            Arc::new(
                // Donor support belongs to the captured source, beyond this processing tile.
                crate::dust_repair::evaluate(
                    &repair,
                    [
                        region.x - 8,
                        region.y - 8,
                        region.width + 32,
                        region.height + 32,
                    ]
                    .into(),
                    |_| Ok([-0.3, 1.8, 0.5]),
                )
                .unwrap(),
            )
        });
        for continuous in [false, true] {
            let integer: Vec<[u16; 4]> = (0..size[0] * size[1])
                .map(|i| {
                    let base = [0u16, 1200, 10000, 30000, 60000][i as usize % 5];
                    [
                        base,
                        (u32::from(base) * 3 / 4) as u16,
                        (u32::from(base) / 2 + 700) as u16,
                        0,
                    ]
                })
                .collect();
            let values: Vec<[f32; 4]> = integer
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    [
                        p[0] as f32
                            + if continuous {
                                0.375 + (i % 3) as f32 * 20000.
                            } else {
                                0.
                            },
                        p[1] as f32,
                        p[2] as f32,
                        0.,
                    ]
                })
                .collect();
            let mut source = if continuous {
                ProcessingSource::new_continuous(&device, &values, matrix).unwrap()
            } else {
                ProcessingSource::new(&device, &integer, matrix).unwrap()
            };
            source
                .set_flat_field(Some(gpu_field.clone()), geometry)
                .unwrap();
            for settings in [
                InputSettings::Digital,
                InputSettings::negative(Calibration {
                    dmin: DVec3::splat(0.7),
                    dmax: DVec3::splat(0.14),
                    input_floor: 0.1,
                }),
            ] {
                let pipeline = Arc::new(
                    GpuPipeline::new(&device, settings, roll, stack.clone(), matrix, region)
                        .unwrap(),
                );
                let scalar_input = InputPipeline::new(settings, matrix).unwrap();
                let mut input = ProcessingInput::from(pipeline.clone());
                if let Some(repaired) = &repaired {
                    input.repairs =
                        crate::gpu::repairs::Snapshot::logical(vec![repaired.clone()]).unwrap();
                }
                let raster = crate::gpu::raster::Raster::region(region, size);
                let photo =
                    Photograph::from_resident(&device, &renderer, &source, &input, raster).unwrap();
                let mut expected = vec![Vec::new(); 4];
                let mut nearest_effect = 0_f64;
                let mut correction_effect = 0_f64;
                let mut grade_effect = 0_f64;
                let mut repair_effect = 0_f64;
                let mut flags_present = false;
                let mut signed_hdr = [false; 2];
                for (i, value) in values.iter().enumerate() {
                    let x = i as u32 % size[0];
                    let y = i as u32 / size[0];
                    let point = [
                        region.x
                            + ((2 * u64::from(x) + 1) * u64::from(region.width)
                                / (2 * u64::from(size[0]))) as u32,
                        region.y
                            + ((2 * u64::from(y) + 1) * u64::from(region.height)
                                / (2 * u64::from(size[1]))) as u32,
                    ];
                    let position = SourcePoint(point.map(|v| f64::from(v) + 0.5));
                    let raw =
                        DVec3::new(value[0] as f64, value[1] as f64, value[2] as f64) / 65535.;
                    let base = scalar_input
                        .apply(raw * field.gain(position.0).unwrap())
                        .unwrap();
                    correction_effect = correction_effect.max(
                        (base.rgb - scalar_input.apply(raw).unwrap().rgb)
                            .abs()
                            .max_element(),
                    );
                    let flags = base
                        .floored
                        .iter()
                        .enumerate()
                        .fold(0u32, |v, (i, on)| v | ((*on as u32) << i));
                    flags_present |= flags != 0;
                    let neutral = repaired.as_ref().map_or(base.rgb, |repair| {
                        DVec3::from_array(
                            repair
                                .pixel(point, base.rgb.to_array().map(|v| v as f32))
                                .map(f64::from),
                        )
                    });
                    repair_effect = repair_effect.max((neutral - base.rgb).abs().max_element());
                    signed_hdr[0] |= neutral.min_element() < 0.;
                    signed_hdr[1] |= neutral.max_element() > 1.;
                    let roll_rgb = scalar_roll.apply(neutral).unwrap();
                    let complete = scalar_stack.apply_rgb(roll_rgb, position).unwrap();
                    let affine = SourcePoint([
                        f64::from(region.x)
                            + (f64::from(x) + 0.5) * f64::from(region.width) / f64::from(size[0]),
                        f64::from(region.y)
                            + (f64::from(y) + 0.5) * f64::from(region.height) / f64::from(size[1]),
                    ]);
                    nearest_effect = nearest_effect.max(
                        (complete - scalar_stack.apply_rgb(roll_rgb, affine).unwrap())
                            .abs()
                            .max_element(),
                    );
                    grade_effect = grade_effect.max((complete - roll_rgb).abs().max_element());
                    for (dst, rgb) in expected
                        .iter_mut()
                        .zip([base.rgb, neutral, roll_rgb, complete])
                    {
                        dst.push([rgb.x, rgb.y, rgb.z, flags as f64]);
                    }
                }
                assert!(correction_effect > 0.01 && grade_effect > 0.01);
                assert_eq!(
                    signed_hdr,
                    [true, true],
                    "each resident fixture must exercise signed/HDR AP1"
                );
                assert_eq!(
                    flags_present,
                    matches!(settings, InputSettings::Negative { .. })
                );
                if native {
                    assert!(repair_effect > 0.01);
                } else {
                    assert!(
                        nearest_effect > 1e-3,
                        "proxy fixture must distinguish nearest original centres from continuous footprint centres"
                    );
                }
                let mut full = None;
                let mut work = Vec::new();
                for (root, expected) in [photo.unrepaired, photo.neutral, photo.roll, photo.graded]
                    .into_iter()
                    .zip(expected)
                {
                    let mut counts = Vec::new();
                    for policy in [Policy::Materialized, Policy::Composed] {
                        let (actual, count) =
                            recorded(&device, &queue, &renderer, root.clone(), &source, policy);
                        for (actual, expected) in actual.iter().zip(&expected) {
                            assert_eq!(actual[3] as f64, expected[3]);
                            for c in 0..3 {
                                assert!(
                                    (actual[c] as f64 - expected[c]).abs() < 2e-5,
                                    "{settings:?} continuous={continuous} native={native}: {actual:?} vs {expected:?}"
                                );
                            }
                        }
                        counts.push(count);
                        full = Some(actual);
                    }
                    work.push(counts);
                }
                assert!(
                    work[3][1] < work[3][0],
                    "canonical camera composition does not materialize its logical grade boundaries"
                );
                let output =
                    crate::gpu::raster::buffer(&device, size, "legacy camera reference").unwrap();
                let mut encoder = device.create_command_encoder(&Default::default());
                renderer
                    .kernel
                    .encode_grade(&device, &mut encoder, &source, &output, &pipeline)
                    .unwrap();
                if native {
                    renderer
                        .repairs
                        .original(
                            &device,
                            &mut encoder,
                            &renderer.kernel,
                            &output,
                            &input,
                            geometry,
                        )
                        .unwrap();
                }
                queue.submit([encoder.finish()]);
                assert_eq!(
                    full.unwrap(),
                    execute::read(&device, &queue, &output),
                    "canonical implementation preserves the existing source-kernel f32 arithmetic"
                );
            }
        }
    }
}

#[test]
#[cfg(target_os = "macos")]
#[ignore = "isolated authored resident-camera execution and allocator comparison"]
fn resident_camera_execution_resources() {
    use std::sync::atomic::{AtomicU64, Ordering};
    crate::cache_observation::with_monitor(|monitor, device, queue| {
        let side = 1536;
        let region = SourceRegion::from([17, 29, side, side]);
        let renderer =
            pollster::block_on(crate::renderer::GpuRenderer::new(device, queue)).unwrap();
        let matrix = super::tests::matrix();
        let values: Vec<_> = (0..side * side)
            .map(|i| {
                let x = (i % side) as f32 / side as f32;
                let y = (i / side) as f32 / side as f32;
                [
                    (0.15 + x) * 65535.,
                    (0.2 + y * 0.5) * 65535.,
                    (0.7 - x * 0.2) * 65535.,
                    0.,
                ]
            })
            .collect();
        let mut source = ProcessingSource::new_continuous(device, &values, matrix).unwrap();
        drop(values);
        source
            .set_geometry(SourceGeometry::new([region.x, region.y, side, side], [side; 2]).unwrap())
            .unwrap();
        let pipeline = Arc::new(
            GpuPipeline::new(
                device,
                InputSettings::Digital,
                RollSettings {
                    exposure: 0.3,
                    ..Default::default()
                },
                AdjustmentLayerSnapshot::new(vec![AdjustmentLayer::global(
                    AdjustmentLayerSettings {
                        contrast: 1.1,
                        saturation: 0.9,
                        ..Default::default()
                    },
                )])
                .unwrap(),
                matrix,
                region,
            )
            .unwrap(),
        );
        let input = ProcessingInput::from(pipeline.clone())
            .with_film_emulation(crate::film_emulation::FilmEmulationPreset::Medium.settings())
            .with_frame_size([side; 2])
            .with_film_seed(17);
        let plan = crate::gpu::execution::raster::resident_plan(&source, &input)
            .unwrap()
            .unwrap();
        let raster = crate::gpu::raster::Raster::planned(&plan);
        let gate = crate::gpu::SubmissionGate::default();
        let peak = Arc::new(AtomicU64::new(0));
        let observed = peak.clone();
        let gpu = device.clone();
        gate.observe_encoded_resources(Arc::new(move || {
            observed.fetch_max(
                crate::gpu::allocated_resource_bytes(&gpu).unwrap() as u64,
                Ordering::Relaxed,
            );
        }));
        let outputs = [false, true].map(|_| {
            crate::gpu::raster::buffer(device, [side; 2], "resident comparison output").unwrap()
        });
        let _ = crate::gpu::test_support::take_evaluation_buffer_creations();
        for iteration in 0..6 {
            let order = if iteration % 2 == 0 {
                [false, true]
            } else {
                [true, false]
            };
            for common in order {
                let output = outputs[usize::from(common)].clone();
                peak.store(0, Ordering::Relaxed);
                monitor.check()?;
                let started = std::time::Instant::now();
                let mut encoder = device.create_command_encoder(&Default::default());
                let prepared = if common {
                    crate::gpu::execution::raster::Request::new(&source, &input, Some(&plan))
                        .encode(&renderer, device, &mut encoder, &output)
                        .unwrap()
                        .unwrap()
                } else {
                    let prepared = crate::gpu::pixel_pipeline::Prepared::with_context(
                        device,
                        plan.pipeline(),
                        raster,
                        None,
                        renderer.evaluation.clone(),
                    )
                    .unwrap();
                    renderer
                        .kernel
                        .encode_grade(device, &mut encoder, &source, &output, &pipeline)
                        .unwrap();
                    prepared
                        .encode_from(device, &mut encoder, &output, &output)
                        .unwrap();
                    prepared
                };
                let submission = gate.submit(queue, [encoder.finish()]);
                device
                    .poll(wgpu::PollType::Wait {
                        submission_index: Some(submission),
                        timeout: Some(std::time::Duration::from_secs(10)),
                    })
                    .map_err(|e| e.to_string())?;
                drop(prepared);
                let [creations, bytes] = crate::gpu::test_support::take_evaluation_buffer_creations();
                println!(
                    "resident_resource common={common} iteration={iteration} seconds={:.9} retained_device={} encoded_device_peak={} evaluation_buffer_creations={creations} evaluation_buffer_bytes={bytes}",
                    started.elapsed().as_secs_f64(),
                    crate::gpu::allocated_resource_bytes(device).unwrap(),
                    peak.load(Ordering::Relaxed)
                );
            }
        }
        let captures = outputs
            .each_ref()
            .map(|output| execute::read(device, queue, output));
        assert_eq!(
            captures[0], captures[1],
            "resident refactoring preserves the previous kernel and effect arithmetic"
        );
        drop((captures, outputs, source, input, pipeline, renderer));
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(10)),
            })
            .map_err(|e| e.to_string())?;
        println!(
            "resident_resource retired_device={}",
            crate::gpu::allocated_resource_bytes(device).unwrap()
        );
        monitor.check()?;
        Ok(())
    }).unwrap();
}
