use super::*;
use crate::{
    auto_crop as framing, geometry::SourcePoint, project::SourceDevelopment,
    scan_regions::PictureGeometry, viewport::CanvasLayout,
};
#[cfg(test)]
use std::rc::Weak;
use std::{cell::RefCell, rc::Rc};

pub(super) enum Action {
    Batch {
        photos: Vec<SourceId>,
        scope: &'static str,
    },
}

#[derive(Clone)]
struct Entry {
    source: SourceId,
    proposal: Crop,
    before: Option<Crop>,
    draft: Option<Crop>,
    file: thumbnails::FileKey,
    development: SourceDevelopment,
    geometry: Option<PictureGeometry>,
    constraints: Vec<[f64; 3]>,
    spatial: Option<Arc<crate::spatial_metadata::Metadata>>,
}

#[derive(Clone)]
pub(super) struct Draft {
    order: Vec<SourceId>,
    scope: &'static str,
    attempted: usize,
    summary: String,
    entries: Vec<Entry>,
    problems: Vec<(SourceId, String)>,
    pub(super) reveal_controls: bool,
    pub(super) show_outline: bool,
    drag: Option<MarginDrag>,
}

impl Draft {
    fn valid(&self, editor: &Editor) -> bool {
        self.belongs_to(editor)
            && !self.entries.is_empty()
            && self.entries.iter().all(|entry| {
                entry.current(editor) && !self.problems.iter().any(|(id, _)| *id == entry.source)
            })
    }

    fn belongs_to(&self, editor: &Editor) -> bool {
        self.order == editor.document.state.project.order
            && self
                .entries
                .iter()
                .find_map(|entry| entry.spatial.as_ref())
                .is_none_or(|metadata| {
                    metadata.groups.iter().all(|group| {
                        group.sources.first().is_some_and(|row| {
                            editor
                                .document
                                .state
                                .project
                                .scan_basis()
                                .is_some_and(|basis| {
                                    basis.require_source(&row.input.development).is_ok()
                                })
                        }) && group.sources.iter().all(|row| {
                            editor
                                .document
                                .project
                                .source(row.input.source)
                                .is_ok_and(|source| source.development == row.input.development)
                        })
                    })
                })
    }
}

impl Entry {
    fn current(&self, editor: &Editor) -> bool {
        let Ok(source) = editor.document.state.project.source(self.source) else {
            return false;
        };
        editor
            .document
            .source_path(self.source)
            .is_ok_and(|path| editor.document.file_key(&path) == self.file)
            && source.crop == self.before
            && (editor.document.visit.current_source != Some(self.source)
                || editor.document.visit.crop == self.before)
            && source.development == self.development
            && if let Some(metadata) = &self.spatial {
                Arc::ptr_eq(metadata, &editor.document.spatial.ready())
            } else {
                editor
                    .document
                    .project
                    .analysis_regions
                    .iter()
                    .find(|r| r.source == self.source)
                    .is_some_and(|r| {
                        !r.areas.picture_edited
                            && r.areas.picture_observed == [true; 4]
                            && r.areas.picture_weak == [false; 4]
                            && r.areas.picture_geometry.as_ref() == self.geometry.as_ref()
                            && r.areas.picture_constraints == self.constraints
                    })
            }
    }
}

fn accepted(crop: Crop, active: Rect) -> Option<Crop> {
    (crop != Crop::from_region(active)).then_some(crop)
}

fn finish_photo_proposal(state: &mut CropEdit, proposal: Result<Crop, Error>) -> String {
    match proposal {
        Ok(crop) => {
            state.install_proposal(crop);
            state.auto_problem = None;
            "Auto-crop proposed".into()
        }
        Err(error) => {
            let problem = format!("Automatic crop unavailable: {}", reason(error));
            let status = format!("Crop unchanged: {problem}");
            state.auto_problem = Some(problem);
            status
        }
    }
}

fn reason(error: Error) -> String {
    match error {
        Error::UnusableSample(s) => s.to_owned(),
        error => error.to_string(),
    }
}

impl Editor {
    pub(super) fn auto_crop_entry_available(&self) -> bool {
        self.tool_start_available()
            && self.selected_area().is_some()
            && !self.document.project.order.is_empty()
    }

    fn loaded_crop_source_current(&self, id: SourceId, file: &thumbnails::FileKey) -> bool {
        if self.document.visit.current_source != Some(id) {
            return true;
        }
        self.document.visit.canvas.ready.as_ref().is_none_or(|r| {
            self.document.state.project.source(id).is_ok_and(|s| {
                r.record.development == s.development
                    && r.area == Rect::from(s.development.active_area)
                    // The captured key already contains the source locator and revision.
                    && &r.file == file
            })
        })
    }

