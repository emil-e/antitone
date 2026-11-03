use super::*;

#[cfg(test)]
mod tests;
use crate::viewport::CanvasLayout;
#[cfg(test)]
use std::rc::Weak;
use std::{cell::RefCell, rc::Rc};

type Review = RefCell<Option<Draft>>;

/// Tests can observe retirement without retaining the unfinished batch.
#[cfg(test)]
#[derive(Default)]
pub(in crate::editor) struct State(Weak<Review>);

#[cfg(test)]
impl State {
    pub(super) fn current(&self) -> Option<Rc<Review>> {
        self.0.upgrade()
    }

    pub(in crate::editor) fn active(&self) -> bool {
        self.current().is_some()
    }

    pub(in crate::editor) fn reviewing(&self) -> bool {
        self.current().is_some_and(|state| state.borrow().is_some())
    }

    #[cfg(test)]
    pub(in crate::editor) fn snapshot(&self) -> Option<Draft> {
        self.current()?.borrow().clone()
    }
}

pub(in crate::editor) struct AutoCropTool {
    pub(super) review: Rc<Review>,
    pub(super) preparation: Option<async_executor::Task<()>>,
    viewport: Viewport,
    visit: Option<u64>,
}

impl AutoCropTool {
    pub(super) fn new(review: Option<Draft>) -> Self {
        Self {
            review: Rc::new(RefCell::new(review)),
            preparation: None,
            viewport: Viewport::default(),
            visit: None,
        }
    }

    pub(super) fn install(self, editor: &mut Editor) {
        #[cfg(test)]
        let state = Rc::downgrade(&self.review);
        editor
            .document
            .set_hosted_tool(tool::Placement::Document, Box::new(self));
        #[cfg(test)]
        {
            editor.document.auto_crop.0 = state;
        }
    }

    fn review_viewport(&mut self, editor: &Editor) -> &mut Viewport {
        if self.visit != Some(editor.document.visit.view) {
            self.visit = Some(editor.document.visit.view);
            self.viewport.fit();
        }
        &mut self.viewport
    }
}

impl tool::Tool for AutoCropTool {
    fn composition(&self, normal: Composition) -> Composition {
        if self.review.borrow().is_some() {
            normal
                .cropped(Crop::from_region(normal.area()))
                .expect("source framing is valid")
        } else {
            normal
        }
    }
    fn presentation(&self, editor: &Editor) -> tool::Presentation {
        let review = self.review.borrow();
        let warning = review.as_ref().and_then(|draft| {
            editor
                .document
                .visit
                .current_source
                .and_then(|id| draft.problem(editor, id))
                .or_else(|| draft.warning(editor).map(str::to_owned))
        });
        tool::Presentation {
            identity: presentation::ToolIdentity {
                icon: icons::Icon::Crop,
                title: "Auto-crop",
            },
            primary: Some(tool::Primary {
                label: "Apply",
                enabled: review.as_ref().is_some_and(|draft| draft.editable(editor)),
            }),
            dismiss: "Cancel",
            warning,
            help: "Drag an edge or corner for the shared inward margin; Space-drag / middle-drag pans.\nBrowsing is optional and never retargets the proposals · H hides artwork.",
            navigation_help: None,
            fit_label: "Fit",
        }
    }

    fn viewport<'a>(&'a mut self, editor: &'a mut Editor) -> &'a mut Viewport {
        if editor.fullscreen.is_none() && self.review.borrow().is_some() {
            self.review_viewport(editor)
        } else {
            &mut editor.document.visit.canvas.viewport
        }
    }

