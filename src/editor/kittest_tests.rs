use super::*;
use egui_kittest::{
    Harness as EguiHarness, TestRenderer,
    kittest::{NodeT, Queryable},
};
use std::{cell::RefCell, rc::Rc, time::Instant};

mod activity;
mod adjustment_layers;
mod analyze;
mod auto_crop;
mod correction;
mod crop;
mod dust;
mod export;
#[cfg(feature = "ocio-reference")]
mod exposure;
mod film_emulation;
mod fullscreen;
mod guide_shots;
mod history;
mod looks;
mod masks;
mod menus;
mod progress;
mod project;
mod roll_selection;
mod tools;
mod typography;
mod workflow;

#[test]
fn parameter_reset_is_a_name_gesture_and_respects_disabled_state() {
    let mut harness = EguiHarness::builder().with_max_steps(12).build_ui_state(
        |ui, state: &mut (f64, bool)| {
            ui.add_enabled_ui(state.1, |ui| {
                controls::parameter(ui, controls::Parameter::Contrast, &mut state.0);
            });
        },
        (1.4, true),
    );
    for role in [
        egui::accesskit::Role::Slider,
        egui::accesskit::Role::SpinButton,
    ] {
        assert!(
            !harness
                .get_by_role_and_label(role, "Contrast")
                .accesskit_node()
                .is_disabled()
        );
    }
    // Option-clicking the name resets the value.
    let name = harness
        .get_by_role_and_label(egui::accesskit::Role::Label, "Contrast")
        .rect()
        .center();
    harness.event(egui::Event::ModifiersChanged(egui::Modifiers::ALT));
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos: name,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::ALT,
        });
    }
    harness.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    harness.run();
    assert_eq!(harness.state().0, 1.);

    *harness.state_mut() = (1.4, false);
    harness.run();
    for role in [
        egui::accesskit::Role::Slider,
        egui::accesskit::Role::SpinButton,
    ] {
        assert!(
            harness
                .get_by_role_and_label(role, "Contrast")
                .accesskit_node()
                .is_disabled()
        );
    }
    harness.event(egui::Event::ModifiersChanged(egui::Modifiers::ALT));
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos: name,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::ALT,
        });
    }
    harness.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    harness.run();
    assert_eq!(harness.state().0, 1.4, "a disabled row must not reset");
}

const UI_MAX_STEPS: u64 = 240;
const UI_STEP_DT: f32 = 1. / 60.;

struct Harness<'a, T> {
    inner: EguiHarness<'a, crate::application::UiContext<'a, T>>,
    application: crate::application::test_support::TestApplication<'a, T>,
    state: &'a RefCell<Option<T>>,
}

impl<'scope, T: 'scope> Harness<'scope, T> {
    fn read<R>(&self, read: impl FnOnce(&T) -> R) -> R {
        self.application.read(read)
    }

    fn update<R>(&self, update: impl FnOnce(&mut T) -> R) -> R {
        self.application.update(update)
    }

    fn enter<R>(
        &self,
        action: impl FnOnce(&mut T, &crate::application::UiContext<'scope, T>) -> R,
    ) -> R {
        self.application.enter(action)
    }

    fn run_until(&mut self, timeout: Duration, done: impl Fn(&T) -> bool) {
        self.application.run_until(timeout, done);
    }

    fn step(&mut self) {
        self.application.pump();
        self.inner.step();
    }

    fn run_steps(&mut self, steps: usize) {
        for _ in 0..steps {
            self.step();
        }
    }

    fn run(&mut self) -> u64 {
        self.run_ui(false).unwrap()
    }

    fn try_run_realtime(&mut self) -> Result<u64, egui_kittest::ExceededMaxStepsError> {
        self.run_ui(true)
    }

    fn run_ui(&mut self, realtime: bool) -> Result<u64, egui_kittest::ExceededMaxStepsError> {
        // Keep kittest's settling/timeout semantics, delivering executor events
        // outside Context::run_ui rather than from the application callback.
        let mut steps = 0;
        loop {
            steps += 1;
            self.step();
            let images = self.ctx.has_pending_images();
            if self.inner.output().viewport_output[&egui::ViewportId::ROOT].repaint_delay
                != Duration::ZERO
                && !images
            {
                return Ok(steps);
            }
            if realtime || images {
                std::thread::sleep(Duration::from_secs_f32(UI_STEP_DT));
            }
            if steps > UI_MAX_STEPS {
                return Err(egui_kittest::ExceededMaxStepsError {
                    max_steps: UI_MAX_STEPS,
                    repaint_causes: self.ctx.repaint_causes(),
                });
            }
        }
    }
}

impl<'a, T> std::ops::Deref for Harness<'a, T> {
    type Target = EguiHarness<'a, crate::application::UiContext<'a, T>>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T> std::ops::DerefMut for Harness<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl<T> Drop for Harness<'_, T> {
    fn drop(&mut self) {
        self.application.close_execution();
        drop(self.state.borrow_mut().take());
    }
}

