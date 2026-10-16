use super::*;

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
        self.receive();
        if self.ready.is_none() || self.working.authority().is_some_and(|id| id != self.id) {
            self.working.clear();
        }
        self.stash_review_region();
        self.seed_thumbnail();
        self.thumbnails.receive(&self.document);
        self.validate_pending_preview();
    }

    pub(super) fn file_actions_available(&self) -> bool {
        !self.busy
            && self.failure.is_none()
            && self.pick.is_none()
            && self.auto_crop_edit.is_none()
            && self.crop_edit.is_none()
            && self.mask_edit.is_none()
            && self.overwrite.is_none()
            && self.transition.is_none()
            && self.project_overwrite.is_none()
            && self.resume_transition.is_none()
            && self.removal_pending.is_none()
            && !self.adjustments.confirming()
    }

    pub(super) fn file_shortcuts(
        &mut self,
        ui: &mut egui::Ui,
        available: bool,
    ) -> Option<FileAction> {
        let mut file_action = None;
        if !self.allow_close && ui.input(|input| input.viewport().close_requested()) {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if available {
                file_action = Some(FileAction::Close);
            } else if self.transition.is_none() && self.project_overwrite.is_none() {
                self.status =
                    "Finish the current operation or cancel the pick before closing".into();
            }
        }
        if self.busy || self.analysis_area_pending.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
        if available {
            file_action = file_action.or_else(|| {
                ui.input_mut(|input| {
                    [
                        (
                            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                            egui::Key::S,
                            FileAction::SaveAs,
                        ),
                        (egui::Modifiers::COMMAND, egui::Key::S, FileAction::Save),
                        (egui::Modifiers::COMMAND, egui::Key::O, FileAction::Open),
                        (egui::Modifiers::COMMAND, egui::Key::N, FileAction::New),
                    ]
                    .into_iter()
                    .find_map(|(modifiers, key, action)| {
                        input.consume_key(modifiers, key).then_some(action)
                    })
                })
            });
        }
        if file_action.is_some() {
            ui.ctx().memory_mut(|memory| {
                if let Some(id) = memory.focused() {
                    memory.surrender_focus(id);
                }
            });
        }
        file_action
    }

    pub(super) fn frame_shortcuts(
        &mut self,
        ui: &mut egui::Ui,
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
        self.analysis_shortcuts(ui, text_was_focused, dialog_escape_available);
        self.workspace_shortcuts(ui, file_action, fullscreen_was_active, text_was_focused);
    }

    fn analysis_shortcuts(
        &mut self,
        ui: &mut egui::Ui,
        text_was_focused: bool,
        dialog_escape_available: bool,
    ) {
        if self.analysis_preparing.is_some()
            && dialog_escape_available
            && !text_was_focused
            && !ui.ctx().egui_wants_keyboard_input()
            && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.cancel_analysis();
        }
        if self.analysis_review.is_some()
            && self.analysis_pending.is_none()
            && dialog_escape_available
            && !text_was_focused
            && !ui.ctx().egui_wants_keyboard_input()
        {
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                self.finish_pick();
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
                        self.browse_analysis_review(id);
                    }
                }
            }
        }
    }

    fn workspace_shortcuts(
        &mut self,
        ui: &mut egui::Ui,
        file_action: &mut Option<FileAction>,
        fullscreen_was_active: bool,
        text_was_focused: bool,
    ) {
        if !self.busy
            && self.fullscreen.is_none()
            && !fullscreen_was_active
            && self.failure.is_none()
            && self.removal_pending.is_none()
            && !self.adjustments.confirming()
            && !(text_was_focused && ui.input(|input| input.key_pressed(egui::Key::Escape)))
            && self.overwrite.is_none()
            && self.transition.is_none()
            && self.project_overwrite.is_none()
            && !ui.ctx().egui_wants_keyboard_input()
            && !egui::Popup::is_any_open(ui.ctx())
        {
            if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Z))
                && !self.mask_edit.as_ref().is_some_and(MaskEdit::dragging)
            {
                self.viewport.fit();
            }
            self.select_active_shortcut(ui.ctx());
            self.tool_shortcuts(ui);
            self.selection_shortcuts(ui, file_action);
        }
    }

    fn tool_shortcuts(&mut self, ui: &mut egui::Ui) {
        if self.pick.is_some()
            && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.status = "Pick cancelled; processing unchanged".into();
            self.finish_pick();
            self.region = None;
        }
        if self.pick.is_some()
            && self.region.is_some()
            && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
        {
            self.apply_pick();
        }
        if self.crop_edit.is_some() {
            if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                self.finish_crop(false);
            } else if ui
                .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
            {
                self.finish_crop(true);
            }
        }
        if self.auto_crop_edit.is_some() {
            if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                self.finish_auto_crop(false);
            } else if ui
                .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
            {
                self.finish_auto_crop(true);
            }
        }
        if self.mask_edit.is_some()
            && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.finish_mask(false);
        }
        if self.dust.cleanup
            && ui.input_mut(|input| {
                input.consume_key(egui::Modifiers::NONE, egui::Key::Delete)
                    || input.consume_key(egui::Modifiers::NONE, egui::Key::Backspace)
            })
        {
            self.delete_selected_dust_repair();
        }
        if self.dust.cleanup
            && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.finish_dust_cleanup();
        }
    }

    fn selection_shortcuts(&mut self, ui: &mut egui::Ui, file_action: &mut Option<FileAction>) {
        let adjustment_delete = self.adjustment_owns_delete();
        if adjustment_delete
            && !self.adjustments.dragging()
            && ui.input_mut(|input| {
                input.consume_key(egui::Modifiers::NONE, egui::Key::Delete)
                    || input.consume_key(egui::Modifiers::NONE, egui::Key::Backspace)
            })
        {
            self.delete_selected_adjustment();
        }
        if !self.dust.cleanup
            && self.auto_crop_edit.is_none()
            && !adjustment_delete
            && !self.adjustments.dragging()
            && self.roll_edit_available()
            && ui.input_mut(|input| {
                input.consume_key(egui::Modifiers::NONE, egui::Key::Delete)
                    || input.consume_key(egui::Modifiers::NONE, egui::Key::Backspace)
            })
        {
            *file_action = Some(FileAction::Remove);
        }
        if self.pick.is_none()
            && self.crop_edit.is_none()
            && self.mask_edit.is_none()
            && !self.adjustments.dragging()
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
                self.select_roll_photo(index, egui::Modifiers::NONE);
            }
        }
    }

    pub(super) fn disable_blocked_workspace(&self, ui: &mut egui::Ui) {
        if self.failure.is_some()
            || self.overwrite.is_some()
            || self.document_pending.is_some()
            || self.transition.is_some()
            || self.project_overwrite.is_some()
            || self.resume_transition.is_some()
            || self.removal_pending.is_some()
            || self.adjustments.confirming()
        {
            ui.disable();
        }
    }

    pub(super) fn workspace_chrome(
        &mut self,
        ui: &mut egui::Ui,
        available: bool,
        file_action: &mut Option<FileAction>,
    ) -> Option<OutputFormat> {
        let mut export_action = None;
        if self.fullscreen.is_none() {
            presentation::app_header(ui, |ui| {
                ui.add_enabled_ui(available, |ui| {
                    ui.menu_button("File", |ui| {
                        for (label, action) in [
                            ("New", FileAction::New),
                            ("Open…", FileAction::Open),
                            ("Add RAWs…", FileAction::Add),
                            ("Save", FileAction::Save),
                            ("Save As…", FileAction::SaveAs),
                        ] {
                            if ui.button(label).clicked() {
                                *file_action = Some(action);
                                ui.close();
                            }
                        }
                    });
                });
                ui.add_enabled_ui(
                    available && self.calibration().is_some() && self.ready.is_some(),
                    |ui| {
                        ui.menu_button("Export", |ui| {
                            for (label, format) in [
                                ("TIFF…", OutputFormat::Tiff(self.target)),
                                ("EXR half…", OutputFormat::Exr(ExrPrecision::Half)),
                                ("EXR full…", OutputFormat::Exr(ExrPrecision::Full)),
                            ] {
                                if ui.button(label).clicked() {
                                    export_action = Some(format);
                                    ui.close();
                                }
                            }
                        });
                    },
                );
                ui.menu_button("Preferences", |ui| self.cache_preferences(ui));
                ui.separator();
                let name = self
                    .document
                    .path
                    .as_ref()
                    .and_then(|path| path.file_name())
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "Untitled".into());
                ui.add(
                    egui::Label::new(format!(
                        "{name}{}",
                        if self.document.dirty() {
                            " • Modified"
                        } else {
                            ""
                        }
                    ))
                    .truncate(),
                )
                .on_hover_text(
                    self.document
                        .path
                        .as_ref()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "Save to create a project document".into()),
                );
            });
            if file_action.is_some() {
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
            self.filmstrip(ui);
            if self.removal_pending.is_some() {
                ui.disable();
            }
        }
        export_action
    }

    pub(super) fn workspace_controls(&mut self, ui: &mut egui::Ui) -> (Option<ScopeSlot>, bool) {
        if self.fullscreen.is_none() {
            egui::Panel::right("controls")
                .default_size(315.)
                .min_size(270.)
                .show(ui, |ui| self.controls(ui))
                .inner
        } else {
            self.hidden_filmstrip(ui);
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
        file_action: Option<FileAction>,
        export_action: Option<OutputFormat>,
        analyze: bool,
    ) {
        let auto_crop = self.auto_crop_requested.take();
        if let Some(transition) = self.resume_transition.take() {
            self.perform_transition(transition, ui.ctx());
        } else if let Some(action) = file_action {
            if self.mask_edit.is_none() {
                self.file_action(action, ui.ctx());
            } else {
                self.status = "Apply or cancel the mask before a file action".into();
            }
        } else if let Some(format) = export_action {
            if self.mask_edit.is_none() {
                self.export_dialog(format);
            } else {
                self.status = "Apply or cancel the mask before exporting".into();
            }
        } else if analyze {
            // Inspect the validated, focus-committed Photo state before confirming a reset.
            self.request_analysis();
        } else if let Some(target) = auto_crop {
            self.auto_crop(target);
            ui.ctx().request_repaint();
        }
    }

    pub(super) fn frame_dialogs(&mut self, ui: &mut egui::Ui, dialog_escape_available: bool) {
        if self.failure.is_some() {
            self.feedback_dialog(ui.ctx(), dialog_escape_available);
            return;
        }
        self.document_dialogs(ui.ctx(), dialog_escape_available);
        self.removal_dialog(ui.ctx(), dialog_escape_available);
        self.adjustment_dialog(ui.ctx(), dialog_escape_available);
        if let Some((path, format)) = self.overwrite.clone() {
            let detail = path.display().to_string();
            let choice = presentation::DecisionDialog::new(
                "Replace this exported file?",
                "A file already exists at this location. Exporting will overwrite it. The project and RAW files are not changed.",
                "Replace file",
            )
            .warning()
            .detail(&detail)
            .show(
                ui.ctx(),
                egui::Id::new("Replace existing output?"),
                dialog_escape_available,
            );
            match choice {
                Some(presentation::DialogChoice::Cancel) => self.overwrite = None,
                Some(presentation::DialogChoice::Confirm) => {
                    self.overwrite = None;
                    self.export(path, format, Overwrite::Confirmed);
                }
                _ => {}
            }
        }
        if self.failure.is_none() {
            self.tasks
                .modal(ui.ctx(), egui::Id::new("working-document"));
        }
        if self.failure.is_some() {
            // A decision action can fail after this frame's feedback check.
            ui.ctx().request_repaint();
        }
    }
}