    fn filmstrip(&mut self, ui: &mut egui::Ui, editor: &mut Editor, cx: &UiContext<'_, Editor>) {
        let mut preview = thumbnails::Preview::default();
        if let Some(draft) = self.review.borrow().as_ref() {
            if draft.belongs_to(editor) {
                for entry in &draft.entries {
                    if entry.current(editor)
                        && let Ok(source) = editor.document.project.source(entry.source)
                    {
                        let crop = entry
                            .draft
                            .unwrap_or(Crop::from_region(entry.development.active_area.into()));
                        let composition = editor
                            .source_composition(source)
                            .cropped(crop)
                            .expect("validated proposal");
                        preview.compositions.push((entry.source, composition));
                    }
                }
            }
            for id in editor.document.project.order.iter().copied() {
                if let Some(problem) = draft.problem(editor, id) {
                    preview.warnings.push((id, problem));
                }
            }
        }
        editor.filmstrip_preview(ui, preview, cx);
    }

    fn drawer(&mut self, ui: &mut egui::Ui, editor: &mut Editor, _cx: &UiContext<'_, Editor>) {
        if let Some(draft) = self.review.borrow_mut().as_mut() {
            draft.controls(ui, editor);
        }
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
                if editor.fullscreen.is_some() {
                    return surface.photograph(editor, cx);
                }
                let mut canvas = surface.selected()?;
                let reviewing = tool.review.borrow().is_some();
                let composition = tool.composition(canvas.composition);
                let rect = canvas.response.rect;
                let ppp = canvas.ui.ctx().pixels_per_point();
                let relayout =
                    |viewport: &mut Viewport| viewport.layout_composition(rect, composition, ppp);
                let layout = canvas.layout(tool.viewport(editor), relayout);
                let (claimed, cursor) = tool
                    .review
                    .borrow_mut()
                    .as_mut()
                    .map_or((false, None), |draft| {
                        draft.margin_interaction(editor, canvas.ui, canvas.response, layout)
                    });
                let mut response = canvas.response.clone();
                if reviewing {
                    navigation::consume_primary(&mut response);
                }
                let layout = if claimed {
                    layout
                } else {
                    navigation::navigate_view(
                        tool.viewport(editor),
                        canvas.ui,
                        &response,
                        layout,
                        relayout,
                    )
                    .0
                };
                if let Some(cursor) = cursor {
                    canvas.ui.ctx().set_cursor_icon(cursor);
                }
                canvas.paint_image(editor, layout, false, cx);
                if let Some(draft) = tool.review.borrow().as_ref()
                    && draft.artwork_visible(editor, canvas.ui)
                    && let Some(outline) = draft.outline(editor, layout)
                {
                    crop_editor::paint_outline(canvas.ui, layout.canvas, outline, false, 64);
                }
                Some(layout)
            },
        )
    }

    fn action(&mut self, action: tool::Action, editor: &mut Editor, _cx: &UiContext<'_, Editor>) {
        match action {
            tool::Action::Primary if !editor.busy && editor.fullscreen.is_none() => {
                let Some(snapshot) = self.review.borrow().clone() else {
                    return;
                };
                if snapshot.entries.is_empty() {
                    editor.status = "Auto-crop has no proposals to apply".into();
                    return;
                }
                match editor.commit_auto_crop(&snapshot) {
                    Ok(()) => editor.end_tool(),
                    Err((source, error)) => {
                        let reason = reason(error);
                        if let Some(source) = source
                            && let Some(draft) = self.review.borrow_mut().as_mut()
                        {
                            draft.record_problem(source, reason.clone());
                        }
                        editor.status =
                            format!("Auto-crop not applied: {reason}. Cancel and start again.");
                    }
                }
            }
            tool::Action::Dismiss => {
                editor.status = "Auto-crop cancelled; previous crops unchanged".into();
                editor.end_tool();
            }
            _ => {}
        }
    }

    fn activity(&self, editor: &Editor) -> Option<TaskRequest> {
        if self.review.borrow().is_some() {
            None
        } else {
            editor
                .document
                .spatial_preparing
                .as_ref()
                .map(|work| TaskRequest::Regions(work.id))
        }
    }
}

impl Draft {
    pub(super) fn editable(&self, editor: &Editor) -> bool {
        !editor.busy && editor.fullscreen.is_none() && self.valid(editor)
    }

