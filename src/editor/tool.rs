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

#[derive(Clone, Copy)]
pub(super) struct Primary {
    pub label: &'static str,
    pub enabled: bool,
}

impl From<&'static str> for Primary {
    fn from(label: &'static str) -> Self {
        Self {
            label,
            enabled: true,
        }
    }
}

#[derive(Clone)]
pub(super) struct Presentation {
    pub identity: presentation::ToolIdentity<'static>,
    pub primary: Option<Primary>,
    pub dismiss: &'static str,
    pub warning: Option<String>,
    pub help: &'static str,
    pub navigation_help: Option<&'static str>,
    pub fit_label: &'static str,
}

pub(super) trait Tool: Any {
    fn presentation(&self, editor: &Editor) -> Presentation;
    fn composition(&self, normal: Composition) -> Composition {
        normal
    }
    fn viewport<'a>(&'a mut self, editor: &'a mut Editor) -> &'a mut Viewport {
        &mut editor.active_canvas_mut().viewport
    }
    fn drawer(&mut self, _ui: &mut egui::Ui, _editor: &mut Editor, _cx: &UiContext<'_, Editor>) {}
    fn filmstrip(&mut self, ui: &mut egui::Ui, editor: &mut Editor, cx: &UiContext<'_, Editor>) {
        editor.filmstrip(ui, cx);
    }
    fn canvas(
        &mut self,
        viewer: &mut canvas::Viewer<'_>,
        editor: &mut Editor,
        cx: &UiContext<'_, Editor>,
    ) -> Option<CanvasLayout> {
        viewer.show(
            self,
            editor,
            cx,
            Editor::photograph_controls,
            |tool, surface, editor, cx| {
                let mut canvas = surface.selected()?;
                let composition = tool.composition(canvas.composition);
                let rect = canvas.response.rect;
                let ppp = canvas.ui.ctx().pixels_per_point();
                let relayout =
                    |viewport: &mut Viewport| viewport.layout_composition(rect, composition, ppp);
                let viewport = tool.viewport(editor);
                let layout = canvas.layout(viewport, relayout);
                let (layout, _) = navigation::navigate_view(
                    viewport,
                    canvas.ui,
                    canvas.response,
                    layout,
                    relayout,
                );
                canvas.paint_image(editor, layout, false, cx);
                Some(layout)
            },
        )
    }
    fn shortcuts(&mut self, _ui: &mut egui::Ui, _editor: &mut Editor) {}
    fn action(&mut self, action: Action, editor: &mut Editor, cx: &UiContext<'_, Editor>);
}

#[derive(Clone)]
pub(super) struct Info {
    kind: TypeId,
    presentation: Presentation,
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
    pub(super) fn occupied(&self) -> bool {
        !matches!(self, Self::Empty)
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
        self.visit.masks.cancel();
        *self.hosted_slot_mut(placement) = Slot::Installed(tool);
    }
    pub(super) fn clear_crop_edit(&mut self) {
        if self.visit.hosted_tool.is::<crop_editor::CropTool>() {
            self.visit.hosted_tool = Slot::Empty;
        }
    }
}

impl Editor {
    pub(super) fn tool_interacting(&self) -> bool {
        self.has_hosted_tool()
    }

    pub(super) fn tool_start_available(&self) -> bool {
        !self.interaction_blocked()
            && !self.document.gamma_pivot.pending()
            && self.document_pending.is_none()
            && !self.export_modal()
            && !self.document.confirming()
            && self.transition.is_none()
            && self.menus.panel.is_none()
    }

    pub(super) fn prepare_tool(&mut self) -> bool {
        if !self.tool_start_available() {
            return false;
        }
        self.document.visit.interaction_action = None;
        self.cancel_active_tool();
        self.canvas_frame = Default::default();
        self.finish_adjustment_layer_interaction();
        true
    }

    pub(super) fn cancel_active_tool(&mut self) {
        if self.has_hosted_tool() {
            self.retire_hosted_tool();
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

    pub(super) fn cropping(&self) -> bool {
        self.document.hosted_tool.is::<crop_editor::CropTool>()
            || self
                .document
                .visit
                .hosted_tool
                .is::<crop_editor::CropTool>()
    }
    pub(super) fn retire_hosted_tool(&mut self) {
        self.retire_fullscreen();
        self.document.clear_hosted_tools();
        self.canvas_frame = Default::default();
        self.document.visit.canvas.last_layout = None;
    }
    pub(super) fn end_tool(&mut self) {
        self.cancel_active_tool();
    }

    pub(super) fn activate_tool(&mut self, placement: Placement, tool: Box<dyn Tool>) -> bool {
        if !self.prepare_tool() {
            return false;
        }
        self.document.set_hosted_tool(placement, tool);
        true
    }

    pub(super) fn with_tool_viewport<R>(&mut self, body: impl FnOnce(&mut Viewport) -> R) -> R {
        if self.current_tool().is_none() {
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
    pub(super) fn dispatch_tool(&mut self, action: Action, cx: &UiContext<'_, Self>) {
        self.with_current_tool(|tool, editor| tool.action(action, editor, cx));
    }
    #[cfg(test)]
    pub(super) fn hosted_action(&mut self, action: Action) {
        self.dispatch_tool(action, &UiContext::render_only());
    }

    pub(super) fn hosted_drawer(&mut self, ui: &mut egui::Ui, cx: &UiContext<'_, Self>) {
        self.with_current_tool(|tool, editor| tool.drawer(ui, editor, cx));
    }
}
