//! Bounded local-stack ownership observation; not pressure or native-presentation acceptance.
use super::*;
use crate::{
    cache_observation as observation,
    geometry::SourcePoint,
    local_adjustments::{GradientMask, LocalAdjustment},
};

#[test]
#[ignore = "bounded 1080p local working-view resources with existing pressure/swap/growth stops"]
fn physical_view_local_stack_resources() {
    observation::with_monitor(exercise).unwrap();
}

fn snapshot(scan: &Scan, local: bool, edit: u32) -> Result<PhotoGradeSnapshot, Error> {
    let area = scan.area;
    let (x, y, w, h) = (
        f64::from(area.x),
        f64::from(area.y),
        f64::from(area.width),
        f64::from(area.height),
    );
    let masks = [
        GradientMask::Linear {
            start: SourcePoint([x, y]),
            end: SourcePoint([x + w, y + h]),
        },
        GradientMask::Radial {
            center: SourcePoint([x + w * 0.5, y + h * 0.5]),
            radii: [w * 0.5, h * 0.7],
            angle: 0.2 + f64::from(edit.saturating_sub(1)) * 0.01,
            feather: 0.8,
        },
    ];
    PhotoGradeSnapshot::new(
        PhotoSettings::default(),
        if local {
            masks
                .into_iter()
                .enumerate()
                .map(|(i, mask)| LocalAdjustment {
                    mask,
                    photo: PhotoSettings {
                        exposure: if i == 0 {
                            0.25 + f64::from(edit.min(1)) * 0.025
                        } else {
                            -0.125
                        },
                        contrast: if i == 0 { 1.01 } else { 1. },
                        saturation: if i == 1 { 1.02 } else { 1. },
                        ..Default::default()
                    },
                    enabled: true,
                    invert: i == 1,
                    strength: 0.6,
                })
                .collect()
        } else {
            vec![]
        },
    )
}

fn checkpoint(device: &wgpu::Device, name: &str) {
    println!(
        "local_resources checkpoint={name} native_bytes={:?}",
        crate::gpu::allocated_resource_bytes(device)
    );
}

