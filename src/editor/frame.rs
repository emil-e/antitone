use super::*;
use crate::platform::menu::Action as MenuAction;

pub(super) struct Actions {
    pub(super) menu: Option<MenuAction>,
    pub(super) file: Option<FileAction>,
    pub(super) export: bool,
}

impl Editor {
    pub(super) fn initialize_ui(&mut self, ui: &mut egui::Ui) -> bool {
        if self.started {
            return false;
        }
        self.started = true;
        ui::configure_branding(ui);
        ui::configure_fonts(ui.ctx());
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(1100., 800.)));
        // Named font families become available at the start of the next pass.
        ui.ctx().request_discard("Initialize editor typography");
        true
    }

    pub(super) fn receive_frame(&mut self) {
        self.observe_gamma_pivot();
        if self.document.visit.canvas.ready.is_none() {
            self.document.visit.working_view.clear();
        }
        self.seed_thumbnail();
        self.validate_pending_preview();
    }

    pub(super) fn application_actions(&mut self, ui: &mut egui::Ui) -> Actions {
        let state = self.application_menu_state();
        let text_input_pending = self.menus.text_focused
            && ui.input(|input| {
                input.events.iter().any(|event| {
                    matches!(
                        event,
                        egui::Event::Text(_)
                            | egui::Event::Paste(_)
                            | egui::Event::Copy
                            | egui::Event::Cut
                            | egui::Event::Key { pressed: true, .. }
                    )
                })
            });
        let requested = self
            .menus
            .pending
            .pop_front()
            .or_else(|| self.menu_shortcut(ui));
        let mut menu = requested.filter(|&action| {
            state.enabled(action) && self.history_menu_input_available(action, ui.ctx())
        });
        if !self.allow_close && ui.input(|input| input.viewport().close_requested()) {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if state.enabled(MenuAction::Close) {
                menu = Some(MenuAction::Close);
            } else if !self.files.pending() {
                self.status =
                    "Finish the current operation or cancel the pick before closing".into();
            }
        }
        // Text input must reach its focused control before a later menu command
        // surrenders focus to commit it. This also orders Paste → Save/Enter.
        if text_input_pending && let Some(action) = menu.take() {
            self.menus.pending.push_front(action);
        }
        if !self.menus.pending.is_empty() {
            ui.ctx().request_repaint();
        }
        if self.interaction_blocked() {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
        if menu.is_some_and(|action| !self.menu_targets_text(action)) {
            ui.ctx().memory_mut(|memory| {
                if let Some(id) = memory.focused() {
                    memory.surrender_focus(id);
                }
            });
        }
        Actions {
            file: menu.and_then(|action| self.menu_file_action(action)),
            export: menu == Some(MenuAction::Export),
            menu,
        }
    }

    pub(super) fn frame_shortcuts(
        &mut self,
        ui: &mut egui::Ui,
        cx: &UiContext<'_, Self>,
        file_action: &mut Option<FileAction>,
        fullscreen_was_active: bool,
        dialog_escape_available: bool,
    ) {
        self.sync_adjustment_layer_drag(ui.ctx());
        // egui clears text focus at pass start on Escape, before tool shortcuts run.
        let text_was_focused = ui.ctx().data(|data| {
            data.get_temp::<bool>(ui.id().with("editor-text-focus"))
                .unwrap_or(false)
        });
        self.fullscreen_shortcuts(ui, text_was_focused, dialog_escape_available);
        self.analysis_shortcuts(ui, cx, text_was_focused, dialog_escape_available);
        self.workspace_shortcuts(ui, cx, file_action, fullscreen_was_active, text_was_focused);
    }

    fn analysis_shortcuts(
        &mut self,
        ui: &mut egui::Ui,
        _cx: &UiContext<'_, Self>,
        text_was_focused: bool,
        dialog_escape_available: bool,
    ) {
        if self.document.spatial_preparing.is_some()
            && !self.document.write.pending()
            && !self.dialogs.is_open()
            && dialog_escape_available
            && !text_was_focused
            && !ui.ctx().egui_wants_keyboard_input()
            && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.cancel_spatial();
        }
        if self.analysis_preparing()
            && dialog_escape_available
            && !text_was_focused
            && !ui.ctx().egui_wants_keyboard_input()
            && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.cancel_analysis();
        }
        if self.document.analysis_review().is_some()
            && !self.analysis_fitting()
            && dialog_escape_available
            && !text_was_focused
            && !ui.ctx().egui_wants_keyboard_input()
            && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.cancel_analysis();
        }
    }

    fn workspace_shortcuts(
        &mut self,
        ui: &mut egui::Ui,
        cx: &UiContext<'_, Self>,
        file_action: &mut Option<FileAction>,
        fullscreen_was_active: bool,
        text_was_focused: bool,
    ) {
        if !self.workspace_input.is_blocked()
            && self.document.look.is_none()
            && self.fullscreen.is_none()
            && !fullscreen_was_active
            && !(text_was_focused && ui.input(|input| input.key_pressed(egui::Key::Escape)))
            && !ui.ctx().egui_wants_keyboard_input()
            && !egui::Popup::is_any_open(ui.ctx())
        {
            if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Z))
                && !self.mask_dragging()
            {
                self.with_tool_viewport(Viewport::fit);
            }
            self.select_active_shortcut(ui.ctx());
            self.tool_shortcuts(ui, cx);
            self.selection_shortcuts(ui, cx, file_action);
        }
    }

    fn tool_shortcuts(&mut self, ui: &mut egui::Ui, cx: &UiContext<'_, Self>) {
        self.with_current_tool(|tool, editor| tool.shortcuts(ui, editor));
        if let Some(tool) = self.current_tool() {
            let primary = tool
                .presentation()
                .primary
                .is_some_and(|primary| primary.enabled);
            if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                self.dispatch_tool(tool::Action::Dismiss, cx);
                ui.ctx().request_repaint();
            } else if primary
                && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
            {
                self.dispatch_tool(tool::Action::Primary, cx);
                ui.ctx().request_repaint();
            }
        }
        if self.document.visit.masks.gesture().is_some()
            && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.finish_mask(false);
        }
    }

    fn selection_shortcuts(
        &mut self,
        ui: &mut egui::Ui,
        cx: &UiContext<'_, Self>,
        file_action: &mut Option<FileAction>,
    ) {
        let adjustment_delete = self.adjustment_layer_owns_delete();
        if adjustment_delete
            && !self.document.visit.adjustment_layers.dragging()
            && ui.input_mut(|input| {
                input.consume_key(egui::Modifiers::NONE, egui::Key::Delete)
                    || input.consume_key(egui::Modifiers::NONE, egui::Key::Backspace)
            })
        {
            self.delete_selected_adjustment_layer(cx);
        }
        if !self.dusting()
            && !adjustment_delete
            && !self.document.visit.adjustment_layers.dragging()
            && self.roll_edit_available()
            && ui.input_mut(|input| {
                input.consume_key(egui::Modifiers::NONE, egui::Key::Delete)
                    || input.consume_key(egui::Modifiers::NONE, egui::Key::Backspace)
            })
        {
            *file_action = Some(FileAction::Remove);
        }
        if !self.picking()
            && !self.cropping()
            && !self.masking()
            && !self.document.visit.adjustment_layers.dragging()
            && !self.document.state.project.order.is_empty()
        {
            let previous = ui
                .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft));
            let next = ui
                .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight));
            let index = if previous {
                self.document.selected.saturating_sub(1)
            } else if next {
                (self.document.selected + 1).min(self.document.state.project.order.len() - 1)
            } else {
                self.document.selected
            };
            if index != self.document.selected {
                self.select_roll_photo(index, egui::Modifiers::NONE, cx);
            }
        }
    }

    pub(super) fn disable_blocked_workspace(&self, ui: &mut egui::Ui) {
        if self.workspace_input.is_blocked() {
            ui.disable();
        }
    }

    pub(super) fn workspace_chrome(
        &mut self,
        ui: &mut egui::Ui,
        cx: &UiContext<'_, Self>,
        actions: &mut Actions,
    ) {
        if self.fullscreen.is_none() {
            // The menu bar belongs to the platform; the header keeps the project identity.
            ui::app_header(ui, |ui| self.project_identity(ui));
        }
        if actions.file.is_some()
            || actions
                .menu
                .is_some_and(|action| !self.menu_targets_text(action))
        {
            ui.ctx().memory_mut(|memory| {
                if let Some(id) = memory.focused() {
                    memory.surrender_focus(id);
                }
            });
        }
        egui::Panel::bottom("status")
            .exact_size(ui::ACTIVITY_BAR_HEIGHT)
            .show(ui, |ui| {
                if let Some(view) = self.activities.view() {
                    view.compact(ui);
                } else {
                    ui.add(egui::Label::new(&self.status).truncate())
                        .on_hover_text(&self.status);
                }
            });
        if self.current_tool().is_some() {
            self.with_current_tool(|tool, editor| tool.filmstrip(ui, editor, cx));
        } else {
            self.filmstrip(ui, cx);
        }
        if self.dialogs.is_open() {
            ui.disable();
        }
    }

    fn project_identity(&self, ui: &mut egui::Ui) {
        ui.separator();
        let name = self
            .document
            .path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".into());
        let label = format!(
            "{name}{}",
            if self.document.dirty() {
                " • Modified"
            } else {
                ""
            }
        );
        let detail = self
            .document
            .path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "Save to create a project document".into());
        ui.add(egui::Label::new(label).truncate())
            .on_hover_text(detail);
    }

    pub(super) fn edit_frame_controls(
        &mut self,
        ui: &mut egui::Ui,
        cx: &UiContext<'_, Self>,
    ) -> Option<ScopeSlot> {
        let scope_slot = self.workspace_controls(ui, cx);
        let adjusting = ui::values_are_being_adjusted(ui.ctx());
        if self.document.visit.adjustment_layers.adjusting_values != adjusting {
            self.document.visit.adjustment_layers.adjusting_values = adjusting;
            ui.ctx().request_repaint();
        }
        scope_slot
    }

    fn workspace_controls(
        &mut self,
        ui: &mut egui::Ui,
        cx: &UiContext<'_, Self>,
    ) -> Option<ScopeSlot> {
        if self.fullscreen.is_none() {
            // The fixed name column and the balance wheel set the useful width; narrower than
            // this leaves the axis tracks too short to drag comfortably.
            egui::Panel::right("controls")
                .default_size(340.)
                .min_size(270.)
                .show(ui, |ui| self.controls(ui, cx))
                .inner
        } else {
            self.retire_control_input();
            self.hidden_filmstrip(ui, cx);
            None
        }
    }

    pub(super) fn dispatch_frame_actions(
        &mut self,
        ui: &mut egui::Ui,
        cx: &UiContext<'_, Self>,
        actions: Actions,
    ) {
        if actions.menu.is_some_and(|action| {
            !self.menu_targets_text(action) && !self.application_menu_state().enabled(action)
        }) {
            return;
        }
        if let Some(action) = actions.file {
            self.file_action(action, cx);
        } else if actions.export {
            self.open_export_dialog();
        } else if let Some(action) = actions.menu {
            if self.menu_targets_text(action) {
                // Fallback menu clicks arrive after text widgets have rendered.
                self.menus.pending.push_front(action);
                ui.ctx().request_repaint();
            } else {
                self.apply_menu_action(action, ui.ctx(), cx);
            }
        }
    }

    pub(super) fn frame_dialogs(
        &mut self,
        ui: &mut egui::Ui,
        cx: &UiContext<'_, Self>,
        dialog_escape_available: bool,
    ) {
        if self.dialogs.is_open() {
            self.dialogs.paint(ui.ctx(), dialog_escape_available);
            return;
        }

        self.export_dialogs(ui.ctx(), dialog_escape_available, cx);
        if !self.dialogs.is_open() {
            self.activities
                .modal(ui.ctx(), egui::Id::new("working-document"));
        }
        if self.dialogs.is_open() {
            // The export form can submit a dialog after this pass's dialog check.
            ui.ctx().request_repaint();
        }
    }
}
