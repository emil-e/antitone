//! Pointer release through real donor search and an offscreen completed GPU display.
//! Timings include harness rendering/readback; they are observations, not native presentation acceptance.
use super::*;

fn done(editor: &Editor, count: usize) -> bool {
    let Some(source) = editor.document.visit.current_source else {
        return false;
    };
    !editor.dust_search_pending()
        && editor
            .dust_repairs(source)
            .map_or(0, |set| set.repairs().len())
            == count
        && editor.document.visit.working_view.idle()
        && editor
            .document
            .visit
            .working_view
            .displayed
            .as_ref()
            .is_some_and(|view| {
                let expected = editor
                    .positive_recipe(
                        editor.input(),
                        editor.document.project_snapshot().roll,
                        editor.photo_settings(),
                    )
                    .unwrap();
                view.capture.recipe().map(Arc::as_ref) == Some(&expected) && !view.in_flight()
            })
}
fn paint_until(h: &mut Harness<'_, Editor>, count: usize) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        h.step();
        h.render().unwrap();
        if h.read(|e| done(e, count)) {
            break;
        }
        h.read(|e| {
            assert!(
                Instant::now() < deadline,
                "latency diagnostic did not settle: {} {:?}",
                e.status,
                e.document.visit.working_view.error
            )
        });
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
#[ignore = "read-only supplied RAW; actual pointer/search/worker/GPU-display diagnostic"]
fn profile_dust_pointer_to_gpu_display() {
    let _logging = crate::logging::init();
    let large = std::env::var_os("ANTITONE_DUST_LARGE").is_some();
    let (device, queue) = pollster::block_on(async {
        let instance = crate::gpu::instance();
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        adapter
            .request_device(&wgpu::DeviceDescriptor {
                required_limits: crate::platform::device_limits(adapter.limits()),
                ..Default::default()
            })
            .await
            .unwrap()
    });
    let mut editor = Editor::new(&device, &queue);
    let pressure = std::env::var("ANTITONE_DUST_PRESSURE").unwrap_or_else(|_| "host".into());
    match pressure.as_str() {
        "host" => {}
        "normal" => editor
            .source_services
            .as_ref()
            .unwrap()
            .renderers
            .get(&device, &queue)
            .unwrap()
            .evaluation
            .materializations()
            .set_test_pressure(crate::sys::MemoryPressure::Normal),
        _ => panic!("ANTITONE_DUST_PRESSURE must be host or normal"),
    }
    eprintln!(
        "APP retention_pressure={pressure} host_pressure={:?}",
        crate::sys::memory_pressure()
    );
    editor.document.thumbnails = Thumbnails::default().with_sources(editor.document.cache.clone());
    let capture = Rc::new(RefCell::new(FloatCapture::default()));
    let renderer = FloatRenderer::new(
        &device,
        &queue,
        host::App::submission_gate(&editor).clone(),
        capture.clone(),
    );
    let owner = RefCell::new(None);
    let mut h = app_harness(&owner, editor, renderer);
    h.set_size(if large {
        egui::vec2(4780., 2508.)
    } else {
        egui::vec2(1600., 1000.)
    });
    let path = PathBuf::from(
        std::env::var_os("ANTITONE_DUST_RAW")
            .expect("supply a read-only RAW path with ANTITONE_DUST_RAW"),
    );
    let started = Instant::now();
    h.enter(|e, cx| e.add_raws(vec![path], cx));
    wait_for_with_timeout(&mut h, Duration::from_secs(60), |e| {
        !e.interaction_blocked() && !e.document.write.pending()
    });
    h.update(|e| {
        e.set_dmin(Some(DVec3::splat(0.8)));
        e.set_dmax(Some(DVec3::splat(0.02)));
        e.document.source = false;
        if large {
            let area = e.selected_area().unwrap();
            let height = (f64::from(area.width.min(area.height)) * 0.96) as u32 / 2 * 2;
            let id = e.document.visit.current_source.unwrap();
            e.document.project.source_mut(id).unwrap().crop = Some(crate::crop::Crop {
                center: [
                    f64::from(area.x) + f64::from(area.width) / 2.,
                    f64::from(area.y) + f64::from(area.height) / 2.,
                ],
                size: [height * 1018 / 1024 / 2 * 2, height],
                angle: 0.,
            });
            e.document.visit.canvas.viewport.zoom = Some(1.);
        }
    });
    paint_until(&mut h, 0);
    eprintln!("APP warm {:?}", started.elapsed());
    eprintln!(
        "APP grids={:?}",
        h.read(|e| e.document.visit.working_view.regional().map(|view| [
            view.spec.frame().output,
            view.spec.underlay().unwrap().frame().output
        ]))
    );
    stage(&mut h, "Photo");
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Dust repair")
        .click();
    h.step();
    h.render().unwrap();
    eprintln!("APP brush={}", h.read(|e| e.document.visit.dust.brush_size));
    let area = h.read(|e| e.selected_area().unwrap());
    for i in 0..12 {
        if large && i == 6 {
            let pixels_per_point = h.ctx.pixels_per_point();
            h.update(|e| {
                let layout = e.document.visit.canvas.last_layout.unwrap().0;
                e.document.visit.canvas.viewport.pan(
                    layout,
                    pixels_per_point,
                    egui::vec2(-128., 96.),
                );
            });
            h.step();
            h.render().unwrap();
            let acquisition_pending = h.read(|e| e.document.visit.working_view.task.is_some());
            h.update(|e| {
                let layout = e.document.visit.canvas.last_layout.unwrap().0;
                e.document.visit.canvas.viewport.pan(
                    layout,
                    pixels_per_point,
                    egui::vec2(-16., 8.),
                );
            });
            h.step();
            h.render().unwrap();
            eprintln!(
                "APP successive pans previous_acquisition_pending={acquisition_pending}; next stroke without waiting for settling"
            );
        }
        if large {
            h.update(|e| e.document.visit.dust.brush_size = [90, 116, 58][i % 3]);
        }
        let source = crate::geometry::SourcePoint([
            f64::from(area.x)
                + f64::from(area.width) * (if large { 0.35 } else { 0.25 } + 0.1 * (i % 4) as f64),
            f64::from(area.y)
                + f64::from(area.height)
                    * (if large {
                        0.4 + 0.1 * (i / 4) as f64
                    } else {
                        0.25 + 0.15 * (i / 4) as f64
                    }),
        ]);
        let point = egui::pos2(source.0[0] as f32, source.0[1] as f32);
        h.read(|e| {
            let layout = e.document.visit.canvas.last_layout.unwrap().0;
            assert!(
                layout.canvas.contains(layout.screen_point(point)),
                "diagnostic source position must be visible before clicking"
            );
        });
        let began = Instant::now();
        super::click(&mut h, point, egui::Modifiers::NONE);
        let mut accepted = None;
        let deadline = began + Duration::from_secs(60);
        loop {
            h.step();
            h.render().unwrap();
            let count = h.read(|e| {
                e.dust_repairs(e.document.visit.current_source.unwrap())
                    .map_or(0, |set| set.repairs().len())
            });
            if count == i + 1 && accepted.is_none() {
                accepted = Some(began.elapsed());
            }
            if h.read(|e| done(e, i + 1)) {
                break;
            }
            h.read(|e| {
                assert!(
                    Instant::now() < deadline,
                    "pointer {} timed out: {}",
                    i + 1,
                    e.status
                )
            });
            std::thread::sleep(Duration::from_millis(2));
        }
        eprintln!(
            "APP click={} donor={:?} GPU_display={:?} capture={:?}",
            i + 1,
            accepted,
            began.elapsed(),
            capture.borrow().size
        );
    }
}
