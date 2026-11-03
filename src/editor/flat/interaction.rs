use super::*;
use crate::viewport::CanvasLayout;
use std::{cell::RefCell, rc::Rc};

pub(in crate::editor) struct FlatTool {
    pub(super) state: Rc<RefCell<FlatEdit>>,
    viewport: Viewport,
    fullscreen_viewport: Option<Viewport>,
    drag: Option<PickDrag>,
}

impl FlatTool {
    pub(in crate::editor) fn open(
        editor: &mut Editor,
        path: PathBuf,
        region: Option<Rect>,
        cx: &UiContext<'_, Editor>,
    ) {
        if !editor.prepare_tool() {
            return;
        }
        editor.sync_document();
        editor.document.thumbnails.cancel();
        let id = editor.request_id();
        let context = FlatEdit {
            services: editor.source_services.clone(),
            view: Default::default(),
            region: region.map(Coverage::rectangle),
            task: None,
            activity: Some(TaskRequest::Reference(id)),
        };
        let tool = Self::new(context);
        editor.busy = true;
        editor.status = "Preparing uncorrected flat-field reference…".into();
        editor.tasks.begin(Task::Reference(id));
        let request = Self::preview_input(editor, path);
        let task = operation::setup(
            request,
            editor.source_services.clone(),
            std::rc::Rc::downgrade(&tool.state),
            cx,
        );
        if let Some(task) = task {
            tool.state.borrow_mut().task = Some(task);
            tool.install(editor);
        } else {
            editor.tasks.finish(TaskRequest::Reference(id));
            editor.busy = false;
            editor.status = "Application stopped".into();
            editor.report_failure(FailureKind::FlatReference, editor.status.clone());
        }
    }

    pub(super) fn preview_input(editor: &Editor, path: PathBuf) -> source::Request {
        let expected = editor
            .document
            .find_source(&path)
            .and_then(|id| editor.document.state.project.source(id).ok())
            .cloned();
        // A retained record is either an ordered photo or the current flat-field reference;
        // both belong to the current stage. Use its saved decode identity when present.
        let wb = editor
            .document
            .state
            .project
            .acquisition_wb(expected.as_ref());
        let file = editor.document.file_key(&path);
        source::Request {
            reference: expected.as_ref().map_or_else(Arc::default, |source| {
                editor
                    .document
                    .exposure_slot(&editor.document.exposures, source.id)
            }),
            input: editor.input(),
            path,
            wb,
            expected,
            file,
            sources: editor.source_provider(),
            correction: None,
        }
    }

    pub(super) fn new(state: FlatEdit) -> Self {
        Self {
            state: Rc::new(RefCell::new(state)),
            viewport: Default::default(),
            fullscreen_viewport: None,
            drag: None,
        }
    }

    pub(super) fn install(self, editor: &mut Editor) {
        editor
            .document
            .set_hosted_tool(tool::Placement::Document, Box::new(self));
    }
}

impl tool::Tool for FlatTool {
    fn presentation(&self, editor: &Editor) -> tool::Presentation {
        let state = self.state.borrow();
        tool::Presentation {
            identity: presentation::ToolIdentity {
                icon: icons::Icon::Crop,
                title: "Flat-field",
            },
            primary: Some(tool::Primary {
                label: "Apply",
                enabled: state.view.ready.is_some() && state.region.is_some() && !editor.busy,
            }),
            dismiss: "Cancel",
            warning: None,
            help: "Select the illuminated area, excluding the holder and including the image centre.",
            navigation_help: None,
            fit_label: "Fit",
        }
    }

    fn viewport<'a>(&'a mut self, editor: &'a mut Editor) -> &'a mut Viewport {
        if editor.fullscreen.is_some() {
            self.fullscreen_viewport.get_or_insert(self.viewport)
        } else {
            self.fullscreen_viewport = None;
            &mut self.viewport
        }
    }

    fn filmstrip(&mut self, ui: &mut egui::Ui, editor: &mut Editor, cx: &UiContext<'_, Editor>) {
        editor.filmstrip_readonly(ui, cx);
    }

