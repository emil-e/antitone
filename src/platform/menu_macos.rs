use super::{Action, Entry, MENUS, State};
use muda::{CheckMenuItem, Menu, MenuId, MenuItem, PredefinedMenuItem, Submenu};

pub(crate) struct NativeMenu {
    root: Menu,
    items: Vec<(Action, muda::MenuItemKind)>,
}

impl NativeMenu {
    pub(crate) fn new(state: &State) -> Result<Self, muda::Error> {
        let mut native = Self {
            root: Menu::new(),
            items: Vec::new(),
        };
        let mut windows = None;
        let mut help = None;
        for &(title, entries) in MENUS {
            let submenu = Submenu::new(title, true);
            native.populate(&submenu, entries, state)?;
            native.root.append(&submenu)?;
            match title {
                "Window" => windows = Some(submenu),
                "Help" => help = Some(submenu),
                _ => {}
            }
        }
        native.root.init_for_nsapp();
        windows.unwrap().set_as_windows_menu_for_nsapp();
        help.unwrap().set_as_help_menu_for_nsapp();
        Ok(native)
    }

    fn populate(
        &mut self,
        menu: &Submenu,
        entries: &[Entry],
        state: &State,
    ) -> Result<(), muda::Error> {
        for entry in entries {
            match entry {
                Entry::Separator => menu.append(&PredefinedMenuItem::separator())?,
                Entry::Submenu(title, children) => {
                    let submenu = Submenu::new(title, true);
                    self.populate(&submenu, children, state)?;
                    menu.append(&submenu)?;
                }
                Entry::Action(Action::Minimize) => {
                    menu.append(&PredefinedMenuItem::minimize(None))?
                }
                Entry::Action(Action::ZoomWindow) => {
                    menu.append(&PredefinedMenuItem::zoom(None))?
                }
                Entry::Action(action) => {
                    if *action == Action::Quit {
                        menu.append_items(&[
                            &PredefinedMenuItem::services(None),
                            &PredefinedMenuItem::separator(),
                            &PredefinedMenuItem::hide(None),
                            &PredefinedMenuItem::hide_others(None),
                            &PredefinedMenuItem::show_all(None),
                            &PredefinedMenuItem::separator(),
                        ])?;
                    }
                    let shortcut = action.shortcut().map(|shortcut| {
                        let key = match shortcut.logical_key {
                            egui::Key::Num0 => "0",
                            egui::Key::Num1 => "1",
                            egui::Key::Comma => ",",
                            key => key.name(),
                        };
                        let shift = if shortcut.modifiers.shift {
                            "Shift+"
                        } else {
                            ""
                        };
                        format!("CmdOrCtrl+{shift}{key}")
                            .parse()
                            .expect("valid menu accelerator")
                    });
                    let item = if let Some(checked) = state.checked(*action) {
                        muda::MenuItemKind::Check(CheckMenuItem::new(
                            action.label(),
                            state.enabled(*action),
                            checked,
                            shortcut,
                        ))
                    } else {
                        muda::MenuItemKind::MenuItem(MenuItem::new(
                            action.label(),
                            state.enabled(*action),
                            shortcut,
                        ))
                    };
                    match &item {
                        muda::MenuItemKind::MenuItem(item) => menu.append(item)?,
                        muda::MenuItemKind::Check(item) => menu.append(item)?,
                        _ => unreachable!(),
                    }
                    self.items.push((*action, item));
                }
            }
        }
        Ok(())
    }

    pub(crate) fn action(&self, id: &MenuId) -> Option<Action> {
        self.items
            .iter()
            .find_map(|(action, item)| (item.id() == id).then_some(*action))
    }

    pub(crate) fn update(&self, state: &State) {
        for &(action, ref item) in &self.items {
            match item {
                muda::MenuItemKind::MenuItem(item) => item.set_enabled(state.enabled(action)),
                muda::MenuItemKind::Check(item) => {
                    item.set_enabled(state.enabled(action));
                    item.set_checked(state.checked(action).unwrap());
                }
                _ => unreachable!(),
            }
        }
    }
}

impl Drop for NativeMenu {
    fn drop(&mut self) {
        self.root.remove_for_nsapp();
    }
}

/// Native file panels have an AppKit responder, not an egui text control. Swap only
/// Edit while a synchronous panel runs its nested event loop, then restore our bridge.
pub(crate) struct NativeDialogEditing {
    item: objc2::rc::Retained<objc2_app_kit::NSMenuItem>,
    previous: objc2::rc::Retained<objc2_app_kit::NSMenu>,
}

impl NativeDialogEditing {
    pub(crate) fn new() -> Option<Self> {
        use objc2::{MainThreadMarker, MainThreadOnly, sel};
        use objc2_app_kit::{NSApplication, NSMenu, NSMenuItem};
        use objc2_foundation::ns_string;

        let main = MainThreadMarker::new()?;
        let app = NSApplication::sharedApplication(main);
        let item = app.mainMenu()?.itemWithTitle(ns_string!("Edit"))?;
        let previous = item.submenu()?;
        let edit = NSMenu::new(main);
        for (title, action, key) in [
            (ns_string!("Cut"), sel!(cut:), ns_string!("x")),
            (ns_string!("Copy"), sel!(copy:), ns_string!("c")),
            (ns_string!("Paste"), sel!(paste:), ns_string!("v")),
            (ns_string!("Select All"), sel!(selectAll:), ns_string!("a")),
        ] {
            // Standard AppKit edit selectors accept a single sender argument;
            // nil targets route to the file panel's native first responder.
            let command = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(main),
                    title,
                    Some(action),
                    key,
                )
            };
            edit.addItem(&command);
        }
        item.setSubmenu(Some(&edit));
        Some(Self { item, previous })
    }
}

impl Drop for NativeDialogEditing {
    fn drop(&mut self) {
        self.item.setSubmenu(Some(&self.previous));
    }
}
