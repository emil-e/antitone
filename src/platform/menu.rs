//! Application actions shared by native and in-window menu presentation.
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Action {
    About,
    Settings,
    Quit,
    New,
    Open,
    AddScans,
    Close,
    Save,
    SaveAs,
    Export,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    Fit,
    ActualSize,
    ZoomIn,
    ZoomOut,
    PhotoInspection,
    Scopes,
    Minimize,
    ZoomWindow,
    Help,
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Self::About => "About Antitone",
            Self::Settings => "Settings…",
            Self::Quit => "Quit Antitone",
            Self::New => "New",
            Self::Open => "Open…",
            Self::AddScans => "Add Scans…",
            Self::Close => "Close Window",
            Self::Save => "Save",
            Self::SaveAs => "Save As…",
            Self::Export => "Export…",
            Self::Undo => "Undo",
            Self::Redo => "Redo",
            Self::Cut => "Cut",
            Self::Copy => "Copy",
            Self::Paste => "Paste",
            Self::SelectAll => "Select All",
            Self::Fit => "Fit Image",
            Self::ActualSize => "Actual Size",
            Self::ZoomIn => "Zoom In",
            Self::ZoomOut => "Zoom Out",
            Self::PhotoInspection => "Photo Inspection",
            Self::Scopes => "Show Scopes",
            Self::Minimize => "Minimize",
            Self::ZoomWindow => "Zoom",
            Self::Help => "Antitone Help",
        }
    }

    pub fn shortcut(self) -> Option<egui::KeyboardShortcut> {
        use egui::{Key, KeyboardShortcut, Modifiers};
        let (modifiers, key) = match self {
            Self::Settings => (Modifiers::COMMAND, Key::Comma),
            Self::Quit => (Modifiers::COMMAND, Key::Q),
            Self::New => (Modifiers::COMMAND, Key::N),
            Self::Open => (Modifiers::COMMAND, Key::O),
            Self::AddScans => (Modifiers::COMMAND | Modifiers::SHIFT, Key::I),
            Self::Close => (Modifiers::COMMAND, Key::W),
            Self::Save => (Modifiers::COMMAND, Key::S),
            Self::SaveAs => (Modifiers::COMMAND | Modifiers::SHIFT, Key::S),
            Self::Undo => (Modifiers::COMMAND, Key::Z),
            Self::Redo => (Modifiers::COMMAND | Modifiers::SHIFT, Key::Z),
            Self::Cut => (Modifiers::COMMAND, Key::X),
            Self::Copy => (Modifiers::COMMAND, Key::C),
            Self::Paste => (Modifiers::COMMAND, Key::V),
            Self::SelectAll => (Modifiers::COMMAND, Key::A),
            Self::Fit => (Modifiers::COMMAND, Key::Num0),
            Self::ActualSize => (Modifiers::COMMAND, Key::Num1),
            Self::Minimize => (Modifiers::COMMAND, Key::M),
            _ => return None,
        };
        Some(KeyboardShortcut::new(modifiers, key))
    }

    pub fn text_edit(self) -> bool {
        matches!(self, Self::Cut | Self::Copy | Self::Paste | Self::SelectAll)
    }
}

pub enum Entry {
    Action(Action),
    Separator,
    Submenu(&'static str, &'static [Entry]),
}

use Action::*;
use Entry::{Action as Item, Separator as Sep};

pub const MENUS: &[(&str, &[Entry])] = &[
    (
        "Antitone",
        &[Item(About), Sep, Item(Settings), Sep, Item(Quit)],
    ),
    (
        "File",
        &[
            Item(New),
            Item(Open),
            Item(AddScans),
            Sep,
            Item(Close),
            Sep,
            Item(Save),
            Item(SaveAs),
            Sep,
            Item(Export),
        ],
    ),
    (
        "Edit",
        &[
            Item(Undo),
            Item(Redo),
            Sep,
            Item(Cut),
            Item(Copy),
            Item(Paste),
            Sep,
            Item(SelectAll),
        ],
    ),
    (
        "View",
        &[
            Item(Fit),
            Item(ActualSize),
            Item(ZoomIn),
            Item(ZoomOut),
            Sep,
            Item(PhotoInspection),
            Sep,
            Item(Scopes),
        ],
    ),
    ("Window", &[Item(Minimize), Item(ZoomWindow)]),
    ("Help", &[Item(Help)]),
];

/// Windows keeps the menu bar in the window and has no application menu, so the app-level
/// entries live in File where Windows users expect them.
pub const WINDOWS_MENUS: &[(&str, &[Entry])] = &[
    (
        "File",
        &[
            Item(New),
            Item(Open),
            Item(AddScans),
            Sep,
            Item(Close),
            Sep,
            Item(Save),
            Item(SaveAs),
            Sep,
            Item(Export),
            Sep,
            Item(Settings),
            Sep,
            Item(Quit),
        ],
    ),
    (
        "Edit",
        &[
            Item(Undo),
            Item(Redo),
            Sep,
            Item(Cut),
            Item(Copy),
            Item(Paste),
            Sep,
            Item(SelectAll),
        ],
    ),
    (
        "View",
        &[
            Item(Fit),
            Item(ActualSize),
            Item(ZoomIn),
            Item(ZoomOut),
            Sep,
            Item(PhotoInspection),
            Sep,
            Item(Scopes),
        ],
    ),
    ("Window", &[Item(Minimize), Item(ZoomWindow)]),
    ("Help", &[Item(Help), Sep, Item(About)]),
];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub enabled: BTreeSet<Action>,
    pub undo_label: Option<String>,
    pub redo_label: Option<String>,
    pub scopes_visible: bool,
    pub photo_inspection: bool,
}

impl State {
    pub fn label(&self, action: Action) -> &str {
        match action {
            Undo => self.undo_label.as_deref(),
            Redo => self.redo_label.as_deref(),
            _ => None,
        }
        .unwrap_or_else(|| action.label())
    }

    pub fn enabled(&self, action: Action) -> bool {
        self.enabled.contains(&action)
    }

    pub fn checked(&self, action: Action) -> Option<bool> {
        match action {
            Scopes => Some(self.scopes_visible),
            PhotoInspection => Some(self.photo_inspection),
            _ => None,
        }
    }
}

/// Route Edit to native responders while a synchronous file panel owns input.
pub fn with_native_dialog<T>(show: impl FnOnce() -> T) -> T {
    #[cfg(target_os = "macos")]
    let _termination = super::termination::native_modal();
    #[cfg(target_os = "macos")]
    let _editing = native::NativeDialogEditing::new();
    show()
}

#[cfg(target_os = "macos")]
#[path = "menu_macos.rs"]
pub(crate) mod native;

#[cfg(windows)]
#[path = "menu_windows.rs"]
pub(crate) mod native;
