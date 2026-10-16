use super::*;
use crate::{
    frame_raster::FrameRaster,
    gpu_scopes::{GpuScopes, ScopePaint},
    scopes::{Basis, Plot, Settings, references},
};

pub(super) struct Scopes {
    pub(super) visible: bool,
    pub(super) vector: bool,
    pub(super) waveform: Plot,
    pub(super) amplification: u32,
    pub(super) height_ratio: f32,
    pub(super) gpu: Option<Arc<GpuScopes>>,
    error: Option<String>,
}

impl Default for Scopes {
    fn default() -> Self {
        Self {
            visible: true,
            vector: true,
            waveform: Plot::Rgb,
            amplification: 1,
            height_ratio: 0.85,
            gpu: None,
            error: None,
        }
    }
}

pub(super) struct Slot {
    painter: egui::Painter,
    plot: egui::Rect,
    response: egui::Response,
    settings: Settings,
}

impl Scopes {
    pub(super) fn menu_label(&self) -> String {
        if self.vector {
            format!("Vectorscope · {}×", self.amplification)
        } else if self.waveform == Plot::Luma {
            "Luma waveform".into()
        } else {
            "RGB waveform".into()
        }
    }

    pub(super) fn new(device: &wgpu::Device) -> Self {
        let mut state = Self::default();
        match pollster::block_on(GpuScopes::new(device)) {
            Ok(gpu) => state.gpu = Some(Arc::new(gpu)),
            Err(error) => state.error = Some(error.to_string()),
        }
        state
    }

    fn panel(
        &mut self,
        ui: &mut egui::Ui,
        target: OutputTarget,
        reserved_height: f32,
    ) -> Option<Slot> {
        icons::row(ui, |ui| {
            let width = ui.available_width();
            // Reserve a stable menu width so switching plots never moves the disclosure.
            let menu_width = 156.;
            let was_visible = self.visible;
            let heading_width = if was_visible {
                (width - menu_width - ui.spacing().item_spacing.x).max(1.)
            } else {
                width
            };
            presentation::section_disclosure(ui, "Scopes", &mut self.visible, heading_width);
            if was_visible != self.visible {
                ui.ctx().request_repaint();
            }
            // Expansion used the full-width heading this frame; show its menu on the next layout.
            if was_visible && self.visible {
                egui::ComboBox::from_id_salt("scope-settings")
                    .selected_text(self.menu_label())
                    .width(menu_width)
                    .show_ui(ui, |ui| {
                        let mut plot = if self.vector {
                            Plot::Vector
                        } else {
                            self.waveform
                        };
                        for (value, label) in [
                            (Plot::Vector, "Vectorscope"),
                            (Plot::Rgb, "RGB waveform"),
                            (Plot::Luma, "Luma waveform"),
                        ] {
                            if ui.selectable_value(&mut plot, value, label).clicked() {
                                self.vector = plot == Plot::Vector;
                                if !self.vector {
                                    self.waveform = plot;
                                }
                                ui.close();
                            }
                        }
                        if self.vector {
                            ui.separator();
                            ui.weak("Magnification");
                            for gain in [1, 2, 4, 8] {
                                if ui
                                    .selectable_value(
                                        &mut self.amplification,
                                        gain,
                                        format!("{gain}×"),
                                    )
                                    .clicked()
                                {
                                    ui.close();
                                }
                            }
                        }
                    });
            }
        });
        if !self.visible {
            return None;
        }
        let width = ui.available_width();
        // Keep a usable inspector scroll region below the independent scope chrome.
        let max_height =
            (ui.available_height() - reserved_height - 8. - ui.spacing().item_spacing.y).max(96.);
        let height = (width * self.height_ratio).clamp(96., max_height);
        let (plot, response) =
            ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
        let plot_response = response;
        let (divider, response) =
            ui.allocate_exact_size(egui::vec2(width, 8.), egui::Sense::click_and_drag());
        let response = response
            .on_hover_cursor(egui::CursorIcon::ResizeVertical)
            .on_hover_text("Drag to resize scopes; double-click to reset.");
        if response.double_clicked() {
            self.height_ratio = 0.85;
            ui.ctx().request_repaint();
        } else if response.dragged() && response.drag_delta().y != 0. {
            self.height_ratio = (height + response.drag_delta().y).clamp(96., max_height) / width;
            ui.ctx().request_repaint();
        }
        let stroke = egui::Stroke::new(
            1.,
            egui::Color32::from_gray(if response.hovered() || response.dragged() {
                120
            } else {
                65
            }),
        );
        ui.painter()
            .line_segment([divider.left_center(), divider.right_center()], stroke);
        ui.painter().line_segment(
            [
                divider.center() - egui::vec2(12., 0.),
                divider.center() + egui::vec2(12., 0.),
            ],
            egui::Stroke::new(2., stroke.color),
        );
        Some(Slot {
            painter: ui.painter().with_clip_rect(ui.clip_rect()),
            plot,
            response: plot_response,
            settings: Settings {
                target,
                plot: if self.vector {
                    Plot::Vector
                } else {
                    self.waveform
                },
                amplification: if self.vector { self.amplification } else { 1 },
            },
        })
    }
}

