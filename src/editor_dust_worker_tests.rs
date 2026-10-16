use super::*;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

#[test]
fn authored_dust_backend_slices_share_presentation_queue_and_retire_without_failed_admission() {
    pollster::block_on(async {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/raw/illuminated-patches.dng");
        let document = document(&path);
        let (device, queue) = gpu().await;
        let gate = crate::gpu::SubmissionGate::default();
        let renderers = Arc::new(crate::renderer::RendererCache::new(gate.clone()));
        let (_, reference) = prepare(&document, &device, &queue, &renderers);

        let epoch = Arc::new(AtomicU64::new(1));
        let (commands, receiver) = mpsc::channel();
        let (send, messages) = mpsc::channel();
        let scheduler = development::Scheduler::new();
        let armed = Arc::new(AtomicBool::new(false));
        let slices = Arc::new(AtomicUsize::new(0));
        let (reached, paused) = mpsc::channel();
        let (resume, release) = mpsc::channel();
        let release = Arc::new(std::sync::Mutex::new(release));
        let worker_armed = armed.clone();
        let worker_slices = slices.clone();
        gate.observe_submissions(Arc::new(move || {
            if worker_armed.load(Ordering::Acquire)
                && std::thread::current().name() == Some("dust backend integration")
                && worker_slices.fetch_add(1, Ordering::Relaxed) == 1
            {
                worker_armed.store(false, Ordering::Release);
                reached.send(()).unwrap();
                release
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(10))
                    .unwrap();
            }
        }));

        let worker_device = device.clone();
        let worker_queue = queue.clone();
        let worker_gate = gate.clone();
        let worker_epoch = epoch.clone();
        let worker_scheduler = scheduler.clone();
        let worker = std::thread::Builder::new()
            .name("dust backend integration".into())
            .spawn(move || {
                backend(
                    renderers,
                    worker_device,
                    (worker_queue, worker_gate),
                    receiver,
                    send,
                    Arc::default(),
                    Arc::default(),
                    worker_scheduler,
                    worker_epoch,
                )
            })
            .unwrap();

        let source_id = document.project.order[0];
        let record = document.project.source(source_id).unwrap();
        let load = |id| {
            commands
                .send(Command::Load(
                    id,
                    path.clone(),
                    wb(&document),
                    Some(Box::new(record.clone())),
                    None,
                    document.project.camera_basis.clone(),
                    LoadCache::new(&path, Some(record), wb(&document), &document),
                ))
                .unwrap()
        };
        load(1);
        let ready = match messages.recv_timeout(Duration::from_secs(10)).unwrap() {
            Message::Ready(1, Ok(ready)) => ready,
            _ => panic!("source not ready"),
        };

        let (mut editor, _, _) = crate::editor::tests::fixture();
        editor.document = document.clone();
        editor.current_source = Some(source_id);
        editor.id = 1;
        editor.ready = Some(*ready);
        editor.dmin = Some(DVec3::splat(0.8));
        editor.dmax = Some(DVec3::splat(0.02));
        editor.floor = crate::processing::CAMERA_INPUT_FLOOR;
        editor.commands = commands.clone();
        editor.begin_dust_cleanup();
        editor.dust.brush_size = 32;
        let composition = Composition::new(
            editor.ready.as_ref().unwrap().area,
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

        let context = egui::Context::default();
        let (wake, woken) = mpsc::channel();
        context.set_request_repaint_callback(move |_| {
            let _ = wake.send(());
        });
        let idle_frame = || {
            let mut output = context.run_ui(Default::default(), |_| {});
            output.textures_delta.clear(); // This callback-only context has no renderer.
        };
        for _ in 0..3 {
            idle_frame();
        }
        let _ = woken.try_iter().count();

        armed.store(true, Ordering::Release);
        editor.schedule_dust_with_wake(editor.dust_signature().unwrap(), &context);
        let first_request = editor.dust.pending_request().unwrap();
        paused
            .recv_timeout(Duration::from_secs(10))
            .expect("automatic heal did not reach two GPU slices");
        assert_eq!(slices.load(Ordering::Relaxed), 2);
        assert_eq!(editor.dust.pending_request(), Some(first_request));

        // Offscreen presentation uses the same queue/gate while the backend is paused
        // after a submitted slice. The observer itself must not hold the gate.
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

        epoch.store(2, Ordering::Relaxed); // Source retirement, not viewport generation.
        resume.send(()).unwrap();
        match messages.recv_timeout(Duration::from_secs(10)).unwrap() {
            Message::Dust(id, Ok(None)) => {
                assert_eq!(id, first_request);
                assert_eq!(editor.dust.pending_request(), Some(first_request));
                editor.receive_dust(id, Ok(None));
            }
            _ => panic!("retired heal did not acknowledge cancellation"),
        }
        assert_eq!(editor.dust.pending_request(), None);
        woken
            .recv_timeout(Duration::from_secs(2))
            .expect("cancel acknowledgment must wake an idle UI");

        for _ in 0..3 {
            idle_frame();
        }
        let _ = woken.try_iter().count();
        load(2);
        let ready = match messages.recv_timeout(Duration::from_secs(10)).unwrap() {
            Message::Ready(2, Ok(ready)) => ready,
            _ => panic!("replacement source not ready"),
        };
        editor.id = 2;
        editor.ready = Some(*ready);
        editor.schedule_dust_with_wake(editor.dust_signature().unwrap(), &context);
        let second_request = editor
            .dust
            .pending_request()
            .expect("cancelled repair was incorrectly memoized as failed");
        let result = match messages.recv_timeout(Duration::from_secs(10)).unwrap() {
            Message::Dust(id, Ok(Some(result))) => {
                assert_eq!(id, second_request);
                result
            }
            _ => panic!("subsequent heal failed"),
        };
        woken
            .recv_timeout(Duration::from_secs(2))
            .expect("completed heal must wake an idle UI without pointer input");

        let request = dust::Request {
            wake: context.clone(),
            request: result.request,
            loaded: result.loaded,
            source: result.source,
            signature: result.signature,
            repair: result.repair.clone(),
            calibration: editor.calibration().unwrap(),
            roll: editor.roll.inversion_only(),
            cancelled: Arc::new(AtomicBool::new(false)),
        };
        let expected = dust::evaluate(&reference, &request).unwrap();
        assert_eq!(
            expected
                .evaluated
                .pixels()
                .map(|(p, v)| (p, v.map(f32::to_bits)))
                .collect::<Vec<_>>(),
            result
                .evaluated
                .pixels()
                .map(|(p, v)| (p, v.map(f32::to_bits)))
                .collect::<Vec<_>>()
        );

        let metadata = |r: &dust::Response| {
            r.evaluated
                .donors
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
        assert_eq!(metadata(&expected), metadata(&result));

        editor.receive_dust(second_request, Ok(Some(*result)));
        assert_eq!(editor.dust.pending_request(), None);
        assert!(
            editor.dust.has_evaluation(source_id, request.repair.id),
            "accepted repair must be admitted, not silently dropped"
        );

        drop(editor);
        drop(commands);
        worker.join().unwrap();
        scheduler.close();
    });
}
