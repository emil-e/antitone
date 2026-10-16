use super::*;

const BUILD: usize = 7;
const SUFFIX: usize = 8;

#[derive(Debug, PartialEq)]
pub(super) struct LogPrefixKey {
    response_without_exposure: ResponseKey,
    source_identity_revision: [u64; 2],
    correction: Option<Arc<crate::flat_field::FlatField>>,
    geometry: Vec<u8>,
}

impl LogPrefixKey {
    pub(super) fn new(
        stamp: [u64; 2],
        source: &ProcessingSource,
        mut p: ProcessingParameters,
    ) -> Self {
        p.anchor.w = 0.;
        p.anchor_low.w = 0.;
        p.anchor_tail.w = 0.;
        let field = match &source.flat_field {
            Some(f) => flat::FieldGeometry::new(f, source.geometry.unwrap()),
            None => flat::FieldGeometry::uncorrected(source.geometry),
        };
        Self {
            response_without_exposure: ResponseKey::new(p),
            source_identity_revision: stamp,
            correction: source.flat_field.as_ref().map(|f| f.field().clone()),
            geometry: bytemuck::bytes_of(&field).to_vec(),
        }
    }
}

async fn setup(
    size: [u32; 2],
    boundary: bool,
) -> (
    wgpu::Device,
    wgpu::Queue,
    ProcessingKernel,
    Probe,
    ProcessingSource,
    Calibration,
    Vec<[u16; 4]>,
) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::METAL,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = instance.request_adapter(&Default::default()).await.unwrap();
    let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
    println!("log_prefix adapter={:?} size={size:?}", adapter.get_info());
    let pixels: Vec<_> = (0u32..size[0] * size[1])
        .map(|i| {
            if boundary {
                [
                    i as u16,
                    i.wrapping_add(1) as u16,
                    i.wrapping_add(2) as u16,
                    42,
                ]
            } else {
                [
                    i.wrapping_mul(1871) as u16,
                    i.wrapping_mul(7919) as u16,
                    i.wrapping_mul(3413) as u16,
                    42,
                ]
            }
        })
        .collect();
    let region = area();
    let geometry =
        SourceGeometry::new([region.x, region.y, region.width, region.height], size).unwrap();
    let mut source = ProcessingSource::new(&device, &pixels, camera_matrix()).unwrap();
    let nodes = if boundary {
        vec![DVec3::ONE; 4]
    } else {
        vec![
            DVec3::new(1.2, 0.9, 1.1),
            DVec3::new(0.95, 1.3, 1.05),
            DVec3::new(1.1, 1.05, 0.88),
            DVec3::new(1.25, 0.98, 1.15),
        ]
    };
    let field = Arc::new(
        crate::flat_field::FlatField::from_nodes(
            [region.x, region.y, region.width, region.height],
            [2, 2],
            nodes,
        )
        .unwrap(),
    );
    source
        .set_flat_field(
            Some(Arc::new(GpuFlatField::new(&device, field.clone()).unwrap())),
            geometry,
        )
        .unwrap();
    let calibration = if boundary {
        Calibration {
            dmin: DVec3::new(2000., 3000., 4000.) / 65535.,
            dmax: DVec3::new(50., 60., 70.) / 65535.,
            input_floor: 1. / 65535.,
        }
    } else {
        calibration()
    };
    let calibration = field.processing_calibration(calibration).unwrap();
    let kernel = ProcessingKernel::new(&device, &queue).await.unwrap();
    let probe = Probe::new(&device, pixels.len()).await;
    (device, queue, kernel, probe, source, calibration, pixels)
}

fn run(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    kernel: &ProcessingKernel,
    probe: &Probe,
    source: &ProcessingSource,
    grade: &GpuPhotoGrade,
    steps: Option<&[usize]>,
) -> f64 {
    // Construction validates the complete snapshot, not separately admissible cuts.
    grade.require_source(source).unwrap();
    let start = Instant::now();
    let mut encoder = device.create_command_encoder(&Default::default());
    if let Some(steps) = steps {
        probe.encode(device, &mut encoder, source, grade, steps);
    } else {
        kernel
            .encode_grade(device, &mut encoder, source, &probe.output, grade)
            .unwrap();
    }
    let submission = queue.submit([encoder.finish()]);
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })
        .unwrap();
    start.elapsed().as_secs_f64() * 1000.
}

