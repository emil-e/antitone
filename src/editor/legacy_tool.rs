use super::*;

/// Legacy interactions not yet migrated to the shared tool host.
pub(super) enum Tool {
    Mask(Box<MaskEdit>),
}

impl Editor {
    pub(super) fn tool_activity(&self) -> Option<TaskRequest> {
        if self.has_hosted_tool() {
            return self.hosted_tool_activity();
        }
        None
    }

    pub(super) fn tool_interacting(&self) -> bool {
        if self.has_hosted_tool() {
            return true;
        }
        self.document.tool.is_some()
    }
}

impl DocumentScope {
    pub fn mask_edit(&self) -> Option<&MaskEdit> {
        match self.tool.as_ref() {
            Some(Tool::Mask(value)) => Some(value),
            _ => None,
        }
    }

    pub fn mask_edit_mut(&mut self) -> Option<&mut MaskEdit> {
        match self.tool.as_mut() {
            Some(Tool::Mask(value)) => Some(value),
            _ => None,
        }
    }

    pub fn set_mask_edit(&mut self, value: Option<MaskEdit>) {
        if let Some(value) = value {
            self.set_legacy_tool(Tool::Mask(Box::new(value)));
        } else {
            self.take_mask_edit();
        }
    }

    pub fn take_mask_edit(&mut self) -> Option<MaskEdit> {
        match self.tool.take() {
            Some(Tool::Mask(value)) => Some(*value),
            other => {
                if let Some(other) = other {
                    self.set_legacy_tool(other);
                }
                None
            }
        }
    }

    pub fn is_photo_bound(&self) -> bool {
        matches!(self.tool.as_ref(), Some(Tool::Mask(_)))
    }
}
