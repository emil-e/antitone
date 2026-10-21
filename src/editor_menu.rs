use super::*;
use crate::platform::menu::{Action, State};

#[cfg(test)]
#[path = "editor_quit_tests.rs"]
mod quit_tests;
#[cfg(test)]
#[path = "editor_menu_tests.rs"]
mod tests;

#[derive(Default)]
pub(super) struct Menus {
    pub(super) native: bool,
    pub(super) pending: std::collections::VecDeque<Action>,
    pub(super) panel: Option<presentation::AppPanel>,
    pub(super) text_focused: bool,
    pub(super) history_input_blocked: bool,
}

impl Editor {
    pub(super) fn menu_targets_text(&self, action: Action) -> bool {
        action.text_edit()
            || self.menus.text_focused && matches!(action, Action::Undo | Action::Redo)
    }

    pub(super) fn history_menu_input_available(
        &self,
        action: Action,
        context: &egui::Context,
    ) -> bool {
        !matches!(action, Action::Undo | Action::Redo)
            || self.menu_targets_text(action)
            || !egui::Popup::is_any_open(context)
                && !context.input(|input| input.pointer.any_down())
    }

    pub(super) fn application_quit_state(&self) -> host::QuitState {
        if self.allow_close {
            return host::QuitState::Approved;
        }
        if self.failure.is_some() {
            return host::QuitState::Running;
        }
        let closing =
            |transition: &Option<Transition>| matches!(transition, Some(Transition::Close));
        if self.menus.pending.contains(&Action::Quit)
            || closing(&self.transition)
            || closing(&self.after_save)
            || self
                .project_overwrite
                .as_ref()
                .is_some_and(|(_, after)| closing(after))
        {
            host::QuitState::Pending
        } else {
            host::QuitState::Running
        }
    }

    pub(super) fn request_application_quit(&mut self) {
        if self.application_quit_state() != host::QuitState::Running {
            return;
        }
        if self.file_available() {
            // Preserve leading text operations before the close guard, but do not
            // strand a native request behind unrelated menu commands or a failure.
            let text = self
                .menus
                .pending
                .iter()
                .take_while(|action| self.menu_targets_text(**action))
                .count();
            self.menus.pending.insert(text, Action::Quit);
        } else {
            self.status =
                "Finish the current operation or cancel the active tool before quitting".into();
        }
    }

    pub(super) fn file_available(&self) -> bool {
        !self.busy
            && !self.application_modal()
            && self.pick.is_none()
            && self.document.auto_crop_edit.is_none()
            && self.document.visit.crop_edit.is_none()
            && self.document.visit.mask_edit.is_none()
    }

    fn application_modal(&self) -> bool {
        self.allow_close
            || self.failure.is_some()
            || self.document_pending.is_some()
            || self.overwrite.is_some()
            || self.transition.is_some()
            || self.project_overwrite.is_some()
            || self.removal_pending.is_some()
            || self.document.visit.adjustments.confirming()
    }

    pub(super) fn application_menu_state(&self) -> State {
        use Action::*;
        let mut state = State {
            scopes_visible: self.scopes.visible,
            photo_inspection: self.fullscreen.is_some(),
            ..Default::default()
        };
        state.enabled.extend([Minimize, ZoomWindow]);
        if self.menus.text_focused {
            state
                .enabled
                .extend([Undo, Redo, Cut, Copy, Paste, SelectAll]);
        }
        if !self.menus.text_focused {
            state.undo_label = self
                .history_label(history::Direction::Undo)
                .map(|label| format!("Undo {label}"));
            state.redo_label = self
                .history_label(history::Direction::Redo)
                .map(|label| format!("Redo {label}"));
        }
        if self.application_modal() {
            return state;
        }
        if self.file_available() {
            state.enabled.insert(Quit);
        }
        if self.menus.panel.is_some() {
            state.enabled.insert(Close);
            return state;
        }
        if !self.menus.text_focused
            && self.history_available()
            && !(self.menus.native && self.menus.history_input_blocked)
        {
            if self.history_label(history::Direction::Undo).is_some() {
                state.enabled.insert(Undo);
            }
            if self.history_label(history::Direction::Redo).is_some() {
                state.enabled.insert(Redo);
            }
        }
        if !self.busy || self.export_operation.pending.is_some() {
            state.enabled.extend([About, Settings, Help]);
        }
        if self.file_available() {
            state
                .enabled
                .extend([New, Open, AddScans, Close, Save, SaveAs]);
            if self.calibration().is_some() && self.document.visit.ready.is_some() {
                state
                    .enabled
                    .extend([ExportTiff, ExportExrHalf, ExportExrFull]);
            }
        }
        if self.document.visit.ready.is_some()
            && !self.busy
            && !self
                .document
                .visit
                .mask_edit
                .as_ref()
                .is_some_and(MaskEdit::dragging)
        {
            state.enabled.extend([Fit, ActualSize, ZoomIn, ZoomOut]);
        }
        if self.fullscreen_available() {
            state.enabled.insert(PhotoInspection);
        }
        if self.fullscreen.is_none()
            && self.pick.is_none()
            && self.document.analysis_review.is_none()
        {
            state.enabled.insert(Scopes);
        }
        state
    }

