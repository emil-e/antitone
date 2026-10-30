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

/// One unfinished interaction; accepted data and view resources stay in their scopes.
pub(super) enum Tool {
    Crop(Box<CropEdit>),
    Mask(Box<MaskEdit>),
    Pick(Vec<async_executor::Task<()>>),
    Analyze(Box<Analyze>),
    AutoCrop(Box<AutoCrop>),
    Flat(Box<FlatEdit>),
    Dust(Box<dust::Cleanup>),
}

impl Editor {
    pub(super) fn tool_activity(&self) -> Option<TaskRequest> {
        match &self.document.tool {
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
            Some(Tool::Crop(edit)) if edit.auto_crop_task.is_some() => self
                .document
                .spatial_preparing
                .as_ref()
                .map(|work| TaskRequest::Regions(work.id)),
            Some(Tool::Flat(edit)) => edit.activity,
            Some(Tool::Pick(_)) => self.document.visit.picking.activity(),
            _ => None,
        }
    }

    pub(super) fn tool_interacting(&self) -> bool {
        match &self.document.tool {
            None => false,
            Some(Tool::Pick(actions)) => {
                self.active_pick().is_some() || actions.iter().any(|task| !task.is_finished())
            }
            _ => true,
        }
    }

    pub(super) fn tool_start_available(&self) -> bool {
        (!self.busy || self.tool_activity().is_some())
            && (self.document_pending.is_none()
                || matches!(self.document.tool, Some(Tool::Flat(_))))
            && !self.export_modal()
            && self.document.removal_pending.is_none()
            && self.transition.is_none()
            && self.menus.panel.is_none()
    }

    pub(super) fn prepare_tool(&mut self) -> bool {
        if !self.tool_start_available() {
            return false;
        }
        self.cancel_active_tool();
        self.finish_adjustment_layer_interaction();
        true
    }

    pub(super) fn cancel_active_tool(&mut self) {
        let activity = self.tool_activity();
        match &self.document.tool {
            Some(Tool::Crop(_)) => self.finish_crop(false),
            Some(Tool::Mask(_)) => self.finish_mask(false),
            Some(Tool::Pick(_)) => {
                self.cancel_picker();
                self.document.tool = None;
            }
            Some(Tool::Analyze(_)) => self.close_analysis(),
            Some(Tool::AutoCrop(_)) => self.finish_auto_crop(false),
            Some(Tool::Flat(_)) => self.finish_flat_edit(),
            Some(Tool::Dust(_)) => self.finish_dust_cleanup(),
            None => {}
        }
        if let Some(activity) = activity
            && self.tasks.finish(activity)
        {
            self.busy = self.document_pending.is_some();
        }
    }
}

impl DocumentScope {
    pub fn crop_edit(&self) -> Option<&CropEdit> {
        match &self.tool {
            Some(Tool::Crop(value)) => Some(value),
            _ => None,
        }
    }

    pub fn crop_edit_mut(&mut self) -> Option<&mut CropEdit> {
        match &mut self.tool {
            Some(Tool::Crop(value)) => Some(value),
            _ => None,
        }
    }

    pub fn set_crop_edit(&mut self, value: Option<CropEdit>) {
        if let Some(value) = value {
            self.tool = Some(Tool::Crop(Box::new(value)));
        } else {
            self.take_crop_edit();
        }
    }

    pub fn take_crop_edit(&mut self) -> Option<CropEdit> {
        match self.tool.take() {
            Some(Tool::Crop(value)) => Some(*value),
            other => {
                self.tool = other;
                None
            }
        }
    }

    pub fn mask_edit(&self) -> Option<&MaskEdit> {
        match &self.tool {
            Some(Tool::Mask(value)) => Some(value),
            _ => None,
        }
    }

    pub fn mask_edit_mut(&mut self) -> Option<&mut MaskEdit> {
        match &mut self.tool {
            Some(Tool::Mask(value)) => Some(value),
            _ => None,
        }
    }

    pub fn set_mask_edit(&mut self, value: Option<MaskEdit>) {
        if let Some(value) = value {
            self.tool = Some(Tool::Mask(Box::new(value)));
        } else {
            self.take_mask_edit();
        }
    }

    pub fn take_mask_edit(&mut self) -> Option<MaskEdit> {
        match self.tool.take() {
            Some(Tool::Mask(value)) => Some(*value),
            other => {
                self.tool = other;
                None
            }
        }
    }

    pub fn flat_edit(&self) -> Option<&FlatEdit> {
        match &self.tool {
            Some(Tool::Flat(value)) => Some(value),
            _ => None,
        }
    }

    pub fn flat_edit_mut(&mut self) -> Option<&mut FlatEdit> {
        match &mut self.tool {
            Some(Tool::Flat(value)) => Some(value),
            _ => None,
        }
    }

    pub fn set_flat_edit(&mut self, value: Option<FlatEdit>) {
        if let Some(value) = value {
            self.tool = Some(Tool::Flat(Box::new(value)));
        } else {
            self.take_flat_edit();
        }
    }

    pub fn take_flat_edit(&mut self) -> Option<FlatEdit> {
        match self.tool.take() {
            Some(Tool::Flat(value)) => Some(*value),
            other => {
                self.tool = other;
                None
            }
        }
    }

    pub fn is_photo_bound(&self) -> bool {
        matches!(
            self.tool,
            Some(Tool::Crop(_)) | Some(Tool::Mask(_)) | Some(Tool::Pick(_)) | Some(Tool::Dust(_))
        )
    }

    pub fn analysis_operation(&self) -> Option<&analysis_workflow::Operation> {
        match &self.tool {
            Some(Tool::Analyze(tool)) => Some(&tool.operation),
            _ => None,
        }
    }

