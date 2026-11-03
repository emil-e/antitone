use super::*;
use std::{
    any::Any,
    cell::{Ref, RefMut},
};

impl Editor {
    pub(in crate::editor) fn begin_flat_edit(
        &mut self,
        path: PathBuf,
        region: Option<Rect>,
        cx: &UiContext<'_, Self>,
    ) {
        FlatTool::open(self, path, region, cx);
    }
    pub(in crate::editor) fn reference_preview_input(&self, path: PathBuf) -> source::Request {
        FlatTool::preview_input(self, path)
    }
    #[cfg(test)]
    pub(in crate::editor) fn finish_flat_edit(&mut self) {
        if self
            .current_tool()
            .is_some_and(|tool| tool.is::<FlatTool>())
        {
            self.end_tool();
        }
    }

    #[cfg(test)]
    pub(in crate::editor) fn apply_flat_edit(&mut self, cx: &UiContext<'_, Self>) {
        if self
            .current_tool()
            .is_some_and(|tool| tool.is::<FlatTool>())
        {
            self.dispatch_tool(tool::Action::Primary, cx);
        }
    }
}

impl DocumentScope {
    pub(in crate::editor) fn flat_edit(&self) -> Option<Ref<'_, FlatEdit>> {
        let tool = self.hosted_tool.installed()?;
        let tool = (tool as &dyn Any).downcast_ref::<FlatTool>()?;
        Some(tool.state.borrow())
    }

    pub(in crate::editor) fn flat_edit_mut(&mut self) -> Option<RefMut<'_, FlatEdit>> {
        let tool = self.hosted_tool.installed()?;
        let tool = (tool as &dyn Any).downcast_ref::<FlatTool>()?;
        Some(tool.state.borrow_mut())
    }

    pub(in crate::editor) fn set_flat_edit(&mut self, state: Option<FlatEdit>) {
        if let Some(state) = state {
            self.set_hosted_tool(tool::Placement::Document, Box::new(FlatTool::new(state)));
        } else if self.flat_edit().is_some() {
            self.hosted_tool = Default::default();
        }
    }
}
