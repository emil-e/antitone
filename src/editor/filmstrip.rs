mod image;

use super::{
    Editor, SourceId, UiContext, auto_crop,
    roll_order::{Drag, View},
    thumbnails,
    ui::ContactSheetLayout,
};
use std::ops::Range;

#[derive(Default)]
pub(super) struct Preview {
    pub images: thumbnails::Preview,
    pub warnings: Vec<(SourceId, String)>,
    pub outlines: Vec<(SourceId, [[f64; 2]; 4])>,
}

impl Editor {
    pub(super) fn hidden_filmstrip(&mut self, ui: &mut egui::Ui, cx: &UiContext<'_, Self>) {
        thumbnails::clear_visible_photos(self, ui.ctx(), cx);
    }

    pub(super) fn filmstrip(&mut self, ui: &mut egui::Ui, cx: &UiContext<'_, Self>) {
        show_preview(self, ui, Preview::default(), cx);
    }
}

pub(super) fn show_preview(
    editor: &mut Editor,
    ui: &mut egui::Ui,
    mut preview: Preview,
    cx: &UiContext<'_, Editor>,
) {
    thumbnails::prepare_filmstrip_images(editor, std::mem::take(&mut preview.images), ui.ctx());

    let mut thumbnails = thumbnails::ThumbnailProvider::default();
    let actions = show_cards(editor, ui, &mut thumbnails, &preview);
    if actions.auto_crop {
        let photos = editor
            .document
            .project
            .order
            .iter()
            .copied()
            .filter(|id| editor.document.roll_selection.contains(*id))
            .collect();
        auto_crop::start_review(editor, photos, "Entry selection", cx);
    }
    if actions.remove {
        editor.request_removal(cx);
    }
    if let Some(slot) = actions.drop_slot {
        editor.drop_photo_order(slot);
        ui.ctx().request_repaint();
    }

    thumbnails.publish_demand(editor, ui.ctx(), cx);
    if let Some((index, modifiers)) = actions.select {
        editor.select_roll_photo(index, modifiers, cx);
    }
}

#[derive(Default)]
struct Actions {
    select: Option<(usize, egui::Modifiers)>,
    remove: bool,
    auto_crop: bool,
    drop_slot: Option<usize>,
}

fn show_cards(
    editor: &mut Editor,
    ui: &mut egui::Ui,
    thumbnails: &mut thumbnails::ThumbnailProvider,
    preview: &Preview,
) -> Actions {
    let mut actions = Actions::default();
    let resized_height = panel(ui, editor.preferences.filmstrip_height, |ui, layout| {
        let has_capture_time = editor
            .document
            .project
            .sources
            .iter()
            .any(|source| source.capture_time.is_some());
        editor.document.roll_order.control(ui, has_capture_time);
        ui.add(egui::Separator::default().spacing(1.));

        let project = editor.document.state.project.clone();
        let order = editor.document.roll_order.sort.ids(&project);
        let can_drag = ui.is_enabled() && order.len() > 1 && editor.reorder_available();
        let view = &mut editor.document.roll_order;
        if !can_drag || view.drag.as_ref().is_some_and(|drag| !drag.current(&order)) {
            view.drag = None;
        }

        visible_cards(ui, layout, order.len(), |ui, content, indices| {
            let pitch = layout.pitch();
            for index in indices {
                let id = order[index];
                let active = editor.document.visit.current_source == Some(id);
                let selected = active || editor.document.roll_selection.contains(id);
                let card = layout.begin(
                    ui,
                    content.min + egui::vec2(index as f32 * pitch, 0.),
                    ui.id().with(id.0),
                    selected,
                    active,
                    ui.is_enabled().then(|| {
                        if can_drag {
                            egui::Sense::click_and_drag()
                        } else {
                            egui::Sense::click()
                        }
                    }),
                );

                let starts_drag =
                    can_drag && card.response.drag_started_by(egui::PointerButton::Primary);
                let source = project.source(id).expect("filmstrip source");
                let path = editor.document.source_path(id).expect("filmstrip locator");
                let contents = image::Card {
                    image: thumbnails.lookup(editor, source, &path),
                    warning: preview
                        .warnings
                        .iter()
                        .find(|(source, _)| *source == id)
                        .map(|(_, reason)| reason.clone())
                        .or_else(|| editor.source_tool_warning(id)),
                    outline: preview
                        .outlines
                        .iter()
                        .find(|(source, _)| *source == id)
                        .map(|(_, points)| (points, source.development.active_area)),
                };
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                let drag_preview = starts_drag
                    .then(|| contents.image.camera.cloned())
                    .flatten();
                let error = contents.show(ui, &card, &name);

                let response = card.response;
                let drag = &mut editor.document.roll_order.drag;
                if starts_drag {
                    *drag = Some(Drag::new(
                        order.to_vec(),
                        id,
                        &editor.document.roll_selection,
                        name.into_owned(),
                        drag_preview,
                    ));
                } else if drag.is_none() {
                    if response.clicked() {
                        actions.select = Some((index, ui.input(|input| input.modifiers)));
                    }
                    if response.secondary_clicked() && !selected {
                        actions.select = Some((index, egui::Modifiers::NONE));
                    }
                }
                if drag.as_ref().is_some_and(|drag| drag.contains(id)) {
                    ui.painter().rect_filled(
                        response.rect,
                        4.,
                        egui::Color32::from_black_alpha(100),
                    );
                }

                let dragging = drag.is_some();
                if !dragging
                    && response.enabled()
                    && response.has_focus()
                    && !egui::Popup::is_any_open(ui.ctx())
                {
                    editor.select_active_shortcut(ui.ctx());
                    if !editor.editing() && removal_shortcut(ui) {
                        actions.remove = true;
                    }
                }
                let menu = card_menu(&response, dragging, selected, || {
                    (auto_crop::review_available(editor), !editor.editing())
                });
                actions.auto_crop |= menu == Some(MenuAction::AutoCrop);
                actions.remove |= menu == Some(MenuAction::Remove);
                if !dragging {
                    let help = error.unwrap_or_else(|| path.display().to_string());
                    response.on_hover_text(&help).on_disabled_hover_text(help);
                }
            }
            actions.drop_slot = drag_feedback(&mut editor.document.roll_order, ui, content, pitch);
        });
    });
    if let Some(height) = resized_height {
        editor.preferences.filmstrip_height = height;
    }
    actions
}

