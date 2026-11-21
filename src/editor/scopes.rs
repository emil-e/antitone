use super::*;
#[cfg(test)]
use crate::gpu::test_inputs::AuthoredPhotograph;
use crate::{
    frame_raster::FrameRaster,
    gpu_scopes::{GpuScopes, ScopePaint},
    scopes::{Basis, Plot, Settings, references},
};

/// Share of the column width given to the scope plot; the rest stays with the stage controls.
pub(super) const DEFAULT_HEIGHT_RATIO: f32 = 0.6;

const MAX_ZOOM_STEP: usize = 3;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ScopeKind {
    Vector,
    Waveform,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Waveform {
    Rgb,
    Luma,
}

pub(super) struct Scopes {
    pub(super) visible: bool,
    kind: ScopeKind,
    waveform: Waveform,
    zoom_step: usize,
    wheel_remainder: f32,
    pub(super) height_ratio: f32,
    pub(super) gpu: Option<Arc<GpuScopes>>,
    error: Option<String>,
}

impl Default for Scopes {
    fn default() -> Self {
        Self {
            visible: true,
            kind: ScopeKind::Vector,
            waveform: Waveform::Rgb,
            zoom_step: 0,
            wheel_remainder: 0.,
            height_ratio: DEFAULT_HEIGHT_RATIO,
            gpu: None,
            error: None,
        }
    }
}

pub(super) struct Slot {
    painter: egui::Painter,
    plot: egui::Rect,
    trace_well: egui::Rect,
    response: egui::Response,
    settings: Settings,
}

impl Scopes {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        let mut state = Self::default();
        match pollster::block_on(GpuScopes::new(device)) {
            Ok(gpu) => state.gpu = Some(Arc::new(gpu)),
            Err(error) => state.error = Some(error.to_string()),
        }
        state
    }

    pub(super) fn settings(&self, target: OutputTarget) -> Settings {
        let plot = match (self.kind, self.waveform) {
            (ScopeKind::Vector, _) => Plot::Vector,
            (ScopeKind::Waveform, Waveform::Rgb) => Plot::Rgb,
            (ScopeKind::Waveform, Waveform::Luma) => Plot::Luma,
        };
        Settings {
            target,
            plot,
            amplification: if plot == Plot::Vector { self.gain() } else { 1 },
        }
    }

    fn gain(&self) -> u32 {
        1 << self.zoom_step
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        use ui::{INSPECTOR_ROW_HEIGHT, icons};

        let row_height = INSPECTOR_ROW_HEIGHT;
        let selector = icons::IconSegments {
            choices: &[
                (ScopeKind::Vector, icons::Icon::Vectorscope, "Vectorscope"),
                (ScopeKind::Waveform, icons::Icon::Waveform, "Waveform"),
            ],
            height: row_height,
        };
        icons::row_at(ui, row_height, |ui| {
            let was_visible = self.visible;
            let gap = ui.spacing().item_spacing.x;
            let tools_width = row_height + selector.width(ui) + 3. * gap;
            let heading_width = if was_visible {
                (ui.available_width() - tools_width).max(1.)
            } else {
                ui.available_width()
            };
            ui::section_disclosure(ui, "Scopes", &mut self.visible, heading_width);
            if was_visible != self.visible {
                ui.ctx().request_repaint();
            }
            if !was_visible || !self.visible {
                return;
            }

            if self.kind == ScopeKind::Waveform {
                let button = icons::button_at(
                    ui,
                    icons::Icon::Inspection,
                    true,
                    "Waveform settings",
                    "Choose RGB or luma.",
                    row_height,
                );
                egui::Popup::from_toggle_button_response(&button)
                    .layout(egui::Layout::top_down(egui::Align::Min))
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .show(|ui| {
                        for (mode, label) in [(Waveform::Rgb, "RGB"), (Waveform::Luma, "Luma")] {
                            if ui
                                .selectable_value(&mut self.waveform, mode, label)
                                .clicked()
                            {
                                ui.close();
                            }
                        }
                    });
            } else {
                // The selector stays put when the waveform-only action appears.
                ui.allocate_space(egui::Vec2::splat(row_height));
            }

            ui.add_space(gap);
            selector.show(ui, &mut self.kind);
        });
    }

    fn zoom_by(&mut self, steps: isize) {
        self.zoom_step = self
            .zoom_step
            .saturating_add_signed(steps)
            .min(MAX_ZOOM_STEP);
    }

    fn wheel_zoom(&mut self, ui: &egui::Ui, plot: egui::Rect) {
        if self.kind != ScopeKind::Vector || !ui.is_enabled() || !ui.rect_contains_pointer(plot) {
            self.wheel_remainder = 0.;
            return;
        }

        ui.input(|input| {
            for event in &input.raw.events {
                let egui::Event::MouseWheel {
                    unit,
                    delta,
                    phase,
                    modifiers,
                } = event
                else {
                    continue;
                };
                if !modifiers.is_none() || delta.x.abs() > delta.y.abs() {
                    continue;
                }
                if *phase == egui::TouchPhase::Start {
                    self.wheel_remainder = 0.;
                }
                // A wheel line is one step. Accumulate high-resolution point input
                // instead of zooming a full step for every trackpad event.
                let steps = match unit {
                    egui::MouseWheelUnit::Point => delta.y / 32.,
                    egui::MouseWheelUnit::Line | egui::MouseWheelUnit::Page => delta.y,
                } + self.wheel_remainder;
                self.zoom_by(steps as isize);
                self.wheel_remainder =
                    if matches!(phase, egui::TouchPhase::End | egui::TouchPhase::Cancel)
                        || (self.zoom_step == 0 && steps < 0.)
                        || (self.zoom_step == MAX_ZOOM_STEP && steps > 0.)
                    {
                        0.
                    } else {
                        steps.fract()
                    };
            }
        });
    }

    fn zoom_rail(&mut self, ui: &mut egui::Ui, plot: egui::Rect) -> egui::Rect {
        use ui::{INSPECTOR_ROW_HEIGHT, icons};

        let size = INSPECTOR_ROW_HEIGHT;
        let inset = 3.;
        let rail = egui::Rect::from_min_max(
            egui::pos2(plot.right() - size - inset, plot.top() + inset),
            plot.right_bottom() - egui::Vec2::splat(inset),
        );
        let mut rail_ui = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("vector-zoom")
                .max_rect(rail)
                .layout(egui::Layout::top_down(egui::Align::Center)),
        );
        rail_ui.set_clip_rect(ui.clip_rect().intersect(plot));
        rail_ui.spacing_mut().item_spacing.y = 0.;

        let zoom_in = icons::button_at(
            &mut rail_ui,
            icons::Icon::ZoomIn,
            self.zoom_step < MAX_ZOOM_STEP,
            "Zoom in on vectorscope",
            "Zoom in on vectorscope",
            size,
        )
        .clicked();
        rail_ui.label(ui::instrument_readout(&format!("{}×", self.gain()), 11.));
        let zoom_out = icons::button_at(
            &mut rail_ui,
            icons::Icon::ZoomOut,
            self.zoom_step > 0,
            "Zoom out on vectorscope",
            "Zoom out on vectorscope",
            size,
        )
        .clicked();
        if zoom_in || zoom_out {
            self.zoom_by(if zoom_in { 1 } else { -1 });
            self.wheel_remainder = 0.;
            ui.ctx().request_repaint();
        }

        egui::Rect::from_min_max(plot.min, egui::pos2(rail.left() - inset, plot.bottom()))
    }

    fn panel(
        &mut self,
        ui: &mut egui::Ui,
        target: OutputTarget,
        reserved_height: f32,
    ) -> Option<Slot> {
        self.header(ui);
        if !self.visible {
            self.wheel_remainder = 0.;
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
        ui.painter()
            .rect_filled(plot, 3., egui::Color32::from_gray(6));
        self.wheel_zoom(ui, plot);
        let trace_well = if self.kind == ScopeKind::Vector {
            self.zoom_rail(ui, plot)
        } else {
            plot
        };

        let (divider, response) =
            ui.allocate_exact_size(egui::vec2(width, 6.), egui::Sense::click_and_drag());
        let response = response
            .on_hover_cursor(egui::CursorIcon::ResizeVertical)
            .on_hover_text("Drag to resize scopes; double-click to reset.");
        if response.double_clicked() {
            self.height_ratio = DEFAULT_HEIGHT_RATIO;
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
            trace_well,
            response: plot_response,
            settings: self.settings(target),
        })
    }
}

