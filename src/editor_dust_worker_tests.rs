use super::*;
use crate::application::test_support::with_application;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

#[test]
fn authored_dust_workflow_slices_share_presentation_queue_and_retire_without_failed_admission() {
    pollster::block_on(async {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/raw/illuminated-patches.dng");
        let document = document(&path);
        let (device, queue) = gpu().await;
        let gate = crate::gpu::SubmissionGate::default();
        let renderers = Arc::new(crate::renderer::RendererCache::new(gate.clone()));
        let (ready, reference) = prepare(&document, &device, &queue, &renderers);
        let source_id = document.project.order[0];
        let epoch = Arc::new(AtomicU64::new(1));
        let armed = Arc::new(AtomicBool::new(false));
        let slices = Arc::new(AtomicUsize::new(0));
        let (reached, paused) = mpsc::channel();
        let (resume, release) = mpsc::channel();
        let release = std::sync::Mutex::new(release);
        let worker_armed = armed.clone();
        let worker_slices = slices.clone();
        // Bootstrap has completed, and presentation starts only after this gate.
        // No worker-thread name is assumed for the established blocking pool.
        gate.observe_submissions(Arc::new(move || {
            if worker_armed.load(Ordering::Acquire)
                && worker_slices.fetch_add(1, Ordering::Relaxed) == 1
            {
                worker_armed.store(false, Ordering::Release);
                reached.send(()).unwrap();
                release
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(15))
                    .unwrap();
            }
        }));

        let mut editor = crate::editor::tests::fixture();
        editor.document.install(document);
        editor.current_source = Some(source_id);
        editor.id = 1;
        editor.source_epoch = epoch.clone();
        editor.document.visit.ready = Some(ready);
        editor.set_dmin(Some(DVec3::splat(0.8)));
        editor.set_dmax(Some(DVec3::splat(0.02)));
        editor.set_floor(crate::processing::CAMERA_INPUT_FLOOR);
        editor.dust_services = Some(dust_workflow::Services::new(
            editor.workflow_jobs.clone(),
            device.clone(),
            queue.clone(),
            gate.clone(),
            Arc::default(),
        ));
        editor.begin_dust_cleanup();
        editor.document.dust.brush_size = 32;
        let composition = Composition::new(
            editor.document.visit.ready.as_ref().unwrap().area,
            Default::default(),
            Default::default(),
        )
        .unwrap();
        let geometry = editor.viewport.layout_composition(
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(368., 304.)),
            composition,
            1.,
        );
        editor.begin_dust_mask(None);
        editor.paint_dust_stroke(
            geometry,
            geometry.screen_point(egui::pos2(120., 60.)),
            false,
        );
        editor.commit_dust_mask();
        let signature = editor.dust_signature().unwrap();
        let repair = editor.dust_repairs(source_id).unwrap().repairs()[0].clone();
        let expected_request = dust::Request {
            request: 0,
            loaded: 1,
            source: source_id,
            signature,
            repair: repair.clone(),
            input: editor.calibration().unwrap().into(),
            cancelled: Arc::new(AtomicBool::new(false)),
        };
        let before = editor.document.project.clone();
        let jobs = editor.workflow_jobs.clone();
        with_application(editor, |app| {
            armed.store(true, Ordering::Release);
            app.enter(|e, access| {
                e.schedule_dust(signature, access);
            });
            let first = app.read(|e| e.document.dust.pending_request().unwrap());
            app.pump();
            paused
                .recv_timeout(Duration::from_secs(15))
                .expect("automatic heal did not reach two GPU slices");
            assert_eq!(slices.load(Ordering::Relaxed), 2);
            app.read(|e| assert_eq!(e.document.dust.pending_request(), Some(first)));

            // The submission observer must not hold the gate while paused. A real
            // presentation submission can complete between two repair slices.
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("interleaved presentation"),
                size: wgpu::Extent3d {
                    width: 8,
                    height: 8,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("presentation between repair slices"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.2,
                                g: 0.3,
                                b: 0.4,
                                a: 1.,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
            }
            let presented = gate.submit(&queue, [encoder.finish()]);
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(presented),
                    timeout: None,
                })
                .unwrap();
            assert!(!jobs.idle());
            epoch.store(2, Ordering::Relaxed);
            resume.send(()).unwrap();
            app.run_until(Duration::from_secs(30), |e| {
                e.document.dust.pending_request().is_none()
            });
            app.read(|e| {
                assert!(!e.document.dust.has_evaluation(source_id, repair.id));
                assert_eq!(e.document.project, before);
            });

            // Candidate failure can restore the accepted source authority. Retrying
            // this same visit/signature protects against memoising cancellation as failure.
            epoch.store(1, Ordering::Relaxed);
            app.enter(|e, access| {
                e.schedule_dust(signature, access);
            });
            let second = app.read(|e| {
                e.document
                    .dust
                    .pending_request()
                    .expect("cancelled repair was memoised as failed")
            });
            assert_ne!(first, second);
            app.run_until(Duration::from_secs(60), |e| {
                e.document.dust.pending_request().is_none()
            });
            let evaluated = app.read(|e| {
                assert_eq!(e.document.project, before);
                assert!(e.document.dust.has_evaluation(source_id, repair.id));
                e.document
                    .dust
                    .evaluations(source_id, 1, signature, std::slice::from_ref(&repair))[0]
                    .clone()
            });
            let expected = dust::evaluate(&reference, &expected_request).unwrap();
            assert_eq!(
                expected
                    .evaluated
                    .pixels()
                    .map(|(p, v)| (p, v.map(f32::to_bits)))
                    .collect::<Vec<_>>(),
                evaluated
                    .pixels()
                    .map(|(p, v)| (p, v.map(f32::to_bits)))
                    .collect::<Vec<_>>()
            );
            let metadata = |r: &crate::dust_repair::EvaluatedRepair| {
                r.donors
                    .iter()
                    .map(|d| {
                        (
                            d.target,
                            d.source,
                            d.context_mse.to_bits(),
                            d.paired_context_pixels,
                            d.matching_score.to_bits(),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(metadata(&expected.evaluated), metadata(&evaluated));
        });
    });
}
