use super::*;
use std::time::Instant;

/// Production GPU ownership/dispatch probe, not editor input-to-display or aggregate admission.
#[test]
#[ignore = "explicit physical-view GPU timing and native-retirement diagnostic"]
fn physical_view_gpu_cost_and_retirement() {
    cost_and_retirement(false);
}

#[test]
#[ignore = "explicit underlay/composition GPU timing and native-retirement diagnostic"]
fn physical_view_underlay_gpu_cost_and_retirement() {
    cost_and_retirement(true);
}

fn cost_and_retirement(with_underlay: bool) {
    pollster::block_on(async {
        let output = match std::env::var("ANTITONE_SCREEN_PERF_SIZE").as_deref() {
            Ok("4k") => [3840, 2160],
            Ok("5k") => [5120, 2880],
            _ => [1920, 1080],
        };
        let (device, queue, renderer, kernels) = context().await;
        let gate = crate::gpu::SubmissionGate::default();
        let (mut scan, cal) = if std::env::var_os("ANTITONE_SCREEN_RAW").is_some() {
            crate::raw_test_support::train_raw::load()
        } else {
            (scan(), calibration())
        };
        let corrected = std::env::var_os("ANTITONE_SCREEN_CORRECTED").is_some();
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
            .map_or(Ok(cal), |f| f.processing_calibration(cal))
            .unwrap();
        let gpu_field = field
            .as_ref()
            .map(|f| Arc::new(GpuFlatField::new(&device, f.clone()).unwrap()));
        let composition =
            Composition::new(scan.area, Default::default(), Default::default()).unwrap();
        let height = f64::from(scan.area.width) * f64::from(output[1]) / f64::from(output[0]);
        let top = (f64::from(scan.area.height) - height) / 2.;
        assert!(top >= 0.);
        let frame = FrameRaster::new(
            composition,
            FramePoint([0., top]),
            FramePoint([f64::from(scan.area.width), top + height]),
            output,
        )
        .unwrap();
        let roll = RollSettings::new_grade();
        let native = || crate::gpu::allocated_resource_bytes(&device);
        eprintln!(
            "physical_view_gpu start output={output:?} corrected={corrected} underlay={with_underlay} correction=synthetic_2x2 native_bytes={:?}",
            native()
        );
        let start = Instant::now();
        let spec = ViewSourceSpec::new(1, frame, cal, roll, scan.camera_to_ap1, field).unwrap();
        let specs = if with_underlay {
            vec![spec.clone(), spec.underlay().unwrap()]
        } else {
            vec![spec]
        };
        let mut sources = ViewSource::acquire_many(
            &device,
            &queue,
            &gate,
            renderer,
            kernels,
            &scan,
            gpu_field,
            specs,
            || false,
        )
        .unwrap()
        .unwrap();
        let source = sources.remove(0);
        let underlay = sources.pop();
        eprintln!(
            "physical_view_gpu retained_source_bytes={} underlay_source_bytes={}",
            source.pixels.size(),
            underlay.as_ref().map_or(0, |u| u.pixels.size())
        );
        eprintln!(
            "physical_view_gpu acquired milliseconds={} native_bytes={:?}",
            start.elapsed().as_secs_f64() * 1000.,
            native()
        );
        let mut accepted: Option<Arc<RenderedView>> = None;
        let mut accepted_underlay: Option<Arc<RenderedView>> = None;
        let mut displayed: Option<Arc<RenderedView>> = None;
        let mut peak = native();
        let context = egui::Context::default();
        let cases = if with_underlay {
            vec![
                "fixed-view",
                "exposure",
                "wb",
                "nonlinear",
                "target-only",
                "navigation-only",
            ]
        } else {
            vec!["exposure", "wb", "nonlinear", "target-only"]
        };
        for case in cases {
            let mut times = Vec::new();
            for edit in 0..6 {
                let previous = accepted.take();
                let previous_underlay = accepted_underlay.take();
                let previous_displayed = displayed.take();
                let photo = if case == "target-only" || case == "navigation-only" {
                    previous
                        .as_ref()
                        .unwrap()
                        .graded
                        .as_ref()
                        .unwrap()
                        .grade
                        .as_ref()
                        .unwrap()
                        .snapshot()
                        .global()
                } else {
                    PhotoSettings {
                        exposure: 0.25 * f64::from(edit),
                        wb: if case == "wb" {
                            Balance {
                                warmth: 0.05 * f64::from(edit),
                                tint: -0.1,
                            }
                        } else {
                            Balance::default()
                        },
                        contrast: if case == "nonlinear" { 1.2 } else { 1. },
                        saturation: if case == "nonlinear" { 1.3 } else { 1. },
                        ..Default::default()
                    }
                };
                let input = processing_input(cal, roll, photo, &scan);
                let target = if case == "target-only" && edit % 2 == 0 {
                    OutputTarget::Srgb
                } else {
                    OutputTarget::DisplayP3
                };
                let start = Instant::now();
                let make = |s: Arc<ViewSource>, old: Option<&Arc<RenderedView>>| {
                    if let Some(old) =
                        old.filter(|r| r.matches(&s.spec, &input.signature(), target))
                    {
                        return old.clone();
                    }
                    RenderedView::new(
                        &device,
                        s,
                        cal,
                        roll,
                        PhotoGradeSnapshot::new(photo, vec![]).unwrap(),
                        &input,
                        target,
                        old,
                    )
                    .unwrap()
                };
                let next = make(source.clone(), previous.as_ref());
                let background = underlay
                    .as_ref()
                    .map(|s| make(s.clone(), previous_underlay.as_ref()));
                let mut requested = frame;
                if case != "fixed-view" && with_underlay {
                    let shift = if case == "navigation-only" {
                        top * f64::from(edit + 1) / 8.
                    } else {
                        top / 2.
                    };
                    requested.min.0[1] += shift;
                    requested.max.0[1] += shift;
                }
                let visible = background.as_ref().map_or_else(
                    || next.clone(),
                    |b| {
                        RenderedView::composite(
                            &device,
                            next.clone(),
                            b.clone(),
                            requested,
                            previous_displayed.as_ref(),
                        )
                        .unwrap()
                    },
                );
                if case == "target-only" {
                    assert!(Arc::ptr_eq(
                        next.graded.as_ref().unwrap(),
                        previous.as_ref().unwrap().graded.as_ref().unwrap()
                    ));
                }
                visible.ensure_submitted(&device, &queue, &gate, &context);
                peak = peak.max(native());
                device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
                assert!(!visible.in_flight());
                let milliseconds = start.elapsed().as_secs_f64() * 1000.;
                eprintln!(
                    "physical_view_gpu case={case} edit={edit} milliseconds={milliseconds} native_bytes={:?}",
                    native()
                );
                if edit > 0 {
                    times.push(milliseconds);
                }
                accepted = Some(next);
                accepted_underlay = background;
                displayed = Some(visible);
            }
            times.sort_by(f64::total_cmp);
            eprintln!(
                "physical_view_gpu median case={case} milliseconds={} native_checkpoint_peak_bytes={peak:?}",
                times[times.len() / 2]
            );
        }
        // No callback owns the renderer/queue/resources. After the last accepted/source owners
        // retire, poll completion/GC without a UI-thread wait or diagnostic image readback.
        drop(displayed);
        drop(accepted_underlay);
        drop(accepted);
        drop(underlay);
        drop(source);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        eprintln!(
            "physical_view_gpu retired native_bytes={:?} excludes=raw_leases_scopes_host_other_owners",
            native()
        );
    });
}