    fn crop_proposal(&self, id: SourceId) -> Result<Entry, Error> {
        let source = self.document.state.project.source(id)?;
        let path = self.document.source_path(id)?;
        let file = self.document.file_key(&path);
        if !self.loaded_crop_source_current(id, &file) {
            return Err(Error::UnusableSample(
                "loaded source changed; reopen the photo",
            ));
        }
        let active = source.development.active_area.into();
        let metadata = self.document.spatial.ready();
        let regions = metadata
            .groups
            .iter()
            .flat_map(|group| &group.sources)
            .find(|row| row.input.source == id);
        let (proposal, geometry, constraints, spatial) = if let Some(regions) = regions {
            if regions.input.development != source.development
                || file.revision.clone() != Some(regions.input.revision.clone())
            {
                return Err(Error::UnusableSample(
                    "shared picture regions are out of date",
                ));
            }
            (
                framing::propose_spatial(regions)?,
                None,
                vec![],
                Some(metadata.clone()),
            )
        } else {
            let areas = &self
                .document
                .project
                .analysis_regions
                .iter()
                .find(|r| r.source == id)
                .ok_or(Error::UnusableSample(
                    "no retained frame evidence or prepared picture regions",
                ))?
                .areas;
            (
                framing::propose(active, areas)?,
                areas.picture_geometry.clone(),
                areas.picture_constraints.clone(),
                None,
            )
        };
        let crop = framing::inward_margin(proposal, active, self.auto_crop_margin)?;
        Ok(Entry {
            source: id,
            proposal,
            before: source.crop,
            draft: accepted(crop, active),
            file,
            development: source.development.clone(),
            geometry,
            constraints,
            spatial,
        })
    }

    pub(super) fn photo_crop_proposal(&self, photo: SourceId) -> Result<Crop, Error> {
        let entry = self.crop_proposal(photo)?;
        Ok(entry
            .draft
            .unwrap_or(Crop::from_region(entry.development.active_area.into())))
    }

    #[cfg(test)]
    fn auto_crop_photo(&mut self, photo: SourceId) {
        if self.interaction_blocked()
            || self.fullscreen.is_some()
            || !self.cropping()
            || self.document.visit.current_source != Some(photo)
        {
            return;
        }
        self.sync_document();
        let result = self.photo_crop_proposal(photo);
        let status = finish_photo_proposal(&mut self.document.crop_edit_mut().unwrap(), result);
        self.status = status;
    }

    fn capture_crop_review(&mut self, photos: Vec<SourceId>, scope: &'static str) -> Option<Draft> {
        if photos.is_empty() || !self.auto_crop_entry_available() {
            return None;
        }
        self.sync_document();
        let attempted = photos.len();
        let mut entries = Vec::new();
        let mut problems = Vec::new();
        for id in photos {
            match self.crop_proposal(id) {
                Ok(entry) => entries.push(entry),
                Err(error) => problems.push((id, format!("No crop proposal: {}", reason(error)))),
            }
        }
        self.status = format!(
            "Auto-crop: {} proposed, {} skipped",
            entries.len(),
            problems.len()
        );
        Some(Draft {
            order: self.document.state.project.order.clone(),
            scope,
            attempted,
            summary: self.status.clone(),
            entries,
            problems,
            reveal_controls: true,
            show_outline: true,
            drag: None,
        })
    }

    #[cfg(test)]
    pub(super) fn auto_crop_preview(&self, id: SourceId) -> Option<Option<Crop>> {
        let state = self.document.auto_crop.current()?;
        let state = state.borrow();
        let edit = state.as_ref()?;
        if !edit.belongs_to(self) {
            return None;
        }
        edit.entries
            .iter()
            .find(|entry| entry.source == id)
            .filter(|entry| entry.current(self))
            .map(|entry| entry.draft)
    }

    #[cfg(test)]
    pub(super) fn auto_crop_problem(&self, id: SourceId) -> Option<String> {
        let state = self.document.auto_crop.current()?;
        state.borrow().as_ref()?.problem(self, id)
    }

