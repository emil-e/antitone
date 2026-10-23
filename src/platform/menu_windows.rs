use super::{Action, State};
use muda::MenuId;

/// The Windows native menu is not implemented yet; the editor's in-window menu is used.
pub(crate) const SUPPORTED: bool = false;

pub(crate) struct NativeMenu;

impl NativeMenu {
    pub(crate) fn new(_state: &State) -> Result<Self, muda::Error> {
        Ok(Self)
    }

    pub(crate) fn action(&self, _id: &MenuId) -> Option<Action> {
        None
    }

    pub(crate) fn update(&mut self, _state: &State) {}
}
