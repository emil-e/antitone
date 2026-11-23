use crate::editor::{Error, GpuImage, Presentation, correction, processing, ui};
use crate::geometry::{Composition, FramePoint};
use crate::gpu::photograph::request::PhotographRequest;
use std::sync::Arc;

/// What a card can show this pass, independent of how the next image is acquired.
pub(super) enum CardImage<'a> {
    Photo(Photograph<'a>),
    Camera(&'a egui::TextureHandle),
    Loading,
    Error(String),
    Empty,
}

impl CardImage<'_> {
    pub fn wants_camera(&self) -> bool {
        matches!(self, Self::Camera(_) | Self::Loading)
    }

    pub fn awaits_correction(&self) -> bool {
        matches!(self, Self::Photo(photo) if photo.awaits_correction())
    }

    pub fn show(
        self,
        ui: &egui::Ui,
        card: &ui::ContactSheetCard,
        name: &str,
        warning: Option<String>,
    ) -> Option<String> {
        use ui::{ContextualStatus, StatusLevel};
        let updating =
            matches!(&self, Self::Photo(photo) if photo.updating) || matches!(self, Self::Loading);
        let error = match self {
            Self::Photo(photo) => photo.draw(ui, card.image_rect).err(),
            Self::Camera(texture) => {
                draw_camera(ui, card.image_rect, texture);
                card.camera_badge(ui);
                None
            }
            Self::Loading => {
                card.loading(ui);
                None
            }
            Self::Error(error) => Some(error),
            Self::Empty => None,
        };

        if error.is_some() {
            card.error(ui);
        }
        card.caption(ui, name);
        let mut status = if let Some(error) = &error {
            Some(ContextualStatus::new(
                StatusLevel::Error,
                format!("Thumbnail unavailable: {error}"),
            ))
        } else if updating {
            Some(ContextualStatus::new(
                StatusLevel::Activity,
                "Preparing thumbnail; navigation preview may differ.",
            ))
        } else {
            None
        };
        if let Some(reason) = warning {
            let warning = ContextualStatus::new(StatusLevel::Warning, reason);
            status = Some(match status {
                Some(status) => status.combine(warning),
                None => warning,
            });
        }
        if let Some(status) = status {
            card.status(ui, &status);
        }
        error
    }
}

pub(super) struct Photograph<'a> {
    pub image: Arc<GpuImage>,
    pub parameters: Option<PhotographRequest>,
    pub composition: Composition,
    pub correction: correction::Resources,
    pub presentation: &'a Result<Presentation, Error>,
    pub outline: Option<(&'a [[f64; 2]; 4], [u32; 4])>,
    pub updating: bool,
}

impl Photograph<'_> {
    fn awaits_correction(&self) -> bool {
        self.parameters.is_some() && self.correction.pending()
    }

    fn draw(self, ui: &egui::Ui, image_rect: egui::Rect) -> Result<(), String> {
        let dimensions = self.composition.size();
        let aspect = dimensions[0] as f32 / dimensions[1] as f32;
        let size = egui::vec2(
            (image_rect.height() * aspect).min(image_rect.width()),
            (image_rect.width() / aspect).min(image_rect.height()),
        );
        let full = egui::Rect::from_center_size(image_rect.center(), size);
        let draw = full.intersect(ui.clip_rect());
        let painter = ui.painter().with_clip_rect(draw);
        let mapping = clipped_mapping(self.composition, full, draw, ui.ctx().pixels_per_point());
        let result = if let Some(mapping) = mapping
            && !self.awaits_correction()
        {
            let field = match self.parameters.as_ref() {
                Some(request) if request.is_inspection() && self.correction.error().is_some() => {
                    Ok(None)
                }
                Some(_) => self.correction.gpu(),
                None => Ok(None),
            };
            field
                .and_then(|field| {
                    let presentation = self.presentation.as_ref().map_err(Clone::clone)?;
                    let paint = processing::image_paint(
                        presentation,
                        self.image,
                        self.parameters,
                        field,
                        mapping,
                    )?;
                    painter.add(egui_wgpu::Callback::new_paint_callback(draw, paint));
                    Ok(())
                })
                .map_err(|error| error.to_string())
        } else {
            Ok(())
        };

        if let Some((outline, [x, y, width, height])) = self.outline {
            let points = outline.map(|point| {
                full.min
                    + egui::vec2(
                        ((point[0] - f64::from(x)) / f64::from(width)) as f32 * full.width(),
                        ((point[1] - f64::from(y)) / f64::from(height)) as f32 * full.height(),
                    )
            });
            painter.add(egui::Shape::closed_line(
                points.to_vec(),
                ui.visuals().selection.stroke,
            ));
        }
        result
    }
}

fn clipped_mapping(
    composition: Composition,
    full: egui::Rect,
    draw: egui::Rect,
    scale: f32,
) -> Option<crate::geometry::TextureMapping> {
    // egui-wgpu clamps offscreen viewports; remap clipped, pixel-rounded bounds
    // instead of squeezing the whole image into the remaining space.
    let min = (full.min.to_vec2() * scale).round();
    let extent = (full.max.to_vec2() * scale).round() - min;
    let draw_min = (draw.min.to_vec2() * scale).round();
    let draw_max = (draw.max.to_vec2() * scale).round();
    if draw_max.x <= draw_min.x || draw_max.y <= draw_min.y {
        return None;
    }
    let dimensions = composition.size();
    let frame_point = |point: egui::Vec2| {
        let fraction = (point - min) / extent;
        FramePoint([
            f64::from(fraction.x) * f64::from(dimensions[0]),
            f64::from(fraction.y) * f64::from(dimensions[1]),
        ])
    };
    Some(composition.texture_mapping(
        frame_point(draw_min),
        frame_point(draw_max),
        composition.area(),
    ))
}

fn draw_camera(ui: &egui::Ui, image_rect: egui::Rect, texture: &egui::TextureHandle) {
    let dimensions = texture.size_vec2();
    let scale = (image_rect.width() / dimensions.x).min(image_rect.height() / dimensions.y);
    let rect = egui::Rect::from_center_size(image_rect.center(), dimensions * scale);
    ui.painter()
        .with_clip_rect(image_rect.intersect(ui.clip_rect()))
        .image(
            texture.id(),
            rect,
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
            egui::Color32::WHITE,
        );
}
