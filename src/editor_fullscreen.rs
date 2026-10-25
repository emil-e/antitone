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
            || self.tasks.blocks_workspace()
            || self.failure.is_some()
            || self.document_pending.is_some()
            || self.transition.is_some()
            || self.project_overwrite.is_some()
            || self.allow_close
            || self.export_modal()
            || self.removal_pending.is_some()
            || self.document.visit.adjustments.confirming()
    }

    pub(super) fn fullscreen_available(&self) -> bool {
        !self.fullscreen_blocked()
            && self.document.analysis_review.is_none()
            && (self.fullscreen.is_some()
                || (self.document.visit.ready.is_some() || self.pending_preview.is_some())
                    && !(self.busy && (self.pick.is_some() || self.flat_edit.is_some())))
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
                viewport: self.viewport,
                layout: self.last_layout,
                source: self.source,
                source_exposure: self.source_exposure,
                source_white_balance: self.source_white_balance,
                calibrated: self.input.ready(),
                native_before,
                restored: false,
                exiting: false,
            });
            if self.viewport.zoom.is_some()
                && let Some((layout, density)) = self
                    .document
                    .visit
                    .crop_edit
                    .as_ref()
                    .and_then(|edit| {
                        edit.layout
                            .map(|layout| (layout, context.pixels_per_point()))
                    })
                    .or(self.last_layout)
            {
                self.viewport.reframe(layout, layout.composition, density);
            }
            self.thumbnails.cancel();
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
        let Some(view) = &mut self.fullscreen else {
            return;
        };
        let context_changed = view.id != self.id;
        let native_exit = !view.exiting && view.native_before && native == Some(false);
        let returned = view.exiting && view.restored;
        if context_changed || native_exit || returned {
            let view = self.fullscreen.take().unwrap();
            if !context_changed {
                self.viewport = view.viewport;
                self.last_layout = view.layout;
                if self.input.ready() == view.calibrated {
                    self.source = view.source;
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
            self.viewport.fit();
        }
    }
}

#[cfg(test)]
#[path = "editor_fullscreen_tests.rs"]
mod tests;