pub(super) struct ScopeInput {
    pub view: Arc<crate::gpu::view::RenderedView>,
    pub frame: FrameRaster,
    pub mapping: crate::geometry::TextureMapping,
}

impl ScopeInput {
    #[cfg(all(test, feature = "ui-tests"))]
    pub(super) fn frame(&self) -> FrameRaster {
        self.frame
    }

    pub(super) fn status(
        &self,
        gpu: &GpuScopes,
        settings: Settings,
    ) -> Option<Result<crate::scopes::Counts, String>> {
        gpu.status_mapped_rendered(&self.view, self.frame, self.mapping, settings)
    }

    fn paint(
        &self,
        gpu: Arc<GpuScopes>,
        settings: Settings,
        submissions: crate::gpu::submission::Queue,
        context: egui::Context,
    ) -> ScopePaint {
        ScopePaint::mapped_rendered(
            gpu,
            self.view.clone(),
            self.frame,
            self.mapping,
            settings,
            submissions,
            context,
        )
    }
}

impl Editor {
    pub(super) fn scope_request(&self) -> Option<ScopeInput> {
        let displayed = self.canvas_frame.displayed.as_ref()?;
        let mut frame = displayed.view.spec.frame();
        let scale = (f64::from(displayed.size[0].max(displayed.size[1])) / 1024.).max(1.);
        frame.output = displayed
            .size
            .map(|v| (f64::from(v) / scale).ceil().max(1.) as u32);
        Some(ScopeInput {
            view: displayed.view.clone(),
            frame,
            mapping: displayed.mapping,
        })
    }

