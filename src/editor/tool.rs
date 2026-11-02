use super::*;
use crate::viewport::CanvasLayout;
use std::any::{Any, TypeId};

#[cfg(test)]
mod tests;

#[derive(Clone, Copy)]
pub(super) enum Placement {
    Document,
    Photo,
}
#[derive(Clone, Copy)]
pub(super) enum Action {
    Primary,
    Dismiss,
}

#[derive(Clone)]
pub(super) struct Presentation {
    pub show_viewer: bool,
    pub identity: presentation::ToolIdentity<'static>,
    pub primary: Option<&'static str>,
    pub dismiss: &'static str,
    pub warning: Option<String>,
    pub help: &'static str,
    pub navigation_help: Option<&'static str>,
    pub fit_label: &'static str,
}

pub(super) trait Tool: Any {
    fn presentation(&self, editor: &Editor) -> Presentation;
    fn composition(&mut self, normal: Composition) -> Composition {
        normal
    }
    fn viewport<'a>(&'a mut self, editor: &'a mut Editor) -> &'a mut Viewport {
        &mut editor.active_canvas_mut().viewport
    }
    fn drawer(&mut self, _ui: &mut egui::Ui, _editor: &mut Editor, _cx: &UiContext<'_, Editor>) {}
    fn canvas(
        &mut self,
        canvas: &mut canvas::Canvas<'_>,
        editor: &mut Editor,
        cx: &UiContext<'_, Editor>,
    ) -> CanvasLayout {
        let composition = self.composition(canvas.composition);
        let rect = canvas.response.rect;
        let ppp = canvas.ui.ctx().pixels_per_point();
        let relayout =
            |viewport: &mut Viewport| viewport.layout_composition(rect, composition, ppp);
        let viewport = self.viewport(editor);
        let layout = canvas.layout(viewport, relayout);
        let (layout, _) =
            navigation::navigate_view(viewport, canvas.ui, canvas.response, layout, relayout);
        canvas.paint_image(editor, layout, false, cx);
        layout
    }
    fn shortcuts(&mut self, _ui: &mut egui::Ui, _editor: &mut Editor) {}
    fn action(&mut self, action: Action, editor: &mut Editor);
    // Compatibility with the existing foreground activity owner; the operation
    // remains responsible for its task and cancellation semantics.
    fn activity(&self, _editor: &Editor) -> Option<TaskRequest> {
        None
    }
}

#[derive(Clone)]
pub(super) struct Info {
    kind: TypeId,
    presentation: Presentation,
    pub activity: Option<TaskRequest>,
}
impl Info {
    pub fn presentation(&self) -> Presentation {
        self.presentation.clone()
    }
    pub fn is<T: Tool>(&self) -> bool {
        self.kind == TypeId::of::<T>()
    }
}

#[derive(Default)]
pub(super) enum Slot {
    #[default]
    Empty,
    Installed(Box<dyn Tool>),
    Executing(Info),
}
impl Slot {
    fn occupied(&self) -> bool {
        !matches!(self, Self::Empty)
    }
    pub(super) fn activity(&self, editor: &Editor) -> Option<TaskRequest> {
        match self {
            Self::Empty => None,
            Self::Executing(info) => info.activity,
            Self::Installed(tool) => tool.activity(editor),
        }
    }
    #[cfg(test)]
    pub(super) fn installed(&self) -> Option<&dyn Tool> {
        match self {
            Self::Installed(tool) => Some(tool.as_ref()),
            _ => None,
        }
    }
    #[cfg(test)]
    pub(super) fn installed_mut(&mut self) -> Option<&mut dyn Tool> {
        match self {
            Self::Installed(tool) => Some(tool.as_mut()),
            _ => None,
        }
    }
    fn info(&self, editor: &Editor) -> Option<Info> {
        match self {
            Self::Empty => None,
            Self::Executing(info) => Some(info.clone()),
            Self::Installed(tool) => Some(Info {
                kind: tool.as_ref().type_id(),
                presentation: tool.presentation(editor),
                activity: tool.activity(editor),
            }),
        }
    }
    pub(super) fn is<T: Tool>(&self) -> bool {
        match self {
            Self::Empty => false,
            Self::Executing(info) => info.is::<T>(),
            Self::Installed(tool) => tool.as_ref().type_id() == TypeId::of::<T>(),
        }
    }
}

/// Legacy state can be inspected or retired, but installation shares the host's
/// exclusive activation boundary rather than exposing a mutable Option.
#[derive(Default)]
pub(super) struct LegacySlot(Option<legacy_tool::Tool>);
impl std::ops::Deref for LegacySlot {
    type Target = Option<legacy_tool::Tool>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl LegacySlot {
    pub fn as_mut(&mut self) -> Option<&mut legacy_tool::Tool> {
        self.0.as_mut()
    }
    pub fn take(&mut self) -> Option<legacy_tool::Tool> {
        self.0.take()
    }
    pub fn clear(&mut self) {
        self.0 = None;
    }
}

impl DocumentScope {
    fn hosted_slot_mut(&mut self, placement: Placement) -> &mut Slot {
        match placement {
            Placement::Document => &mut self.hosted_tool,
            Placement::Photo => &mut self.visit.hosted_tool,
        }
    }
    fn clear_hosted_tools(&mut self) {
        self.hosted_tool = Slot::Empty;
        self.visit.hosted_tool = Slot::Empty;
    }
    pub(super) fn set_hosted_tool(&mut self, placement: Placement, tool: Box<dyn Tool>) {
        self.clear_hosted_tools();
        self.tool.clear();
        *self.hosted_slot_mut(placement) = Slot::Installed(tool);
    }
    pub(super) fn set_legacy_tool(&mut self, tool: legacy_tool::Tool) {
        self.clear_hosted_tools();
        self.tool.0 = Some(tool);
    }
    pub(super) fn clear_crop_edit(&mut self) {
        if self.visit.hosted_tool.is::<crop_editor::CropTool>() {
            self.visit.hosted_tool = Slot::Empty;
        }
    }
}

impl Editor {
    pub(super) fn tool_start_available(&self) -> bool {
        (!self.busy || self.tool_activity().is_some())
            && (self.document_pending.is_none()
                || matches!(
                    self.document.tool.as_ref(),
                    Some(legacy_tool::Tool::Flat(_))
                ))
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
        if self.has_hosted_tool() {
            self.retire_hosted_tool();
            return;
        }
        let activity = self.tool_activity();
        match self.document.tool.as_ref() {
            Some(legacy_tool::Tool::Mask(_)) => self.finish_mask(false),
            Some(legacy_tool::Tool::Pick(_)) => {
                self.cancel_picker();
                self.document.tool.clear();
            }
            Some(legacy_tool::Tool::Analyze(_)) => self.close_analysis(),
            Some(legacy_tool::Tool::AutoCrop(_)) => self.finish_auto_crop(false),
            Some(legacy_tool::Tool::Flat(_)) => self.finish_flat_edit(),
            None => {}
        }
        if let Some(activity) = activity
            && self.tasks.finish(activity)
        {
            self.busy = self.document_pending.is_some();
        }
    }

