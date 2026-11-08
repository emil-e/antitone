use super::*;

pub(super) struct FullscreenView {
    id: u64,
    viewport: Viewport,
    layout: Option<(crate::viewport::CanvasLayout, f32)>,
    source: bool,
    source_exposure: f64,
    source_white_balance: crate::raw::SourceWhiteBalance,
    calibrated: bool,
    native_before: bool,
    restored: bool,
    exiting: bool,
}

impl Editor {
    fn fullscreen_blocked(&self) -> bool {
        self.menus.panel.is_some()
            || self.activities.blocks_workspace()
            || self.dialogs.is_open()
            || self.document_pending.is_some()
            || self.files.pending()
            || self.allow_close
            || self.export_modal()
            || self.dialogs.is_open()
    }

    pub(super) fn fullscreen_available(&self) -> bool {
        !self.fullscreen_blocked()
            && (self.fullscreen.is_some()
                || (self.canvas_frame.layout.is_some() || self.selected_view().is_some())
                    && !(self.interaction_blocked() && self.picking()))
    }

    pub(super) fn toggle_fullscreen(&mut self, context: &egui::Context) {
        if !self.fullscreen_available() {
            return;
        }
        if let Some(view) = &mut self.fullscreen {
            if !view.exiting {
                view.exiting = true;
            }
        } else {
            let inspection = self.canvas_inspection();
            let picker_viewport = (self.document.visit.picker.display().is_some()
                && self.document.visit.hosted_tool.occupied())
            .then(|| self.with_tool_viewport(|viewport| *viewport));
            let normal_viewport = std::ptr::from_ref(&self.active_canvas().viewport);
            let normal_layout =
                self.with_tool_viewport(|viewport| std::ptr::eq(viewport, normal_viewport));
            let native_before = context.input(|input| input.viewport().fullscreen == Some(true));
            self.fullscreen = Some(FullscreenView {
                id: self.id,
                viewport: self.active_canvas().viewport,
                layout: self.active_canvas().last_layout,
                source: self.document.source,
                source_exposure: self.source_exposure,
                source_white_balance: self.source_white_balance,
                calibrated: self.document.project.input.ready(),
                native_before,
                restored: false,
                exiting: false,
            });
            if let Some(viewport) = picker_viewport {
                self.document.source = inspection;
                self.active_canvas_mut().viewport = viewport;
            }
            // A picker carries its current view into inspection; Crop keeps its private draft view.
            if (normal_layout || picker_viewport.is_some())
                && self.active_canvas().viewport.zoom.is_some()
                && let Some((layout, density)) = self.active_canvas().last_layout
            {
                self.active_canvas_mut()
                    .viewport
                    .reframe(layout, layout.composition, density);
            }
            self.document.thumbnails.cancel();
            presentation::ViewerBar::reset_overlay(context);
        }
        context.memory_mut(|memory| {
            if let Some(id) = memory.focused() {
                memory.surrender_focus(id);
            }
        });
        context.request_repaint();
    }

    pub(super) fn retire_fullscreen(&mut self) {
        let Some(view) = self.fullscreen.take() else {
            return;
        };
        if view.id == self.id {
            self.document.visit.canvas.viewport = view.viewport;
            self.document.visit.canvas.last_layout = view.layout;
            if self.document.project.input.ready() == view.calibrated {
                self.document.source = view.source;
            }
            self.source_exposure = view.source_exposure;
            self.source_white_balance = view.source_white_balance;
        }
    }

    pub(super) fn fullscreen_requested(&self) -> bool {
        self.fullscreen.as_ref().is_some_and(|view| !view.exiting)
    }

    pub(super) fn fullscreen_changed(&mut self, active: bool) {
        if !active && let Some(view) = &mut self.fullscreen {
            view.restored = true;
        }
    }

    pub(super) fn update_fullscreen(&mut self, context: &egui::Context) {
        let native = context.input(|input| input.viewport().fullscreen);
        let Some(view) = &self.fullscreen else {
            return;
        };
        let context_changed = view.id != self.id;
        let native_exit = !view.exiting && view.native_before && native == Some(false);
        let returned = view.exiting && view.restored;
        if context_changed || native_exit || returned {
            let view = self.fullscreen.take().unwrap();
            if !context_changed {
                self.active_canvas_mut().viewport = view.viewport;
                self.active_canvas_mut().last_layout = view.layout;
                if self.document.project.input.ready() == view.calibrated {
                    self.document.source = view.source;
                }
                self.source_exposure = view.source_exposure;
                self.source_white_balance = view.source_white_balance;
            }
            context.request_repaint();
        }
    }

    pub(super) fn fullscreen_shortcuts(
        &mut self,
        ui: &egui::Ui,
        text_was_focused: bool,
        popup_was_closed: bool,
    ) {
        if !self.fullscreen_available()
            || !popup_was_closed
            || egui::Popup::is_any_open(ui.ctx())
            || ui.ctx().text_edit_focused()
            || text_was_focused && ui.input(|input| input.key_pressed(egui::Key::Escape))
            || ui.input(|input| input.pointer.any_down() || input.multi_touch().is_some())
        {
            return;
        }
        if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::F))
            || self.fullscreen.is_some()
                && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.toggle_fullscreen(ui.ctx());
            self.update_fullscreen(ui.ctx());
        } else if self.fullscreen.is_some()
            && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Z))
        {
            self.with_tool_viewport(Viewport::fit);
        }
    }
}

#[cfg(test)]
mod tests;
