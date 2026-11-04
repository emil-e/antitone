use super::*;
use crate::viewport::CanvasLayout;

#[cfg(test)]
mod ownership_tests;
#[cfg(test)]
mod tests;

pub(super) struct AnalyzeTool {
    pub operation: analysis_workflow::Operation,
    pub review: Option<analysis_review::Review>,
    pub(super) drag: Option<PickDrag>,
    pub(super) viewport: Viewport,
    pub(super) visit: Option<u64>,
    fullscreen_viewport: Option<Viewport>,
}

impl AnalyzeTool {
    pub fn new(operation: analysis_workflow::Operation) -> Self {
        Self {
            operation,
            review: None,
            drag: None,
            viewport: Viewport::default(),
            visit: None,
            fullscreen_viewport: None,
        }
    }
    pub fn fitting(&self) -> bool {
        matches!(self.operation.phase, analysis_workflow::Phase::Fitting(_))
    }
    fn observe_visit(&mut self, editor: &Editor) {
        if self.visit != Some(editor.document.visit.view) {
            self.visit = Some(editor.document.visit.view);
            self.viewport.fit();
            self.fullscreen_viewport = None;
            self.drag = None;
        }
    }
}

impl tool::Tool for AnalyzeTool {
    fn presentation(&self, editor: &Editor) -> tool::Presentation {
        let mut warning = editor
            .document
            .visit
            .current_source
            .and_then(|id| self.review_warning(id))
            .map(str::to_owned);
        if let Some(review) = &self.review {
            let missing = review
                .approved
                .entries
                .iter()
                .filter(|entry| !entry.areas.ready())
                .count();
            if warning.is_none() && missing > 0 {
                warning = Some(format!(
                    "Sampling areas missing: {missing} photo{}",
                    if missing == 1 { "" } else { "s" }
                ));
            }
            if review.replaces_photos {
                let consequence =
                    "Analyze will replace Photo layers and remove any adjustment layers and masks.";
                if let Some(warning) = &mut warning {
                    warning.push_str(&format!("\n{consequence}"));
                } else {
                    warning = Some(consequence.into());
                }
            }
        }
        tool::Presentation {
            identity: presentation::ToolIdentity {
                icon: icons::Icon::Analyze,
                title: "Analyze",
            },
            primary: Some(tool::Primary {
                label: "Analyze roll",
                enabled: !editor.busy
                    && !self.fitting()
                    && self.review.as_ref().is_some_and(|review| {
                        review
                            .approved
                            .entries
                            .iter()
                            .all(|entry| entry.areas.ready())
                    })
                    && editor.document.visit.canvas.ready.is_some(),
            }),
            dismiss: "Cancel",
            warning,
            help: "Choose Opening or Picture on the source; drag edges/corners to resize.\nBrowse to edit exceptions before analyzing the roll.",
            navigation_help: None,
            fit_label: "Fit",
        }
    }
    fn composition(&self, normal: Composition) -> Composition {
        if self.review.is_some() {
            Composition::new(
                normal.area(),
                RollOrientation::default(),
                PhotoOrientation::default(),
            )
            .expect("source composition")
        } else {
            normal
        }
    }
    fn viewport<'a>(&'a mut self, editor: &'a mut Editor) -> &'a mut Viewport {
        if self.review.is_some() {
            self.observe_visit(editor);
            if editor.fullscreen.is_some() {
                self.fullscreen_viewport.get_or_insert(self.viewport)
            } else {
                self.fullscreen_viewport = None;
                &mut self.viewport
            }
        } else {
            &mut editor.document.visit.canvas.viewport
        }
    }
    fn canvas(
        &mut self,
        viewer: &mut canvas::Viewer<'_>,
        editor: &mut Editor,
        cx: &UiContext<'_, Editor>,
    ) -> Option<CanvasLayout> {
        if self.review.is_none() {
            return viewer.show(
                self,
                editor,
                cx,
                Editor::photograph_controls,
                |_, surface, editor, cx| surface.photograph(editor, cx),
            );
        }
        self.observe_visit(editor);
        viewer.show(
            self,
            editor,
            cx,
            Editor::source_controls,
            |tool, surface, editor, cx| {
                let mut canvas = surface.selected()?;
                let composition = tool.composition(canvas.composition);
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
                if editor.fullscreen.is_none()
                    && canvas.ui.is_enabled()
                    && !editor.busy
                    && !panning
                    && !canvas.ui.input(|input| input.key_down(egui::Key::Space))
                {
                    tool.select(canvas.ui, canvas.response, layout, editor);
                    if (canvas.response.hovered() || canvas.response.dragged())
                        && let Some(point) = canvas.ui.input(|input| input.pointer.hover_pos())
                    {
                        let dragging = canvas.response.dragged_by(egui::PointerButton::Primary);
                        let region = editor.document.visit.current_source.and_then(|id| {
                            if dragging {
                                tool.selected_region(id)
                            } else {
                                let picture = tool.review_area_at(editor, layout, point)?;
                                let areas = tool.review_areas(id)?;
                                crate::scan_regions::Areas::coverage(if picture {
                                    areas.picture?
                                } else {
                                    areas.opening?
                                })
                                .ok()
                            }
                        });
                        canvas.ui.ctx().set_cursor_icon(navigation::picker_cursor(
                            layout, point, region, tool.drag, dragging,
                        ));
                    }
                }
                if !canvas
                    .ui
                    .input(|input| input.pointer.button_down(egui::PointerButton::Primary))
                {
                    tool.drag = None;
                }
                canvas.paint_sample(editor, layout, true, cx);
                if editor.fullscreen.is_none() {
                    tool.paint_review_areas(editor, canvas.ui, layout, layout.canvas);
                }
                Some(layout)
            },
        )
    }
    fn filmstrip(&mut self, ui: &mut egui::Ui, editor: &mut Editor, cx: &UiContext<'_, Editor>) {
        let Some(review) = &self.review else {
            editor.filmstrip(ui, cx);
            return;
        };
        let mut preview = thumbnails::Preview::default();
        for row in &review.approved.entries {
            let Ok(source) = review.document.project.source(row.source) else {
                continue;
            };
            preview.compositions.push((
                row.source,
                Composition::new(
                    source.development.active_area.into(),
                    RollOrientation::default(),
                    PhotoOrientation::default(),
                )
                .expect("review source"),
            ));
            preview.sources.push((
                row.source,
                editor
                    .source_parameters_for(source, editor.source_exposure)
                    .map_err(|error| error.to_string()),
            ));
            if let Some(quad) = if review.picture {
                row.areas.picture
            } else {
                row.areas.opening
            } {
                preview.outlines.push((row.source, quad));
            }
            if let Some(warning) = self.review_warning(row.source) {
                preview.warnings.push((row.source, warning.into()));
            }
        }
        editor.filmstrip_preview(ui, preview, cx);
    }
    fn action(&mut self, action: tool::Action, editor: &mut Editor, cx: &UiContext<'_, Editor>) {
        match action {
            tool::Action::Primary => self.confirm_analysis_review(editor, cx),
            tool::Action::Dismiss => self.cancel(editor),
        }
    }
    fn activity(&self, editor: &Editor) -> Option<TaskRequest> {
        match self.operation.phase {
            analysis_workflow::Phase::Preparing(id) | analysis_workflow::Phase::Fitting(id) => {
                Some(TaskRequest::Analyze(id))
            }
            analysis_workflow::Phase::Regions => editor
                .document
                .spatial_preparing
                .as_ref()
                .map(|pending| TaskRequest::Regions(pending.id)),
            analysis_workflow::Phase::Review => None,
        }
    }
}

