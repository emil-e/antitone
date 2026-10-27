use super::*;
use crate::adjustment_layers::AdjustmentLayerSettings;
use crate::processing::Calibration;
use crate::{
    color::Balance,
    geometry::{Composition, FramePoint},
    viewer::readback::image_pixels as image,
};
use std::sync::atomic::AtomicU64;

#[cfg(target_os = "macos")]
#[path = "gpu_working_view_adjustment_layer_resource_tests.rs"]
mod adjustment_layer_resources;
#[path = "gpu_working_view_colour_tests.rs"]
mod colour;
#[path = "gpu_working_view_identity_tests.rs"]
mod identity;
#[path = "gpu_working_view_inspection_tests.rs"]
mod inspection;
#[path = "gpu_working_view_performance_tests.rs"]
mod performance;
#[path = "gpu_repair_view_tests.rs"]
mod repairs;
#[path = "gpu_working_view_source_scale_tests.rs"]
mod source_scale;
#[path = "gpu_working_view_underlay_tests.rs"]
mod underlay;

async fn context() -> (
    wgpu::Device,
    wgpu::Queue,
    Arc<crate::renderer::GpuRenderer>,
    Arc<ViewKernels>,
) {
    let instance = crate::gpu::instance();
    let adapter = instance.request_adapter(&Default::default()).await.unwrap();
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            required_limits: crate::platform::device_limits(adapter.limits()),
            ..Default::default()
        })
        .await
        .unwrap();
    let renderer = crate::renderer::GpuRenderer::new(&device, &queue)
        .await
        .unwrap();
    let kernels = ViewKernels::new(&device).await.unwrap();
    (device, queue, renderer, kernels)
}

fn scan() -> Scan {
    Scan::open(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng"),
        None,
    )
    .unwrap()
}

fn calibration() -> Calibration {
    Calibration {
        dmin: DVec3::new(0.19, 0.14, 0.12),
        dmax: DVec3::new(0.008, 0.004, 0.002),
        input_floor: crate::processing::CAMERA_INPUT_FLOOR,
    }
}

pub(super) fn negative(gamma: f64) -> InputSettings {
    let calibration = calibration();
    InputSettings::Negative {
        dmin: Some(calibration.dmin),
        dmax: Some(calibration.dmax),
        input_floor: calibration.input_floor,
        response: crate::processing::Response::film(),
        gamma,
        balance: crate::color::Balance::default(),
    }
}

fn frame(scan: &Scan) -> FrameRaster {
    FrameRaster::full(Composition::new(scan.area, Default::default(), Default::default()).unwrap())
        .unwrap()
}

fn processing_input(
    device: &wgpu::Device,
    input: impl Into<InputSettings>,
    roll: RollSettings,
    photo: AdjustmentLayerSettings,
    scan: &Scan,
) -> ProcessingInput {
    crate::gpu::GpuAdjustmentLayers::new(
        device,
        input,
        roll,
        AdjustmentLayerSnapshot::new(vec![crate::adjustment_layers::AdjustmentLayer::global(
            photo,
        )])
        .unwrap(),
        scan.camera_to_ap1,
        scan.area,
    )
    .map(Arc::new)
    .unwrap()
    .into()
}

fn read(device: &wgpu::Device, queue: &wgpu::Queue, source: &wgpu::Buffer) -> Vec<[f32; 4]> {
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: source.size(),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(source, 0, &staging, 0, source.size());
    let submission = queue.submit([encoder.finish()]);
    let (send, receive) = std::sync::mpsc::channel();
    staging.map_async(wgpu::MapMode::Read, .., move |r| send.send(r).unwrap());
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })
        .unwrap();
    receive.recv().unwrap().unwrap();
    let mapped = staging.get_mapped_range(..).unwrap();
    let result = bytemuck::cast_slice(&mapped).to_vec();
    drop(mapped);
    staging.unmap();
    result
}

#[test]
fn acquisition_keys_track_authority_response_correction_and_view_but_not_grading() {
    let scan = scan();
    let frame = frame(&scan);
    let settings = negative(0.6);
    let spec = ViewSourceSpec::new(7, frame, settings, scan.camera_to_ap1, None).unwrap();
    let key = |frame, settings: &InputSettings, field| {
        ViewSourceSpec::new(7, frame, settings, scan.camera_to_ap1, field).unwrap()
    };
    assert!(spec == key(frame, &settings, None));
    let mut gamma_changed = settings;
    if let InputSettings::Negative { gamma, .. } = &mut gamma_changed {
        *gamma += 0.01;
    }
    assert!(spec != key(frame, &gamma_changed, None));
    let mut calibration_changed = settings;
    if let InputSettings::Negative {
        dmin: Some(dmin), ..
    } = &mut calibration_changed
    {
        *dmin *= 1.01;
    }
    assert!(spec != key(frame, &calibration_changed, None));
    let mut changed = spec.clone();
    changed.authority += 1;
    assert!(spec != changed);
    let changed = key(
        FrameRaster {
            output: frame.output.map(|v| v * 2),
            ..frame
        },
        &settings,
        None,
    );
    assert!(spec != changed);
    assert!(spec.same_source(&changed));
    let field = Arc::new(
        crate::flat_field::FlatField::from_nodes(
            [scan.area.x, scan.area.y, scan.area.width, scan.area.height],
            [2, 2],
            vec![DVec3::splat(1.1); 4],
        )
        .unwrap(),
    );
    assert!(spec != key(frame, &settings, Some(field)));
}

