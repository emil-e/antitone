use super::*;
use crate::viewport::CanvasLayout;
use futures_channel::oneshot;
use std::{
    cell::RefCell,
    rc::{Rc, Weak},
};

#[cfg(test)]
mod operation_tests;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Display {
    pub label: &'static str,
    pub area: bool,
    pub lock_layer_selection: bool,
    pub help: Option<&'static str>,
}

impl Display {
    pub const fn sample(label: &'static str) -> Self {
        Self {
            label,
            area: false,
            lock_layer_selection: false,
            help: None,
        }
    }
}

type Submit = Box<dyn FnOnce(Coverage, &Editor)>;

struct Interaction {
    display: Display,
    region: Option<Coverage>,
    drag: Option<PickDrag>,
    submit: Option<Submit>,
}

impl Interaction {
    fn new(display: Display, submit: Option<Submit>) -> Self {
        Self {
            display,
            region: None,
            drag: None,
            submit,
        }
    }
}

/// Read-only observation of the measurement picker; the tool owns the interaction.
#[derive(Default)]
pub(super) struct State {
    active: Weak<RefCell<Interaction>>,
}

impl State {
    pub fn display(&self) -> Option<Display> {
        self.active.upgrade().map(|state| state.borrow().display)
    }

    #[cfg(test)]
    pub fn region(&self) -> Option<Coverage> {
        self.active
            .upgrade()
            .and_then(|state| state.borrow().region)
    }

    #[cfg(test)]
    pub fn set_region(&self, region: Option<Coverage>) {
        self.active
            .upgrade()
            .expect("active picker fixture")
            .borrow_mut()
            .region = region;
    }
}

fn install(editor: &mut Editor, state: Rc<RefCell<Interaction>>) {
    let tool = PickTool {
        state: state.clone(),
    };
    editor
        .document
        .set_hosted_tool(tool::Placement::Photo, Box::new(tool));
    editor.document.visit.picker.active = Rc::downgrade(&state);
}

#[cfg(test)]
pub(super) fn preview(editor: &mut Editor, display: Display) {
    if editor.prepare_tool() {
        install(
            editor,
            Rc::new(RefCell::new(Interaction::new(display, None))),
        );
    }
}

impl Editor {
    pub(super) fn picking(&self) -> bool {
        self.document.visit.picker.display().is_some()
    }
}

pub(super) struct PickTool {
    state: Rc<RefCell<Interaction>>,
}

impl PickTool {
    fn select(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        layout: CanvasLayout,
    ) -> Option<Coverage> {
        if response.clicked_by(egui::PointerButton::Primary)
            && let Some(point) = response
                .interact_pointer_pos()
                .filter(|point| layout.image.contains(*point))
        {
            return Some(layout.pick_patch(point, 32));
        }

        let mut state = self.state.borrow_mut();
        if response.drag_started_by(egui::PointerButton::Primary) {
            state.drag = ui
                .input(|input| input.pointer.press_origin())
                .filter(|point| layout.image.contains(*point))
                .map(|point| PickDrag::begin(layout, point, state.region));
        }
        if response.dragged_by(egui::PointerButton::Primary)
            && let (Some(drag), Some(point)) = (state.drag, response.interact_pointer_pos())
        {
            state.region = Some(drag.update(layout, point));
        }
        None
    }

    fn submit(&mut self, region: Coverage, editor: &mut Editor) {
        let submit = self.state.borrow_mut().submit.take();
        if let Some(submit) = submit {
            submit(region, editor);
            editor.end_tool();
        }
    }
}

impl tool::Tool for PickTool {
    fn presentation(&self, editor: &Editor) -> tool::Presentation {
        let state = self.state.borrow();
        tool::Presentation {
            identity: ui::ToolIdentity {
                icon: if state.display.area {
                    ui::icons::Icon::Crop
                } else {
                    ui::icons::Icon::Pick
                },
                title: state.display.label,
            },
            primary: Some(tool::Primary {
                label: "Apply",
                enabled: state.region.is_some() && !editor.interaction_blocked(),
            }),
            dismiss: "Cancel",
            warning: None,
            help: state
                .display
                .help
                .unwrap_or("Click to measure a patch, or drag a rectangle and choose Apply. Drag its edges or corners to resize."),
            navigation_help: None,
            fit_label: "Fit",
        }
    }

