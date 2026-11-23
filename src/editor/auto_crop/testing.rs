use super::*;

impl Editor {
    pub(in crate::editor) fn begin_auto_crop_review(
        &mut self,
        photos: Vec<SourceId>,
        scope: &'static str,
    ) {
        if !self.prepare_tool() {
            return;
        }
        if let Some(review) = capture_review(self, photos, scope) {
            AutoCropTool::new(Some(review)).install(self);
        }
    }

    pub(in crate::editor) fn finish_auto_crop(&mut self, apply: bool) {
        if self
            .current_tool()
            .is_some_and(|tool| tool.is::<AutoCropTool>())
        {
            self.hosted_action(if apply {
                tool::Action::Primary
            } else {
                tool::Action::Dismiss
            });
        }
    }

    pub(in crate::editor) fn auto_crop_preview(&self, id: SourceId) -> Option<Option<Crop>> {
        let state = self.document.auto_crop.current()?;
        let state = state.borrow();
        let draft = state.as_ref()?;
        let inputs = review_inputs(self);
        if !draft.belongs_to(&inputs) {
            return None;
        }
        draft
            .entries
            .iter()
            .find(|entry| entry.source == id)
            .filter(|entry| entry.current(&inputs))
            .map(|entry| entry.draft)
    }

    pub(in crate::editor) fn auto_crop_problem(&self, id: SourceId) -> Option<String> {
        let state = self.document.auto_crop.current()?;
        state
            .borrow()
            .as_ref()?
            .problem(&review_inputs(self), id, !self.interaction_blocked())
    }

    pub(in crate::editor) fn auto_crop_valid(&self) -> bool {
        self.document.auto_crop.current().is_some_and(|state| {
            state
                .borrow()
                .as_ref()
                .is_some_and(|draft| draft.valid(&review_inputs(self)))
        })
    }

    pub(in crate::editor) fn adjust_auto_crop_margin(&mut self, margin: f64) {
        if let Some(state) = self.document.auto_crop.current()
            && let Some(draft) = state.borrow_mut().as_mut()
        {
            draft.adjust_margin(self, margin);
        }
    }

    pub(in crate::editor) fn auto_crop_context(&self) -> bool {
        self.document.auto_crop.current().is_some_and(|state| {
            state
                .borrow()
                .as_ref()
                .is_some_and(|draft| draft.belongs_to(&review_inputs(self)))
        })
    }

    pub(in crate::editor) fn auto_crop_outline(
        &self,
        layout: CanvasLayout,
    ) -> Option<[egui::Pos2; 4]> {
        let state = self.document.auto_crop.current()?;
        let state = state.borrow();
        state.as_ref()?.outline(&review_inputs(self), layout)
    }
}
