use super::{NavigationImage, NavigationPhotograph, ui};
use crate::geometry::SourcePoint;

pub(in crate::editor) struct Card<'a> {
    pub image: NavigationImage<'a>,
    pub warning: Option<String>,
    pub outline: Option<&'a [[f64; 2]; 4]>,
}

impl Card<'_> {
    pub fn show(self, ui: &egui::Ui, card: &ui::ContactSheetCard, name: &str) -> Option<String> {
        use ui::{ContextualStatus, StatusLevel};
        let has_image = self.image.photograph.is_some();
        let error = match self.image.photograph {
            Some(Ok(photo)) => draw_photo(photo, self.outline, ui, card.image_rect).err(),
            Some(Err(error)) => Some(error),
            None => {
                if let Some(texture) = self.image.camera {
                    draw_camera(ui, card.image_rect, texture);
                    card.camera_badge(ui);
                } else if self.image.updating {
                    card.loading(ui);
                }
                None
            }
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
        } else if self.image.updating {
            Some(ContextualStatus::new(
                StatusLevel::Activity,
                if has_image {
                    "Updating preview; showing the previous image."
                } else {
                    "Preparing preview."
                },
            ))
        } else {
            None
        };
        if let Some(reason) = self.warning {
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

fn draw_photo(
    photo: NavigationPhotograph<'_>,
    outline: Option<&[[f64; 2]; 4]>,
    ui: &egui::Ui,
    image_rect: egui::Rect,
) -> Result<(), String> {
    let full = photo.fitted_rect(image_rect);
    let composition = photo.composition;
    let result = photo.show(ui, full);

    if let Some(outline) = outline {
        let [width, height] = composition.size().map(f64::from);
        let points = outline.map(|point| {
            let [x, y] = composition.source_to_frame(SourcePoint(point)).0;
            full.min + egui::vec2((x / width) as f32, (y / height) as f32) * full.size()
        });
        ui.painter()
            .with_clip_rect(full.intersect(ui.clip_rect()))
            .add(egui::Shape::closed_line(
                points.to_vec(),
                ui.visuals().selection.stroke,
            ));
    }
    result
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
