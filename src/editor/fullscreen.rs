use super::*;

pub(super) struct Fullscreen {
    native_before: bool,
    restored: bool,
    exiting: bool,
}

impl Editor {
    fn fullscreen_blocked(&self) -> bool {
        self.workspace_input.is_blocked() || self.allow_close
    }

    pub(super) fn fullscreen_available(&self) -> bool {
        !self.fullscreen_blocked()
            && (self.fullscreen.is_some()
                || self.canvas_frame.layout.is_some()
                || self.selected_view().is_some())
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
            self.preserve_fullscreen_anchor();
            self.fullscreen = Some(Fullscreen {
                native_before: context.input(|input| input.viewport().fullscreen == Some(true)),
                restored: false,
                exiting: false,
            });
            ui::ViewerBar::reset_overlay(context);
        }
        context.memory_mut(|memory| {
            if let Some(id) = memory.focused() {
                memory.surrender_focus(id);
            }
        });
        context.request_repaint();
    }

    fn preserve_fullscreen_anchor(&mut self) {
        if let Some((layout, density)) = self.canvas_frame.layout {
            self.with_tool_viewport(|viewport| {
                if viewport.zoom.is_some() {
                    viewport.reframe(layout, layout.composition, density);
                }
            });
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
        let native_exit = !view.exiting && view.native_before && native == Some(false);
        let returned = view.exiting && view.restored;
        if native_exit || returned {
            // An Escape arriving with the host's return still belongs to fullscreen.
            context.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            self.preserve_fullscreen_anchor();
            self.fullscreen = None;
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
        }
    }
}

#[cfg(test)]
mod tests;
