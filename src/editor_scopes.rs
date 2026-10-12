use super::*;
use crate::{
    frame_raster::FrameRaster,
    geometry::FramePoint,
    gpu_scopes::{GpuScopes, ScopePaint},
    scopes::{Basis, Plot, Settings, references},
};

pub(super) struct Scopes {
    pub(super) visible: bool,
    pub(super) vector: bool,
    pub(super) waveform: Plot,
    pub(super) amplification: u32,
    height_ratio: f32,
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
    caption: egui::Rect,
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
    fn panel(
        &mut self,
        ui: &mut egui::Ui,
        target: OutputTarget,
        reserved_height: f32,
    ) -> Option<Slot> {
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.visible, "Scopes");
            if self.visible {
                egui::ComboBox::from_id_salt("scope-plot")
                    .selected_text(if self.vector {
                        "Vectorscope"
                    } else {
                        "Waveform"
                    })
                    .width(120.)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.vector, false, "Waveform");
                        ui.selectable_value(&mut self.vector, true, "Vectorscope");
                    });
            }
        });
        if !self.visible {
            return None;
        }
        icons::row(ui, |ui| {
            if self.vector {
                ui.small("Scale");
                for gain in [1, 2, 4, 8] {
                    ui.selectable_value(&mut self.amplification, gain, format!("{gain}×"));
                }
                if icons::button(
                    ui,
                    icons::Icon::Reset,
                    self.amplification != 1,
                    "Reset vectorscope scale",
                    "Reset vectorscope scale to 1×",
                )
                .clicked()
                {
                    self.amplification = 1;
                }
            } else {
                ui.selectable_value(&mut self.waveform, Plot::Rgb, "RGB");
                ui.selectable_value(&mut self.waveform, Plot::Luma, "Luma");
            }
        });
        let width = ui.available_width();
        // The caller also reserves its pinned orientation/composition controls.
        let max_height = (ui.available_height() - reserved_height).max(96.);
        let height = (width * self.height_ratio).clamp(96., max_height);
        let (plot, response) =
            ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
        let mut help = String::from(
            "Approximate whole-frame scope from nearest-centre samples, after the selected ACES output transform and target transfer. Independent of viewer zoom/pan. Calibration and exports never use these samples.",
        );
        if self.vector {
            help.push_str("\nRim letters/ticks indicate colour direction only; small boxes mark 100% RGB/CMY references. The dashed Skin line is an approximate positive-I hue guide, not a skin-colour correctness test or a standard Display P3 axis.");
        }
        response.on_hover_text(help);
        let (caption, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.), egui::Sense::hover());
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
            caption,
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

impl Editor {
    pub(super) fn scope_panel(&mut self, ui: &mut egui::Ui) -> Option<Slot> {
        let rows = if self.tab == 1 && self.crop_edit.is_some() {
            10.
        } else {
            6.
        };
        let geometry_height = rows * (ui.spacing().interact_size.y + ui.spacing().item_spacing.y);
        self.scopes.panel(ui, self.target, 120. + geometry_height)
    }