pub(super) enum ScopeInput {
    Proxy(Arc<GpuImage>, Box<ProcessingInput>, FrameRaster),
    Rendered(Arc<crate::gpu::view::RenderedView>, FrameRaster),
}

impl ScopeInput {
    #[cfg(test)]
    pub(super) fn frame(&self) -> FrameRaster {
        match self {
            Self::Proxy(_, _, f) | Self::Rendered(_, f) => *f,
        }
    }

    pub(super) fn status(
        &self,
        gpu: &GpuScopes,
        settings: Settings,
    ) -> Option<Result<crate::scopes::Counts, String>> {
        match self {
            Self::Proxy(image, input, frame) => {
                gpu.status_processing(image, input, *frame, settings)
            }
            Self::Rendered(view, frame) => gpu.status_rendered(view, *frame, settings),
        }
    }

    fn paint(
        &self,
        gpu: Arc<GpuScopes>,
        settings: Settings,
        submissions: crate::gpu::SubmissionGate,
        context: egui::Context,
    ) -> ScopePaint {
        match self {
            Self::Rendered(view, frame) => {
                ScopePaint::rendered(gpu, view.clone(), *frame, settings, submissions, context)
            }
            Self::Proxy(image, input, frame) => ScopePaint::with_processing(
                gpu,
                image.clone(),
                input.as_ref().clone(),
                *frame,
                settings,
                context,
            )
            .expect("validated scope input"),
        }
    }
}

