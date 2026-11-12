use super::{TextAction, dialogs};
use egui::{Response, Ui};

const HEADER_HEIGHT: f32 = 32.;

pub(in crate::editor) fn app_header<R>(ui: &mut Ui, contents: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Panel::top("header")
        .exact_size(HEADER_HEIGHT)
        .frame(egui::Frame::side_top_panel(ui.style()).inner_margin(egui::Margin::symmetric(12, 2)))
        .show(ui, |ui| {
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                wordmark(ui);
                contents(ui)
            })
            .inner
        })
        .inner
}

fn wordmark(ui: &mut Ui) -> Response {
    let id = egui::Id::new("antitone-wordmark-texture");
    let cached = ui
        .ctx()
        .data_mut(|data| data.get_temp::<egui::TextureHandle>(id));
    let texture = cached.unwrap_or_else(|| {
        let image = image::load_from_memory_with_format(
            include_bytes!("../../../assets/branding/wordmark-header.png"),
            image::ImageFormat::Png,
        )
        .expect("valid embedded wordmark PNG")
        .into_rgba8();
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [image.width() as usize, image.height() as usize],
            image.as_raw(),
        );
        let texture =
            ui.ctx()
                .load_texture("Antitone wordmark", image, egui::TextureOptions::LINEAR);
        ui.ctx()
            .data_mut(|data| data.insert_temp(id, texture.clone()));
        texture
    });
    // The outlined 16-point inscription is rasterized at 4×, independent of UI fonts.
    let size = texture.size_vec2() / 4.;
    let response = ui.add(egui::Image::from_texture((texture.id(), size)));
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Label, response.enabled(), "Antitone")
    });
    response
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AppPanel {
    Settings,
    About,
    Help,
}

pub fn app_panel(
    context: &egui::Context,
    panel: AppPanel,
    escape_available: bool,
    settings: impl FnOnce(&mut egui::Ui),
) -> bool {
    let response = dialogs::modal(context, egui::Id::new(("application-panel", panel)), |ui| {
        match panel {
            AppPanel::Settings => {
                ui.heading("Settings");
                ui.strong("Memory");
                ui.label(
                    "These limits control optional cached data, not total application memory.",
                );
                settings(ui);
            }
            AppPanel::About => {
                ui.heading("Antitone");
                ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
                ui.label("An editor for camera-scanned film negatives.");
                ui.label("Analyze a roll, refine your photographs, and export TIFF or EXR.");
                ui.small("Copyright © Emil Sahlén · No warranty");
                ui.hyperlink_to(
                    "Original Antitone code: MIT license",
                    "https://opensource.org/license/MIT",
                );
                ui.hyperlink_to(
                    "spektrafilm by Andrea Volpato",
                    "https://github.com/andreavolpato/spektrafilm",
                );
                ui.small("Profile data: CC BY-SA 4.0 · Native algorithm ports: GPLv3");
                ui.small("Packaged builds include third-party licenses in Resources/Licenses.");
                ui.small("Development build · macOS on Apple Silicon");
            }
            AppPanel::Help => {
                ui.heading("Antitone Help");
                egui::ScrollArea::vertical()
                    .max_height((context.content_rect().height() - 240.).clamp(80., 420.))
                    .auto_shrink([false, false])
                    .show(ui, help_contents);
            }
        }
        ui.add_space(8.);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let close = TextAction::new("Close").show(ui);
            dialogs::focus_default(ui, &close);
            close.clicked()
        })
        .inner
    });
    response.inner || dialogs::escape_requested(&response, escape_available)
}

#[cfg(test)]
mod tests;

fn help_contents(ui: &mut egui::Ui) {
    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
    ui.strong("Start a roll");
    ui.label("Choose File → Add Scans… to add camera RAW files. Originals stay in place and are never modified.");
    ui.strong("Develop your photographs");
    ui.label("Use Roll to set calibration and scan orientation. Analyze proposes sampling areas for review before applying a starting grade. Use Photo for individual grading, cropping and cleanup; Rendering chooses the preview and TIFF colour space.");
    ui.strong("Save and export");
    ui.label("Save stores your settings and source references in an .antitone project. Keep the original scans available. File → Export writes selected photos or the whole roll as TIFF or EXR; finish any active draft first.");
    ui.label(
        "Dust repairs are saved in the project and included in exports and filmstrip thumbnails.",
    );
    ui.strong("Inspect the image");
    ui.label("Use the viewer controls to fit, zoom and compare the developed source. F enters photo-only inspection; F or Escape returns. This covers the current display without opening a macOS fullscreen Space.");
    ui.label("Double-click or Option-click a control’s name to reset that value; the reset in a section heading clears the whole group. Hold Shift while dragging a slider or its value, or with the arrow keys, for finer steps.");
    ui.label("Hover an unfamiliar icon for its name and shortcut. Tool-specific gestures are available from the viewer’s help control.");
}
