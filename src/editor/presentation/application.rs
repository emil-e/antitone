use super::{TextAction, dialogs};

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
                ui.small("Copyright © Emil Sahlén · MIT · No warranty");
                ui.hyperlink_to(
                    "License and redistribution terms",
                    "https://opensource.org/license/MIT",
                );
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
