use super::*;
use crate::platform::menu::{Action, State};

#[cfg(test)]
mod quit_tests;
#[cfg(test)]
mod tests;

#[derive(Default)]
pub(super) struct Menus {
    pub(super) native: bool,
    pub(super) pending: std::collections::VecDeque<Action>,
    pub(super) panel: Option<Panel>,
    pub(super) text_focused: bool,
    pub(super) history_input_blocked: bool,
}

pub(super) struct Panel {
    pub kind: ui::AppPanel,
    workspace: workspace_input::WorkspaceBlock,
}

#[derive(Clone, Copy)]
pub(super) struct Command {
    pub action: Action,
    file: Option<FileAction>,
}

impl Command {
    pub fn is_file(self) -> bool {
        self.file.is_some()
    }
}

impl Editor {
    fn open_application_panel(&mut self, kind: ui::AppPanel) {
        self.menus.panel = Some(Panel {
            kind,
            workspace: self.workspace_input.block(),
        });
    }

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

        if self.menus.pending.contains(&Action::Quit) || self.document.files.closing() {
            host::QuitState::Pending
        } else {
            host::QuitState::Running
        }
    }

    pub(super) fn request_application_quit(&mut self) {
        if self.application_quit_state() != host::QuitState::Running {
            return;
        }
        if self.application_menu_state().enabled(Action::Quit) {
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
                "Finish or cancel the current operation or interaction before quitting".into();
        }
    }

    pub(super) fn document_command_available(&self) -> bool {
        !self.workspace_input.is_blocked() && !self.editing() && !self.allow_close
    }

    pub(super) fn application_menu_state(&self) -> State {
        use Action::*;
        let mut state = State {
            lightbox: self.showing_lightbox(),
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
        if let Some(panel) = &self.menus.panel {
            if !self.allow_close && !panel.workspace.is_covered() {
                state.enabled.insert(Close);
                if !self.editing() {
                    state.enabled.insert(Quit);
                }
            }
            return state;
        }
        if self.allow_close || self.workspace_input.is_blocked() {
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
        if !self.interaction_blocked() {
            state.enabled.extend([About, Settings, Help]);
        }
        if self.document_command_available() {
            state
                .enabled
                .extend([New, Open, AddScans, Close, Quit, Save, SaveAs]);
            if self.document.project.input.ready() && self.selected_area().is_some() {
                state.enabled.insert(Export);
            }
        }
        if self.selected_area().is_some() && !self.interaction_blocked() && !self.showing_lightbox()
        {
            state.enabled.extend([Fit, ActualSize, ZoomIn, ZoomOut]);
        }
        if self.view_mode_available() && !self.menus.text_focused {
            state.enabled.extend([Lightbox, SinglePhoto]);
        }
        if self.fullscreen_available() {
            state.enabled.insert(PhotoInspection);
        }
        if self.fullscreen.is_none() && !self.showing_lightbox() {
            state.enabled.insert(Scopes);
        }
        state
    }

    pub(super) fn take_menu_command(&mut self, ui: &mut egui::Ui) -> Option<Command> {
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
            if state.enabled(Action::Close) {
                menu = Some(Action::Close);
            } else if !self.document.files.pending() {
                self.status =
                    "Finish or cancel the current edit or modal operation before closing".into();
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
        let action = menu?;
        if !self.menu_targets_text(action) {
            ui::surrender_keyboard_focus(ui.ctx());
        }
        Some(Command {
            action,
            file: self.menu_file_action(action),
        })
    }

    pub(super) fn menu_command_available(&self, command: Command) -> bool {
        self.menu_targets_text(command.action)
            || self.application_menu_state().enabled(command.action)
    }

    pub(super) fn run_menu_command(
        &mut self,
        command: Command,
        context: &egui::Context,
        cx: &UiContext<'_, Self>,
    ) {
        if let Some(file) = command.file {
            self.file_action(file, cx);
        } else if self.menu_targets_text(command.action) {
            // Menu clicks arrive after text widgets have rendered.
            self.menus.pending.push_front(command.action);
            context.request_repaint();
        } else {
            self.apply_menu_action(command.action, context, cx);
        }
    }

    fn menu_shortcut(&self, ui: &egui::Ui) -> Option<Action> {
        let state = self.application_menu_state();
        // Bare letters can arrive as window input even with a native menu installed.
        if self.workspace_input.shortcuts_allowed(ui.ctx())
            && let Some(action) =
                [Action::Lightbox, Action::SinglePhoto]
                    .into_iter()
                    .find(|&action| {
                        state.enabled(action)
                            && ui.input_mut(|input| {
                                input.modifiers.is_none()
                                    && input.consume_shortcut(&action.shortcut().unwrap())
                            })
                    })
        {
            return Some(action);
        }
        if self.menus.native {
            return None;
        }
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

    fn menu_file_action(&self, action: Action) -> Option<FileAction> {
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

    fn apply_menu_action(
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
            About => self.open_application_panel(ui::AppPanel::About),
            Settings => self.open_application_panel(ui::AppPanel::Settings),
            Help => self.open_application_panel(ui::AppPanel::Help),
            Close => self.menus.panel = None,
            Quit => {
                // Closing Settings commits its focused field before the next frame's
                // guarded document transition. Quit must never bypass dirty protection.
                self.menus.panel = None;
                self.menus.pending.push_front(Quit);
            }
            Export => return self.open_export_dialog(),
            Lightbox => self.set_view_mode(viewer_ui::Mode::Lightbox),
            SinglePhoto => self.set_view_mode(viewer_ui::Mode::Photo),
            Fit => self.with_tool_viewport(Viewport::fit),
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
        if let Some(panel) = self.menus.panel.as_ref().map(|panel| panel.kind)
            && ui::app_panel(context, panel, escape_available, |ui| {
                self.cache_preferences(ui)
            })
        {
            self.menus.panel = None;
            context.request_repaint();
        }
    }
}
