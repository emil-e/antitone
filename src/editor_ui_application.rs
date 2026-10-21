use super::{TextAction, dialogs};
use crate::platform::menu::{Action, Entry, MENUS, State};
use egui::containers::menu::{MenuButton, SubMenuButton};

#[derive(Clone, Default)]
struct MenuMemory {
    open: bool,
    focused: Option<egui::Id>,
}

pub struct ApplicationMenuResponse {
    pub action: Option<Action>,
    pub focus: Option<MenuFocus>,
}

/// Retain text ownership during menu pointer input, not during unrelated clicks.
/// The host's input policy must be restored even if a later modal returns early.
pub struct MenuFocus {
    context: egui::Context,
    previous: egui::SurrenderFocusOn,
}

impl Drop for MenuFocus {
    fn drop(&mut self) {
        self.context
            .options_mut(|options| options.input_options.surrender_focus_on = self.previous);
    }
}

pub fn application_menus(
    ui: &mut egui::Ui,
    state: &State,
    trailing: impl FnOnce(&mut egui::Ui),
) -> ApplicationMenuResponse {
    let memory_id = egui::Id::new("application-menu-focus");
    let previous = ui
        .ctx()
        .data(|data| data.get_temp::<MenuMemory>(memory_id))
        .unwrap_or_default();
    // egui clears text focus on Escape before widgets run. A menu owns that
    // Escape; restore its text target after closing the popup, not the draft.
    let focused = ui
        .memory(|memory| memory.focused())
        .or_else(|| previous.open.then_some(previous.focused).flatten());
    let escaping = previous.open && ui.input(|input| input.key_pressed(egui::Key::Escape));
    let mut selected = None;
    let mut pointer_in_menu = false;
    let mut menu_open = false;
    egui::MenuBar::new().ui(ui, |ui| {
        for &(title, entries) in MENUS {
            let (button, popup) = MenuButton::new(title).ui(ui, |ui| {
                menu_entries(ui, state, entries, &mut selected, &mut pointer_in_menu)
            });
            pointer_in_menu |= button.contains_pointer();
            if let Some(popup) = popup {
                menu_open = true;
                pointer_in_menu |= popup.response.contains_pointer();
            }
        }
        trailing(ui);
    });
    if previous.open || menu_open {
        ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    }
    let open = menu_open && egui::Popup::is_any_open(ui.ctx());
    ui.ctx()
        .data_mut(|data| data.insert_temp(memory_id, MenuMemory { open, focused }));
    let focus = if (pointer_in_menu || escaping)
        && let Some(focused) = focused
    {
        let context = ui.ctx().clone();
        let previous = context.options(|options| options.input_options.surrender_focus_on);
        context.options_mut(|options| {
            options.input_options.surrender_focus_on = egui::SurrenderFocusOn::Never
        });
        context.memory_mut(|memory| memory.request_focus(focused));
        Some(MenuFocus { context, previous })
    } else {
        None
    };
    ApplicationMenuResponse {
        action: selected,
        focus,
    }
}

fn menu_entries(
    ui: &mut egui::Ui,
    state: &State,
    entries: &[Entry],
    selected: &mut Option<Action>,
    pointer_in_menu: &mut bool,
) {
    for entry in entries {
        match entry {
            Entry::Separator => {
                ui.separator();
            }
            Entry::Submenu(title, entries) => {
                let (button, popup) = SubMenuButton::new(*title).ui(ui, |ui| {
                    menu_entries(ui, state, entries, selected, pointer_in_menu)
                });
                *pointer_in_menu |= button.contains_pointer();
                if let Some(popup) = popup {
                    *pointer_in_menu |= popup.response.contains_pointer();
                }
                button.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, button.enabled(), *title)
                });
            }
            Entry::Action(action) => {
                let mut button = if let Some(checked) = state.checked(*action) {
                    egui::Button::selectable(checked, state.label(*action))
                } else {
                    egui::Button::new(state.label(*action))
                };
                if let Some(shortcut) = action.shortcut() {
                    button = button.shortcut_text(ui.ctx().format_shortcut(&shortcut));
                }
                let response = ui.add_enabled(state.enabled(*action), button);
                response.widget_info(|| {
                    let mut info = egui::WidgetInfo::labeled(
                        egui::WidgetType::Button,
                        response.enabled(),
                        state.label(*action),
                    );
                    info.selected = state.checked(*action);
                    info
                });
                if response.clicked() {
                    *selected = Some(*action);
                    ui.close();
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AppPanel {
    Settings,
    About,
    Help,
}

pub fn app_panel(
    context: &egui::Context,
    panel: AppPanel,
    escape_available: bool,
    settings: impl FnOnce(&mut egui::Ui),
) -> bool {
    let response = dialogs::modal(context, egui::Id::new(("application-panel", panel)), |ui| {
        match panel {
            AppPanel::Settings => {
                ui.heading("Settings");
                ui.strong("Memory");
                ui.label(
                    "These limits control optional cached data, not total application memory.",
                );
                settings(ui);
            }
            AppPanel::About => {
                ui.heading("Antitone");
                ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
                ui.label("An editor for camera-scanned film negatives.");
                ui.label("Analyze a roll, refine your photographs, and export TIFF or EXR.");
                ui.small("Development build · macOS on Apple Silicon");
            }
            AppPanel::Help => {
                ui.heading("Antitone Help");
                egui::ScrollArea::vertical()
                    .max_height((context.content_rect().height() - 240.).clamp(80., 420.))
                    .auto_shrink([false, false])
                    .show(ui, help_contents);
            }
        }
        ui.add_space(8.);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let close = TextAction::new("Close").show(ui);
            dialogs::focus_default(ui, &close);
            close.clicked()
        })
        .inner
    });
    response.inner || dialogs::escape_requested(&response, escape_available)
}

fn help_contents(ui: &mut egui::Ui) {
    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
    ui.strong("Start a roll");
    ui.label("Choose File → Add Scans… to add camera RAW files. Originals stay in place and are never modified.");
    ui.strong("Develop your photographs");
    ui.label("Use Roll to set calibration and scan orientation. Analyze proposes sampling areas for review before applying a starting grade. Use Photo for individual grading, cropping and cleanup; Rendering chooses the preview and TIFF colour space.");
    ui.strong("Save and export");
    ui.label("Save stores your settings and source references in an .antitone project. Keep the original scans available. File → Export writes selected photos or the whole roll as TIFF or EXR; finish any active draft first.");
    ui.label("Dust repairs are saved in the project, but TIFF/EXR exports and filmstrip thumbnails do not yet include them.");
    ui.strong("Inspect the image");
    ui.label("Use the viewer controls to fit, zoom and compare the developed source. F enters photo-only inspection; F or Escape returns. This covers the current display without opening a macOS fullscreen Space.");
    ui.label("Hover an unfamiliar icon for its name and shortcut. Tool-specific gestures are available from the viewer’s help control.");
}