    fn commit_auto_crop(&mut self, edit: &Draft) -> Result<(), (Option<SourceId>, Error)> {
        let mut failed_source = None;
        let candidate = (|| -> Result<_, Error> {
            if !edit.valid(self) {
                return Err(Error::UnusableSample(
                    "document, crops or measurements changed",
                ));
            }
            self.sync_document();
            let ready = self.document.capture().ready_spatial_metadata();
            if edit.entries.iter().any(|entry| {
                entry
                    .spatial
                    .as_ref()
                    .is_some_and(|metadata| **metadata != ready)
            }) {
                return Err(Error::UnusableSample(
                    "shared region dependencies changed since the proposal",
                ));
            }
            let mut candidate = self.document.state.project.clone();
            for entry in &edit.entries {
                failed_source = Some(entry.source);
                if !self.loaded_crop_source_current(entry.source, &entry.file) {
                    return Err(Error::UnusableSample("a source changed since the proposal"));
                }
                if let Some(crop) = entry.draft {
                    crop.region(entry.development.active_area.into())?;
                }
                candidate.source_mut(entry.source)?.crop = entry.draft;
            }
            failed_source = None;
            candidate.validate()?;
            Ok(candidate)
        })()
        .map_err(|error| (failed_source, error))?;
        let before = self.history_snapshot();
        self.document.state.project = candidate;
        if let Some(id) = self.document.visit.current_source {
            self.document.visit.crop = self
                .document
                .project
                .source(id)
                .expect("active source")
                .crop;
        }
        self.record_edit(before, "Auto-crop", None);
        self.document.visit.canvas.viewport.fit();
        self.status = format!("Auto-crop applied to {} photos", edit.entries.len());
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn begin_auto_crop_review(&mut self, photos: Vec<SourceId>, scope: &'static str) {
        if !self.prepare_tool() {
            return;
        }
        if let Some(review) = self.capture_crop_review(photos, scope) {
            AutoCropTool::new(Some(review)).install(self);
        }
    }

    #[cfg(test)]
    pub(super) fn finish_auto_crop(&mut self, apply: bool) {
        if self
            .current_tool()
            .is_some_and(|tool| tool.is::<AutoCropTool>())
        {
            self.hosted_action(if apply {
                tool::Action::Primary
            } else {
                tool::Action::Dismiss
            });
        }
    }

    #[cfg(test)]
    pub(super) fn auto_crop_valid(&self) -> bool {
        self.document
            .auto_crop
            .current()
            .is_some_and(|state| state.borrow().as_ref().is_some_and(|edit| edit.valid(self)))
    }

    #[cfg(test)]
    pub(super) fn adjust_auto_crop_margin(&mut self, margin: f64) {
        if let Some(state) = self.document.auto_crop.current()
            && let Some(edit) = state.borrow_mut().as_mut()
        {
            edit.adjust_margin(self, margin);
        }
    }
}

type Review = RefCell<Option<Draft>>;

/// Tests can observe retirement without retaining the unfinished batch.
#[cfg(test)]
#[derive(Default)]
pub(super) struct State(Weak<Review>);

#[cfg(test)]
impl State {
    fn current(&self) -> Option<Rc<Review>> {
        self.0.upgrade()
    }

    pub(super) fn active(&self) -> bool {
        self.current().is_some()
    }

    pub(super) fn reviewing(&self) -> bool {
        self.current().is_some_and(|state| state.borrow().is_some())
    }

    #[cfg(test)]
    pub(super) fn snapshot(&self) -> Option<Draft> {
        self.current()?.borrow().clone()
    }
}

pub(super) struct AutoCropTool {
    review: Rc<Review>,
    preparation: Option<async_executor::Task<()>>,
    viewport: Viewport,
    visit: Option<u64>,
}

impl AutoCropTool {
    fn new(review: Option<Draft>) -> Self {
        Self {
            review: Rc::new(RefCell::new(review)),
            preparation: None,
            viewport: Viewport::default(),
            visit: None,
        }
    }

