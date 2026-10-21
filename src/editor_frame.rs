use super::*;
use crate::platform::menu::{Action as MenuAction, State as MenuState};

pub(super) struct Actions {
    pub(super) menu: Option<MenuAction>,
    pub(super) file: Option<FileAction>,
    pub(super) export: bool,
    text_input_pending: bool,
    state: MenuState,
}

impl Editor {
    pub(super) fn initialize_ui(&mut self, ui: &mut egui::Ui) -> bool {
        if self.started {
            return false;
        }
        self.started = true;
        presentation::configure_branding(ui);
        presentation::configure_fonts(ui.ctx());
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(1100., 800.)));
        // Named font families become available at the start of the next pass.
        ui.ctx().request_discard("Initialize editor typography");
        true
    }

    pub(super) fn receive_frame(&mut self) {
        if self.document.visit.ready.is_none()
            || self.working.authority().is_some_and(|id| id != self.id)
        {
            self.working.clear();
        }
        self.stash_review_region();
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
        let mut menu = self
            .menus
            .pending
            .pop_front()
            .or_else(|| self.menu_shortcut(ui))
            .filter(|&action| {
                state.enabled(action) && self.history_menu_input_available(action, ui.ctx())
            });
        if !self.allow_close && ui.input(|input| input.viewport().close_requested()) {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if state.enabled(MenuAction::Close) {
                menu = Some(MenuAction::Close);
            } else if self.transition.is_none() && self.project_overwrite.is_none() {
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
        if self.busy {
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
            text_input_pending,
            state,
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
        self.sync_adjustment_drag(ui.ctx());
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
        cx: &UiContext<'_, Self>,
        text_was_focused: bool,
        dialog_escape_available: bool,
    ) {
        if self.document.analysis_preparing.is_some()
            && dialog_escape_available
            && !text_was_focused
            && !ui.ctx().egui_wants_keyboard_input()
            && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.cancel_analysis();
        }
        if self.document.analysis_review.is_some()
            && self.document.analysis_pending.is_none()
            && dialog_escape_available
            && !text_was_focused
            && !ui.ctx().egui_wants_keyboard_input()
        {
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                self.finish_pick(cx);
            } else {
                let previous =
                    ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft));
                let next =
                    ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight));
                if previous || next {
                    let index = if previous {
                        self.selected.saturating_sub(1)
                    } else {
                        (self.selected + 1).min(self.document.project.order.len().saturating_sub(1))
                    };
                    if let Some(&id) = self.document.project.order.get(index) {
                        self.browse_analysis_review(id, cx);
                    }
                }
            }
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
        if !self.busy
            && self.menus.panel.is_none()
            && self.fullscreen.is_none()
            && !fullscreen_was_active
            && self.failure.is_none()
            && self.removal_pending.is_none()
            && !self.document.visit.adjustments.confirming()
            && !(text_was_focused && ui.input(|input| input.key_pressed(egui::Key::Escape)))
            && !self.export_modal()
            && self.transition.is_none()
            && self.project_overwrite.is_none()
            && !ui.ctx().egui_wants_keyboard_input()
            && !egui::Popup::is_any_open(ui.ctx())
        {
            if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Z))
                && !self
                    .document
                    .visit
                    .mask_edit
                    .as_ref()
                    .is_some_and(MaskEdit::dragging)
            {
                self.viewport.fit();
            }
            self.select_active_shortcut(ui.ctx());
            self.tool_shortcuts(ui, cx);
            self.selection_shortcuts(ui, cx, file_action);
        }
    }

    fn tool_shortcuts(&mut self, ui: &mut egui::Ui, cx: &UiContext<'_, Self>) {
        if self.pick.is_some()
            && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.status = "Pick cancelled; processing unchanged".into();
            self.finish_pick(cx);
            self.region = None;
        }
        if self.pick.is_some()
            && self.region.is_some()
            && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
        {
            self.apply_pick(cx);
        }
        if self.document.visit.crop_edit.is_some() {
            if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                self.finish_crop(false);
            } else if ui
                .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
            {
                self.finish_crop(true);
            }
        }
        if self.document.auto_crop_edit.is_some() {
            if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                self.finish_auto_crop(false);
            } else if ui
                .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
            {
                self.finish_auto_crop(true);
            }
        }
        if self.document.visit.mask_edit.is_some()
            && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.finish_mask(false);
        }
        if self.document.dust.cleanup
            && ui.input_mut(|input| {
                input.consume_key(egui::Modifiers::NONE, egui::Key::Delete)
                    || input.consume_key(egui::Modifiers::NONE, egui::Key::Backspace)
            })
        {
            self.delete_selected_dust_repair();
        }
        if self.document.dust.cleanup
            && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.finish_dust_cleanup();
        }
    }

    fn selection_shortcuts(
        &mut self,
        ui: &mut egui::Ui,
        cx: &UiContext<'_, Self>,
        file_action: &mut Option<FileAction>,
    ) {
        let adjustment_delete = self.adjustment_owns_delete();
        if adjustment_delete
            && !self.document.visit.adjustments.dragging()
            && ui.input_mut(|input| {
                input.consume_key(egui::Modifiers::NONE, egui::Key::Delete)
                    || input.consume_key(egui::Modifiers::NONE, egui::Key::Backspace)
            })
        {
            self.delete_selected_adjustment();
        }
        if !self.document.dust.cleanup
            && self.document.auto_crop_edit.is_none()
            && !adjustment_delete
            && !self.document.visit.adjustments.dragging()
            && self.roll_edit_available()
            && ui.input_mut(|input| {
                input.consume_key(egui::Modifiers::NONE, egui::Key::Delete)
                    || input.consume_key(egui::Modifiers::NONE, egui::Key::Backspace)
            })
        {
            *file_action = Some(FileAction::Remove);
        }
        if self.pick.is_none()
            && self.document.visit.crop_edit.is_none()
            && self.document.visit.mask_edit.is_none()
            && !self.document.visit.adjustments.dragging()
            && !self.document.project.order.is_empty()
        {
            let previous = ui
                .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft));
            let next = ui
                .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight));
            let index = if previous {
                self.selected.saturating_sub(1)
            } else if next {
                (self.selected + 1).min(self.document.project.order.len() - 1)
            } else {
                self.selected
            };
            if index != self.selected {
                self.select_roll_photo(index, egui::Modifiers::NONE, cx);
            }
        }
    }

    pub(super) fn disable_blocked_workspace(&self, ui: &mut egui::Ui) {
        if self.menus.panel.is_some()
            || self.tasks.blocks_workspace()
            || self.failure.is_some()
            || self.export_modal()
            || self.document_pending.is_some()
            || self.transition.is_some()
            || self.project_overwrite.is_some()
            || self.removal_pending.is_some()
            || self.document.visit.adjustments.confirming()
        {
            ui.disable();
        }
    }

    pub(super) fn workspace_chrome(
        &mut self,
        ui: &mut egui::Ui,
        cx: &UiContext<'_, Self>,
        actions: &mut Actions,
    ) -> Option<presentation::MenuFocus> {
        let mut focus = None;
        if self.fullscreen.is_none() {
            presentation::app_header(ui, |ui| {
                if self.menus.native {
                    self.project_identity(ui);
                } else {
                    let response = presentation::application_menus(ui, &actions.state, |ui| {
                        self.project_identity(ui)
                    });
                    focus = response.focus;
                    if let Some(action) = response.action {
                        if actions.text_input_pending {
                            self.menus.pending.push_back(action);
                            ui.ctx().request_repaint();
                        } else {
                            actions.menu = Some(action);
                            actions.file = self.menu_file_action(action);
                            actions.export = action == MenuAction::Export;
                        }
                    }
                }
            });
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
                .exact_size(presentation::ACTIVITY_BAR_HEIGHT)
                .show(ui, |ui| {
                    if let Some(view) = self.tasks.view() {
                        if let Some(request) = view.compact(ui) {
                            self.cancel_task(request);
                        }
                    } else {
                        ui.add(egui::Label::new(&self.status).truncate())
                            .on_hover_text(&self.status);
                    }
                });
            self.filmstrip(ui, cx);
            if self.removal_pending.is_some() {
                ui.disable();
            }
        }
        focus
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
        previous_roll: RollSettings,
        previous_photo: PhotoSettings,
    ) -> (Option<ScopeSlot>, bool) {
        self.begin_control_history(match self.tab {
            0 => "Roll settings",
            1 => "Photo settings",
            _ => "Rendering target",
        });
        let reset_scope = match (self.tab, self.current_source) {
            (0, _) => editing::Scope::Roll,
            (1, Some(source)) => editing::Scope::Photo(source, self.adjustment_target()),
            _ => editing::Scope::Rendering,
        };
        ui.ctx()
            .data_mut(|data| data.insert_temp(egui::Id::new("defer-control-resets"), true));
        let controls_photo = (self.tab == 1).then_some(self.current_source).flatten();
        let controls = self.workspace_controls(ui, cx);
        self.validate_frame_edits(previous_roll, previous_photo);
        self.record_controls(controls_photo, ui);
        let resets = ui.ctx().data_mut(|data| {
            data.remove::<bool>(egui::Id::new("defer-control-resets"));
            data.remove_temp::<Vec<controls::Reset>>(egui::Id::new("control-resets"))
                .unwrap_or_default()
        });
        self.control_commands.extend(
            resets
                .into_iter()
                .map(|reset| editing::Command::Reset(reset_scope, reset)),
        );
        for command in std::mem::take(&mut self.control_commands) {
            self.apply_control_command(command, cx);
            ui.ctx().request_repaint();
        }
        controls
    }

    fn workspace_controls(
        &mut self,
        ui: &mut egui::Ui,
        cx: &UiContext<'_, Self>,
    ) -> (Option<ScopeSlot>, bool) {
        if self.fullscreen.is_none() {
            egui::Panel::right("controls")
                .default_size(315.)
                .min_size(270.)
                .show(ui, |ui| self.controls(ui, cx))
                .inner
        } else {
            self.hidden_filmstrip(ui, cx);
            (None, false)
        }
    }

    pub(super) fn validate_frame_edits(
        &mut self,
        previous_roll: RollSettings,
        previous_photo: PhotoSettings,
    ) {
        if (self.roll != previous_roll || self.photo != previous_photo)
            && let Some(calibration) = self.calibration()
            && let Err(error) = if self.roll != previous_roll {
                self.validate_processing(calibration, self.roll, self.photo)
            } else {
                self.dust_validate_processing(calibration, self.roll, self.photo)
                    .and_then(|()| {
                        self.positive_processing(calibration, self.roll, self.photo)
                            .map(|_| ())
                    })
            }
        {
            self.roll = previous_roll;
            self.photo = previous_photo;
            self.status = format!("Edit rejected: {error}");
        }
    }

    pub(super) fn dispatch_frame_actions(
        &mut self,
        ui: &mut egui::Ui,
        cx: &UiContext<'_, Self>,
        actions: Actions,
        analyze: bool,
    ) {
        let auto_crop = self.auto_crop_requested.take();
        if let Some(action) = actions.file {
            if self.document.visit.mask_edit.is_none() {
                self.file_action(action, cx);
            } else {
                self.status = "Apply or cancel the mask before a file action".into();
            }
        } else if actions.export {
            if self.document.visit.mask_edit.is_none() {
                self.open_export_dialog();
            } else {
                self.status = "Apply or cancel the mask before exporting".into();
            }
        } else if let Some(action) = actions.menu {
            if self.menu_targets_text(action) {
                // Fallback menu clicks arrive after text widgets have rendered.
                self.menus.pending.push_front(action);
                ui.ctx().request_repaint();
            } else {
                self.apply_menu_action(action, ui.ctx(), cx);
            }
        } else if analyze {
            // Inspect the validated, focus-committed Photo state before confirming a reset.
            self.request_analysis(cx);
        } else if let Some(target) = auto_crop {
            self.auto_crop(target);
            ui.ctx().request_repaint();
        }
    }

    pub(super) fn frame_dialogs(
        &mut self,
        ui: &mut egui::Ui,
        cx: &UiContext<'_, Self>,
        dialog_escape_available: bool,
    ) {
        if self.failure.is_some() {
            self.feedback_dialog(ui.ctx(), dialog_escape_available);
            return;
        }
        self.document_dialogs(ui.ctx(), dialog_escape_available, cx);
        self.removal_dialog(ui.ctx(), dialog_escape_available, cx);
        self.adjustment_dialog(ui.ctx(), dialog_escape_available);
        self.export_dialogs(ui.ctx(), dialog_escape_available, cx);
        if self.failure.is_none()
            && let Some(request) = self
                .tasks
                .modal(ui.ctx(), egui::Id::new("working-document"))
        {
            self.cancel_task(request);
        }
        if self.failure.is_some() {
            // A decision action can fail after this frame's feedback check.
            ui.ctx().request_repaint();
        }
    }
}