    pub(super) fn scope_input(
        &self,
    ) -> Result<(Arc<GpuImage>, ProcessingParameters, FrameRaster), &'static str> {
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
            .positive_parameters(calibration, self.roll, self.photo)
            .map_err(|_| "Positive grade unavailable")?;
        let composition = Composition::with_crop(
            ready.area,
            self.roll_orientation,
            self.photo_orientation,
            self.crop,
        )
        .map_err(|_| "Composition unavailable")?;
        let image = ready.images[self.target as usize].clone();
        // Preserve the whole-source proxy's sampling pitch rather than imply finer crop detail.
        let pitch = (f64::from(ready.area.width) / f64::from(image.linear.width()))
            .max(f64::from(ready.area.height) / f64::from(image.linear.height()));
        let extent = composition.size();
        let size = extent.map(|v| (f64::from(v) / pitch).round().clamp(1., 1024.) as u32);
        let frame = FrameRaster::new(
            composition,
            FramePoint([0., 0.]),
            FramePoint(extent.map(f64::from)),
            size,
        )
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
        let input = self.scope_input();
        let (image, parameters, frame) = match input {
            Ok(input) => input,
            Err(reason) => {
                if let Some(gpu) = &self.scopes.gpu {
                    gpu.invalidate();
                }
                slot.painter.text(
                    slot.plot.center(),
                    egui::Align2::CENTER_CENTER,
                    reason,
                    egui::FontId::proportional(12.),
                    egui::Color32::GRAY,
                );
                return;
            }
        };
        let Some(gpu) = &self.scopes.gpu else {
            slot.painter.text(
                slot.plot.center(),
                egui::Align2::CENTER_CENTER,
                self.scopes.error.as_deref().unwrap_or("Scopes unavailable"),
                egui::FontId::proportional(11.),
                egui::Color32::LIGHT_RED,
            );
            return;
        };
        let mut trace = slot.plot.shrink(4.);
        if slot.settings.plot == Plot::Vector {
            trace = egui::Rect::from_center_size(
                trace.center(),
                egui::Vec2::splat(trace.height().min(trace.width())),
            );
        } else {
            trace.min.x += 24.;
        }
        slot.painter
            .with_clip_rect(slot.plot)
            .add(egui_wgpu::Callback::new_paint_callback(
                trace,
                ScopePaint::new(
                    gpu.clone(),
                    image.clone(),
                    parameters,
                    frame,
                    slot.settings,
                    slot.painter.ctx().clone(),
                ),
            ));
        graticule(
            &slot.painter.with_clip_rect(slot.plot),
            trace,
            slot.settings,
        );
        let target = if self.target == OutputTarget::Srgb {
            "sRGB"
        } else {
            "Display P3"
        };
        slot.painter.text(
            slot.caption.center_top(),
            egui::Align2::CENTER_TOP,
            format!("{target} · Whole frame · Approximate"),
            egui::FontId::proportional(10.),
            egui::Color32::GRAY,
        );
        let (text, color) = match gpu.status(&image, parameters, frame, slot.settings) {
            Some(Ok(counts)) if counts.invalid > 0 => (
                format!("Invalid rendered samples: {}", counts.invalid),
                egui::Color32::LIGHT_RED,
            ),
            Some(Ok(counts)) if counts.outside > 0 => (
                format!(
                    "Outside plot: {:.1}%",
                    100. * counts.outside as f64 / counts.valid.max(1) as f64
                ),
                egui::Color32::GRAY,
            ),
            Some(Ok(_)) => (
                format!("{}×{} samples", frame.output[0], frame.output[1]),
                egui::Color32::GRAY,
            ),
            Some(Err(error)) => (error, egui::Color32::LIGHT_RED),
            None => ("Updating…".into(), egui::Color32::GRAY),
        };
        slot.painter.text(
            slot.caption.center_bottom(),
            egui::Align2::CENTER_BOTTOM,
            text,
            egui::FontId::proportional(10.),
            color,
        );
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
    let skin_color = egui::Color32::from_rgb(200, 154, 85);
    painter.extend(egui::Shape::dashed_line(
        &[center, skin_end],
        egui::Stroke::new(1., egui::Color32::from_rgb(160, 123, 68)),
        4.,
        4.,
    ));
    painter.text(
        center.lerp(skin_end, 0.65) - egui::vec2(3., 3.),
        egui::Align2::RIGHT_BOTTOM,
        "Skin",
        egui::FontId::proportional(9.),
        skin_color,
    );
    for (label, rgb) in references() {
        let direction = basis.vector(rgb);
        let color = egui::Color32::from_rgb(
            (80. + 150. * rgb.x) as u8,
            (80. + 150. * rgb.y) as u8,
            (80. + 150. * rgb.z) as u8,
        );
        let galley = painter.layout_no_wrap(label.into(), egui::FontId::proportional(10.), color);
        let label_rect =
            egui::Rect::from_center_size(direction_on_rim(rect, direction, 12.), galley.size());
        let point = direction * scale;
        let position = center + egui::vec2(point.x as f32, -point.y as f32);
        let marker = egui::Rect::from_center_size(position, egui::vec2(5., 5.));
        if rect.shrink(7.).contains(position) && !label_rect.expand(2.).intersects(marker) {
            painter.rect_stroke(
                marker,
                0.,
                egui::Stroke::new(1., color),
                egui::StrokeKind::Inside,
            );
        }
        // Direction ticks stay on the rim; unlike the boxes, they do not indicate chroma magnitude.
        painter.line_segment(
            [
                direction_on_rim(rect, direction, 2.),
                direction_on_rim(rect, direction, 7.),
            ],
            egui::Stroke::new(1., color),
        );
        painter.galley(label_rect.min, galley, color);
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