#[test]
fn exposure_independent_log_prefix_preserves_codes_signs_zeros_flags_and_full_grades() {
    pollster::block_on(async {
        let (device, queue, kernel, probe, mut source, calibration, pixels) =
            setup([256, 256], true).await;
        let roll = RollSettings {
            gamma: 0.65,
            ..RollSettings::new_grade()
        };
        let make = |calibration, roll, snapshot, matrix| {
            GpuPhotoGrade::new(&device, calibration, roll, snapshot, matrix, area()).unwrap()
        };
        let base = make(
            calibration,
            roll,
            snapshot(PhotoSettings::default(), 0, false),
            camera_matrix(),
        );
        run(
            &device,
            &queue,
            &kernel,
            &probe,
            &source,
            &base,
            Some(&[BUILD]),
        );
        let key = LogPrefixKey::new([1, 0], &source, base.parameters);
        let prefix = read_words(&device, &queue, &probe.prefix);
        let (records, remainder) = prefix.as_chunks::<10>();
        assert!(remainder.is_empty());
        for c in 0..3 {
            assert!(
                records.iter().any(|p| p[9] & (1 << (9 + c)) != 0),
                "missing channel {c} exact zero"
            );
            assert!(
                records.iter().any(|p| p[9] & (1 << c) != 0),
                "missing channel {c} floor"
            );
            assert!(
                records.iter().any(|p| (p[9] >> (3 + 2 * c)) & 3 == 3),
                "missing channel {c} negative sign"
            );
        }
        for ev in [-16., -2., 0., 0.75, 2., 16.] {
            let photo = PhotoSettings {
                exposure: ev,
                ..photographic()
            };
            let changed_roll = RollSettings {
                exposure: 0.375,
                wb: Balance {
                    warmth: -0.003,
                    tint: 0.002,
                },
                ..roll
            };
            let grade = make(
                calibration,
                changed_roll,
                snapshot(photo, 2, true),
                camera_matrix(),
            );
            assert_eq!(key, LogPrefixKey::new([1, 0], &source, grade.parameters));
            run(&device, &queue, &kernel, &probe, &source, &grade, None);
            let reference = read_words(&device, &queue, &probe.output);
            run(
                &device,
                &queue,
                &kernel,
                &probe,
                &source,
                &grade,
                Some(&[SUFFIX]),
            );
            exact(
                &format!("log exposure={ev}"),
                "warm",
                &reference,
                &read_words(&device, &queue, &probe.output),
            );
            if ev == 0. {
                let values = reference.as_chunks::<4>().0;
                assert!(
                    values
                        .iter()
                        .any(|p| p[..3].iter().any(|v| f32::from_bits(*v) < 0.))
                );
                assert!(
                    values
                        .iter()
                        .any(|p| p[..3].iter().any(|v| f32::from_bits(*v) > 1.))
                );
            }
            let scalar = SpatialPipeline::new(
                calibration,
                changed_roll,
                grade.snapshot().clone(),
                camera_matrix(),
            )
            .unwrap();
            for i in (0..pixels.len()).step_by(257) {
                let scan =
                    DVec3::from_array(std::array::from_fn(|c| f64::from(pixels[i][c]) / 65535.));
                let expected = scalar
                    .apply(scan, source.geometry.unwrap().pixel(i as u32).unwrap())
                    .unwrap();
                for c in 0..3 {
                    let actual = f64::from(f32::from_bits(reference[4 * i + c]));
                    assert!(
                        (actual - expected.rgb[c]).abs() <= 2e-5 + 2e-4 * expected.rgb[c].abs(),
                        "log scalar exposure={ev} pixel={i} channel={c} actual={actual} expected={}",
                        expected.rgb[c],
                    );
                }
                let flags = (0..3).fold(0, |bits, c| bits | (u32::from(expected.floored[c]) << c));
                assert_eq!(f32::from_bits(reference[4 * i + 3]), flags as f32);
            }
        }
        let local = snapshot(PhotoSettings::default(), 1, false).locals()[0];
        let unsupported = PhotoGradeSnapshot::new(
            PhotoSettings::default(),
            (0..14)
                .map(|i| LocalAdjustment {
                    photo: PhotoSettings {
                        exposure: if i < 7 { -16. } else { 16. },
                        ..Default::default()
                    },
                    strength: 1.,
                    ..local
                })
                .collect(),
        )
        .unwrap();
        assert!(
            matches!(GpuPhotoGrade::new(&device, calibration, roll, unsupported, camera_matrix(), area()),
            Err(Error::Gpu(message)) if message == "local stack can amplify signals below the normal GPU precision domain")
        );
        assert_eq!(prefix, read_words(&device, &queue, &probe.prefix));

        // Matrix conversion and inspection are downstream; they cannot acquire stale AP1.
        source = ProcessingSource::new(&device, &pixels, DMat3::IDENTITY).unwrap();
        let geometry = SourceGeometry::new(
            [area().x, area().y, area().width, area().height],
            [256, 256],
        )
        .unwrap();
        source.set_geometry(geometry).unwrap();
        let mut grade = make(
            calibration,
            roll,
            snapshot(PhotoSettings::default(), 0, false),
            DMat3::IDENTITY,
        );
        run(
            &device,
            &queue,
            &kernel,
            &probe,
            &source,
            &grade,
            Some(&[BUILD]),
        );
        run(&device, &queue, &kernel, &probe, &source, &grade, None);
        let reference = read_words(&device, &queue, &probe.output);
        run(
            &device,
            &queue,
            &kernel,
            &probe,
            &source,
            &grade,
            Some(&[SUFFIX]),
        );
        exact(
            "log new camera matrix",
            "warm",
            &reference,
            &read_words(&device, &queue, &probe.output),
        );
        grade.parameters = ProcessingParameters::source_inspection(2.).unwrap();
        run(&device, &queue, &kernel, &probe, &source, &grade, None);
        let reference = read_words(&device, &queue, &probe.output);
        run(
            &device,
            &queue,
            &kernel,
            &probe,
            &source,
            &grade,
            Some(&[SUFFIX]),
        );
        exact(
            "log inspection",
            "bypass",
            &reference,
            &read_words(&device, &queue, &probe.output),
        );
    });
}