    pub fn analysis_operation_mut(&mut self) -> Option<&mut analysis_workflow::Operation> {
        match &mut self.tool {
            Some(Tool::Analyze(tool)) => Some(&mut tool.operation),
            _ => None,
        }
    }

    pub fn set_analysis_operation(&mut self, operation: Option<analysis_workflow::Operation>) {
        match operation {
            Some(operation) => {
                self.tool = Some(Tool::Analyze(Box::new(Analyze {
                    operation,
                    review: None,
                })));
            }
            None if matches!(self.tool, Some(Tool::Analyze(_))) => self.tool = None,
            None => {}
        }
    }

    pub fn analysis_review(&self) -> Option<&analysis_review::Review> {
        match &self.tool {
            Some(Tool::Analyze(tool)) => tool.review.as_ref(),
            _ => None,
        }
    }

    pub fn analysis_review_mut(&mut self) -> Option<&mut analysis_review::Review> {
        match &mut self.tool {
            Some(Tool::Analyze(tool)) => tool.review.as_mut(),
            _ => None,
        }
    }

    pub fn set_analysis_review(&mut self, review: Option<analysis_review::Review>) {
        let Some(Tool::Analyze(tool)) = &mut self.tool else {
            panic!("review belongs to the active Analyze action");
        };
        tool.review = review;
    }

    pub fn take_analysis_review(&mut self) -> Option<analysis_review::Review> {
        match &mut self.tool {
            Some(Tool::Analyze(tool)) => tool.review.take(),
            _ => None,
        }
    }

    pub fn auto_crop_operation(&self) -> Option<&auto_crop::Operation> {
        match &self.tool {
            Some(Tool::AutoCrop(tool)) => Some(&tool.operation),
            _ => None,
        }
    }

    pub fn auto_crop_operation_mut(&mut self) -> Option<&mut auto_crop::Operation> {
        match &mut self.tool {
            Some(Tool::AutoCrop(tool)) => Some(&mut tool.operation),
            _ => None,
        }
    }

    pub fn set_auto_crop_operation(&mut self, operation: Option<auto_crop::Operation>) {
        match operation {
            Some(operation) => {
                self.tool = Some(Tool::AutoCrop(Box::new(AutoCrop {
                    operation,
                    draft: None,
                })));
            }
            None if matches!(self.tool, Some(Tool::AutoCrop(_))) => self.tool = None,
            None => {}
        }
    }

    pub fn auto_crop_edit(&self) -> Option<&auto_crop::Draft> {
        match &self.tool {
            Some(Tool::AutoCrop(tool)) => tool.draft.as_ref(),
            _ => None,
        }
    }

    pub fn auto_crop_edit_mut(&mut self) -> Option<&mut auto_crop::Draft> {
        match &mut self.tool {
            Some(Tool::AutoCrop(tool)) => tool.draft.as_mut(),
            _ => None,
        }
    }

    pub fn set_auto_crop_edit(&mut self, draft: Option<auto_crop::Draft>) {
        match &mut self.tool {
            Some(Tool::AutoCrop(tool)) => tool.draft = draft,
            _ if draft.is_some() => {
                self.tool = Some(Tool::AutoCrop(Box::new(AutoCrop {
                    operation: Default::default(),
                    draft,
                })));
            }
            _ => {}
        }
    }

    pub fn take_auto_crop_edit(&mut self) -> Option<auto_crop::Draft> {
        match &mut self.tool {
            Some(Tool::AutoCrop(tool)) => tool.draft.take(),
            _ => None,
        }
    }

    pub fn pick_actions_mut(&mut self) -> &mut Vec<async_executor::Task<()>> {
        let Some(Tool::Pick(actions)) = &mut self.tool else {
            panic!("picker actions belong to the active picker");
        };
        actions
    }

    pub fn begin_dust(&mut self) {
        if !matches!(self.tool, Some(Tool::Dust(_))) {
            self.tool = Some(Tool::Dust(Default::default()));
        }
    }

    pub fn clear_dust(&mut self) {
        if matches!(self.tool, Some(Tool::Dust(_))) {
            self.tool = None;
        }
    }

    pub fn dust_mask_draft(&self) -> Option<&dust::MaskDraft> {
        self.dust()?.mask_draft.as_ref()
    }

    pub fn dust_mask_draft_mut(&mut self) -> Option<&mut dust::MaskDraft> {
        self.dust_mut()?.mask_draft.as_mut()
    }

    pub fn set_dust_mask_draft(&mut self, value: Option<dust::MaskDraft>) {
        if let Some(tool) = self.dust_mut() {
            tool.mask_draft = value;
        }
    }

    pub fn dust_donor_draft(&self) -> Option<&dust::DonorDraft> {
        self.dust()?.donor_draft.as_ref()
    }

    pub fn dust_donor_draft_mut(&mut self) -> Option<&mut dust::DonorDraft> {
        self.dust_mut()?.donor_draft.as_mut()
    }

    pub fn set_dust_donor_draft(&mut self, value: Option<dust::DonorDraft>) {
        if let Some(tool) = self.dust_mut() {
            tool.donor_draft = value;
        }
    }

    pub fn take_dust_donor_draft(&mut self) -> Option<dust::DonorDraft> {
        self.dust_mut()?.donor_draft.take()
    }

    pub fn dust(&self) -> Option<&dust::Cleanup> {
        match &self.tool {
            Some(Tool::Dust(tool)) => Some(tool),
            _ => None,
        }
    }

    pub fn dust_mut(&mut self) -> Option<&mut dust::Cleanup> {
        match &mut self.tool {
            Some(Tool::Dust(tool)) => Some(tool),
            _ => None,
        }
    }
}