    fn canvas(
        &mut self,
        viewer: &mut canvas::Viewer<'_>,
        editor: &mut Editor,
        cx: &UiContext<'_, Editor>,
    ) -> Option<CanvasLayout> {
        let mut patch = None;
        let layout = viewer.show(
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
                let mut response = canvas.response.clone();
                navigation::consume_primary(&mut response);
                let (layout, panning) = {
                    let viewport = tool.viewport(editor);
                    let layout = canvas.layout(viewport, relayout);
                    navigation::navigate_view(viewport, canvas.ui, &response, layout, relayout)
                };
                let space = canvas.ui.input(|input| input.key_down(egui::Key::Space));
                if canvas.ui.is_enabled() && !editor.interaction_blocked() && !panning && !space {
                    patch = tool.select(canvas.ui, canvas.response, layout);
                    if patch.is_none()
                        && (canvas.response.hovered() || canvas.response.dragged())
                        && let Some(point) = canvas.ui.input(|input| input.pointer.hover_pos())
                    {
                        let state = tool.state.borrow();
                        canvas.ui.ctx().set_cursor_icon(navigation::picker_cursor(
                            layout,
                            point,
                            state.region,
                            state.drag,
                            canvas.response.dragged_by(egui::PointerButton::Primary),
                        ));
                    }
                }
                if !canvas
                    .ui
                    .input(|input| input.pointer.button_down(egui::PointerButton::Primary))
                {
                    tool.state.borrow_mut().drag = None;
                }
                canvas.paint_image(editor, layout, false, cx);
                if patch.is_none()
                    && let Some(region) = tool.state.borrow().region
                {
                    paint_selection(canvas.ui, layout, region);
                }
                Some(layout)
            },
        );

        if let Some(patch) = patch {
            self.submit(patch, editor);
            viewer.ui.ctx().request_repaint();
        }
        layout
    }

    fn action(&mut self, action: tool::Action, editor: &mut Editor, _cx: &UiContext<'_, Editor>) {
        match action {
            tool::Action::Primary if !editor.interaction_blocked() => {
                let region = self.state.borrow().region;
                if let Some(region) = region {
                    self.submit(region, editor);
                }
            }
            tool::Action::Dismiss => editor.end_tool(),
            _ => {}
        }
    }
}

pub(super) fn paint_selection(ui: &egui::Ui, layout: CanvasLayout, region: Coverage) {
    let screen = layout.screen_coverage(region);
    let painter = ui.painter().with_clip_rect(layout.canvas);
    painter.rect_stroke(
        screen,
        0.,
        egui::Stroke::new(1., egui::Color32::WHITE),
        egui::StrokeKind::Inside,
    );
    for point in [
        screen.min,
        screen.max,
        screen.left_bottom(),
        screen.right_top(),
    ] {
        painter.rect_filled(
            egui::Rect::from_center_size(point, egui::vec2(6., 6.)),
            0.,
            egui::Color32::WHITE,
        );
    }
}

/// The UI host observes this request; its caller owns the future and the lease.
struct InteractionLease<'scope> {
    state: Weak<RefCell<Interaction>>,
    cx: UiContext<'scope, Editor>,
}

impl Drop for InteractionLease<'_> {
    fn drop(&mut self) {
        if self.state.strong_count() == 0 {
            return;
        }
        let state = self.state.clone();
        // A caller may drop its future while Editor is borrowed. Defer only UI cleanup,
        // not feature work, and never remove an interaction installed after this one.
        if let Some(cleanup) = self.cx.spawn(move |cx| async move {
            cx.update(|editor| {
                if state.upgrade().is_some()
                    && Weak::ptr_eq(&state, &editor.document.visit.picker.active)
                {
                    editor.end_tool();
                }
            });
        }) {
            cleanup.detach();
        }
    }
}

#[cfg(test)]
pub(super) async fn pick(cx: &UiContext<'_, Editor>, display: Display) -> Option<Coverage> {
    pick_with(cx, display, |_| ())
        .await
        .map(|(region, ())| region)
}

/// Capture feature inputs on click or Apply; the caller resumes after the tool ends.
pub(super) async fn pick_with<T: 'static>(
    cx: &UiContext<'_, Editor>,
    display: Display,
    capture: impl FnOnce(&Editor) -> T + 'static,
) -> Option<(Coverage, T)> {
    let (submit, selected) = oneshot::channel();
    let send = Box::new(move |region, editor: &Editor| {
        let _ = submit.send((region, capture(editor)));
    });
    let state = Rc::new(RefCell::new(Interaction::new(display, Some(send))));
    let _lease = InteractionLease {
        state: Rc::downgrade(&state),
        cx: cx.clone(),
    };
    let installed = cx.update(|editor| {
        if !editor.tool_start_available() || editor.has_hosted_tool() {
            return false;
        }
        install(editor, state);
        true
    })?;
    if !installed {
        return None;
    }
    selected.await.ok()
}