fn panel(
    ui: &mut egui::Ui,
    preferred_height: f32,
    body: impl FnOnce(&mut egui::Ui, ContactSheetLayout),
) -> Option<f32> {
    let max_height = (ui.ctx().content_rect().height() * 0.35).clamp(52., 240.);
    let height = preferred_height.min(max_height).min(ui.available_height());
    let panel_id = egui::Id::new("thumbnail-filmstrip");
    if let Some(mut state) = egui::containers::panel::PanelState::load(ui.ctx(), panel_id) {
        // Window constraints must not replace the user's preferred height.
        state.outer_rect.min.y = state.outer_rect.max.y - height;
        ui.ctx()
            .data_mut(|data| data.insert_persisted(panel_id, state));
    }
    let strip = egui::Panel::bottom(panel_id)
        .resizable(true)
        .default_size(height)
        .min_size(52.)
        .max_size(max_height)
        .frame(
            egui::Frame::side_top_panel(ui.style())
                .inner_margin(egui::Margin::symmetric(10, 6))
                .fill(egui::Color32::from_gray(22)),
        )
        .show(ui, |ui| {
            ui.take_available_height();
            let layout = ContactSheetLayout::new(ui.available_height(), height < 90.);
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                body(ui, layout)
            });
        });
    let actual = strip.response.rect.height();
    ((actual - height).abs() > 0.5).then(|| actual.clamp(52., 240.))
}

fn visible_cards(
    ui: &mut egui::Ui,
    layout: ContactSheetLayout,
    count: usize,
    body: impl FnOnce(&mut egui::Ui, egui::Rect, Range<usize>),
) {
    egui::ScrollArea::horizontal()
        .animated(false)
        .show_viewport(ui, |ui, viewport| {
            let pitch = layout.pitch();
            let width = (count as f32 * pitch).max(ui.available_width());
            let (content, _) =
                ui.allocate_exact_size(egui::vec2(width, layout.size().y), egui::Sense::hover());
            let first = (viewport.min.x / pitch).floor().max(0.) as usize;
            let last = (viewport.max.x / pitch).ceil().max(0.) as usize;
            body(ui, content, first..last.min(count));
        });
}

fn drag_feedback(view: &mut View, ui: &egui::Ui, content: egui::Rect, pitch: f32) -> Option<usize> {
    let drag = view.drag.as_ref()?;
    let (focused, released, held) = ui.input(|input| {
        (
            input.focused,
            input.pointer.button_released(egui::PointerButton::Primary),
            input.pointer.primary_down(),
        )
    });
    if focused
        && (released || held)
        && let Some(slot) = drag.position(ui, content, pitch)
    {
        if released {
            return Some(slot);
        }
        drag.paint(ui, content, pitch, slot, view.sort);
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    }
    if !held || !focused {
        view.drag = None;
    }
    None
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MenuAction {
    AutoCrop,
    Remove,
}

fn card_menu(
    response: &egui::Response,
    dragging: bool,
    selected: bool,
    availability: impl FnOnce() -> (bool, bool),
) -> Option<MenuAction> {
    let mut action = None;
    response.context_menu(|ui| {
        // Popup UIs do not inherit the card's disabled parent.
        if !response.enabled() || dragging {
            ui.disable();
        }
        let (can_auto_crop, can_remove) = availability();
        if ui
            .add_enabled(
                can_auto_crop && selected,
                egui::Button::new("Auto-crop selected photos"),
            )
            .clicked()
        {
            action = Some(MenuAction::AutoCrop);
            ui.close();
        }
        if ui
            .add_enabled(
                can_remove && selected,
                egui::Button::new("Remove selected photos…"),
            )
            .clicked()
        {
            action = Some(MenuAction::Remove);
            ui.close();
        }
    });
    action
}

fn removal_shortcut(ui: &egui::Ui) -> bool {
    ui.input_mut(|input| {
        input.consume_key(egui::Modifiers::NONE, egui::Key::Delete)
            || input.consume_key(egui::Modifiers::NONE, egui::Key::Backspace)
    })
}
