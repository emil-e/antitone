use egui::{Color32, Ui};

pub(in crate::editor) const CONTROL_HEIGHT: f32 = 28.;
pub(in crate::editor) const INSCRIPTION: Color32 = Color32::from_rgb(223, 170, 96);

const READING_FONT: &str = "plex-text";
const INSTRUMENT_FONT: &str = "plex-medium";

pub(in crate::editor) fn configure_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        READING_FONT.into(),
        egui::FontData::from_static(include_bytes!("../../../assets/fonts/IBMPlexSans-Text.ttf"))
            .into(),
    );
    fonts.font_data.insert(
        INSTRUMENT_FONT.into(),
        egui::FontData::from_static(include_bytes!(
            "../../../assets/fonts/IBMPlexSans-Medium.ttf"
        ))
        .into(),
    );
    let proportional = fonts
        .families
        .get_mut(&egui::FontFamily::Proportional)
        .unwrap();
    proportional.insert(0, READING_FONT.into());
    let mut instrument = proportional.clone();
    instrument.insert(0, INSTRUMENT_FONT.into());
    fonts
        .families
        .insert(egui::FontFamily::Name(INSTRUMENT_FONT.into()), instrument);
    ctx.set_fonts(fonts);
}

pub(super) fn instrument_font(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name(INSTRUMENT_FONT.into()))
}

pub(in crate::editor) fn instrument_readout(text: &str, size: f32) -> egui::RichText {
    egui::RichText::new(text).font(instrument_font(size))
}

pub(in crate::editor) fn instrument_label(text: &str, size: f32) -> egui::RichText {
    instrument_readout(&text.to_uppercase(), size).extra_letter_spacing(size * 0.045)
}

pub(in crate::editor) fn configure_branding(ui: &mut Ui) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = Color32::from_rgb(34, 36, 38);
    visuals.window_fill = visuals.panel_fill;
    visuals.selection.stroke.color = INSCRIPTION;
    visuals.selection.bg_fill = INSCRIPTION.gamma_multiply(0.25);
    visuals.hyperlink_color = INSCRIPTION;
    ui.ctx().set_theme(egui::Theme::Dark);
    ui.ctx().set_visuals(visuals.clone());
    *ui.visuals_mut() = visuals;
}

#[cfg(test)]
mod tests;