    pub(super) fn warning(&self, editor: &Editor) -> Option<&'static str> {
        if self.entries.is_empty() {
            Some("No crop proposals are available for this batch")
        } else if !self.valid(editor) {
            Some("Crop batch is out of date; cancel and start again")
        } else {
            None
        }
    }

    pub(super) fn problem(&self, editor: &Editor, id: SourceId) -> Option<String> {
        let entry = self.entries.iter().find(|entry| entry.source == id);
        let problem = self.problems.iter().find(|(source, _)| *source == id);
        if (entry.is_some() || problem.is_some()) && !self.belongs_to(editor) {
            return Some("Crop proposals are out of date".into());
        }
        if let Some(entry) = entry {
            if !entry.current(editor) {
                return Some("Crop proposal is out of date".into());
            }
            if !editor.busy
                && editor.document.visit.current_source == Some(id)
                && !editor.loaded_crop_source_current(id, &entry.file)
            {
                return Some("Loaded photo no longer matches the crop proposal".into());
            }
        }
        problem.map(|(_, reason)| reason.clone())
    }

    pub(super) fn record_problem(&mut self, source: SourceId, reason: String) {
        if let Some((_, old)) = self.problems.iter_mut().find(|(id, _)| *id == source) {
            *old = reason;
        } else {
            self.problems.push((source, reason));
        }
    }

    fn controls(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        if !editor.document.input.capabilities().auto_crop {
            return;
        }
        if std::mem::take(&mut self.reveal_controls) {
            ui.scroll_to_cursor(Some(egui::Align::Min));
        }
        let scope = format!(
            "{} · {}/{} proposals",
            self.scope,
            self.entries.len(),
            self.attempted
        );
        let mut show = self.show_outline;
        let mut margin = editor.auto_crop_margin;
        let editable = self.editable(editor);
        presentation::ToolIdentity {
            icon: icons::Icon::Crop,
            title: "Auto-crop",
        }
        .inspector(
            ui,
            |ui| {
                ui.add(egui::Label::new(&scope).truncate())
                    .on_hover_text(&self.summary);
            },
            |ui, _| {
                if !self.entries.is_empty() {
                    ui.add_enabled_ui(editable, |ui| {
                        controls::parameter(ui, controls::Parameter::CropMargin, &mut margin);
                        ui.checkbox(&mut show, "Show crop outline")
                            .on_hover_text("Hold H to hide the artwork temporarily.");
                    });
                }
            },
        );
        self.show_outline = show;
        if margin != editor.auto_crop_margin {
            self.adjust_margin(editor, margin);
        }
    }

    pub(super) fn adjust_margin(&mut self, editor: &mut Editor, percent: f64) {
        if !self.editable(editor) {
            return;
        }
        if !percent.is_finite() || !(0.0..=framing::MAX_MARGIN_PERCENT).contains(&percent) {
            editor.status = "Auto-crop margin rejected: invalid percentage".into();
            return;
        }
        let mut failed_source = None;
        let proposals = self
            .entries
            .iter()
            .map(|entry| {
                failed_source = Some(entry.source);
                if !editor.loaded_crop_source_current(entry.source, &entry.file) {
                    return Err(Error::UnusableSample(
                        "source changed; Cancel and start Auto-crop again",
                    ));
                }
                let active = entry.development.active_area.into();
                framing::inward_margin(entry.proposal, active, percent)
                    .map(|crop| accepted(crop, active))
            })
            .collect::<Result<Vec<_>, _>>();
        match proposals {
            Ok(proposals) => {
                for (entry, draft) in self.entries.iter_mut().zip(proposals) {
                    entry.draft = draft;
                }
                editor.auto_crop_margin = percent;
                editor.status =
                    format!("Auto-crop draft margin: {percent:.2}% · Apply to accept, or Cancel.");
            }
            Err(error) => {
                let reason = reason(error);
                self.record_problem(
                    failed_source.expect("failed proposal retains source"),
                    reason.clone(),
                );
                editor.status = format!("Auto-crop draft unchanged: {reason}");
            }
        }
    }
}