    fn shortcuts(&mut self, ui: &mut egui::Ui, _editor: &mut Editor) {
        ui.input_mut(|input| {
            input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft);
            input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight);
        });
    }

    fn drawer(&mut self, ui: &mut egui::Ui, _editor: &mut Editor, _cx: &UiContext<'_, Editor>) {
        presentation::ToolIdentity { icon: icons::Icon::Crop, title: "Flat-field" }.inspector(ui, |_| false, |ui, _| {
            if let Some(ready) = self.state.borrow().view.ready.as_ref() {
                ui.add(egui::Label::new(ready.path.file_name().unwrap_or_default().to_string_lossy()).truncate()).on_hover_text(ready.path.display().to_string());
            }
            ui.small("Select the illuminated area, excluding the holder and including the image centre.");
            ui.small("Dmin/Dmax stay unchanged; re-pick them after applying correction.");
        });
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
            Editor::source_controls,
            |tool, surface, editor, cx| {
                let (area, proxy) = tool
                    .state
                    .borrow()
                    .view
                    .ready
                    .as_ref()
                    .map(|ready| (ready.area, ready.proxy.clone()))?;
                let composition = Composition::new(area, Default::default(), Default::default())
                    .expect("prepared source area");
                let mut canvas = surface.image(composition, proxy);
                let rect = canvas.response.rect;
                let ppp = canvas.ui.ctx().pixels_per_point();
                let relayout =
                    |viewport: &mut Viewport| viewport.layout_composition(rect, composition, ppp);
                let mut response = canvas.response.clone();
                if editor.fullscreen.is_none() {
                    navigation::consume_primary(&mut response);
                }
                let viewport = tool.viewport(editor);
                let layout = canvas.layout(viewport, relayout);
                let (layout, panning) =
                    navigation::navigate_view(viewport, canvas.ui, &response, layout, relayout);
                let services = tool.state.borrow().services.clone();
                tool.state.borrow_mut().view.paint(
                    &mut canvas,
                    layout,
                    editor,
                    services.as_ref(),
                    cx,
                );
                let selecting = editor.fullscreen.is_none()
                    && !editor.busy
                    && canvas.ui.is_enabled()
                    && !panning
                    && !canvas.ui.input(|input| input.key_down(egui::Key::Space));
                if selecting {
                    let mut state = tool.state.borrow_mut();
                    if canvas.response.clicked_by(egui::PointerButton::Primary)
                        && let Some(point) = canvas
                            .response
                            .interact_pointer_pos()
                            .filter(|point| layout.image.contains(*point))
                    {
                        state.region = Some(layout.pick_patch(point, 32));
                    }
                    if canvas
                        .response
                        .drag_started_by(egui::PointerButton::Primary)
                    {
                        tool.drag = canvas
                            .ui
                            .input(|input| input.pointer.press_origin())
                            .filter(|point| layout.image.contains(*point))
                            .map(|point| PickDrag::begin(layout, point, state.region));
                    }
                    if canvas.response.dragged_by(egui::PointerButton::Primary)
                        && let (Some(drag), Some(point)) =
                            (tool.drag, canvas.response.interact_pointer_pos())
                    {
                        state.region = Some(drag.update(layout, point));
                    }
                    if (canvas.response.hovered() || canvas.response.dragged())
                        && let Some(point) = canvas.ui.input(|input| input.pointer.hover_pos())
                    {
                        canvas.ui.ctx().set_cursor_icon(navigation::picker_cursor(
                            layout,
                            point,
                            state.region,
                            tool.drag,
                            canvas.response.dragged_by(egui::PointerButton::Primary),
                        ));
                    }
                }
                if !canvas
                    .ui
                    .input(|input| input.pointer.button_down(egui::PointerButton::Primary))
                {
                    tool.drag = None;
                }
                if editor.fullscreen.is_none()
                    && let Some(region) = tool.state.borrow().region
                {
                    picking::paint_selection(canvas.ui, layout, region);
                }
                Some(layout)
            },
        )
    }

    fn action(&mut self, action: tool::Action, editor: &mut Editor, cx: &UiContext<'_, Editor>) {
        match action {
            tool::Action::Primary if !editor.busy && editor.document_pending.is_none() => {
                editor.sync_document();
                let state = self.state.borrow();
                let Some(reference) = &state.view.ready else {
                    return;
                };
                let Some(coverage) = state.region else { return };
                let Some(region) = coverage.exact_rectangle(reference.area) else {
                    editor.status = "Flat-field support must be an integer source rectangle".into();
                    editor.report_failure(FailureKind::FlatSupport, editor.status.clone());
                    return;
                };
                let input = CorrectionInput {
                    services: state.services.clone(),
                    sources: state
                        .services
                        .as_ref()
                        .map(|services| services.provider(Some(editor.document.cache.clone()))),
                    region,
                    document: editor.document.capture(),
                    selected: editor.document.visit.current_source,
                    source: source::Context {
                        source: reference.developed.clone(),
                        reference: reference.reference.clone(),
                        correction: None,
                    },
                    record: reference.record.clone(),
                    correction: editor.document.correction.clone(),
                };
                operation::correct(editor, input, cx);
            }
            tool::Action::Dismiss => {
                editor.status = "Flat-field setup cancelled; correction unchanged".into();
                editor.end_tool();
            }
            _ => {}
        }
    }

    fn activity(&self, _editor: &Editor) -> Option<TaskRequest> {
        self.state.borrow().activity
    }
}
