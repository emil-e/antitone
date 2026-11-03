use super::*;
use crate::viewport::CanvasLayout;

pub(in crate::editor) struct PickTool {
    state: Rc<RefCell<Interaction>>,
    _task: Option<async_executor::Task<()>>,
    viewport: Viewport,
    orientation: (RollOrientation, PhotoOrientation),
    crop: Option<Crop>,
    initial_view: Option<Viewport>,
}

impl PickTool {
    pub(super) fn new(
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
            show_viewer: true,
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

    fn composition(&mut self, normal: Composition) -> Composition {
        match self.state.borrow().display.view {
            View::Working => normal,
            View::Source => Composition::new(normal.area(), self.orientation.0, self.orientation.1)
                .expect("selected source composition"),
            View::Native => Composition::new(
                normal.area(),
                RollOrientation::default(),
                PhotoOrientation::default(),
            )
            .expect("selected native composition"),
        }
    }

    fn viewport<'a>(&'a mut self, editor: &'a mut Editor) -> &'a mut Viewport {
        if self.state.borrow().display.view == View::Working {
            &mut editor.document.visit.canvas.viewport
        } else {
            &mut self.viewport
        }
    }

    fn canvas(
        &mut self,
        canvas: &mut canvas::Canvas<'_>,
        editor: &mut Editor,
        cx: &UiContext<'_, Editor>,
    ) -> CanvasLayout {
        let composition = self.composition(canvas.composition);
        let rect = canvas.response.rect;
        let ppp = canvas.ui.ctx().pixels_per_point();
        self.initialise_view(composition.area(), rect, ppp);
        let relayout =
            |viewport: &mut Viewport| viewport.layout_composition(rect, composition, ppp);
        let mut response = canvas.response.clone();
        navigation::consume_primary(&mut response);
        let (layout, panning) = {
            let viewport = self.viewport(editor);
            let layout = canvas.layout(viewport, relayout);
            navigation::navigate_view(viewport, canvas.ui, &response, layout, relayout)
        };
        let space = canvas.ui.input(|input| input.key_down(egui::Key::Space));
        if canvas.ui.is_enabled() && !editor.busy && !panning && !space {
            self.select(canvas.ui, canvas.response, layout);
            if (canvas.response.hovered() || canvas.response.dragged())
                && let Some(point) = canvas.ui.input(|input| input.pointer.hover_pos())
            {
                let state = self.state.borrow();
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
            self.state.borrow_mut().drag = None;
        }
        canvas.paint_sample(
            editor,
            layout,
            self.state.borrow().display.view != View::Working,
            cx,
        );
        if let Some(region) = self.state.borrow().region {
            paint_selection(canvas.ui, layout, region);
        }
        layout
    }

    fn action(&mut self, action: tool::Action, editor: &mut Editor) {
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

pub(in crate::editor) fn paint_selection(ui: &egui::Ui, layout: CanvasLayout, region: Coverage) {
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