    pub(super) fn current_tool(&self) -> Option<Info> {
        self.document
            .hosted_tool
            .info(self)
            .or_else(|| self.document.visit.hosted_tool.info(self))
    }
    pub(super) fn has_hosted_tool(&self) -> bool {
        self.document.hosted_tool.occupied() || self.document.visit.hosted_tool.occupied()
    }
    pub(super) fn hosted_tool_activity(&self) -> Option<TaskRequest> {
        self.document
            .hosted_tool
            .activity(self)
            .or_else(|| self.document.visit.hosted_tool.activity(self))
    }
    pub(super) fn cropping(&self) -> bool {
        self.document.hosted_tool.is::<crop_editor::CropTool>()
            || self
                .document
                .visit
                .hosted_tool
                .is::<crop_editor::CropTool>()
    }
    pub(super) fn retire_hosted_tool(&mut self) {
        let activity = self.hosted_tool_activity();
        self.document.clear_hosted_tools();
        self.document.visit.canvas.last_layout = None;
        if let Some(activity) = activity
            && self.tasks.finish(activity)
        {
            self.busy = self.document_pending.is_some();
        }
    }
    pub(super) fn end_tool(&mut self) {
        self.cancel_active_tool();
    }

    pub(super) fn activate_tool(&mut self, placement: Placement, mut tool: Box<dyn Tool>) -> bool {
        if !self.prepare_tool() {
            return false;
        }
        if let Some(ready) = &self.document.visit.canvas.ready {
            tool.composition(self.normal_canvas_composition(ready.area, self.active_pick()));
        }
        self.document.set_hosted_tool(placement, tool);
        true
    }

    pub(super) fn with_tool_viewport<R>(&mut self, body: impl FnOnce(&mut Viewport) -> R) -> R {
        if self.fullscreen.is_some() || self.current_tool().is_none() {
            return body(&mut self.active_canvas_mut().viewport);
        }
        self.with_current_tool(|tool, editor| body(tool.viewport(editor)))
            .expect("viewport access cannot recursively invoke the executing tool")
    }

    pub(super) fn with_current_tool<R>(
        &mut self,
        body: impl FnOnce(&mut dyn Tool, &mut Editor) -> R,
    ) -> Option<R> {
        let placement = if self.document.hosted_tool.occupied() {
            Placement::Document
        } else if self.document.visit.hosted_tool.occupied() {
            Placement::Photo
        } else {
            return None;
        };
        let info = self.current_tool()?;
        let slot = self.document.hosted_slot_mut(placement);
        if !matches!(slot, Slot::Installed(_)) {
            return None;
        }
        let Slot::Installed(mut tool) = std::mem::replace(slot, Slot::Executing(info)) else {
            unreachable!()
        };
        let result = body(tool.as_mut(), self);
        let slot = self.document.hosted_slot_mut(placement);
        if matches!(slot, Slot::Executing(_)) {
            *slot = Slot::Installed(tool);
        }
        Some(result)
    }
    pub(super) fn hosted_action(&mut self, action: Action) {
        self.with_current_tool(|tool, editor| tool.action(action, editor));
    }
    pub(super) fn hosted_drawer(&mut self, ui: &mut egui::Ui, cx: &UiContext<'_, Self>) {
        self.with_current_tool(|tool, editor| tool.drawer(ui, editor, cx));
    }
}