fn app_harness<'scope>(
    state: &'scope RefCell<Option<Editor>>,
    editor: Editor,
    renderer: FloatRenderer,
) -> Harness<'scope, Editor> {
    use std::sync::OnceLock;
    let context = Arc::new(OnceLock::<egui::Context>::new());
    let repaint = context.clone();
    *state.borrow_mut() = Some(editor);
    let application = crate::application::test_support::TestApplication::new(state, move || {
        if let Some(context) = repaint.get() {
            context.request_repaint();
        }
    });
    let access = application.access();
    let inner = EguiHarness::builder()
        .with_size(egui::vec2(1100., 800.))
        .with_max_steps(UI_MAX_STEPS)
        .with_step_dt(UI_STEP_DT)
        .renderer(renderer)
        .build_ui_state(
            move |ui, editor| {
                let _ = context.set(ui.ctx().clone());
                let input = editor.with_mut(|state| host::App::menu_input(state, None));
                ui.input_mut(|state| {
                    state.events.splice(0..0, input);
                });
                editor.with_mut(|state| host::App::ui(state, ui, editor));
            },
            access,
        );
    Harness {
        inner,
        application,
        state,
    }
}

#[derive(Default)]
struct FloatCapture {
    size: [u32; 2],
    pixels: Vec<[f32; 4]>,
}

// Kittest's built-in readback assumes RGBA8. Retain the host's RGBA16Float path;
// only the optional inspection PNG is quantized/clipped, not the numeric readback.
struct FloatRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    submissions: crate::platform::SubmissionGate,
    renderer: egui_wgpu::Renderer,
    capture: Rc<RefCell<FloatCapture>>,
}

impl FloatRenderer {
    fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        submissions: crate::platform::SubmissionGate,
        capture: Rc<RefCell<FloatCapture>>,
    ) -> Self {
        Self {
            device: device.clone(),
            queue: queue.clone(),
            submissions,
            capture,
            renderer: egui_wgpu::Renderer::new(
                device,
                wgpu::TextureFormat::Rgba16Float,
                egui_wgpu::RendererOptions {
                    dithering: false,
                    ..Default::default()
                },
            ),
        }
    }
}

impl TestRenderer for FloatRenderer {
    fn handle_delta(&mut self, delta: &mut egui::TexturesDelta) {
        for (id, updates) in delta.set.drain() {
            for update in updates {
                self.renderer
                    .update_texture(&self.device, &self.queue, id, &update);
            }
        }
        for id in delta.free.drain() {
            self.renderer.free_texture(&id);
        }
    }

    fn render(
        &mut self,
        ctx: &egui::Context,
        output: &egui::FullOutput,
    ) -> Result<image::RgbaImage, String> {
        let size = ctx.content_rect().size() * output.pixels_per_point;
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [size.x.round() as u32, size.y.round() as u32],
            pixels_per_point: output.pixels_per_point,
        };
        let [width, height] = screen.size_in_pixels;
        let jobs = ctx.tessellate(output.shapes.clone(), output.pixels_per_point);
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("kittest offscreen RGBA16Float"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let row = (width * 8).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("kittest half-float readback"),
            size: u64::from(row) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let commands =
            self.renderer
                .update_buffers(&self.device, &self.queue, &mut encoder, &jobs, &screen);
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("kittest production callbacks and egui"),
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
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(height),
                },
            },
            texture.size(),
        );
        self.submissions
            .submit(&self.queue, commands.into_iter().chain([encoder.finish()]));
        let (send, receive) = mpsc::channel();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = send.send(result);
            });
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(Duration::from_secs(10)),
            })
            .map_err(|e| e.to_string())?;
        receive
            .recv_timeout(Duration::from_secs(10))
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        let mapped = buffer
            .slice(..)
            .get_mapped_range()
            .map_err(|e| e.to_string())?;
        let mut pixels = Vec::with_capacity((width * height) as usize);
        let mut png = image::RgbaImage::new(width, height);
        for (y, bytes) in mapped.chunks_exact(row as usize).enumerate() {
            for (x, code) in bytes[..(width * 8) as usize]
                .as_chunks::<8>()
                .0
                .iter()
                .enumerate()
            {
                let pixel: [f32; 4] = std::array::from_fn(|c| {
                    half::f16::from_le_bytes(code[c * 2..c * 2 + 2].try_into().unwrap()).to_f32()
                });
                if pixel.iter().any(|v| !v.is_finite()) {
                    return Err("nonfinite offscreen pixel".into());
                }
                pixels.push(pixel);
                png.put_pixel(
                    x as u32,
                    y as u32,
                    image::Rgba(pixel.map(|v| (v.clamp(0., 1.) * 255.).round() as u8)),
                );
            }
        }
        drop(mapped);
        buffer.unmap();
        *self.capture.borrow_mut() = FloatCapture {
            size: [width, height],
            pixels,
        };
        Ok(png)
    }
}

