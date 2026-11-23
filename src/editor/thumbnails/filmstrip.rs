mod image;

use super::*;
use crate::editor::roll_order::{Drag, View};
use crate::editor::ui::ContactSheetLayout;
use image::{CardImage, Photograph};
use std::ops::Range;

#[derive(Default)]
pub(super) struct Actions {
    pub select: Option<(usize, egui::Modifiers)>,
    pub remove: bool,
    pub auto_crop: bool,
    pub drop_slot: Option<usize>,
}

#[derive(Default)]
pub(super) struct Demand {
    pub visible: Vec<SourceId>,
    pub protected: Vec<ImageKey>,
    pub camera_wanted: Vec<(usize, ImageKey, PathBuf)>,
    pub awaiting_correction: bool,
}

#[derive(Default)]
pub(super) struct Pass {
    pub actions: Actions,
    pub demand: Demand,
    pub resized_height: Option<f32>,
}

pub(super) fn show(editor: &mut Editor, ui: &mut egui::Ui) -> Pass {
    let mut pass = Pass::default();
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
                let source = project.source(id).expect("filmstrip source");
                let path = editor.document.source_path(id).expect("filmstrip locator");
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

                let name = path.file_name().unwrap_or_default().to_string_lossy();
                let key = editor.thumbnail_key(source, &path);
                let warning = editor
                    .document
                    .thumbnails
                    .preview
                    .warnings
                    .iter()
                    .find(|(source, _)| *source == id)
                    .map(|(_, reason)| reason.clone())
                    .or_else(|| editor.source_tool_warning(id));
                let image = card_image(editor, source, key.as_ref());
                pass.demand.awaiting_correction |= image.awaits_correction();
                let wants_camera = image.wants_camera();
                let error = image.show(ui, &card, &name, warning);

                let response = card.response;
                let starts_drag =
                    can_drag && response.drag_started_by(egui::PointerButton::Primary);
                let drag_preview = starts_drag
                    .then(|| {
                        key.as_ref()
                            .and_then(|key| editor.document.thumbnails.embedded.get(&key.source))
                            .cloned()
                    })
                    .flatten();
                pass.demand.visible.push(id);
                if let Some(key) = key {
                    if wants_camera {
                        pass.demand
                            .camera_wanted
                            .push((index, key.clone(), path.clone()));
                    }
                    pass.demand.protected.push(key);
                }

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
                        pass.actions.select = Some((index, ui.input(|input| input.modifiers)));
                    }
                    if response.secondary_clicked() && !selected {
                        pass.actions.select = Some((index, egui::Modifiers::NONE));
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
                        pass.actions.remove = true;
                    }
                }
                let menu = card_menu(&response, dragging, selected, || {
                    (auto_crop::review_available(editor), !editor.editing())
                });
                pass.actions.auto_crop |= menu == Some(MenuAction::AutoCrop);
                pass.actions.remove |= menu == Some(MenuAction::Remove);
                if !dragging {
                    let help = error.unwrap_or_else(|| path.display().to_string());
                    response.on_hover_text(&help).on_disabled_hover_text(help);
                }
            }
            pass.actions.drop_slot =
                drag_feedback(&mut editor.document.roll_order, ui, content, pitch);
        });
    });
    Pass {
        resized_height,
        ..pass
    }
}

fn card_image<'a>(
    editor: &'a mut Editor,
    source: &Source,
    key: Option<&ImageKey>,
) -> CardImage<'a> {
    let Some(key) = key else {
        return CardImage::Empty;
    };
    let current = editor.document.thumbnails.cache.get(key).cloned();
    let thumbnails = &editor.document.thumbnails;
    let camera = thumbnails.embedded.get(&key.source);
    let updating = current.is_none() && camera.is_none();
    // Keep the previous accepted grade visible until its replacement is ready.
    // Exact-key failures still take precedence over navigation feedback.
    let entry = current.or_else(|| {
        thumbnails
            .previous_product(key)
            .map(|(_, product)| Ok(product.clone()))
            .or_else(|| {
                thumbnails
                    .cache
                    .entries
                    .iter()
                    .rev()
                    .find_map(|(old, value)| {
                        (key.same_presentation(old) && value.is_err()).then(|| value.clone())
                    })
            })
    });

    match entry {
        Some(Ok(product)) => {
            let parameters = if product.positive.is_none() {
                match editor.thumbnail_request(source) {
                    Ok(request) => Some(request),
                    Err(error) => return CardImage::Error(error),
                }
            } else {
                None
            };
            CardImage::Photo(Photograph {
                image: product.image.clone(),
                parameters,
                composition: editor.thumbnail_composition(source),
                correction: editor.correction(),
                presentation: &editor.presentation,
                outline: thumbnails
                    .preview
                    .outlines
                    .iter()
                    .find(|(id, _)| *id == source.id)
                    .map(|(_, points)| (points, source.development.active_area)),
                updating,
            })
        }
        Some(Err(error)) => CardImage::Error(error),
        None => camera.map_or(CardImage::Loading, CardImage::Camera),
    }
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