    pub(super) fn scope_panel(&mut self, ui: &mut egui::Ui) -> Option<Slot> {
        egui::Frame::new()
            .fill(ui.visuals().faint_bg_color)
            .inner_margin(4)
            .show(ui, |ui| self.scopes.panel(ui, self.document.target(), 180.))
            .inner
    }

    #[cfg(test)]
    pub(super) fn scope_input(
        &self,
    ) -> Result<(Arc<GpuImage>, AuthoredPhotograph, FrameRaster), &'static str> {
        let ScopeInput { view, frame, .. } = self.scope_request().ok_or("No displayed image")?;
        let device = self
            .document
            .visit
            .working_view
            .device()
            .ok_or("No device")?;
        let input = view
            .capture
            .processing(&device.limits())
            .map_err(|_| "View processing unavailable")?;
        Ok((view.image.clone(), input, frame))
    }

    pub(super) fn paint_scope(&self, slot: Option<Slot>) {
        let Some(mut slot) = slot else {
            if let Some(gpu) = &self.scopes.gpu {
                gpu.invalidate();
            }
            return;
        };
        // The plotted codes belong to the painted image, even while a new target is pending.
        slot.settings.target = self
            .canvas_frame
            .displayed
            .as_ref()
            .map_or(self.document.target(), |image| image.view.target);
        let Some(input) = self.scope_request() else {
            if let Some(gpu) = &self.scopes.gpu {
                gpu.invalidate();
            }
            return;
        };
        let Some(gpu) = &self.scopes.gpu else {
            scope_notice(
                &slot.painter,
                slot.trace_well,
                "Scopes unavailable".into(),
                egui::Color32::LIGHT_RED,
                egui::Align2::CENTER_CENTER,
            );
            if let Some(error) = &self.scopes.error {
                slot.response.on_hover_text(error);
            }
            return;
        };
        let mut trace = slot.trace_well.shrink(2.);
        if slot.settings.plot == Plot::Vector {
            let side = trace.height().min(trace.width());
            let center_x = slot.plot.center().x.min(trace.right() - side * 0.5);
            trace = egui::Rect::from_center_size(
                egui::pos2(center_x, trace.center().y),
                egui::Vec2::splat(side),
            );
        } else {
            trace.min.x += 24.;
            // Endpoint labels need vertical breathing room; the vectorscope has no text gutter.
            trace.min.y += 4.;
            trace.max.y -= 4.;
        }
        let Some(queue) = self.document.visit.working_view.queue() else {
            return;
        };
        let paint = input.paint(
            gpu.clone(),
            slot.settings,
            queue.clone(),
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
        if let Some((text, color)) = scope_feedback(status)
            && scope_notice(
                &slot.painter,
                slot.trace_well,
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
            format!("Invalid samples: {}", counts.invalid),
            egui::Color32::LIGHT_RED,
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
mod tests;