#[test]
fn decision_dialogs_expose_safe_actions_and_fit_normal_and_narrow_viewports() {
    let (device, queue) = pollster::block_on(async {
        let instance = crate::gpu::instance();
        let adapter = instance
            .request_adapter(&Default::default())
            .await
            .expect("dialog captures require Metal");
        adapter.request_device(&Default::default()).await.unwrap()
    });
    let mut editor = Editor::new(&device, &queue);
    editor.thumbnails = Thumbnails::default();
    let capture = Rc::new(RefCell::new(FloatCapture::default()));
    let renderer = FloatRenderer::new(
        &device,
        &queue,
        host::App::submission_gate(&editor).clone(),
        capture,
    );
    let state = RefCell::new(None);
    let mut harness = app_harness(&state, editor, renderer);
    harness.enter(|editor, cx| {
        editor.add_raws(
            vec![PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng")],
            cx,
        )
    });
    wait_for(&mut harness, |editor| {
        !editor.busy && editor.document_pending.is_none()
    });
    harness.read(|editor| assert!(editor.document.visit.ready.is_some(), "{}", editor.status));
    harness.update(|editor| editor.adjustment_layer_settings.exposure = 1.25);
    harness.update(|editor| editor.sync_document());
    let before = harness.read(|editor| editor.document.project.clone());
    let selected = harness.read(|editor| editor.current_source);
    harness.get_by_label("Viewer help").click();
    harness.run();
    assert!(egui::Popup::is_any_open(&harness.ctx));
    harness.update(|editor| editor.transition = Some(Transition::New));
    harness.run();
    harness.event(super::presentation::tests::key(egui::Key::Escape).remove(0));
    harness.step();
    assert!(!egui::Popup::is_any_open(&harness.ctx));
    harness.read(|editor| {
        assert!(
            editor.transition.is_some(),
            "popover Escape also cancelled the decision"
        )
    });
    harness.event(super::presentation::tests::key(egui::Key::Escape).remove(0));
    harness.step();
    harness.read(|editor| assert!(editor.transition.is_none()));
    for (layout, size) in [
        ("normal", egui::vec2(1100., 800.)),
        ("narrow", egui::vec2(800., 600.)),
    ] {
        harness.set_size(size);
        for (decision, name, confirm) in [
            (0, "save", "Save changes"),
            (1, "project-replace", "Replace project"),
            (2, "output-replace", "Replace and export"),
        ] {
            harness.update(|editor| match decision {
                0 => editor.transition = Some(Transition::New),
                1 => {
                    editor.project_overwrite = Some((
                        PathBuf::from("/projects/Scans/照片-long-project-name.antitone.json"),
                        Some(Transition::New),
                    ))
                }
                2 => {
                    let mut draft = export_batch::Draft::new(
                        OutputTarget::Srgb,
                        PathBuf::from("/exports/Scans/照片-long-export-folder"),
                    );
                    draft.collision = export_batch::Collision::Replace;
                    draft.confirming_replace = true;
                    editor.export_dialog = Some(draft);
                }
                _ => unreachable!(),
            });
            harness.hover_at(egui::pos2(10., size.y - 10.));
            harness.run();
            assert_eq!(harness.query_by_label("Warning").is_some(), decision != 0);
            assert!(harness.query_by_label("Information").is_none());
            assert!(harness.query_by_label("Error").is_none());
            let cancel = harness.get_by_label("Cancel");
            assert!(!cancel.accesskit_node().is_disabled());
            assert!(
                harness
                    .get_all_by_label(confirm)
                    .any(|node| !node.accesskit_node().is_disabled())
            );
            let bounds = cancel.rect();
            assert!(bounds.min.x >= 0. && bounds.max.x <= size.x && bounds.max.y <= size.y);
            let png = harness.render().unwrap();
            assert_eq!(png.dimensions(), (size.x as u32, size.y as u32));
            if let Some(directory) = std::env::var_os("ANTITONE_UI_CAPTURE") {
                let directory = PathBuf::from(directory);
                std::fs::create_dir_all(&directory).unwrap();
                png.save(directory.join(format!("dialog-{name}-{layout}-offscreen.png")))
                    .unwrap();
            }
            harness.get_by_label("Cancel").click();
            harness.run();
            if decision == 2 {
                harness.read(|editor| {
                    assert!(!editor.export_dialog.as_ref().unwrap().confirming_replace)
                });
                harness.get_by_label("Cancel").click();
                harness.run();
            }
            harness.read(|editor| {
                assert!(
                    editor.transition.is_none()
                        && editor.project_overwrite.is_none()
                        && editor.export_dialog.is_none()
                )
            });
            harness.read(|editor| assert!(!editor.busy));
            harness.read(|editor| assert_eq!(editor.document.project, before));
            harness.read(|editor| assert_eq!(editor.current_source, selected));
        }
    }
}

#[test]
fn operation_failures_are_acknowledged_without_promoting_failed_candidates() {
    let (device, queue) = pollster::block_on(async {
        let instance = crate::gpu::instance();
        let adapter = instance
            .request_adapter(&Default::default())
            .await
            .expect("feedback captures require Metal");
        adapter.request_device(&Default::default()).await.unwrap()
    });
    let mut editor = Editor::new(&device, &queue);
    editor.thumbnails = Thumbnails::default();
    let renderer = FloatRenderer::new(
        &device,
        &queue,
        host::App::submission_gate(&editor).clone(),
        Rc::new(RefCell::new(FloatCapture::default())),
    );
    let state = RefCell::new(None);
    let mut harness = app_harness(&state, editor, renderer);
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng");
    harness.enter(|editor, cx| editor.add_raws(vec![fixture.clone()], cx));
    wait_for(&mut harness, |editor| {
        !editor.busy && editor.document_pending.is_none()
    });
    harness.read(|editor| assert!(editor.document.visit.ready.is_some(), "{}", editor.status));
    harness.update(|editor| editor.adjustment_layer_settings.exposure = 1.25);
    harness.update(|editor| editor.sync_document());
    let directory = tempfile::tempdir().unwrap();
    let project_path = directory.path().join("project.json");
    let saved = harness.read(|editor| {
        editor
            .document
            .save(&project_path, crate::project::Save::New)
            .unwrap()
    });
    harness.update(|editor| *editor.document = saved);
    let saved_bytes = std::fs::read(&project_path).unwrap();
    harness.update(|editor| editor.adjustment_layer_settings.exposure = 2.);
    harness.update(|editor| editor.sync_document());
    let before = harness.read(|editor| editor.document.project.clone());
    let selected = harness.read(|editor| editor.current_source);
    for (layout, size) in [
        ("normal", egui::vec2(1100., 800.)),
        ("narrow", egui::vec2(800., 600.)),
    ] {
        harness.set_size(size);
        for kind in [
            FailureKind::FlatReference,
            FailureKind::FlatSupport,
            FailureKind::FlatCorrection,
            FailureKind::Save,
            FailureKind::Export,
            FailureKind::Look,
            FailureKind::LookRendering,
        ] {
            match kind {
                FailureKind::FlatReference => harness.enter(|editor, cx| {
                    editor.begin_flat_edit(directory.path().join("missing-reference.dng"), None, cx)
                }),
                FailureKind::FlatSupport | FailureKind::FlatCorrection => {
                    harness.enter(|editor, cx| editor.begin_flat_edit(fixture.clone(), None, cx));
                    wait_for(&mut harness, |editor| !editor.busy);
                    harness.read(|editor| assert!(editor.flat_edit.is_some()));
                    let area =
                        harness.read(|editor| editor.document.visit.ready.as_ref().unwrap().area);
                    harness.update(|editor| {
                        editor.region = Some(if kind == FailureKind::FlatSupport {
                            let x = f64::from(area.x) + 0.25;
                            let y = f64::from(area.y);
                            Coverage::new(
                                [[x, y], [x + 2., y], [x + 2., y + 2.], [x, y + 2.]]
                                    .map(crate::geometry::SourcePoint),
                            )
                            .unwrap()
                        } else {
                            Coverage::rectangle(Rect {
                                x: area.x,
                                y: area.y,
                                width: 2,
                                height: 2,
                            })
                        })
                    });
                    harness.enter(|editor, cx| editor.apply_flat_edit(cx));
                }
                FailureKind::Look => harness.enter(|editor, cx| {
                    editor.begin_look(
                        look_ui::Source::Imported(directory.path().join("missing-look.cube")),
                        cx,
                    )
                }),
                FailureKind::LookRendering => harness.update(|editor| {
                    editor.report_failure(
                        FailureKind::LookRendering,
                        "The selected look encountered a nonfinite scene-linear value.".into(),
                    )
                }),
                FailureKind::Save => harness.enter(|editor, cx| {
                    editor.save_project(
                        directory.path().join("missing-parent/project.json"),
                        crate::project::Save::New,
                        Some(Transition::Close),
                        cx,
                    )
                }),
                FailureKind::Export => {
                    let path = harness.read(|editor| editor.document.path.clone().unwrap());
                    harness.enter(|editor, cx| {
                        editor.export(
                            path,
                            OutputFormat::Exr(ExrPrecision::Full),
                            Overwrite::Confirmed,
                            cx,
                        )
                    });
                }
            }
            wait_for(&mut harness, |editor| {
                !editor.busy && editor.failure.is_some()
            });
            harness.read(|editor| assert_eq!(editor.failure.as_ref().unwrap().kind, kind));
            harness.read(|editor| assert_eq!(editor.document.project, before));
            harness.hover_at(egui::pos2(10., size.y - 10.));
            harness.run();
            assert!(!harness.get_by_label("OK").accesskit_node().is_disabled());
            let name = match kind {
                FailureKind::FlatReference => "reference",
                FailureKind::FlatSupport => "support",
                FailureKind::FlatCorrection => "correction",
                FailureKind::Save => "save",
                FailureKind::Export => "export",
                FailureKind::Look => "look",
                FailureKind::LookRendering => "look-rendering",
            };
            let png = harness.render().unwrap();
            if let Some(output) = std::env::var_os("ANTITONE_UI_CAPTURE") {
                let output = PathBuf::from(output);
                std::fs::create_dir_all(&output).unwrap();
                png.save(output.join(format!("feedback-{name}-{layout}-offscreen.png")))
                    .unwrap();
            }
            if kind == FailureKind::Save {
                assert!(harness.query_by_label("Save changes").is_none());
                if harness
                    .get_by_label("Details")
                    .accesskit_node()
                    .data()
                    .is_expanded()
                    != Some(true)
                {
                    harness.get_by_label("Details").click();
                    harness.run();
                }
                assert_eq!(
                    harness
                        .get_by_label("Details")
                        .accesskit_node()
                        .data()
                        .is_expanded(),
                    Some(true)
                );
                harness.hover_at(egui::pos2(10., size.y - 10.));
                harness.run();
                let png = harness.render().unwrap();
                if let Some(output) = std::env::var_os("ANTITONE_UI_CAPTURE") {
                    png.save(
                        PathBuf::from(output)
                            .join(format!("feedback-save-details-{layout}-offscreen.png")),
                    )
                    .unwrap();
                }
            }
            harness.get_by_label("OK").click();
            harness.run();
            harness.read(|editor| assert!(editor.failure.is_none()));
            if kind == FailureKind::Save {
                harness.read(|editor| assert_eq!(editor.transition, Some(Transition::Close)));
                harness.get_by_label("Cancel").click();
                harness.run();
                harness.read(|editor| assert!(!editor.allow_close));
            }
            if kind == FailureKind::FlatSupport {
                harness.read(|editor| assert!(editor.flat_edit.is_some()));
                harness.get_by_label("Cancel").click();
                harness.run();
            }
            harness.read(|editor| assert_eq!(editor.document.project, before));
            harness.read(|editor| assert_eq!(editor.current_source, selected));
            harness.read(|editor| assert!(editor.flat_edit.is_none()));
            harness.read(|editor| assert_eq!(editor.document.path.as_ref(), Some(&project_path)));
            harness.read(|editor| assert!(editor.document.dirty()));
            assert_eq!(std::fs::read(&project_path).unwrap(), saved_bytes);
        }
    }
}

fn wait_for(harness: &mut Harness<'_, Editor>, done: impl Fn(&Editor) -> bool) {
    wait_for_with_timeout(harness, Duration::from_secs(60), done);
}

fn wait_for_with_timeout(
    harness: &mut Harness<'_, Editor>,
    timeout: Duration,
    done: impl Fn(&Editor) -> bool,
) {
    let deadline = Instant::now() + timeout;
    loop {
        harness.read(|editor| {
            assert!(
                Instant::now() < deadline,
                "Editor wait timed out: {}; working updating={} error={:?}, dust error={:?}, evaluations={:?}, original pending={:?}, source epoch={}/{}",
                editor.status, editor.working.updating, editor.working.error,
                editor.document.dust.render_error, editor.dust_signature().map(|s| editor.dust_evaluations(s).len()), editor.original.refining,
                editor.source_epoch.load(std::sync::atomic::Ordering::Relaxed), editor.id
            )
        });
        if harness.read(&done) {
            break;
        }
        harness.step();
        std::thread::sleep(Duration::from_millis(5));
    }
    harness.step();
}

fn editor_harness<R>(test: impl FnOnce(Harness<'_, Editor>, Rc<RefCell<FloatCapture>>) -> R) -> R {
    editor_harness_with_thumbnails(false, test)
}

fn editor_harness_with_thumbnails<R>(
    thumbnails: bool,
    test: impl FnOnce(Harness<'_, Editor>, Rc<RefCell<FloatCapture>>) -> R,
) -> R {
    editor_harness_with_gpu(thumbnails, |harness, capture, _, _| test(harness, capture))
}

fn editor_harness_with_gpu<R>(
    thumbnails: bool,
    test: impl FnOnce(Harness<'_, Editor>, Rc<RefCell<FloatCapture>>, wgpu::Device, wgpu::Queue) -> R,
) -> R {
    let (device, queue) = pollster::block_on(async {
        let instance = crate::gpu::instance();
        let adapter = instance
            .request_adapter(&Default::default())
            .await
            .expect("composed UI test requires Metal");
        adapter.request_device(&Default::default()).await.unwrap()
    });
    let mut editor = Editor::new(&device, &queue);
    editor.thumbnails = if thumbnails {
        editor
            .thumbnails
            .without_disk(editor.developments.as_ref().unwrap().clone())
    } else {
        Thumbnails::default()
    };
    let capture = Rc::new(RefCell::new(FloatCapture::default()));
    let renderer = FloatRenderer::new(
        &device,
        &queue,
        host::App::submission_gate(&editor).clone(),
        capture.clone(),
    );
    let owner = RefCell::new(None);
    let harness = app_harness(&owner, editor, renderer);
    test(harness, capture, device, queue)
}

#[test]
fn dmin_picker_cancels_or_applies_measurement_and_paints_source() {
    editor_harness(|mut harness, capture| {
        let fixture =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng");
        harness.enter(|editor, cx| editor.add_raws(vec![fixture.clone()], cx));
        wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
        harness.read(|editor| {
            assert!(editor.document.visit.ready.is_some(), "{}", editor.status);
            assert!(editor.dmin().is_none());
        });
        let project = harness.read(|editor| editor.document.project.clone());
        harness.get_by_label("Pick Dmin (film base)").scroll_to_me();
        harness.try_run_realtime().unwrap();
        harness.get_by_label("Pick Dmin (film base)").click();
        harness.step();
        assert_eq!(harness.read(|editor| editor.pick), Some(Pick::Dmin));
        assert!(harness.get_by_label("Apply").accesskit_node().is_disabled());
        harness.get_by_label("Cancel").click();
        harness.step();
        harness.read(|editor| {
            assert!(editor.pick.is_none());
            assert_eq!(editor.document.project, project);
        });
        harness.get_by_label("Pick Dmin (film base)").click();
        harness.step();
        assert_eq!(harness.read(|editor| editor.pick), Some(Pick::Dmin));
        let point = harness.read(|editor| {
            editor
                .last_layout
                .unwrap()
                .0
                .screen_point(egui::pos2(32., 32.))
        });
        harness.hover_at(point);
        for pressed in [true, false] {
            harness.event(egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
        }
        harness.step();
        assert!(harness.read(|editor| editor.region.is_some()));
        harness.get_by_label("Apply").click();
        harness.step();
        assert!(harness.read(|editor| editor.busy));
        // This fixture delivers completion without another UI pass.
        harness.run_until(Duration::from_secs(5), |editor| !editor.busy);
        harness.read(|editor| {
            assert!(editor.pick.is_none());
            assert_eq!(editor.status, "Dmin measured");
        });
        let measured = harness.read(|editor| editor.dmin().unwrap());
        assert!((measured - DVec3::splat(3000. / 4031.)).abs().max_element() <= 2. / 65535.);
        assert_eq!(harness.read(|editor| editor.roll), project.roll);
        let mut expected_project = project;
        expected_project.set_dmin(Some(measured.to_array()));
        harness.read(|editor| {
            assert_eq!(editor.document.project, expected_project);
            assert!(editor.preference_store.is_none());
        });
        let timing = Arc::new(SelectionTiming::new(
            harness.read(|editor| editor.id),
            Instant::now(),
        ));
        harness.update(|editor| editor.selection_timing = Some(timing.clone()));
        harness.step();
        let png = harness.render().unwrap();
        assert!(
            timing.ready_was_painted(),
            "production selected-image callback did not paint"
        );
        let geometry = harness.read(|editor| editor.last_layout.unwrap().0);
        let output = capture.borrow();
        assert_eq!(png.dimensions(), (output.size[0], output.size[1]));
        harness.read(|editor| {
            assert!(editor.source);
            assert_eq!(editor.source_exposure, 0.);
        });
        let metadata = crate::raw::RawFile::open(&fixture).unwrap().metadata;
        let target = harness.read(|editor| editor.target);
        let cpu = crate::rendering::CpuRenderer::new(target).unwrap();
        for (source, code) in [(egui::pos2(32., 32.), 3000.), (egui::pos2(96., 32.), 128.)] {
            let position = geometry.screen_point(source);
            let pixel = output.pixels[position.y.floor() as usize * output.size[0] as usize
                + position.x.floor() as usize];
            // Source inspection bypasses inversion, not ACES or managed presentation encoding.
            let working = metadata.camera_to_ap1().unwrap() * DVec3::splat(2. * code / 4031.);
            let linear = target.linear_to_srgb() * cpu.render(&[working]).unwrap()[0].as_dvec3();
            for c in 0..3 {
                let value = linear[c].abs();
                let expected = linear[c].signum()
                    * if value <= 0.0031308 {
                        value * 12.92
                    } else {
                        1.055 * value.powf(1. / 2.4) - 0.055
                    };
                assert!(
                    (f64::from(pixel[c]) - expected).abs() <= 0.001,
                    "{pixel:?} vs {expected}"
                );
            }
        }
        save_capture(&png, "authored-dmin-offscreen");
    });
}

fn scan_setup(path: &std::path::Path, fixed_wb: [f64; 4]) -> crate::project::ScanSetup {
    let source = document::read_source(path, Some(fixed_wb)).unwrap();
    assert_eq!(source.development.decode.fixed_wb, fixed_wb);
    crate::project::ScanSetup {
        basis: crate::project::ScanBasis::new(&source),
        flat_field: None,
    }
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/raw")
        .join(name)
}

fn stage(harness: &mut Harness<'_, Editor>, name: &str) {
    harness
        .get_by_role_and_label(egui::accesskit::Role::Tab, name)
        .click();
    harness.step();
    harness.try_run_realtime().unwrap();
}

fn select_target(harness: &mut Harness<'_, Editor>, target: OutputTarget) {
    let current = if harness.read(|editor| editor.target == OutputTarget::DisplayP3) {
        "Display P3"
    } else {
        "sRGB"
    };
    harness
        .get_by(|node| {
            node.role() == egui::accesskit::Role::ComboBox
                && node.value().as_deref() == Some(current)
        })
        .click();
    harness.step();
    harness.step();
    harness
        .get_by_label(if target == OutputTarget::DisplayP3 {
            "Display P3"
        } else {
            "sRGB"
        })
        .click();
    harness.step();
    harness.read(|editor| assert_eq!(editor.target, target));
}

fn drag(harness: &mut Harness<'_, Editor>, start: egui::Pos2, end: egui::Pos2) {
    harness.hover_at(start);
    harness.event(egui::Event::PointerButton {
        pos: start,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
    harness.hover_at(end);
    harness.step();
    harness.event(egui::Event::PointerButton {
        pos: end,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
}

fn set_layout(harness: &mut Harness<'_, Editor>, size: egui::Vec2, width: f32) {
    harness.set_size(size);
    let mut panel =
        egui::containers::panel::PanelState::load(&harness.ctx, egui::Id::new("controls")).unwrap();
    panel.outer_rect.min.x = panel.outer_rect.max.x - width;
    harness
        .ctx
        .data_mut(|data| data.insert_persisted(egui::Id::new("controls"), panel));
    harness.step();
}

/// Deliver a File command the way the native menu bar does; the editor draws no menu of its own.
fn native_file_action(harness: &mut Harness<'_, Editor>, action: crate::platform::menu::Action) {
    harness.update(|editor| host::App::menu_action(editor, action));
    harness.step();
}

fn save_existing_project(harness: &mut Harness<'_, Editor>) {
    let path = harness.read(|editor| {
        editor
            .document
            .path
            .clone()
            .expect("fixture save destination")
    });
    let project = harness.read(|editor| editor.document.project.clone());
    let metadata = harness.read(|editor| editor.document.capture().ready_spatial_metadata());
    let request = harness.read(|editor| editor.next_request);
    native_file_action(harness, crate::platform::menu::Action::Save);
    harness.read(|editor| assert!(editor.next_request > request, "Save did not dispatch"));
    wait_for(harness, |e| !e.busy && e.document_pending.is_none());
    harness.read(|editor| assert!(!editor.document.dirty(), "{}", editor.status));
    harness.read(|editor| assert_eq!(editor.document.project, project));
    let (accepted, regions) = crate::project::Project::load_with_spatial_metadata(&path).unwrap();
    assert_eq!(accepted, project);
    assert_eq!(regions, metadata);
}

// Supply the native chooser's result; exercise the real transition guard and worker.
fn open_project(harness: &mut Harness<'_, Editor>, path: PathBuf) {
    harness.enter(|editor, cx| editor.guard_transition(files::Transition::Open(path), cx));
    harness.step();
    harness.step();
}

fn draw_region(harness: &mut Harness<'_, Editor>, area: Rect) {
    harness.get_by_label("Fit").click();
    harness.step();
    let geometry = harness.read(|editor| editor.last_layout.unwrap().0);
    harness.read(|editor| assert!(editor.region.is_none()));
    drag(
        harness,
        geometry.screen_point(egui::pos2(area.x as f32, area.y as f32)),
        geometry.screen_point(egui::pos2(
            (area.x + area.width) as f32,
            (area.y + area.height) as f32,
        )),
    );
    harness.read(|editor| {
        assert_eq!(
            editor
                .region
                .unwrap()
                .exact_rectangle(editor.document.visit.ready.as_ref().unwrap().area),
            Some(area)
        )
    });
}

fn save_capture(png: &image::RgbaImage, name: &str) {
    if let Some(directory) = std::env::var_os("ANTITONE_UI_CAPTURE") {
        let directory = PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        png.save(directory.join(format!("{name}.png"))).unwrap();
    }
}

#[test]
#[should_panic(expected = "Editor wait timed out")]
fn unfinished_editor_operation_fails_at_wait_deadline() {
    editor_harness(|mut harness, _| {
        harness.update(|editor| editor.busy = true);
        wait_for_with_timeout(&mut harness, Duration::ZERO, |e| !e.busy);
    });
}

#[test]
fn stage_tabs_preserve_project_and_fit_normal_and_narrow_layouts() {
    editor_harness(|mut harness, _capture| {
        let fixture =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng");
        harness.enter(|editor, cx| editor.add_raws(vec![fixture], cx));
        wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
        harness.read(|editor| assert!(editor.document.visit.ready.is_some(), "{}", editor.status));
        harness.update(|editor| {
            editor.set_dmin(Some(DVec3::splat(3000. / 4031.)));
            editor.set_dmax(Some(DVec3::splat(128. / 4031.)));
            editor.set_gamma(0.75);
            editor.adjustment_layer_settings.exposure = 0.5;
            editor.adjustment_layer_settings.wb = Balance {
                warmth: 0.1,
                tint: -0.05,
            };
            editor.source = false;
            editor.sync_document();
        });
        let project = harness.read(|editor| editor.document.project.clone());
        let source = harness.read(|editor| editor.current_source);
        for (layout, size, width) in [
            ("normal", egui::vec2(1100., 800.), 340.),
            ("narrow", egui::vec2(800., 600.), 270.),
        ] {
            set_layout(&mut harness, size, width);
            for (index, stage, heading, absent) in [
                (0, "Roll", "Reset scan orientation", "Crop & straighten"),
                (1, "Photo", "Crop & straighten", "Target"),
                (2, "Rendering", "Target", "Reset scan orientation"),
            ] {
                harness
                    .get_by_role_and_label(egui::accesskit::Role::Tab, stage)
                    .click();
                harness.step();
                harness.try_run_realtime().unwrap();
                harness.read(|editor| assert_eq!(editor.tab, index));
                for name in ["Roll", "Photo", "Rendering"] {
                    let tab = harness.get_by_role_and_label(egui::accesskit::Role::Tab, name);
                    assert_eq!(tab.accesskit_node().is_selected(), Some(name == stage));
                    let rect = tab.rect();
                    assert!(
                        rect.left() >= size.x - width && rect.right() <= size.x,
                        "{layout}/{stage}: {name} {rect:?}"
                    );
                    assert!(rect.top() >= 0. && rect.bottom() <= size.y);
                }
                let header = harness
                    .get_by_role_and_label(egui::accesskit::Role::Button, heading)
                    .rect();
                let tabs = harness
                    .get_by_role_and_label(egui::accesskit::Role::Tab, stage)
                    .rect();
                assert!(header.top() >= tabs.bottom());
                assert!(header.left() >= size.x - width && header.right() <= size.x);
                assert!(harness.query_by_label(absent).is_none());
                harness.read(|editor| assert_eq!(editor.document.project, project));
                harness.read(|editor| assert_eq!(editor.current_source, source));
                harness.read(|editor| assert!(editor.preference_store.is_none()));
                workflow::wait_settled(&mut harness);
                harness.render().unwrap();
                wait_for(&mut harness, |e| {
                    let gpu = e.scopes.gpu.as_ref().expect("scope GPU resources");
                    let input = e.scope_request().unwrap();
                    let frame = input.frame();
                    let settings = crate::scopes::Settings {
                        target: e.target,
                        plot: crate::scopes::Plot::Vector,
                        amplification: 1,
                    };
                    let Some(counts) = input.status(gpu, settings) else {
                        return false;
                    };
                    let counts = counts.expect("scope diagnostics");
                    assert_eq!(counts.invalid, 0);
                    assert_eq!(counts.valid, frame.output[0] * frame.output[1]);
                    true
                });
                let timing = harness
                    .read(|editor| Arc::new(SelectionTiming::new(editor.id, Instant::now())));
                harness.update(|editor| editor.selection_timing = Some(timing.clone()));
                harness.step();
                let png = harness.render().unwrap();
                assert_eq!(png.dimensions(), (size.x as u32, size.y as u32));
                assert!(timing.ready_was_painted());
                save_capture(
                    &png,
                    &format!("{}-{layout}-offscreen", stage.to_lowercase()),
                );
            }
        }
        harness.read(|editor| assert_eq!(editor.document.project, project));
        harness.read(|editor| assert_eq!(editor.current_source, source));
    });
}