    fn install(self, editor: &mut Editor) {
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
            tool::Action::Primary
                if !editor.interaction_blocked() && editor.fullscreen.is_none() =>
            {
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
}

impl Draft {
    fn editable(&self, editor: &Editor) -> bool {
        !editor.interaction_blocked() && editor.fullscreen.is_none() && self.valid(editor)
    }

    fn warning(&self, editor: &Editor) -> Option<&'static str> {
        if self.entries.is_empty() {
            Some("No crop proposals are available for this batch")
        } else if !self.valid(editor) {
            Some("Crop batch is out of date; cancel and start again")
        } else {
            None
        }
    }

    fn problem(&self, editor: &Editor, id: SourceId) -> Option<String> {
        let entry = self.entries.iter().find(|entry| entry.source == id);
        let problem = self.problems.iter().find(|(source, _)| *source == id);
        if (entry.is_some() || problem.is_some()) && !self.belongs_to(editor) {
            return Some("Crop proposals are out of date".into());
        }
        if let Some(entry) = entry {
            if !entry.current(editor) {
                return Some("Crop proposal is out of date".into());
            }
            if !editor.interaction_blocked()
                && editor.document.visit.current_source == Some(id)
                && !editor.loaded_crop_source_current(id, &entry.file)
            {
                return Some("Loaded photo no longer matches the crop proposal".into());
            }
        }
        problem.map(|(_, reason)| reason.clone())
    }

    fn record_problem(&mut self, source: SourceId, reason: String) {
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

    fn adjust_margin(&mut self, editor: &mut Editor, percent: f64) {
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

pub(super) fn propose_photo_crop(
    editor: &mut Editor,
    photo: SourceId,
    state: std::rc::Rc<std::cell::RefCell<CropEdit>>,
    cx: &UiContext<'_, Editor>,
) -> Option<async_executor::Task<()>> {
    if editor.interaction_blocked()
        || editor.selected_area().is_none()
        || editor.document.visit.current_source != Some(photo)
    {
        return None;
    }
    editor.sync_document();
    let regions = editor.prepare_regions(cx)?;
    cx.spawn(move |cx| async move {
        if regions.await.is_none() {
            return;
        }
        let result = cx.read(|editor| editor.photo_crop_proposal(photo));
        let status = finish_photo_proposal(&mut state.borrow_mut(), result);
        cx.update(|editor| editor.status = status);
    })
}

impl Editor {
    pub(super) fn review_crop_proposals(
        &mut self,
        photos: Vec<SourceId>,
        scope: &'static str,
        cx: &UiContext<'_, Self>,
    ) {
        if photos.is_empty() || !self.auto_crop_entry_available() || !self.prepare_tool() {
            return;
        }
        let Some(regions) = self.prepare_regions(cx) else {
            return;
        };
        let mut tool = AutoCropTool::new(None);
        let state = std::rc::Rc::downgrade(&tool.review);
        let Some(task) = cx.spawn(move |cx| async move {
            if regions.await.is_none() {
                cx.update(Editor::end_tool);
                return;
            }
            cx.update(|editor| {
                let Some(state) = state.upgrade() else { return };
                match editor.capture_crop_review(photos, scope) {
                    Some(review) => *state.borrow_mut() = Some(review),
                    None => editor.end_tool(),
                }
            });
        }) else {
            return;
        };
        tool.preparation = Some(task);
        tool.install(self);
    }
}

#[derive(Clone, Copy)]
struct MarginDrag {
    source: SourceId,
    proposal: Crop,
    handle: [i8; 2],
    start: [f64; 2],
    percent: f64,
}

impl MarginDrag {
    fn margin(self, point: SourcePoint) -> f64 {
        let local = self.proposal.local_point(point);
        let delta = (0..2)
            .filter(|&i| self.handle[i] != 0)
            .map(|i| (self.start[i] - local[i]) * f64::from(self.handle[i]))
            .max_by(|a, b| a.abs().total_cmp(&b.abs()))
            .unwrap_or(0.);
        let percent = self.percent
            + delta * 100. / f64::from(self.proposal.size[0].min(self.proposal.size[1]));
        ((percent.clamp(0., framing::MAX_MARGIN_PERCENT) * 100.).round() / 100.)
            .clamp(0., framing::MAX_MARGIN_PERCENT)
    }
}

fn hit(q: [egui::Pos2; 4], point: egui::Pos2) -> Option<[i8; 2]> {
    if let Some((_, h)) = crop_editor::outline_handles(q)
        .into_iter()
        .find(|(p, _)| p.distance(point) <= 9.)
    {
        return Some(h);
    }
    [
        (0, 3, [-1, 0]),
        (1, 2, [1, 0]),
        (0, 1, [0, -1]),
        (3, 2, [0, 1]),
    ]
    .into_iter()
    .find_map(|(a, b, h)| {
        let d = q[b] - q[a];
        let t = ((point - q[a]).dot(d) / d.length_sq()).clamp(0., 1.);
        (point.distance(q[a] + d * t) <= 6.).then_some(h)
    })
}

fn cursor(q: [egui::Pos2; 4], handle: [i8; 2]) -> egui::CursorIcon {
    let d = (q[1] - q[0]).normalized() * f32::from(handle[0])
        + (q[3] - q[0]).normalized() * f32::from(handle[1]);
    match ((d.angle() / std::f32::consts::FRAC_PI_4).round() as i32).rem_euclid(4) {
        0 => egui::CursorIcon::ResizeHorizontal,
        1 => egui::CursorIcon::ResizeNwSe,
        2 => egui::CursorIcon::ResizeVertical,
        _ => egui::CursorIcon::ResizeNeSw,
    }
}

impl Draft {
    fn context(&self, editor: &Editor) -> bool {
        editor.fullscreen.is_none() && self.belongs_to(editor)
    }

    fn margin_entry(&self, editor: &Editor) -> Option<&Entry> {
        let id = editor.document.visit.current_source?;
        self.entries
            .iter()
            .find(|entry| entry.source == id)
            .filter(|entry| {
                entry.current(editor) && editor.loaded_crop_source_current(id, &entry.file)
            })
    }

    fn artwork_visible(&self, editor: &Editor, ui: &egui::Ui) -> bool {
        self.context(editor) && self.show_outline && !ui.input(|input| input.key_down(egui::Key::H))
    }

    fn outline(&self, editor: &Editor, layout: CanvasLayout) -> Option<[egui::Pos2; 4]> {
        if !self.context(editor) {
            return None;
        }
        let entry = self.margin_entry(editor)?;
        let crop = entry
            .draft
            .unwrap_or(Crop::from_region(entry.development.active_area.into()));
        Some(
            crop.coverage()
                .ok()?
                .corners()
                .map(|point| layout.screen_frame_point(layout.composition.source_to_frame(point))),
        )
    }

    fn margin_interaction(
        &mut self,
        editor: &mut Editor,
        ui: &egui::Ui,
        response: &egui::Response,
        layout: CanvasLayout,
    ) -> (bool, Option<egui::CursorIcon>) {
        if !ui.is_enabled()
            || !response.enabled()
            || !self.artwork_visible(editor, ui)
            || !self.editable(editor)
            || ui.input(|input| input.key_down(egui::Key::Space))
        {
            self.drag = None;
            return (false, None);
        }
        let Some(outline) = self.outline(editor, layout) else {
            self.drag = None;
            return (false, None);
        };
        let press = ui
            .input(|input| input.pointer.press_origin())
            .and_then(|point| hit(outline, point).map(|handle| (point, handle)));
        if response.drag_started_by(egui::PointerButton::Primary) {
            self.drag = None;
            if let Some((point, handle)) = press {
                let entry = self.margin_entry(editor).expect("outline retains source");
                self.drag = Some(MarginDrag {
                    source: entry.source,
                    proposal: entry.proposal,
                    handle,
                    start: entry.proposal.local_point(
                        layout
                            .composition
                            .frame_to_source(layout.frame_point(point)),
                    ),
                    percent: editor.auto_crop_margin,
                });
            }
        }
        let drag = self.drag.filter(|drag| {
            editor.document.visit.current_source == Some(drag.source)
                && self
                    .margin_entry(editor)
                    .is_some_and(|entry| entry.proposal == drag.proposal)
        });
        let claimed = drag.is_some() && response.dragged_by(egui::PointerButton::Primary)
            || press.is_some() && response.clicked_by(egui::PointerButton::Primary);
        if let Some(drag) = drag
            && response.dragged_by(egui::PointerButton::Primary)
            && let Some(point) = response.interact_pointer_pos()
        {
            let percent = drag.margin(
                layout
                    .composition
                    .frame_to_source(layout.frame_point(point)),
            );
            if percent != editor.auto_crop_margin {
                self.adjust_margin(editor, percent);
            }
        }
        let handle = if response.dragged_by(egui::PointerButton::Primary) {
            drag.map(|drag| drag.handle)
        } else {
            response.hover_pos().and_then(|point| hit(outline, point))
        };
        if response.drag_stopped_by(egui::PointerButton::Primary) {
            self.drag = None;
        }
        (claimed, handle.map(|handle| cursor(outline, handle)))
    }
}

impl Editor {
    #[cfg(test)]
    pub(super) fn auto_crop_context(&self) -> bool {
        self.document.auto_crop.current().is_some_and(|state| {
            state
                .borrow()
                .as_ref()
                .is_some_and(|draft| draft.context(self))
        })
    }

    #[cfg(test)]
    pub(super) fn auto_crop_outline(&self, layout: CanvasLayout) -> Option<[egui::Pos2; 4]> {
        let state = self.document.auto_crop.current()?;
        let state = state.borrow();
        state.as_ref()?.outline(self, layout)
    }
}

#[cfg(test)]
mod interaction_tests;
#[cfg(test)]
mod operation_tests;
#[cfg(test)]
mod overlay_tests;
#[cfg(test)]
mod tests;