fn exercise(
    monitor: &Arc<observation::Monitor>,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> Result<(), String> {
    let (renderer, kernels) = monitor.phase("local-infrastructure", || {
        pollster::block_on(async {
            Ok::<_, Error>((
                crate::renderer::GpuRenderer::new(device, queue).await?,
                ViewKernels::new(device).await?,
            ))
        })
    })?;
    let private_raw = std::env::var_os("ANTITONE_SCREEN_RAW").is_some();
    let (mut scan, basis) = if private_raw {
        let root = std::path::PathBuf::from(
            std::env::var_os("ANTITONE_RAW_SAMPLES").ok_or("set absolute ANTITONE_RAW_SAMPLES")?,
        );
        if !root.is_absolute() {
            return Err("sample path must be absolute".into());
        }
        for name in ["DSC04378.ARW", "DSC04397.ARW", "DSC04524.ARW"] {
            let raw =
                crate::raw::RawFile::open(&root.join(name)).map_err(|error| error.to_string())?;
            observation::source_bound(&raw.metadata)?;
        }
        monitor.phase("read-only-native-source", || {
            Ok::<_, Error>(crate::raw_test_support::train_raw::load())
        })?
    } else {
        (
            monitor.phase("authored-source", || Ok::<_, Error>(scan()))?,
            calibration(),
        )
    };
    let gate = SubmissionGate::default();
    let context = egui::Context::default();
    println!(
        "local_resources output=1920x1080 private_raw={private_raw} native_source_bytes={:?} excludes=native_host_scopes isolated_timings=false",
        scan.source().retained_bytes()
    );
    for corrected in [false, true] {
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
        let (cal, sources) = monitor.phase("local-acquisition", || {
            scan.set_flat_field(field.clone())?;
            let cal = scan.processing_calibration(basis)?;
            let mut frame = frame(&scan);
            frame.output = [1920, 1080];
            let spec = ViewSourceSpec::new(1, frame, cal, scan.camera_to_ap1, field.clone())?;
            let specs = vec![spec.clone(), spec.underlay()?];
            let gpu_field = field
                .as_ref()
                .map(|f| GpuFlatField::new(device, f.clone()).map(Arc::new))
                .transpose()?;
            let sources = ViewSource::acquire_many(
                device,
                queue,
                &gate,
                renderer.clone(),
                kernels.clone(),
                &scan,
                gpu_field,
                specs,
                || false,
            )?
            .ok_or(Error::UnusableSample("local acquisition cancelled"))?;
            Ok::<_, Error>((cal, sources))
        })?;
        let detail = sources[0].clone();
        let underlay = sources[1].clone();
        drop(sources);
        let owners = [Arc::downgrade(&detail), Arc::downgrade(&underlay)];
        let mut accepted: Option<(Arc<RenderedView>, Arc<RenderedView>, Arc<RenderedView>)> = None;
        let mut encoded: Option<(wgpu::CommandBuffer, wgpu::Texture)> = None;
        for (case, local, edit, target) in [
            ("global", false, 0, OutputTarget::DisplayP3),
            ("locals", true, 0, OutputTarget::DisplayP3),
            ("numeric", true, 1, OutputTarget::DisplayP3),
            ("geometry", true, 2, OutputTarget::DisplayP3),
            ("target", true, 2, OutputTarget::Srgb),
        ] {
            let previous = accepted.take();
            let next = monitor.phase(case, || {
                let snapshot = snapshot(&scan, local, edit)?;
                let input: ProcessingInput = if local {
                    Arc::new(GpuPhotoGrade::new(
                        device,
                        cal,
                        RollSettings::default(),
                        snapshot.clone(),
                        scan.camera_to_ap1,
                        scan.area,
                    )?)
                    .into()
                } else {
                    processing_input(cal, RollSettings::default(), snapshot.global(), &scan)
                };
                let make = |source, old| {
                    RenderedView::new(
                        device,
                        source,
                        cal,
                        RollSettings::default(),
                        snapshot.clone(),
                        &input,
                        target,
                        old,
                    )
                };
                let next = make(detail.clone(), previous.as_ref().map(|p| &p.0))?;
                let background = make(underlay.clone(), previous.as_ref().map(|p| &p.1))?;
                let visible = RenderedView::composite(
                    device,
                    next.clone(),
                    background.clone(),
                    detail.spec.frame,
                    previous.as_ref().map(|p| &p.2),
                )?;
                if case == "target" {
                    assert!(Arc::ptr_eq(
                        next.graded.as_ref().unwrap(),
                        previous.as_ref().unwrap().0.graded.as_ref().unwrap()
                    ));
                }
                Ok::<_, Error>((next, background, visible))
            })?;
            checkpoint(device, "current-building-encoded");
            next.2.ensure_submitted(device, queue, &gate, &context);
            if let Some((command, texture)) = encoded.take() {
                gate.submit_with_completion(queue, [command], || {});
                drop(texture);
            }
            drop(previous);
            checkpoint(device, "submitted-old-owners-retired");
            monitor.phase("local-gpu-complete", || observation::wait(device, queue))?;
            assert!(!next.2.in_flight());
            assert!(!gate.pending());
            // A deferred copy exercises an encoded consumer's native lifetime, not host presentation.
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("deferred local observation"),
                size: next.2.image.linear.size(),
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: next.2.image.linear.format(),
                usage: wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            encoder.copy_texture_to_texture(
                next.2.image.linear.as_image_copy(),
                texture.as_image_copy(),
                texture.size(),
            );
            encoded = Some((encoder.finish(), texture));
            accepted = Some(next);
            println!("local_resources corrected={corrected} case={case}");
        }
        drop(accepted);
        drop(detail);
        drop(underlay);
        assert!(owners.iter().all(|owner| owner.upgrade().is_none()));
        checkpoint(device, "source-owners-retired-encoded-live");
        if let Some((command, texture)) = encoded.take() {
            gate.submit_with_completion(queue, [command], || {});
            drop(texture);
        }
        monitor.phase("local-retirement", || observation::wait(device, queue))?;
        assert!(!gate.pending());
        checkpoint(device, "completed-retired");
    }
    drop(renderer);
    drop(kernels);
    monitor.phase("local-infrastructure-retired", || {
        observation::wait(device, queue)
    })?;
    checkpoint(device, "infrastructure-retired");
    Ok(())
}
