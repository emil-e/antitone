use super::*;

pub(super) struct FullscreenView {
    id: u64,
    reference: bool,
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
            || self.tasks.blocks_workspace()
            || self.failure.is_some()
            || self.document_pending.is_some()
            || self.transition.is_some()
            || self.project_overwrite.is_some()
            || self.allow_close
            || self.export_modal()
            || self.document.removal_pending.is_some()
            || self.document.visit.adjustment_layers.confirming()
    }

    pub(super) fn fullscreen_available(&self) -> bool {
        !self.fullscreen_blocked()
            && self.document.analysis_review().is_none()
            && (self.fullscreen.is_some()
                || self.selected_view().is_some()
                    && !(self.busy
                        && (self.active_pick().is_some() || self.document.flat_edit().is_some())))
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
            let native_before = context.input(|input| input.viewport().fullscreen == Some(true));
            self.fullscreen = Some(FullscreenView {
                id: self.id,
                reference: self.document.flat_edit().is_some(),
                viewport: self.active_canvas().viewport,
                layout: self.active_canvas().last_layout,
                source: self.document.source,
                source_exposure: self.source_exposure,
                source_white_balance: self.source_white_balance,
                calibrated: self.document.input.ready(),
                native_before,
                restored: false,
                exiting: false,
            });
            if self.active_canvas().viewport.zoom.is_some()
                && let Some((layout, density)) = self
                    .document
                    .crop_edit()
                    .and_then(|edit| {
                        edit.layout
                            .map(|layout| (layout, context.pixels_per_point()))
                    })
                    .or(self.active_canvas().last_layout)
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
        let context_changed =
            view.id != self.id || view.reference != self.document.flat_edit().is_some();
        let native_exit = !view.exiting && view.native_before && native == Some(false);
        let returned = view.exiting && view.restored;
        if context_changed || native_exit || returned {
            let view = self.fullscreen.take().unwrap();
            if !context_changed {
                self.active_canvas_mut().viewport = view.viewport;
                self.active_canvas_mut().last_layout = view.layout;
                if self.document.input.ready() == view.calibrated {
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
            self.active_canvas_mut().viewport.fit();
        }
    }
}

#[cfg(test)]
mod tests;
