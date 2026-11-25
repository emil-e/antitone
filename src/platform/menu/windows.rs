use std::cell::RefCell;
use std::rc::{Rc, Weak};

use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{CheckMenuItem, Menu, MenuId, MenuItem, MenuItemKind, PredefinedMenuItem, Submenu};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::{
    Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL},
    WindowsAndMessaging::{MSG, TranslateAcceleratorW, WM_KEYDOWN, WM_SYSKEYDOWN},
};
use winit::event_loop::EventLoopBuilder;
use winit::platform::windows::EventLoopBuilderExtWindows;

use super::{Action, Entry, State, WINDOWS_MENUS};

/// Windows draws the menu bar inside the window, as its photographs editors do.
pub(crate) const SUPPORTED: bool = true;

pub(crate) struct MenuAccelerators(Rc<RefCell<Weak<AttachedMenu>>>);

impl MenuAccelerators {
    pub(crate) fn install<T>(builder: &mut EventLoopBuilder<T>) -> Self {
        let target = Rc::new(RefCell::new(Weak::<AttachedMenu>::new()));
        let lookup = target.clone();
        builder.with_msg_hook(move |message| {
            // Winit supplies a valid MSG on its window thread for this call.
            let message = unsafe { &*message.cast::<MSG>() };
            let target = lookup.borrow().upgrade();
            target.is_some_and(|menu| menu.translate(message))
        });
        Self(target)
    }

    pub(crate) fn attach(&self, menu: &NativeMenu) {
        self.0.replace(Rc::downgrade(&menu.attached));
    }
}

struct AttachedMenu {
    root: Menu,
    hwnd: HWND,
}

impl AttachedMenu {
    fn translate(&self, message: &MSG) -> bool {
        if message.hwnd != self.hwnd || !matches!(message.message, WM_KEYDOWN | WM_SYSKEYDOWN) {
            return false;
        }
        // Leave unmodified shortcuts to the editor's focus and popup rules. Windows
        // can consume a keystroke even when its menu action is disabled.
        if unsafe { GetKeyState(VK_CONTROL.into()) } >= 0 {
            return false;
        }

        // muda owns the table. Translating sends WM_COMMAND to the menu subclass,
        // which posts the same event as a click; winit skips dispatching the key.
        unsafe { TranslateAcceleratorW(self.hwnd, self.root.haccel() as _, message) != 0 }
    }
}

pub(crate) struct NativeMenu {
    attached: Rc<AttachedMenu>,
    items: Vec<(Action, MenuItemKind)>,
    state: State,
}

impl NativeMenu {
    pub(crate) fn new(
        state: &State,
        window: &winit::window::Window,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

        let handle = window.window_handle()?;
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return Err("the Windows menu needs a Win32 window handle".into());
        };
        let mut native = Self {
            attached: Rc::new(AttachedMenu {
                root: Menu::new(),
                hwnd: handle.hwnd.get() as HWND,
            }),
            items: Vec::new(),
            state: state.clone(),
        };
        for &(title, entries) in WINDOWS_MENUS {
            let submenu = Submenu::new(title, true);
            native.populate(&submenu, entries, state)?;
            native.attached.root.append(&submenu)?;
        }
        // The host initializes the menu on the window's own thread, and the tree is complete
        // before attachment: growing it afterwards would leave the attached menu stale.
        unsafe { native.attached.root.init_for_hwnd(handle.hwnd.get()) }?;
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
                Entry::Action(action) => {
                    let accelerator = accelerator(*action);
                    let item = if let Some(checked) = state.checked(*action) {
                        MenuItemKind::Check(CheckMenuItem::new(
                            state.label(*action),
                            state.enabled(*action),
                            checked,
                            accelerator,
                        ))
                    } else {
                        MenuItemKind::MenuItem(MenuItem::new(
                            state.label(*action),
                            state.enabled(*action),
                            accelerator,
                        ))
                    };
                    match &item {
                        MenuItemKind::MenuItem(item) => menu.append(item)?,
                        MenuItemKind::Check(item) => menu.append(item)?,
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

    pub(crate) fn update(&mut self, state: &State) {
        if self.state == *state {
            return;
        }
        for &(action, ref item) in &self.items {
            match item {
                MenuItemKind::MenuItem(item) => {
                    item.set_enabled(state.enabled(action));
                    if self.state.label(action) != state.label(action) {
                        item.set_text(state.label(action));
                    }
                }
                MenuItemKind::Check(item) => {
                    item.set_enabled(state.enabled(action));
                    item.set_checked(state.checked(action).unwrap());
                }
                _ => unreachable!(),
            }
        }
        self.state = state.clone();
    }
}

/// Windows accelerators are Control-based where macOS uses Command.
fn accelerator(action: Action) -> Option<Accelerator> {
    let shortcut = action.shortcut()?;
    let mut modifiers = Modifiers::empty();
    if shortcut.modifiers.command {
        modifiers |= Modifiers::CONTROL;
    }
    if shortcut.modifiers.shift {
        modifiers |= Modifiers::SHIFT;
    }
    if shortcut.modifiers.alt {
        modifiers |= Modifiers::ALT;
    }
    let code = match shortcut.logical_key {
        egui::Key::E => Code::KeyE,
        egui::Key::G => Code::KeyG,
        egui::Key::Q => Code::KeyQ,
        egui::Key::N => Code::KeyN,
        egui::Key::O => Code::KeyO,
        egui::Key::I => Code::KeyI,
        egui::Key::W => Code::KeyW,
        egui::Key::S => Code::KeyS,
        egui::Key::Z => Code::KeyZ,
        egui::Key::X => Code::KeyX,
        egui::Key::C => Code::KeyC,
        egui::Key::V => Code::KeyV,
        egui::Key::A => Code::KeyA,
        egui::Key::M => Code::KeyM,
        egui::Key::Num0 => Code::Digit0,
        egui::Key::Num1 => Code::Digit1,
        egui::Key::Comma => Code::Comma,
        _ => return None,
    };
    Some(Accelerator::new(modifiers, code))
}
