//! Bounded repeated real Editor input/paint/backend exercise. Offscreen, not physical paint.
use super::observation::{self as calibration, Monitor};
use super::*;
use crate::project::Development;
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::Instant,
};
type Result<T> = std::result::Result<T, String>;

struct Paint {
    context: egui::Context,
    renderer: egui_wgpu::Renderer,
    target: wgpu::Texture,
}

impl Paint {
    fn new(device: &wgpu::Device) -> Self {
        let context = egui::Context::default();
        context.enable_accesskit();
        Self {
            context,
            renderer: egui_wgpu::Renderer::new(
                device,
                wgpu::TextureFormat::Rgba16Float,
                Default::default(),
            ),
            target: device.create_texture(&wgpu::TextureDescriptor {
                label: Some("bounded editor switching target"),
                size: wgpu::Extent3d {
                    width: 1100,
                    height: 800,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            }),
        }
    }

    fn frame(
        &mut self,
        editor: &mut crate::application::test_support::TestApplication<'_, Editor>,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        events: Vec<egui::Event>,
    ) -> Result<(egui::FullOutput, f64, f64)> {
        editor.pump();
        let start = Instant::now();
        let mut output = self.context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100., 800.),
                )),
                events,
                ..Default::default()
            },
            |ui| crate::editor::tests::paint_controller(editor, ui),
        );
        let ui_seconds = start.elapsed().as_secs_f64();
        for (id, deltas) in &output.textures_delta.set {
            for delta in deltas {
                self.renderer.update_texture(device, queue, *id, delta);
            }
        }
        for id in &output.textures_delta.free {
            self.renderer.free_texture(id);
        }
        output.textures_delta.clear();
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [1100, 800],
            pixels_per_point: 1.,
        };
        let jobs = self
            .context
            .tessellate(output.shapes.clone(), output.pixels_per_point);
        let mut encoder = device.create_command_encoder(&Default::default());
        let commands = self
            .renderer
            .update_buffers(device, queue, &mut encoder, &jobs, &screen);
        let view = self.target.create_view(&Default::default());
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            self.renderer
                .render(&mut pass.forget_lifetime(), &jobs, &screen);
        }
        editor
            .read(|state| state.submissions.clone())
            .submit(queue, commands.into_iter().chain([encoder.finish()]));
        calibration::wait(device, queue).map_err(|e| e.to_string())?;
        Ok((output, ui_seconds, start.elapsed().as_secs_f64()))
    }

    fn card(output: &egui::FullOutput, name: &str) -> Result<(egui::Pos2, bool)> {
        let node = output
            .platform_output
            .accesskit_update
            .as_ref()
            .ok_or("accessibility output missing")?
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some(name))
            .map(|(_, node)| node)
            .ok_or("source card missing")?;
        let b = node.bounds().ok_or("source card bounds missing")?;
        Ok((
            egui::pos2(((b.x0 + b.x1) / 2.) as f32, ((b.y0 + b.y1) / 2.) as f32),
            node.is_disabled(),
        ))
    }
}

fn pointer(point: egui::Pos2, pressed: bool) -> Vec<egui::Event> {
    vec![
        egui::Event::PointerMoved(point),
        egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        },
    ]
}

