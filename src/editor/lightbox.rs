use super::{
    Editor, UiContext,
    thumbnails::{ThumbnailProvider, card},
    ui,
    viewer_ui::Mode,
};

const TILE_WIDTHS: [f32; 4] = [120., 180., 240., 320.];

pub(super) struct View {
    reveal: ui::Reveal,
    focus_active: bool,
    size: usize,
}

impl Default for View {
    fn default() -> Self {
        Self {
            reveal: Default::default(),
            focus_active: false,
            size: TILE_WIDTHS.len() - 1,
        }
    }
}

impl View {
    pub(super) fn enter(&mut self) {
        self.reveal = Default::default();
        self.focus_active = true;
    }

    fn size_controls(&mut self, ui: &mut egui::Ui, enabled: bool) {
        use ui::icons::{Icon, button};
        if button(
            ui,
            Icon::ZoomOut,
            enabled && self.size > 0,
            "Smaller tiles",
            "Show smaller tiles.",
        )
        .clicked()
        {
            self.size -= 1;
        }
        if button(
            ui,
            Icon::ZoomIn,
            enabled && self.size + 1 < TILE_WIDTHS.len(),
            "Larger tiles",
            "Show larger tiles.",
        )
        .clicked()
        {
            self.size += 1;
        }
    }
}

impl Editor {
    pub(super) fn lightbox_bar(&mut self, ui: &mut egui::Ui, fullscreen: bool) -> bool {
        let enabled = !self.interaction_blocked();
        self.document.lightbox.size_controls(ui, enabled);
        self.fullscreen_button(ui, fullscreen)
    }

    pub(super) fn show_lightbox(
        &mut self,
        ui: &mut egui::Ui,
        cx: &UiContext<'_, Self>,
    ) -> ThumbnailProvider {
        let mut demand = ThumbnailProvider::default();
        let focus_active = std::mem::take(&mut self.document.lightbox.focus_active);
        let preferred_width = TILE_WIDTHS[self.document.lightbox.size];
        let mut action = None;
        let active = self.document.visit.current_source;
        let order = self.document.displayed_order().into_owned();
        let tiles = ui.id().with("lightbox-photo");
        egui::Frame::new().inner_margin(12).show(ui, |ui| {
            let width = ui.available_width().max(1.);
            let gutter = ui::ContactSheetLayout::GUTTER;
            let columns = (((width + gutter) / (preferred_width + gutter)).floor() as usize).max(1);
            let tile_width = (width + gutter) / columns as f32 - gutter;
            let layout = ui::ContactSheetLayout::grid(tile_width);
            let row_height = layout.size().y;
            let row_count = order.len().div_ceil(columns);
            ui.spacing_mut().item_spacing.y = gutter;

            let mut scroll = egui::ScrollArea::vertical()
                .id_salt("lightbox-grid")
                .animated(false);
            let target = active.and_then(|id| {
                let index = order.iter().position(|source| *source == id)?;
                let start = (index / columns) as f32 * (row_height + gutter);
                Some((id, start, start + row_height))
            });
            if let Some(offset) = self.document.lightbox.reveal.offset(target, focus_active) {
                scroll = scroll.vertical_scroll_offset(offset);
            }
            let output = scroll.show_rows(ui, row_height, row_count, |ui, rows| {
                for row in rows {
                    let (bounds, _) =
                        ui.allocate_exact_size(egui::vec2(width, row_height), egui::Sense::hover());
                    if !ui.is_rect_visible(bounds) {
                        continue;
                    }
                    let first = row * columns;
                    let last = (first + columns).min(order.len());
                    for (column, &id) in order[first..last].iter().enumerate() {
                        let Ok(source) = self.document.project.source(id) else {
                            continue;
                        };
                        let Ok(path) = self.document.source_path(id) else {
                            continue;
                        };
                        let card = layout.begin(
                            ui,
                            bounds.min + egui::vec2(column as f32 * layout.pitch(), 0.),
                            tiles.with(id),
                            self.document.roll_selection.contains(id),
                            active == Some(id),
                            ui.is_enabled().then(egui::Sense::click),
                        );
                        let contents = card::Card {
                            image: demand.lookup(self, source, &path),
                            warning: None,
                            outline: None,
                        };
                        let name = path.file_name().unwrap_or_default().to_string_lossy();
                        let error = contents.show(ui, &card, &name);
                        let response = card.response;
                        if response.clicked()
                            || (focus_active && active == Some(id) && response.enabled())
                        {
                            response.request_focus();
                        }
                        let index = first + column;
                        if response.clicked() {
                            let modifiers = ui.input(|input| input.modifiers);
                            let open_photo = modifiers.is_none() && response.double_clicked();
                            action = Some((index, modifiers, open_photo));
                        } else if response.enabled()
                            && response.has_focus()
                            && self.workspace_input.shortcuts_allowed(ui.ctx())
                        {
                            self.select_active_shortcut(ui.ctx());
                            if let Some(index) = navigate(ui.ctx(), index, columns, order.len()) {
                                action = Some((index, egui::Modifiers::NONE, false));
                            }
                        }
                        response.on_hover_text(error.unwrap_or_else(|| path.display().to_string()));
                    }
                }
            });
            self.document
                .lightbox
                .reveal
                .observe(output.state.offset.y, output.inner_rect.height());
        });
        if let Some((index, modifiers, open_photo)) = action {
            self.select_roll_photo(index, modifiers, cx);
            if open_photo {
                self.set_view_mode(Mode::Photo);
            } else if modifiers.is_none() {
                // The selected tile may be outside the virtualized rows in this pass.
                self.document.lightbox.focus_active = true;
            }
            ui.ctx().request_repaint();
        }
        if let Some(id) = self.document.visit.current_source {
            demand.prioritize(id);
        }
        demand
    }
}

fn navigate(context: &egui::Context, index: usize, columns: usize, count: usize) -> Option<usize> {
    use egui::{Key, Modifiers};
    let direction = context.input_mut(|input| {
        [
            Key::ArrowLeft,
            Key::ArrowRight,
            Key::ArrowUp,
            Key::ArrowDown,
        ]
        .into_iter()
        .find(|&key| input.consume_key(Modifiers::NONE, key))
    })?;
    // This tile owns the arrows, not egui's generic spatial widget navigation.
    context.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
    let column = index % columns;
    match direction {
        Key::ArrowLeft if column > 0 => index.checked_sub(1),
        Key::ArrowRight if column + 1 < columns => index.checked_add(1),
        Key::ArrowUp => index.checked_sub(columns),
        Key::ArrowDown => index.checked_add(columns),
        _ => None,
    }
    .filter(|&target| target < count)
}