impl Editor {
    pub(super) fn scope_request(&self) -> Result<ScopeInput, &'static str> {
        let (ready, parameters, composition) = self.scope_basis()?;
        if let Some(view) = &self.working.displayed
            && let Some(visible) = self.working.current_frame()
        {
            // Bound scope sampling, not photographic detail. Read the accepted target raster without
            // grading, ACES, source-proxy mapping or a second composition.
            let scale = (f64::from(visible.output[0].max(visible.output[1])) / 1024.).max(1.);
            let frame = FrameRaster {
                output: visible.output.map(|v| (f64::from(v) / scale).ceil() as u32),
                ..visible
            };
            Ok(ScopeInput::Rendered(view.clone(), frame))
        } else {
            let (image, parameters, frame) =
                self.proxy_scope_input(ready, parameters, composition)?;
            Ok(ScopeInput::Proxy(image, Box::new(parameters), frame))
        }
    }

    pub(super) fn scope_panel(&mut self, ui: &mut egui::Ui) -> Option<Slot> {
        egui::Frame::new()
            .fill(ui.visuals().faint_bg_color)
            .inner_margin(4)
            .show(ui, |ui| self.scopes.panel(ui, self.target, 180.))
            .inner
    }

    #[cfg(test)]
    pub(super) fn scope_input(
        &self,
    ) -> Result<(Arc<GpuImage>, ProcessingInput, FrameRaster), &'static str> {
        let (ready, parameters, composition) = self.scope_basis()?;
        self.proxy_scope_input(ready, parameters, composition)
    }

    fn scope_basis(&self) -> Result<(&Ready, ProcessingInput, Composition), &'static str> {
        if self.source || self.pick_return.is_some() || self.flat_edit.is_some() {
            return Err("Positive grade only");
        }
        if self.busy || self.document_pending.is_some() {
            return Err("Waiting for image");
        }
        if self.crop_edit.is_some() {
            return Err("Apply crop to update scopes");
        }
        let ready = self.ready.as_ref().ok_or("No image")?;
        let calibration = self.calibration().ok_or("Calibration required")?;
        let parameters = self
            .view_processing(calibration, self.roll, self.photo)
            .map_err(|_| "Positive grade unavailable")?;
        let composition = Composition::with_crop(
            ready.area,
            self.roll_orientation,
            self.photo_orientation,
            if self.auto_crop_context() {
                None
            } else {
                self.crop
            },
        )
        .map_err(|_| "Composition unavailable")?;
        self.last_layout.ok_or("Waiting for view")?;
        Ok((ready, parameters, composition))
    }

    fn proxy_scope_input(
        &self,
        ready: &Ready,
        parameters: ProcessingInput,
        composition: Composition,
    ) -> Result<(Arc<GpuImage>, ProcessingInput, FrameRaster), &'static str> {
        let image = ready
            .proxy
            .image(self.target)
            .map_err(|_| "View unavailable")?;
        if parameters.grade().is_some() {
            image
                .require_processing(&parameters)
                .map_err(|_| "Positive grade unavailable")?;
        }
        let (layout, pixels_per_point) = self.last_layout.ok_or("Waiting for view")?;
        let mut viewport = self.viewport;
        let visible = viewport
            .layout_composition(layout.canvas, composition, pixels_per_point)
            .frame_raster(pixels_per_point);
        // Zoom changes coverage, not the retained proxy's source sampling pitch.
        let pitch = (f64::from(ready.area.width) / f64::from(image.linear.width()))
            .max(f64::from(ready.area.height) / f64::from(image.linear.height()));
        let size = std::array::from_fn(|i| {
            ((visible.max.0[i] - visible.min.0[i]) / pitch)
                .round()
                .clamp(1., 1024.) as u32
        });
        let frame = FrameRaster::new(composition, visible.min, visible.max, size)
            .map_err(|_| "Composition unavailable")?;
        Ok((image, parameters, frame))
    }

    pub(super) fn paint_scope(&self, slot: Option<Slot>) {
        let Some(mut slot) = slot else {
            if let Some(gpu) = &self.scopes.gpu {
                gpu.invalidate();
            }
            return;
        };
        // Output can change later in the controls; use the accepted end-of-frame target/settings.
        slot.settings.target = self.target;
        slot.painter
            .rect_filled(slot.plot, 3., egui::Color32::from_gray(6));
        if let Some(error) = self.look_render_failure() {
            if let Some(gpu) = &self.scopes.gpu {
                gpu.invalidate();
            }
            scope_notice(
                &slot.painter,
                slot.plot,
                "Look rendering failed".into(),
                egui::Color32::LIGHT_RED,
                egui::Align2::CENTER_CENTER,
            );
            slot.response.on_hover_text(error.to_string());
            return;
        }
        let input = self.scope_request();
        let input = match input {
            Ok(input) => input,
            Err(reason) => {
                if let Some(gpu) = &self.scopes.gpu {
                    gpu.invalidate();
                }
                scope_notice(
                    &slot.painter,
                    slot.plot,
                    reason.into(),
                    egui::Color32::GRAY,
                    egui::Align2::CENTER_CENTER,
                );
                return;
            }
        };
        let Some(gpu) = &self.scopes.gpu else {
            let error = self.scopes.error.as_deref().unwrap_or("Scopes unavailable");
            if scope_notice(
                &slot.painter,
                slot.plot,
                error.into(),
                egui::Color32::LIGHT_RED,
                egui::Align2::CENTER_CENTER,
            ) {
                slot.response.on_hover_text(error);
            }
            return;
        };
        let mut trace = slot.plot.shrink(2.);
        if slot.settings.plot == Plot::Vector {
            trace = egui::Rect::from_center_size(
                trace.center(),
                egui::Vec2::splat(trace.height().min(trace.width())),
            );
        } else {
            trace.min.x += 24.;
            // Endpoint labels need vertical breathing room; the vectorscope has no text gutter.
            trace.min.y += 4.;
            trace.max.y -= 4.;
        }
        let paint = input.paint(
            gpu.clone(),
            slot.settings,
            self.submissions.clone(),
            slot.painter.ctx().clone(),
        );
        slot.painter
            .with_clip_rect(slot.plot)
            .add(egui_wgpu::Callback::new_paint_callback(trace, paint));
        graticule(
            &slot.painter.with_clip_rect(slot.plot),
            trace,
            slot.settings,
        );
        let status = input.status(gpu, slot.settings);
        let status = if matches!(input, ScopeInput::Rendered(..)) && self.working.updating {
            None
        } else {
            status
        };
        if let Some((text, color)) = scope_feedback(status)
            && scope_notice(
                &slot.painter,
                slot.plot,
                text.clone(),
                color,
                egui::Align2::LEFT_BOTTOM,
            )
        {
            slot.response.on_hover_text(text);
        }
    }
}