#[test]
fn log_prefix_keys_track_only_upstream_dependencies() {
    pollster::block_on(async {
        let (device, _queue, _kernel, _probe, mut source, calibration, _) =
            setup([8, 8], false).await;
        let roll = RollSettings::new_grade();
        let p = |calibration, roll, photo, matrix| {
            ProcessingParameters::positive(calibration, roll, photo, matrix).unwrap()
        };
        let photo = PhotoSettings::default();
        let base = p(calibration, roll, photo, camera_matrix());
        let key = LogPrefixKey::new([1, 0], &source, base);
        for (roll, photo, matrix) in [
            (
                roll,
                PhotoSettings {
                    exposure: 2.,
                    ..photo
                },
                camera_matrix(),
            ),
            (
                RollSettings {
                    exposure: -2.,
                    ..roll
                },
                photo,
                camera_matrix(),
            ),
            (
                RollSettings {
                    wb: photographic().wb,
                    ..roll
                },
                photographic(),
                camera_matrix(),
            ),
            (roll, photo, DMat3::IDENTITY),
        ] {
            assert_eq!(
                key,
                LogPrefixKey::new([1, 0], &source, p(calibration, roll, photo, matrix))
            );
        }
        for (calibration, roll) in [
            (
                Calibration {
                    dmin: calibration.dmin * 1.01,
                    ..calibration
                },
                roll,
            ),
            (
                Calibration {
                    dmax: calibration.dmax * 0.99,
                    ..calibration
                },
                roll,
            ),
            (
                Calibration {
                    input_floor: calibration.input_floor * 2.,
                    ..calibration
                },
                roll,
            ),
            (
                calibration,
                RollSettings {
                    gamma: 0.65,
                    ..roll
                },
            ),
            (
                calibration,
                RollSettings {
                    response: Response::Original,
                    ..roll
                },
            ),
        ] {
            assert_ne!(
                key,
                LogPrefixKey::new(
                    [1, 0],
                    &source,
                    p(calibration, roll, photo, camera_matrix())
                )
            );
        }
        for stamp in [[2, 0], [1, 1]] {
            assert_ne!(key, LogPrefixKey::new(stamp, &source, base));
        }
        let field = source.flat_field.as_ref().unwrap().field();
        let changed = Arc::new(
            crate::flat_field::FlatField::from_nodes(
                field.area(),
                field.size(),
                vec![DVec3::splat(1.1); 4],
            )
            .unwrap(),
        );
        source
            .set_flat_field(
                Some(Arc::new(GpuFlatField::new(&device, changed).unwrap())),
                source.geometry.unwrap(),
            )
            .unwrap();
        assert_ne!(key, LogPrefixKey::new([1, 0], &source, base));
        let geometry = SourceGeometry::new([100, 200, 1024, 256], [8, 8]).unwrap();
        source.set_geometry(geometry).unwrap();
        assert_ne!(key, LogPrefixKey::new([1, 0], &source, base));
    });
}

