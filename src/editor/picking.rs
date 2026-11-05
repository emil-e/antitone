use super::*;
use crate::{cancellation::Cancellation, viewport::CanvasLayout};
use futures_channel::oneshot;
use std::{
    cell::RefCell,
    rc::{Rc, Weak},
};

#[cfg(test)]
mod measurement_tests;
#[cfg(test)]
mod operation_tests;
pub(super) mod query;
#[cfg(test)]
pub(super) mod testing;
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum View {
    Working,
    Source,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Display {
    pub label: &'static str,
    pub view: View,
    pub area: bool,
    pub lock_layer_selection: bool,
    pub help: Option<&'static str>,
}

impl Display {
    pub const fn sample(label: &'static str, view: View) -> Self {
        Self {
            label,
            view,
            area: false,
            lock_layer_selection: false,
            help: None,
        }
    }
}

struct Interaction {
    display: Display,
    region: Option<Coverage>,
    drag: Option<PickDrag>,
    submit: Option<oneshot::Sender<Coverage>>,
    request: Option<u64>,
}

impl Interaction {
    fn activity(&self) -> Option<TaskRequest> {
        self.request
            .filter(|_| self.submit.is_none())
            .map(TaskRequest::Measurement)
    }

    fn submit(&mut self, editor: &mut Editor) {
        if let Some(region) = self.region
            && let Some(send) = self.submit.take()
        {
            let id = self.request.expect("measurement owns its request");
            editor
                .tasks
                .begin(Task::Measurement(id, self.display.label));
            editor.busy = true;
            editor.status = "Measuring full-resolution patch…".into();
            let _ = send.send(region);
        }
    }
}

/// Read-only observation of the measurement picker; the tool owns the interaction.
#[derive(Default)]
pub(super) struct State {
    active: Weak<RefCell<Interaction>>,
    #[cfg(test)]
    pub measurement_gate: Option<testing::Gate>,
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

fn install(
    editor: &mut Editor,
    display: Display,
    previous: Option<(crate::viewport::CanvasLayout, f32)>,
    submit: Option<oneshot::Sender<Coverage>>,
    request: Option<u64>,
    task: Option<async_executor::Task<()>>,
) {
    let state = Rc::new(RefCell::new(Interaction {
        display,
        region: None,
        drag: None,
        submit,
        request,
    }));
    let tool = PickTool::new(editor, state.clone(), previous, task);
    editor.document.thumbnails.cancel();
    editor
        .document
        .set_hosted_tool(tool::Placement::Photo, Box::new(tool));
    editor.document.visit.picker.active = Rc::downgrade(&state);
}

#[cfg(test)]
pub(super) fn preview(editor: &mut Editor, display: Display) {
    let previous = editor.document.visit.canvas.last_layout;
    if editor.prepare_tool() {
        install(editor, display, previous, None, None, None);
    }
}

impl Editor {
    pub(super) fn picking(&self) -> bool {
        self.document.visit.picker.display().is_some()
    }

    #[cfg(test)]
    pub(super) fn measurement_pending(&self) -> bool {
        matches!(
            self.hosted_tool_activity(),
            Some(TaskRequest::Measurement(_))
        )
    }
}

pub(super) struct PickTool {
    state: Rc<RefCell<Interaction>>,
    _task: Option<async_executor::Task<()>>,
    viewport: Viewport,
    orientation: (RollOrientation, PhotoOrientation),
    crop: Option<Crop>,
    initial_view: Option<Viewport>,
}

impl PickTool {
    fn new(
        editor: &Editor,
        state: Rc<RefCell<Interaction>>,
        previous: Option<(CanvasLayout, f32)>,
        task: Option<async_executor::Task<()>>,
    ) -> Self {
        let temporary_view = state.borrow().display.view != View::Working;
        let mut tool = Self {
            state,
            _task: task,
            crop: editor.document.visit.crop,
            initial_view: temporary_view.then_some(editor.document.visit.canvas.viewport),
            viewport: editor.document.visit.canvas.viewport,
            orientation: (
                editor.document.roll_orientation,
                editor.document.visit.photo_orientation,
            ),
        };
        if let Some((previous, ppp)) = previous {
            let area = editor
                .document
                .visit
                .canvas
                .ready
                .as_ref()
                .map_or(previous.composition.area(), |ready| ready.area);
            tool.initialise_view(area, previous.canvas, ppp);
        }
        tool
    }

    fn initialise_view(&mut self, area: Rect, canvas: egui::Rect, ppp: f32) {
        let Some(mut normal) = self.initial_view.take() else {
            return;
        };
        let basis = Composition::with_crop(area, self.orientation.0, self.orientation.1, self.crop)
            .expect("accepted working composition");
        let previous = normal.layout_composition(canvas, basis, ppp);
        let next = tool::Tool::composition(self, basis);
        self.viewport.reframe(previous, next, ppp);
    }

    fn select(&mut self, ui: &egui::Ui, response: &egui::Response, layout: CanvasLayout) {
        let mut state = self.state.borrow_mut();
        if response.clicked_by(egui::PointerButton::Primary)
            && let Some(point) = response
                .interact_pointer_pos()
                .filter(|point| layout.image.contains(*point))
        {
            state.region = Some(layout.pick_patch(point, 32));
        }
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
    }
}

impl tool::Tool for PickTool {
    fn presentation(&self, editor: &Editor) -> tool::Presentation {
        let state = self.state.borrow();
        tool::Presentation {
            identity: presentation::ToolIdentity {
                icon: if state.display.area { icons::Icon::Crop } else { icons::Icon::Pick },
                title: state.display.label,
            },
            primary: Some(tool::Primary { label: "Apply", enabled: state.region.is_some() && !editor.busy }),
            dismiss: "Cancel",
            warning: None,
            help: state.display.help.unwrap_or("Click a patch or drag an area; drag its edges/corners to resize.\nEnter applies · Escape cancels"),
            navigation_help: None,
            fit_label: "Fit",
        }
    }

    fn composition(&self, normal: Composition) -> Composition {
        match self.state.borrow().display.view {
            View::Working => normal,
            View::Source => Composition::new(normal.area(), self.orientation.0, self.orientation.1)
                .expect("selected source composition"),
        }
    }

    fn viewport<'a>(&'a mut self, editor: &'a mut Editor) -> &'a mut Viewport {
        if editor.fullscreen.is_some() || self.state.borrow().display.view == View::Working {
            &mut editor.document.visit.canvas.viewport
        } else {
            &mut self.viewport
        }
    }

    fn canvas(
        &mut self,
        viewer: &mut canvas::Viewer<'_>,
        editor: &mut Editor,
        cx: &UiContext<'_, Editor>,
    ) -> Option<CanvasLayout> {
        let source = self.state.borrow().display.view != View::Working;
        let fullscreen = editor.fullscreen.is_some();
        viewer.show(
            self,
            editor,
            cx,
            move |editor, ui, navigating| {
                if fullscreen {
                    editor.photograph_controls(ui, navigating);
                } else if source {
                    editor.source_controls(ui, navigating);
                }
            },
            |tool, surface, editor, cx| {
                if editor.fullscreen.is_some() {
                    return surface.photograph(editor, cx);
                }
                let mut canvas = surface.selected()?;
                let composition = tool.composition(canvas.composition);
                let rect = canvas.response.rect;
                let ppp = canvas.ui.ctx().pixels_per_point();
                tool.initialise_view(composition.area(), rect, ppp);
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
                if canvas.ui.is_enabled() && !editor.busy && !panning && !space {
                    tool.select(canvas.ui, canvas.response, layout);
                    if (canvas.response.hovered() || canvas.response.dragged())
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
                canvas.paint_sample(
                    editor,
                    layout,
                    tool.state.borrow().display.view != View::Working,
                    cx,
                );
                if let Some(region) = tool.state.borrow().region {
                    paint_selection(canvas.ui, layout, region);
                }
                Some(layout)
            },
        )
    }

    fn action(&mut self, action: tool::Action, editor: &mut Editor, _cx: &UiContext<'_, Editor>) {
        match action {
            tool::Action::Primary if !editor.busy => {
                self.state.borrow_mut().submit(editor);
            }
            tool::Action::Dismiss => editor.end_tool(),
            _ => {}
        }
    }

    fn activity(&self, _editor: &Editor) -> Option<TaskRequest> {
        self.state.borrow().activity()
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

pub(super) fn capture_scan(editor: &Editor) -> Result<Scan, String> {
    let ready = editor
        .document
        .visit
        .canvas
        .ready
        .as_ref()
        .ok_or("No developed source for measurement")?;
    let mut scan = Scan::with_reference(ready.developed.clone(), ready.reference.clone());
    scan.set_flat_field(
        editor
            .document
            .correction
            .as_ref()
            .map(|correction| correction.settings.map.clone()),
    )
    .map_err(|error| error.to_string())?;
    Ok(scan)
}

pub(super) fn query_services(
    services: Option<&crate::editor::source::Services>,
) -> Result<&crate::editor::source::Services, Error> {
    services.ok_or_else(|| Error::Gpu("image-query services unavailable".into()))
}

/// Open a picker-owned operation and await its typed result independently of the tool.
pub(super) fn pick<'scope, T, F>(
    editor: &mut Editor,
    display: Display,
    measure: F,
    cx: UiContext<'scope, Editor>,
) -> impl std::future::Future<Output = Option<T>> + use<'scope, T, F>
where
    T: Send + 'static,
    F: FnOnce(Coverage, Cancellation) -> Result<T, Error> + Send + 'static,
{
    let (mut reply, receive) = oneshot::channel();
    let previous = editor.document.visit.canvas.last_layout;
    if editor.prepare_tool() {
        editor.retire_pick_actions();
        let (submit, selected) = oneshot::channel();
        let id = editor.request_id();
        #[cfg(test)]
        let gate = editor.document.visit.picker.measurement_gate.clone();
        // The task retains work and communication endpoints, never its tool/UI owner.
        let task = cx.spawn(move |cx| async move {
            let measured = async {
                let region = selected.await.ok()?;
                Some(
                    crate::background::run(move |cancelled| {
                        #[cfg(test)]
                        if let Some(gate) = &gate
                            && !gate.is_after_work()
                        {
                            gate.wait(cancelled.clone())?;
                        }
                        let result =
                            measure(region, cancelled.clone()).map_err(|error| error.to_string());
                        #[cfg(test)]
                        if let Some(gate) = &gate
                            && gate.is_after_work()
                        {
                            gate.wait(cancelled)?;
                        }
                        result
                    })
                    .await
                    .map_err(|_| "Measurement worker panicked".to_owned())
                    .and_then(|result| result),
                )
            };
            let stopped = async {
                reply.cancellation().await;
                None
            };
            let result = futures_lite::future::or(stopped, measured).await;
            let value = cx
                .update(|editor| {
                    if let Some(Err(error)) = &result {
                        editor.status = error.clone();
                    }
                    editor.end_tool();
                    result.and_then(Result::ok)
                })
                .flatten();
            if let Some(value) = value {
                let _ = reply.send(value);
            }
        });
        if let Some(task) = task {
            install(
                editor,
                display,
                previous,
                Some(submit),
                Some(id),
                Some(task),
            );
        }
    }
    async move { receive.await.ok() }
}
