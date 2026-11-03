use super::*;

#[cfg(test)]
mod tests;

pub(super) struct Analyze {
    operation: analysis_workflow::Operation,
    review: Option<analysis_review::Review>,
}

#[derive(Default)]
pub(super) struct AutoCrop {
    operation: auto_crop::Operation,
    draft: Option<auto_crop::Draft>,
}

/// Legacy interactions not yet migrated to the shared tool host.
pub(super) enum Tool {
    Mask(Box<MaskEdit>),
    Analyze(Box<Analyze>),
    AutoCrop(Box<AutoCrop>),
    Flat(Box<FlatEdit>),
}

impl Editor {
    pub(super) fn tool_activity(&self) -> Option<TaskRequest> {
        if self.has_hosted_tool() {
            return self.hosted_tool_activity();
        }
        match self.document.tool.as_ref() {
            Some(Tool::Analyze(action)) => match action.operation.phase {
                analysis_workflow::Phase::Preparing(id) | analysis_workflow::Phase::Fitting(id) => {
                    Some(TaskRequest::Analyze(id))
                }
                analysis_workflow::Phase::Regions => self
                    .document
                    .spatial_preparing
                    .as_ref()
                    .map(|work| TaskRequest::Regions(work.id)),
                analysis_workflow::Phase::Review => None,
            },
            Some(Tool::AutoCrop(action)) if action.draft.is_none() => self
                .document
                .spatial_preparing
                .as_ref()
                .map(|work| TaskRequest::Regions(work.id)),
            Some(Tool::Flat(edit)) => edit.activity,
            _ => None,
        }
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

    pub fn flat_edit(&self) -> Option<&FlatEdit> {
        match self.tool.as_ref() {
            Some(Tool::Flat(value)) => Some(value),
            _ => None,
        }
    }

    pub fn flat_edit_mut(&mut self) -> Option<&mut FlatEdit> {
        match self.tool.as_mut() {
            Some(Tool::Flat(value)) => Some(value),
            _ => None,
        }
    }

    pub fn set_flat_edit(&mut self, value: Option<FlatEdit>) {
        if let Some(value) = value {
            self.set_legacy_tool(Tool::Flat(Box::new(value)));
        } else {
            self.take_flat_edit();
        }
    }

    pub fn take_flat_edit(&mut self) -> Option<FlatEdit> {
        match self.tool.take() {
            Some(Tool::Flat(value)) => Some(*value),
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

    pub fn analysis_operation(&self) -> Option<&analysis_workflow::Operation> {
        match self.tool.as_ref() {
            Some(Tool::Analyze(tool)) => Some(&tool.operation),
            _ => None,
        }
    }

    pub fn analysis_operation_mut(&mut self) -> Option<&mut analysis_workflow::Operation> {
        match self.tool.as_mut() {
            Some(Tool::Analyze(tool)) => Some(&mut tool.operation),
            _ => None,
        }
    }

    pub fn set_analysis_operation(&mut self, operation: Option<analysis_workflow::Operation>) {
        match operation {
            Some(operation) => {
                self.set_legacy_tool(Tool::Analyze(Box::new(Analyze {
                    operation,
                    review: None,
                })));
            }
            None if matches!(self.tool.as_ref(), Some(Tool::Analyze(_))) => self.tool.clear(),
            None => {}
        }
    }

    pub fn analysis_review(&self) -> Option<&analysis_review::Review> {
        match self.tool.as_ref() {
            Some(Tool::Analyze(tool)) => tool.review.as_ref(),
            _ => None,
        }
    }

    pub fn analysis_review_mut(&mut self) -> Option<&mut analysis_review::Review> {
        match self.tool.as_mut() {
            Some(Tool::Analyze(tool)) => tool.review.as_mut(),
            _ => None,
        }
    }

    pub fn set_analysis_review(&mut self, review: Option<analysis_review::Review>) {
        let Some(Tool::Analyze(tool)) = self.tool.as_mut() else {
            panic!("review belongs to the active Analyze action");
        };
        tool.review = review;
    }

    pub fn take_analysis_review(&mut self) -> Option<analysis_review::Review> {
        match self.tool.as_mut() {
            Some(Tool::Analyze(tool)) => tool.review.take(),
            _ => None,
        }
    }

    pub fn auto_crop_operation(&self) -> Option<&auto_crop::Operation> {
        match self.tool.as_ref() {
            Some(Tool::AutoCrop(tool)) => Some(&tool.operation),
            _ => None,
        }
    }

    pub fn auto_crop_operation_mut(&mut self) -> Option<&mut auto_crop::Operation> {
        match self.tool.as_mut() {
            Some(Tool::AutoCrop(tool)) => Some(&mut tool.operation),
            _ => None,
        }
    }

    pub fn set_auto_crop_operation(&mut self, operation: Option<auto_crop::Operation>) {
        match operation {
            Some(operation) => {
                self.set_legacy_tool(Tool::AutoCrop(Box::new(AutoCrop {
                    operation,
                    draft: None,
                })));
            }
            None if matches!(self.tool.as_ref(), Some(Tool::AutoCrop(_))) => self.tool.clear(),
            None => {}
        }
    }

    pub fn auto_crop_edit(&self) -> Option<&auto_crop::Draft> {
        match self.tool.as_ref() {
            Some(Tool::AutoCrop(tool)) => tool.draft.as_ref(),
            _ => None,
        }
    }

    pub fn auto_crop_edit_mut(&mut self) -> Option<&mut auto_crop::Draft> {
        match self.tool.as_mut() {
            Some(Tool::AutoCrop(tool)) => tool.draft.as_mut(),
            _ => None,
        }
    }

    pub fn set_auto_crop_edit(&mut self, draft: Option<auto_crop::Draft>) {
        match self.tool.as_mut() {
            Some(Tool::AutoCrop(tool)) => tool.draft = draft,
            _ if draft.is_some() => {
                self.set_legacy_tool(Tool::AutoCrop(Box::new(AutoCrop {
                    operation: Default::default(),
                    draft,
                })));
            }
            _ => {}
        }
    }

    pub fn take_auto_crop_edit(&mut self) -> Option<auto_crop::Draft> {
        match self.tool.as_mut() {
            Some(Tool::AutoCrop(tool)) => tool.draft.take(),
            _ => None,
        }
    }
}