    pub(super) fn menu_shortcut(&self, ui: &egui::Ui) -> Option<Action> {
        if self.menus.native {
            return None;
        }
        let state = self.application_menu_state();
        // Shifted shortcuts must precede their unshifted alternatives.
        [
            Action::Redo,
            Action::Undo,
            Action::SaveAs,
            Action::AddScans,
            Action::Save,
            Action::Open,
            Action::New,
            Action::Close,
            Action::Quit,
            Action::Settings,
            Action::Fit,
            Action::ActualSize,
        ]
        .into_iter()
        .find(|&action| {
            state.enabled(action)
                && !(self.menu_targets_text(action)
                    && matches!(action, Action::Undo | Action::Redo))
                && self.history_menu_input_available(action, ui.ctx())
                && ui.input_mut(|input| input.consume_shortcut(&action.shortcut().unwrap()))
        })
    }

    pub(super) fn menu_file_action(&self, action: Action) -> Option<FileAction> {
        Some(match action {
            Action::New => FileAction::New,
            Action::Open => FileAction::Open,
            Action::AddScans => FileAction::Add,
            Action::Close | Action::Quit if self.menus.panel.is_none() => FileAction::Close,
            Action::Save => FileAction::Save,
            Action::SaveAs => FileAction::SaveAs,
            _ => return None,
        })
    }

    pub(super) fn menu_export(&self, action: Action) -> Option<OutputFormat> {
        match action {
            Action::ExportTiff => Some(OutputFormat::Tiff(self.target)),
            Action::ExportExrHalf => Some(OutputFormat::Exr(ExrPrecision::Half)),
            Action::ExportExrFull => Some(OutputFormat::Exr(ExrPrecision::Full)),
            _ => None,
        }
    }

    pub(super) fn menu_input(&mut self, clipboard: Option<String>) -> Vec<egui::Event> {
        let mut events = Vec::new();
        let state = self.application_menu_state();
        while let Some(action) = self
            .menus
            .pending
            .front()
            .copied()
            .filter(|action| self.menu_targets_text(*action))
        {
            self.menus.pending.pop_front();
            if !state.enabled(action) {
                continue;
            }
            let event = match action {
                Action::Cut => egui::Event::Cut,
                Action::Copy => egui::Event::Copy,
                Action::Paste => {
                    let Some(text) = &clipboard else {
                        continue;
                    };
                    egui::Event::Paste(text.clone())
                }
                Action::Undo | Action::Redo => egui::Event::Key {
                    key: egui::Key::Z,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: if action == Action::Redo {
                        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT
                    } else {
                        egui::Modifiers::COMMAND
                    },
                },
                Action::SelectAll => egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::COMMAND,
                },
                _ => unreachable!(),
            };
            events.push(event);
        }
        events
    }

    pub(super) fn apply_menu_action(
        &mut self,
        action: Action,
        context: &egui::Context,
        cx: &UiContext<'_, Self>,
    ) {
        use Action::*;
        if !self.history_menu_input_available(action, context) {
            return;
        }
        match action {
            Undo => self.restore_history(history::Direction::Undo, cx),
            Redo => self.restore_history(history::Direction::Redo, cx),
            About => self.menus.panel = Some(presentation::AppPanel::About),
            Settings => self.menus.panel = Some(presentation::AppPanel::Settings),
            Help => self.menus.panel = Some(presentation::AppPanel::Help),
            Close => self.menus.panel = None,
            Quit => {
                // Closing Settings commits its focused field before the next frame's
                // guarded document transition. Quit must never bypass dirty protection.
                self.menus.panel = None;
                self.menus.pending.push_front(Quit);
            }
            Fit => self.viewport.fit(),
            Scopes => {
                self.scopes.visible = !self.scopes.visible;
                self.preferences.scopes_visible = self.scopes.visible;
                self.persist_preferences();
            }
            PhotoInspection => self.toggle_fullscreen(context),
            Minimize => context.send_viewport_cmd(egui::ViewportCommand::Minimized(true)),
            ZoomWindow => {
                let maximized = context.input(|input| input.viewport().maximized == Some(true));
                context.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
            }
            _ => {}
        }
        context.request_repaint();
    }

    pub(super) fn application_panel(&mut self, context: &egui::Context, escape_available: bool) {
        if let Some(panel) = self.menus.panel
            && presentation::app_panel(context, panel, escape_available, |ui| {
                self.cache_preferences(ui)
            })
        {
            self.menus.panel = None;
            context.request_repaint();
        }
    }
}