impl AnalyzeTool {
    fn select(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        layout: CanvasLayout,
        editor: &mut Editor,
    ) {
        if response.clicked_by(egui::PointerButton::Primary)
            && let Some(point) = response.interact_pointer_pos()
        {
            self.select_review_area(editor, layout, point);
        }
        if response.drag_started_by(egui::PointerButton::Primary)
            && let Some(point) = ui.input(|input| input.pointer.press_origin())
        {
            self.select_review_area(editor, layout, point);
            self.drag = Some(PickDrag::begin(
                layout,
                point,
                editor
                    .document
                    .visit
                    .current_source
                    .and_then(|id| self.selected_region(id)),
            ));
        }
        if response.dragged_by(egui::PointerButton::Primary)
            && let (Some(drag), Some(point)) = (self.drag, response.interact_pointer_pos())
            && let Some(id) = editor.document.visit.current_source
        {
            self.edit_region(id, drag.update(layout, point));
        }
    }
    pub fn cancel(&mut self, editor: &mut Editor) {
        match self.operation.phase {
            analysis_workflow::Phase::Regions => {
                editor.cancel_spatial();
                editor.status = "Analysis cancelled; areas and grade unchanged".into();
                editor.end_tool();
            }
            analysis_workflow::Phase::Review => {
                editor.status = "Analysis review cancelled; areas and grade unchanged".into();
                editor.end_tool();
            }
            analysis_workflow::Phase::Preparing(id) | analysis_workflow::Phase::Fitting(id) => {
                self.operation.cancelled.cancel();
                editor.tasks.cancelling(TaskRequest::Analyze(id));
                editor.status = "Cancelling analysis…".into();
            }
        }
    }
}

impl DocumentScope {
    pub(super) fn analyze_tool(&self) -> Option<&AnalyzeTool> {
        let tool::Slot::Installed(tool) = &self.hosted_tool else {
            return None;
        };
        let value: &dyn std::any::Any = tool.as_ref();
        value.downcast_ref()
    }
    pub(super) fn analyze_tool_mut(&mut self) -> Option<&mut AnalyzeTool> {
        let tool::Slot::Installed(tool) = &mut self.hosted_tool else {
            return None;
        };
        let value: &mut dyn std::any::Any = tool.as_mut();
        value.downcast_mut()
    }
    pub fn analysis_operation(&self) -> Option<&analysis_workflow::Operation> {
        Some(&self.analyze_tool()?.operation)
    }
    pub fn analysis_operation_mut(&mut self) -> Option<&mut analysis_workflow::Operation> {
        Some(&mut self.analyze_tool_mut()?.operation)
    }
    pub fn set_analysis_operation(&mut self, operation: Option<analysis_workflow::Operation>) {
        if let Some(operation) = operation {
            self.set_hosted_tool(
                tool::Placement::Document,
                Box::new(AnalyzeTool::new(operation)),
            );
        } else if self.hosted_tool.is::<AnalyzeTool>() {
            self.hosted_tool = tool::Slot::Empty;
        }
    }
    pub fn analysis_review(&self) -> Option<&analysis_review::Review> {
        self.analyze_tool()?.review.as_ref()
    }
    #[cfg(test)]
    pub fn analysis_review_mut(&mut self) -> Option<&mut analysis_review::Review> {
        self.analyze_tool_mut()?.review.as_mut()
    }
}

impl Editor {
    pub(super) fn with_analyze_tool<R>(
        &mut self,
        body: impl FnOnce(&mut AnalyzeTool, &mut Editor) -> R,
    ) -> Option<R> {
        if !self.document.hosted_tool.is::<AnalyzeTool>() {
            return None;
        }
        self.with_current_tool(|tool, editor| {
            let value: &mut dyn std::any::Any = tool;
            body(value.downcast_mut().expect("Analyze owner"), editor)
        })
    }
}