#[test]
fn evicted_log_prefixes_survive_deferred_and_abandoned_encoders() {
    pollster::block_on(async {
        let (device, queue, kernel, mut probe, source, calibration, _) = setup(SIZE, false).await;
        let grade = |gamma| {
            GpuPhotoGrade::new(
                &device,
                calibration,
                RollSettings {
                    gamma,
                    ..RollSettings::new_grade()
                },
                snapshot(PhotoSettings::default(), 0, false),
                camera_matrix(),
                area(),
            )
            .unwrap()
        };
        let first = grade(0.60);
        run(&device, &queue, &kernel, &probe, &source, &first, None);
        let expected = read_words(&device, &queue, &probe.output);
        let bytes = probe.prefix.size();
        let baseline = allocated_resource_bytes(&device).unwrap();
        let fresh = || {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("immutable experimental log prefix version"),
                size: bytes,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })
        };
        let initialize = |probe: &Probe, grade: &GpuPhotoGrade| {
            grade.require_source(&source).unwrap();
            let mut encoder = device.create_command_encoder(&Default::default());
            probe.encode(&device, &mut encoder, &source, grade, &[BUILD]);
            // Submit initialization before exposing a version to any suffix encoder.
            queue.submit([encoder.finish()])
        };
        initialize(&probe, &first);
        let mut deferred = device.create_command_encoder(&Default::default());
        probe.encode(&device, &mut deferred, &source, &first, &[SUFFIX]);
        probe.prefix = fresh();
        let second = grade(0.65);
        initialize(&probe, &second);
        let mut abandoned = device.create_command_encoder(&Default::default());
        probe.encode(&device, &mut abandoned, &source, &second, &[SUFFIX]);
        probe.prefix = fresh();
        let last = initialize(&probe, &grade(0.70));
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(last),
                timeout: None,
            })
            .unwrap();
        let three = allocated_resource_bytes(&device).unwrap();
        assert!(three >= baseline + 2 * bytes as usize);
        drop(abandoned);
        device.poll(wgpu::PollType::Poll).unwrap();
        let two = allocated_resource_bytes(&device).unwrap();
        assert!(three - two >= bytes as usize);
        let submitted = queue.submit([deferred.finish()]);
        let retained = allocated_resource_bytes(&device).unwrap();
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submitted),
                timeout: None,
            })
            .unwrap();
        let one = allocated_resource_bytes(&device).unwrap();
        assert!(two - one >= bytes as usize);
        exact(
            "evicted log prefix",
            "late consumer after newer initialization",
            &expected,
            &read_words(&device, &queue, &probe.output),
        );
        println!(
            "log_prefix_lifetime prefix_bytes={bytes} metal_baseline_bytes={baseline} three_versions_bytes={three} after_abandon_bytes={two} after_submit_bytes={retained} after_completion_bytes={one} excludes=cpu_driver_nonresource_memory"
        );
    });
}

