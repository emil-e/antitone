use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

#[derive(Default)]
struct State {
    blocks: Cell<usize>,
    context: RefCell<Option<egui::Context>>,
}

/// Shared input availability; observers do not keep a modal scope alive.
#[derive(Clone, Default)]
pub(super) struct WorkspaceInput {
    state: Rc<State>,
}

impl WorkspaceInput {
    pub fn set_context(&self, context: &egui::Context) {
        *self.state.context.borrow_mut() = Some(context.clone());
    }

    pub fn block(&self) -> WorkspaceBlock {
        self.state.blocks.set(self.state.blocks.get() + 1);
        self.repaint();
        WorkspaceBlock {
            input: self.clone(),
        }
    }

    pub fn is_blocked(&self) -> bool {
        self.state.blocks.get() != 0
    }

    fn repaint(&self) {
        if let Some(context) = self.state.context.borrow().as_ref() {
            context.request_repaint();
        }
    }
}

/// Reserves underlying workspace input, not editing authority or physical work.
#[must_use = "retain the block in its modal owner"]
pub(super) struct WorkspaceBlock {
    input: WorkspaceInput,
}

impl WorkspaceBlock {
    /// A modal owner's shell actions remain available unless another owner blocks them.
    pub fn is_covered(&self) -> bool {
        self.input.state.blocks.get() > 1
    }
}

impl Drop for WorkspaceBlock {
    fn drop(&mut self) {
        self.input
            .state
            .blocks
            .set(self.input.state.blocks.get() - 1);
        self.input.repaint();
    }
}

#[cfg(test)]
mod tests;
