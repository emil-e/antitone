use super::*;
use std::any::Any;

impl DocumentScope {
    fn dust_fixture(&self) -> Option<&DustTool> {
        let tool = self.visit.hosted_tool.installed()?;
        (tool as &dyn Any).downcast_ref::<DustTool>()
    }

    pub(in crate::editor) fn clear_dust(&mut self) {
        if self.visit.hosted_tool.is::<DustTool>() {
            self.visit.hosted_tool = tool::Slot::Empty;
        }
    }

    pub(in crate::editor) fn dust_mask_draft(&self) -> Option<Ref<'_, MaskDraft>> {
        Ref::filter_map(self.dust_fixture()?.interaction.edit.borrow(), |edit| {
            edit.mask_draft.as_ref()
        })
        .ok()
    }

    pub(in crate::editor) fn dust_mask_draft_mut(&mut self) -> Option<RefMut<'_, MaskDraft>> {
        RefMut::filter_map(self.dust_fixture()?.interaction.edit.borrow_mut(), |edit| {
            edit.mask_draft.as_mut()
        })
        .ok()
    }

    pub(in crate::editor) fn dust_donor_draft(&self) -> Option<DonorDraft> {
        self.dust_fixture()?
            .interaction
            .edit
            .borrow()
            .donor_draft
            .clone()
    }
}

pub(super) fn searches(editor: &Editor) -> &RefCell<SearchState> {
    &editor
        .document
        .dust_fixture()
        .expect("Dust fixture installed")
        .interaction
        .searches
}

impl Editor {
    fn with_dust_fixture<R>(&mut self, body: impl FnOnce(&mut DustTool, &mut Editor) -> R) -> R {
        assert!(self.dusting(), "Dust fixture installed");
        self.with_current_tool(|tool, editor| {
            let tool = (tool as &mut dyn Any)
                .downcast_mut::<DustTool>()
                .expect("Dust fixture kind");
            body(tool, editor)
        })
        .expect("Dust fixture is not already executing")
    }

    pub(in crate::editor) fn set_dust_outlines(&mut self, value: bool) {
        self.with_dust_fixture(|tool, _| tool.interaction.edit.borrow_mut().show_masks = value);
    }

    pub(in crate::editor) fn finish_dust_cleanup(&mut self) {
        if self.dusting() {
            self.end_tool();
        }
    }

    pub(in crate::editor) fn begin_dust_mask(&mut self, target: Option<RepairId>) {
        if !self.dusting() {
            self.begin_dust_cleanup();
        }
        self.with_dust_fixture(|tool, editor| tool.begin_dust_mask(editor, target));
    }

    pub(in crate::editor) fn cancel_dust_mask(&mut self) {
        self.with_dust_fixture(|tool, _| tool.cancel_dust_mask());
    }

    pub(in crate::editor) fn commit_dust_mask(&mut self, cx: &UiContext<'_, Self>) {
        self.with_dust_fixture(|tool, editor| tool.commit_dust_mask(editor, cx));
    }

    pub(in crate::editor) fn begin_dust_stroke(&mut self) {
        self.with_dust_fixture(|tool, _| tool.begin_dust_stroke());
    }

    pub(in crate::editor) fn paint_dust_stroke(
        &mut self,
        layout: crate::viewport::CanvasLayout,
        point: egui::Pos2,
        erase: bool,
    ) {
        self.with_dust_fixture(|tool, editor| tool.paint_dust_stroke(editor, layout, point, erase));
    }

    pub(in crate::editor) fn select_dust_repair(&mut self, id: RepairId) {
        if self.dusting() {
            self.with_dust_fixture(|tool, editor| tool.select_dust_repair(editor, id));
        } else {
            let source = self
                .document
                .visit
                .current_source
                .expect("selected repair has a source");
            assert!(
                self.dust_repairs(source)
                    .is_some_and(|set| set.repairs().iter().any(|repair| repair.id == id))
            );
            self.document.dust.selected.insert(source, id);
        }
    }

    pub(in crate::editor) fn delete_selected_dust_repair(&mut self) {
        self.with_dust_fixture(|tool, editor| tool.delete_selected_dust_repair(editor));
    }

    pub(in crate::editor) fn dust_repair_at(
        &self,
        layout: crate::viewport::CanvasLayout,
        point: egui::Pos2,
    ) -> Option<RepairId> {
        self.document
            .dust_fixture()
            .expect("Dust fixture installed")
            .dust_repair_at(self, layout, point)
    }

    pub(in crate::editor) fn dust_canvas_cursor(
        &self,
        layout: crate::viewport::CanvasLayout,
        point: Option<egui::Pos2>,
        modifying: bool,
    ) -> egui::CursorIcon {
        self.document
            .dust_fixture()
            .expect("Dust fixture installed")
            .dust_canvas_cursor(self, layout, point, modifying)
    }

    pub(in crate::editor) fn dust_donor_regions(&self) -> Vec<DonorRegion> {
        self.document
            .dust_fixture()
            .expect("Dust fixture installed")
            .dust_donor_regions(self)
    }

    pub(in crate::editor) fn dust_donor_hit(
        &self,
        layout: crate::viewport::CanvasLayout,
        point: egui::Pos2,
    ) -> Option<DonorRegion> {
        self.document
            .dust_fixture()
            .expect("Dust fixture installed")
            .dust_donor_hit(self, layout, point)
    }

    pub(in crate::editor) fn dust_donor_at(
        &self,
        layout: crate::viewport::CanvasLayout,
        point: egui::Pos2,
    ) -> bool {
        self.document
            .dust_fixture()
            .expect("Dust fixture installed")
            .dust_donor_at(self, layout, point)
    }

    pub(in crate::editor) fn dust_donor_dragging(&self) -> bool {
        self.document
            .dust_fixture()
            .expect("Dust fixture installed")
            .dust_donor_dragging()
    }

    pub(in crate::editor) fn begin_dust_donor_drag(
        &mut self,
        layout: crate::viewport::CanvasLayout,
        point: egui::Pos2,
    ) {
        self.with_dust_fixture(|tool, editor| tool.begin_dust_donor_drag(editor, layout, point));
    }

    pub(in crate::editor) fn drag_dust_donor(
        &mut self,
        layout: crate::viewport::CanvasLayout,
        point: egui::Pos2,
    ) {
        self.with_dust_fixture(|tool, editor| tool.drag_dust_donor(editor, layout, point));
    }

    pub(in crate::editor) fn end_dust_donor_drag(&mut self) {
        self.with_dust_fixture(|tool, editor| tool.end_dust_donor_drag(editor));
    }
}