fn scope_notice(
    painter: &egui::Painter,
    plot: egui::Rect,
    text: String,
    color: egui::Color32,
    align: egui::Align2,
) -> bool {
    // Exceptional feedback overlays the plot; only truncated messages need hover details.
    let painter = painter.with_clip_rect(plot);
    let mut job = egui::text::LayoutJob::simple(
        text,
        egui::FontId::proportional(10.),
        color,
        plot.width() - 8.,
    );
    job.wrap.max_rows = 3;
    job.wrap.break_anywhere = true;
    let galley = painter.layout_job(job);
    let rect = align.align_size_within_rect(galley.size(), plot.shrink(4.));
    painter.rect_filled(rect.expand(2.), 2., egui::Color32::from_gray(6));
    let truncated = galley.elided;
    painter.galley(rect.min, galley, color);
    truncated
}

fn scope_feedback(
    status: Option<Result<crate::scopes::Counts, String>>,
) -> Option<(String, egui::Color32)> {
    match status {
        Some(Ok(counts)) if counts.invalid > 0 => Some((
            format!("Invalid rendered samples: {}", counts.invalid),
            egui::Color32::LIGHT_RED,
        )),
        Some(Ok(counts)) if counts.outside > 0 => Some((
            format!(
                "Outside plot: {:.1}%",
                100. * counts.outside as f64 / counts.valid.max(1) as f64
            ),
            egui::Color32::GRAY,
        )),
        Some(Err(error)) => Some((error, egui::Color32::LIGHT_RED)),
        Some(Ok(_)) | None => None,
    }
}

fn graticule(painter: &egui::Painter, rect: egui::Rect, settings: Settings) {
    let stroke = egui::Stroke::new(1., egui::Color32::from_rgb(82, 72, 110));
    if settings.plot != Plot::Vector {
        for level in [0., 0.25, 0.5, 0.75, 1.] {
            let y = egui::lerp(rect.bottom()..=rect.top(), level);
            painter.line_segment(
                [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
                stroke,
            );
            painter.text(
                egui::pos2(rect.left() - 3., y),
                egui::Align2::RIGHT_CENTER,
                format!("{}", (level * 100.) as u32),
                egui::FontId::proportional(9.),
                egui::Color32::GRAY,
            );
        }
        return;
    }
    let painter = painter.with_clip_rect(rect);
    let basis = Basis::new(settings.target);
    let scale = rect.width() as f64 / (2. * basis.extent / settings.amplification as f64);
    let center = rect.center();
    painter.line_segment([rect.left_center(), rect.right_center()], stroke);
    painter.line_segment([rect.center_top(), rect.center_bottom()], stroke);
    for radius in [0.25, 0.5] {
        painter.circle_stroke(center, (radius * scale) as f32, stroke);
    }
    let skin_end = direction_on_rim(rect, basis.skin_reference_direction(), 8.);
    painter.extend(egui::Shape::dashed_line(
        &[center, skin_end],
        egui::Stroke::new(1., egui::Color32::from_rgb(160, 123, 68)),
        4.,
        4.,
    ));
    for (_, rgb) in references() {
        let direction = basis.vector(rgb);
        let color = egui::Color32::from_rgb(
            (80. + 150. * rgb.x) as u8,
            (80. + 150. * rgb.y) as u8,
            (80. + 150. * rgb.z) as u8,
        );
        let point = direction * scale;
        let position = center + egui::vec2(point.x as f32, -point.y as f32);
        let marker = egui::Rect::from_center_size(position, egui::vec2(5., 5.));
        if rect.contains_rect(marker) {
            painter.rect_stroke(
                marker,
                0.,
                egui::Stroke::new(1., color),
                egui::StrokeKind::Inside,
            );
        }
    }
}

fn direction_on_rim(rect: egui::Rect, direction: glam::DVec2, inset: f32) -> egui::Pos2 {
    let screen = egui::vec2(direction.x as f32, -direction.y as f32);
    let half = rect.size() * 0.5 - egui::Vec2::splat(inset);
    let scale = (half.x / screen.x.abs()).min(half.y / screen.y.abs());
    rect.center() + screen * scale
}

#[cfg(test)]
#[path = "editor_scopes_tests.rs"]
mod tests;