fn workload(
    monitor: &Arc<Monitor>,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    samples: PathBuf,
) -> Result<()> {
    let start = Instant::now();
    let reference = monitor.phase("switch_wb_header", || {
        crate::raw::RawFile::open(&samples.join("DSC04397.ARW"))
    })?;
    let wb = reference.metadata.cam_mul;
    drop(reference);
    let mut document = Document::default();
    for name in ["DSC04378.ARW", "DSC04393.ARW"] {
        monitor.check()?;
        let (source, raw) =
            document::open_source(&samples.join(name), None).map_err(|e| e.to_string())?;
        calibration::source_bound(&raw.metadata)?;
        drop(raw);
        document
            .register_with_development(source, Development::new(crate::raw::version(), wb))
            .map_err(|e| e.to_string())?;
    }
    let count = Arc::new(AtomicUsize::new(0));
    let observed = count.clone();
    let native_monitor = monitor.clone();
    let scheduler = source_loading::SourceLoader::new_observed(move |path, phase, metadata| {
        native_monitor.check()?;
        if let Some(metadata) = metadata {
            calibration::source_bound(metadata)?;
        }
        if matches!(phase, source_loading::NativePhase::UnpackBegin) {
            observed.fetch_add(1, Ordering::Relaxed);
        }
        println!(
            "switch_native file={} phase={phase:?} seconds={:.6}",
            path.file_name().unwrap().to_string_lossy(),
            start.elapsed().as_secs_f64()
        );
        Ok(())
    });
    let renderers = Arc::new(crate::renderer::RendererCache::default());
    let epoch = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let mut editor = Editor::state();
    let directory = tempfile::tempdir().map_err(|e| e.to_string())?;
    editor.document.thumbnails = (thumbnails::Thumbnails::with_developments(
        device.clone(),
        queue.clone(),
        renderers.clone(),
        Some(thumbnails::disk::DiskCache::at(
            directory.path().join("thumbnails"),
        )),
        scheduler.clone(),
    ))
    .with_sources(editor.document.cache.clone());
    editor.document.thumbnails.pressure_override = None;
    *editor.document = document;
    editor.developments = Some(scheduler.clone());
    editor.source_epoch = epoch.clone();
    editor
        .document
        .thumbnails
        .set_limits(cache_limits::CacheLimits {
            cpu_bytes: 1100 * cache_limits::MIB,
            gpu_bytes: 16 * cache_limits::MIB,
        });
    editor.set_dmin(Some(DVec3::new(0.28, 0.20, 0.14)));
    editor.set_dmax(Some(DVec3::new(0.028, 0.020, 0.014)));
    editor.sync_document();
    editor.initialize_viewer(device, queue);
    editor.source_services = Some(source::Services {
        developments: scheduler.clone(),
        renderers: renderers.clone(),
        device: device.clone(),
        queue: queue.clone(),
    });
    editor.export_services = Some(export_services::Services::new(
        renderers.clone(),
        device.clone(),
        queue.clone(),
        editor.submissions.clone(),
        editor.source_services.clone().unwrap(),
    ));
    let exercise = crate::application::test_support::with_application(editor, |app| {
        let exercise = (|| -> Result<()> {
            let mut paint = Paint::new(device);
            app.enter(|editor, access| {
                editor.document.selected = 0;
                let path = editor
                    .document
                    .source_path(editor.document.state.project.order[0])
                    .map_err(|e| e.to_string())?;
                editor.load(path, access);
                Ok::<_, String>(())
            })?;
            let mut all_ui = Vec::new();
            let mut all_frame = Vec::new();
            for step in 0..17 {
                let selected = step % 2;
                let selection_start = Instant::now();
                let native_before = count.load(Ordering::Relaxed);
                if step > 0 {
                    let (output, ui, total) = paint.frame(app, device, queue, vec![])?;
                    all_ui.push(ui);
                    all_frame.push(total);
                    let (point, disabled) = Paint::card(
                        &output,
                        if selected == 0 {
                            "DSC04378.ARW"
                        } else {
                            "DSC04393.ARW"
                        },
                    )?;
                    if disabled {
                        return Err("ready source card unexpectedly disabled".into());
                    }
                    let (_, ui, total) = paint.frame(app, device, queue, pointer(point, true))?;
                    all_ui.push(ui);
                    all_frame.push(total);
                    let (_, ui, total) = paint.frame(app, device, queue, pointer(point, false))?;
                    all_ui.push(ui);
                    all_frame.push(total);
                    if app.read(|e| e.document.selected != selected || !e.busy) {
                        return Err("actual source-card input did not start selection".into());
                    }
                }
                let mut ready = None;
                let mut blocked_probe = false;
                let mut per_ui = Vec::new();
                let mut per_frame = Vec::new();
                while app.read(|e| e.busy || !e.document.visit.working_view.idle()) {
                    monitor.check()?;
                    if start.elapsed() > Duration::from_secs(90) {
                        return Err("bounded switching exercise deadline".into());
                    }
                    let (output, ui, total) = paint.frame(app, device, queue, vec![])?;
                    per_ui.push(ui);
                    per_frame.push(total);
                    all_ui.push(ui);
                    all_frame.push(total);
                    if step == 0 && app.read(|e| e.busy) && !blocked_probe {
                        let (point, disabled) = Paint::card(
                            &output,
                            if selected == 0 {
                                "DSC04393.ARW"
                            } else {
                                "DSC04378.ARW"
                            },
                        )?;
                        let request = app.read(|e| e.id);
                        let (_, ui, total) =
                            paint.frame(app, device, queue, pointer(point, true))?;
                        all_ui.push(ui);
                        all_frame.push(total);
                        let (output, ui, total) =
                            paint.frame(app, device, queue, pointer(point, false))?;
                        all_ui.push(ui);
                        all_frame.push(total);
                        let replaced = app.read(|e| e.id);
                        if disabled || replaced == request || app.read(|e| e.document.selected != 1)
                        {
                            return Err("pending selection did not accept replacement".into());
                        }
                        let (point, disabled) = Paint::card(&output, "DSC04378.ARW")?;
                        if disabled {
                            return Err("replacement selection blocked return click".into());
                        }
                        let (_, ui, total) =
                            paint.frame(app, device, queue, pointer(point, true))?;
                        all_ui.push(ui);
                        all_frame.push(total);
                        let (_, ui, total) =
                            paint.frame(app, device, queue, pointer(point, false))?;
                        all_ui.push(ui);
                        all_frame.push(total);
                        if app.read(|e| e.id == replaced || e.document.selected != 0) {
                            return Err("latest selection did not replace pending work".into());
                        }
                        println!(
                            "switch_busy_input step={step} disabled=false request_before={request} replaced={replaced} returned={} native_developments={}",
                            app.read(|e| e.id),
                            count.load(Ordering::Relaxed)
                        );
                        blocked_probe = true;
                    }
                    if app.read(|e| !e.busy) && ready.is_none() {
                        ready = Some(selection_start.elapsed().as_secs_f64());
                    }
                    if app.read(|e| e.document.visit.ready.is_none() && !e.busy) {
                        return Err(
                            app.read(|e| format!("source preparation failed: {}", e.status))
                        );
                    }
                    if app.read(|e| {
                        e.document.visit.grade.original.refinement_error.is_some()
                            || e.document.visit.working_view.error.is_some()
                    }) {
                        return Err(app.read(|e| format!("refinement failed: {}", e.status)));
                    }
                    std::thread::sleep(Duration::from_millis(8));
                }
                println!(
                    "switch_step step={step} selected={selected} ready_seconds={:.6} accepted_view_seconds={:.6} native_before={native_before} native_after={} ui_max_ms={:.6} draw_wait_max_ms={:.6} cpu_optional={} gpu_cached_images={}",
                    ready.ok_or("ready boundary missing")?,
                    selection_start.elapsed().as_secs_f64(),
                    count.load(Ordering::Relaxed),
                    per_ui.iter().copied().fold(0., f64::max) * 1000.,
                    per_frame.iter().copied().fold(0., f64::max) * 1000.,
                    app.read(|e| e.document.cache.charges().retained),
                    app.read(|e| e
                        .document
                        .thumbnails
                        .ready_count(&e.document, e.document.target))
                );
            }
            if count.load(Ordering::Relaxed) != 2 {
                return Err("retained two-source switching redeveloped a source".into());
            }
            all_ui.sort_by(f64::total_cmp);
            all_frame.sort_by(f64::total_cmp);
            println!(
                "switch_summary settled_selections=17 pending_replacements=2 native_developments=2 frames={} ui_p50_ms={:.6} ui_p95_ms={:.6} ui_max_ms={:.6} draw_wait_p50_ms={:.6} draw_wait_p95_ms={:.6} draw_wait_max_ms={:.6}",
                all_ui.len(),
                all_ui[all_ui.len() / 2] * 1000.,
                all_ui[all_ui.len() * 95 / 100] * 1000.,
                all_ui.last().unwrap() * 1000.,
                all_frame[all_frame.len() / 2] * 1000.,
                all_frame[all_frame.len() * 95 / 100] * 1000.,
                all_frame.last().unwrap() * 1000.
            );
            Ok(())
        })();
        app.update(|editor| {
            editor.document.visit.grade.original.invalidate();
            editor.document.visit.working_view.clear();
            editor.source_epoch.fetch_add(1, Ordering::Relaxed);
            editor.document = super::document::Document::default().into();
            editor.document.thumbnails.cancel();
            editor.document.thumbnails.services.take();
        });
        scheduler.close();
        app.close_execution();
        exercise
    });
    scheduler.close_and_wait()?;
    drop(renderers);
    exercise?;
    monitor.phase("switch_released", || calibration::wait(device, queue))?;
    Ok(())
}

#[test]
#[ignore = "bounded real Editor two-source repeated input/paint exercise; local RAWs and Metal"]
fn retained_source_switching_and_busy_input() {
    let samples = PathBuf::from(
        std::env::var_os("ANTITONE_SWITCH_SAMPLES").expect("existing samples directory required"),
    );
    println!(
        "switch_exercise sources=2 settled_selections=17 pending_replacements=2 cpu_optional_limit=1100MiB gpu_optional_limit=16MiB source_limit=600MiB footprint_growth_stop=2GiB native_policy=production window=false paint=offscreen scope_panel=false samples_read_only=true preferences_persisted=false"
    );
    calibration::with_monitor(|monitor, device, queue| workload(monitor, device, queue, samples))
        .unwrap();
}