#[test]
#[ignore = "normal-detail corrected pre-exposure prefix paired timing and full-raster parity"]
fn corrected_exposure_independent_prefix_performance() {
    pollster::block_on(async {
        let (device, queue, kernel, probe, source, calibration, _) = setup(SIZE, false).await;
        let roll = RollSettings::new_grade();
        let base = GpuPhotoGrade::new(
            &device,
            calibration,
            roll,
            snapshot(PhotoSettings::default(), 0, false),
            camera_matrix(),
            area(),
        )
        .unwrap();
        let key = LogPrefixKey::new([1, 0], &source, base.parameters);
        println!(
            "log_prefix bytes={} samples=9 timing=encode_submit_gpu_completion excludes=upload_compilation_readback_aces_reconstruction_presentation",
            probe.prefix.size()
        );
        for (name, photo, locals) in [
            (
                "exposure-minus-two",
                PhotoSettings {
                    exposure: -2.,
                    ..Default::default()
                },
                0,
            ),
            ("exposure-zero", PhotoSettings::default(), 0),
            (
                "exposure-plus-two",
                PhotoSettings {
                    exposure: 2.,
                    ..Default::default()
                },
                0,
            ),
            ("global-photographic", photographic(), 0),
            ("two-photographic-locals", PhotoSettings::default(), 2),
        ] {
            let grade = GpuPhotoGrade::new(
                &device,
                calibration,
                roll,
                snapshot(photo, locals, true),
                camera_matrix(),
                area(),
            )
            .unwrap();
            assert_eq!(key, LogPrefixKey::new([1, 0], &source, grade.parameters));
            run(&device, &queue, &kernel, &probe, &source, &grade, None);
            let reference = read_words(&device, &queue, &probe.output);
            run(
                &device,
                &queue,
                &kernel,
                &probe,
                &source,
                &base,
                Some(&[BUILD]),
            );
            run(
                &device,
                &queue,
                &kernel,
                &probe,
                &source,
                &grade,
                Some(&[SUFFIX]),
            );
            exact(
                name,
                "log warm",
                &reference,
                &read_words(&device, &queue, &probe.output),
            );
            run(
                &device,
                &queue,
                &kernel,
                &probe,
                &source,
                &grade,
                Some(&[BUILD, SUFFIX]),
            );
            exact(
                name,
                "log rebuild",
                &reference,
                &read_words(&device, &queue, &probe.output),
            );
            let mut full = Vec::new();
            let mut warm = Vec::new();
            let mut rebuild = Vec::new();
            for i in 0..9 {
                let order = if i % 2 == 0 { [0, 1, 2] } else { [2, 1, 0] };
                for variant in order {
                    let (values, steps) = match variant {
                        0 => (&mut full, None),
                        1 => (&mut warm, Some(&[SUFFIX][..])),
                        _ => (&mut rebuild, Some(&[BUILD, SUFFIX][..])),
                    };
                    values.push(run(
                        &device, &queue, &kernel, &probe, &source, &grade, steps,
                    ));
                }
            }
            // A post-inversion cut is faster for WB/creative edits, but exposure invalidates it.
            run(
                &device,
                &queue,
                &kernel,
                &probe,
                &source,
                &grade,
                Some(&[2, 3]),
            );
            exact(
                name,
                "camera warm",
                &reference,
                &read_words(&device, &queue, &probe.output),
            );
            let camera: Vec<_> = (0..9)
                .map(|_| {
                    run(
                        &device,
                        &queue,
                        &kernel,
                        &probe,
                        &source,
                        &grade,
                        Some(&[3]),
                    )
                })
                .collect();
            println!(
                "log_prefix case={name} full_ms={:.3} warm_ms={:.3} rebuild_ms={:.3} camera_warm_ms={:.3} camera_warm_excludes_exposure_edits=true",
                median(&full),
                median(&warm),
                median(&rebuild),
                median(&camera)
            );
        }
    });
}
