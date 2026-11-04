use super::*;

#[cfg(test)]
mod tests;

/// Contextual handles and the unfinished gesture belong to the photo visit.
#[derive(Default)]
pub(in crate::editor) struct MaskEditor {
    gesture: Option<MaskEdit>,
    // Creation is a separate hosted tool; this observation does not retain it.
    pub(super) creation: Weak<RefCell<MaskEdit>>,
}

impl MaskEditor {
    pub(in crate::editor) fn gesture(&self) -> Option<&MaskEdit> {
        self.gesture.as_ref()
    }

    #[cfg(test)]
    pub(in crate::editor) fn gesture_mut(&mut self) -> Option<&mut MaskEdit> {
        self.gesture.as_mut()
    }

    pub(in crate::editor) fn editing(&self) -> bool {
        self.gesture.is_some()
    }

    pub(in crate::editor) fn dragging(&self) -> bool {
        self.gesture.as_ref().is_some_and(MaskEdit::dragging)
    }

    pub(in crate::editor) fn cancel(&mut self) {
        self.gesture = None;
    }

    pub(super) fn begin(&mut self, edit: MaskEdit) {
        self.gesture = Some(edit);
    }

    pub(super) fn finish(&mut self) -> Option<MaskEdit> {
        self.gesture.take()
    }

    pub(super) fn appearance(&self, selected: Option<AdjustmentLayer>) -> Option<AdjustmentLayer> {
        let Some(edit) = &self.gesture else {
            return selected;
        };
        let id = edit.id?;
        let mut adjustment = edit
            .source
            .adjustment_layers
            .as_ref()?
            .layers
            .iter()
            .find(|record| record.id == id)?
            .adjustment;
        adjustment.mask = edit.mask?;
        Some(adjustment)
    }

    pub(super) fn pointer_interest(
        &self,
        selected: Option<AdjustmentLayer>,
        layout: CanvasLayout,
        point: egui::Pos2,
    ) -> bool {
        self.editing()
            || self
                .appearance(selected)
                .is_some_and(|layer| hits(layer.mask, layout, point))
    }

    pub(super) fn begin_drag(&mut self, layout: CanvasLayout, origin: egui::Pos2) {
        if let Some(edit) = &mut self.gesture {
            edit.begin_drag(layout, origin);
        }
    }

    pub(super) fn update(&mut self, point: egui::Pos2) -> Option<Result<Source, Error>> {
        let edit = self.gesture.as_mut()?;
        edit.mask = Some(edit.drag.as_ref()?.update(point));
        if let Some(AdjustmentLayerCoverage::Radial { feather, .. }) = edit.mask {
            edit.feather = feather;
        }
        Some(edit.candidate().map(|(source, _)| source))
    }

    pub(super) fn preview(&mut self, candidate: Result<Source, Error>) {
        if let Some(edit) = &mut self.gesture {
            match candidate {
                Ok(source) => {
                    edit.error = None;
                    edit.preview = Some(source);
                }
                Err(error) => {
                    edit.error = Some(error.to_string());
                    edit.preview = None;
                }
            }
        }
    }

    pub(super) fn paint(
        &self,
        ui: &egui::Ui,
        layout: CanvasLayout,
        selected: Option<AdjustmentLayer>,
        coverage: bool,
    ) {
        if let Some(layer) = self.appearance(selected) {
            paint_geometry(
                ui,
                layout,
                layer.mask,
                layer.invert,
                coverage,
                self.dragging(),
            );
        }
    }
}