#[test]
fn physical_view_acquisition_and_publication_preserve_original_reference_and_target_reuse() {
    pollster::block_on(async {
        let (device, queue, renderer, kernels) = context().await;
        let gate = SubmissionGate::default();
        let context = egui::Context::default();
        for corrected in [false, true] {
            let mut scan = scan();
            let field = corrected.then(|| {
                Arc::new(
                    crate::flat_field::FlatField::from_nodes(
                        [scan.area.x, scan.area.y, scan.area.width, scan.area.height],
                        [2, 2],
                        vec![
                            DVec3::new(0.8, 0.9, 1.),
                            DVec3::new(1.2, 1.1, 1.05),
                            DVec3::new(1., 1.1, 0.9),
                            DVec3::new(1.1, 0.95, 1.2),
                        ],
                    )
                    .unwrap(),
                )
            });
            scan.set_flat_field(field.clone()).unwrap();
            let cal = field
                .as_ref()
                .map_or(Ok(calibration()), |f| {
                    f.processing_calibration(calibration())
                })
                .unwrap();
            let gpu_field = field
                .as_ref()
                .map(|f| Arc::new(GpuFlatField::new(&device, f.clone()).unwrap()));
            let frame = if corrected {
                FrameRaster::full(
                    Composition::new(
                        scan.area,
                        Default::default(),
                        crate::geometry::PhotoOrientation {
                            turns: crate::geometry::QuarterTurns::One,
                            ..Default::default()
                        },
                    )
                    .unwrap(),
                )
                .unwrap()
            } else {
                frame(&scan)
            };
            let roll = RollSettings::default();
            let spec = ViewSourceSpec::new(7, frame, cal, scan.camera_to_ap1, field).unwrap();
            let source = ViewSource::acquire(
                &device,
                &queue,
                &gate,
                renderer.clone(),
                kernels.clone(),
                &scan,
                gpu_field.clone(),
                spec.clone(),
                || false,
            )
            .unwrap()
            .unwrap();
            let original = read(&device, &queue, &source.pixels);
            assert!(original.iter().all(|p| (p[3] as u32 & 8) != 0));
            let photo = AdjustmentLayerSettings {
                exposure: 0.7,
                wb: Balance {
                    warmth: 0.2,
                    tint: -0.1,
                },
                contrast: 1.2,
                saturation: 1.3,
                ..Default::default()
            };
            let snapshot = AdjustmentLayerSnapshot::new(vec![
                crate::adjustment_layers::AdjustmentLayer::global(photo),
            ])
            .unwrap();
            let input = processing_input(&device, cal, roll, photo, &scan);
            let view = RenderedView::new(
                &device,
                source.clone(),
                cal,
                roll,
                snapshot.clone(),
                &input,
                OutputTarget::Srgb,
                None,
            )
            .unwrap();
            assert!(!view.in_flight());
            let buffer = view.graded.as_ref().unwrap().pixels.clone();
            view.ensure_submitted(&device, &queue, &gate, &context);
            view.ensure_submitted(&device, &queue, &gate, &context);
            let actual = image(&device, &queue, &view.image);
            assert!(!view.in_flight());
            assert!(!gate.pending());
            let generation = AtomicU64::new(9);
            let reference = crate::refinement::refine(
                &renderer,
                &device,
                (&queue, &gate),
                (&scan, gpu_field.clone()),
                input.clone(),
                OutputTarget::Srgb,
                (&generation, 9),
                crate::refinement::ViewRequest {
                    area: frame.bounds().unwrap(),
                    output: frame.output,
                    frame: Some(frame),
                },
            )
            .unwrap()
            .unwrap();
            let expected = image(&device, &queue, &reference);
            let mut maximum = 0f32;
            for (a, b) in actual.iter().zip(expected) {
                for c in 0..3 {
                    assert!(a[c].is_finite() && b[c].is_finite());
                    maximum = maximum.max((a[c] - b[c]).abs());
                }
                assert_eq!(a[3], b[3]);
            }
            assert!(
                maximum <= 2e-5,
                "corrected={corrected} rendered max={maximum}"
            );
            let p3 = RenderedView::new(
                &device,
                source.clone(),
                cal,
                roll,
                snapshot.clone(),
                &input,
                OutputTarget::DisplayP3,
                Some(&view),
            )
            .unwrap();
            assert_eq!(buffer, p3.graded.as_ref().unwrap().pixels);
            p3.ensure_submitted(&device, &queue, &gate, &context);
            image(&device, &queue, &p3.image);
            assert_eq!(original, read(&device, &queue, &source.pixels));
            let other = AdjustmentLayerSnapshot::new(vec![
                crate::adjustment_layers::AdjustmentLayer::global(AdjustmentLayerSettings {
                    exposure: 1.3,
                    ..photo
                }),
            ])
            .unwrap();
            assert!(
                RenderedView::new(
                    &device,
                    source.clone(),
                    cal,
                    roll,
                    other,
                    &input,
                    OutputTarget::DisplayP3,
                    Some(&p3)
                )
                .is_err()
            );
            let changed_cal = Calibration {
                dmin: cal.dmin * 1.01,
                ..cal
            };
            let changed_input = processing_input(&device, changed_cal, roll, photo, &scan);
            assert!(
                RenderedView::new(
                    &device,
                    source.clone(),
                    changed_cal,
                    roll,
                    snapshot.clone(),
                    &changed_input,
                    OutputTarget::DisplayP3,
                    Some(&p3)
                )
                .is_err()
            );
            let mut zoomed = frame;
            zoomed.min = FramePoint(frame.max.0.map(|v| v * 0.25));
            zoomed.max = FramePoint(frame.max.0.map(|v| v * 0.75));
            let mapping = view.mapping(zoomed);
            assert_eq!(mapping.origin, [0.25; 2]);
            assert_eq!(mapping.x_axis, [0.5, 0.]);
            assert_eq!(mapping.y_axis, [0., 0.5]);
            // A target raster's coverage is not its inversion-floor flags. In particular, scopes
            // must neither count an unsupported pixel nor compose the already-rotated raster again.
            let mut partial = original.clone();
            partial.last_mut().unwrap()[3] = 0.;
            let partial = Arc::new(ViewSource {
                spec: source.spec.clone(),
                renderer: renderer.clone(),
                kernels: kernels.clone(),
                pixels: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: None,
                    contents: bytemuck::cast_slice(&partial),
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                }),
            });
            let partial = RenderedView::new(
                &device,
                partial,
                cal,
                roll,
                snapshot,
                &input,
                OutputTarget::Srgb,
                None,
            )
            .unwrap();
            partial.ensure_submitted(&device, &queue, &gate, &context);
            let actual = image(&device, &queue, &partial.image);
            assert_eq!(actual.last().unwrap()[3], 0.);
            let gpu = Arc::new(crate::gpu_scopes::GpuScopes::new(&device).await.unwrap());
            let settings = crate::scopes::Settings {
                target: OutputTarget::Srgb,
                ..Default::default()
            };
            let paint = crate::gpu_scopes::ScopePaint::rendered(
                gpu.clone(),
                partial.clone(),
                frame,
                settings,
                gate.clone(),
                context.clone(),
            );
            let mut encoder = device.create_command_encoder(&Default::default());
            egui_wgpu::CallbackTrait::prepare(
                &paint,
                &device,
                &queue,
                &egui_wgpu::ScreenDescriptor {
                    size_in_pixels: frame.output,
                    pixels_per_point: 1.,
                },
                &mut encoder,
                &mut Default::default(),
            );
            gate.submit(&queue, [encoder.finish()]);
            let totals = read(&device, &queue, &gpu.totals);
            let words: Vec<u32> = totals.iter().flatten().map(|v| v.to_bits()).collect();
            let (expected_bins, expected_columns, expected_counts) =
                crate::scopes::reference(frame.output, &actual, settings).unwrap();
            assert_eq!(
                gpu.status_rendered(&partial, frame, settings)
                    .unwrap()
                    .unwrap(),
                expected_counts
            );
            assert_eq!(expected_counts.valid, frame.output[0] * frame.output[1] - 1);
            assert_eq!(&words[4..], &expected_columns);
            let bins: Vec<u32> = read(&device, &queue, &gpu.bins)
                .iter()
                .flatten()
                .map(|v| v.to_bits())
                .collect();
            assert_eq!(bins, expected_bins);
            assert_eq!(gpu.reduction_count(), 1);
            let calls = std::cell::Cell::new(0);
            let cancelled = ViewSource::acquire(
                &device,
                &queue,
                &gate,
                renderer.clone(),
                kernels.clone(),
                &scan,
                gpu_field,
                spec,
                || {
                    calls.set(calls.get() + 1);
                    calls.get() > 3
                },
            )
            .unwrap();
            assert!(cancelled.is_none());
            assert_eq!(original, read(&device, &queue, &source.pixels));
        }
    });
}
