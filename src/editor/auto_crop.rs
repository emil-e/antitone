use super::*;
use crate::{auto_crop as framing, geometry::SourcePoint, viewport::CanvasLayout};
#[cfg(test)]
use std::rc::Weak;
use std::{cell::RefCell, rc::Rc};

mod review;
use review::{Draft, Entry, Inputs, LoadedPhoto, Rejection, SelectedPhoto, accepted, reason};

#[cfg(test)]
mod testing;
#[cfg(test)]
pub(super) use testing::declared_picture_regions;

fn review_inputs(editor: &Editor) -> Inputs<'_> {
    let scope = &editor.document;
    let selected = scope.visit.current_source.map(|id| SelectedPhoto {
        id,
        effective_crop: editor.photo_crop(),
        loaded: editor
            .active_canvas()
            .ready
            .as_ref()
            .map(|ready| LoadedPhoto {
                file: &ready.file,
                development: &ready.record.development,
                area: ready.area,
            }),
    });
    Inputs::new(
        &scope.state,
        selected,
        scope.spatial.ready(),
        &scope.initialization,
    )
}

pub(super) fn review_available(editor: &Editor) -> bool {
    editor.tool_start_available()
        && editor.selected_area().is_some()
        && !editor.document.project.order.is_empty()
}

fn capture_review(
    editor: &mut Editor,
    photos: Vec<SourceId>,
    scope: &'static str,
) -> Option<Draft> {
    if photos.is_empty() || !review_available(editor) {
        return None;
    }
    let draft = Draft::capture(
        &review_inputs(editor),
        photos,
        scope,
        editor.auto_crop_margin,
    );
    editor.status.clone_from(&draft.summary);
    Some(draft)
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

#[derive(Default)]
pub(super) struct ReviewView {
    pub viewport: Viewport,
    drag: Option<MarginDrag>,
}

pub(super) struct AutoCropTool {
    review: Rc<Review>,
    preparation: Option<async_executor::Task<()>>,
}

impl AutoCropTool {
    fn new(review: Option<Draft>) -> Self {
        Self {
            review: Rc::new(RefCell::new(review)),
            preparation: None,
        }
    }

    fn install(self, editor: &mut Editor) {
        editor.document.visit.auto_crop_view = ReviewView::default();
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
        let (warning, enabled) = self
            .review
            .borrow()
            .as_ref()
            .map_or((None, false), |draft| {
                let inputs = review_inputs(editor);
                let blocked = editor.interaction_blocked();
                let warning = inputs
                    .selected
                    .as_ref()
                    .and_then(|photo| draft.problem(&inputs, photo.id, !blocked))
                    .or_else(|| draft.warning(&inputs).map(str::to_owned));
                (warning, !blocked && draft.valid(&inputs))
            });
        tool::Presentation {
            identity: ui::ToolIdentity {
                icon: ui::icons::Icon::BatchCrop,
                title: "Auto-crop",
            },
            actions: tool::Actions::Confirm(tool::Primary {
                label: "Apply",
                enabled,
            }),
            warning,
            help: "Drag an edge or corner for the shared inward margin; Space-drag / middle-drag pans.\nBrowsing is optional and never retargets the proposals · H hides artwork.",
            navigation_help: None,
            fit_label: "Fit",
        }
    }

    fn viewport<'a>(&'a mut self, editor: &'a mut Editor) -> &'a mut Viewport {
        if self.review.borrow().is_some() {
            &mut editor.document.visit.auto_crop_view.viewport
        } else {
            &mut editor.active_canvas_mut().viewport
        }
    }

    fn roll_preview(&self, editor: &Editor) -> tool::RollPreview {
        let mut preview = tool::RollPreview::default();
        if let Some(draft) = self.review.borrow().as_ref() {
            let inputs = review_inputs(editor);
            let project = &inputs.document.project;
            if draft.belongs_to(&inputs) {
                for entry in &draft.entries {
                    if entry.current(&inputs)
                        && let Ok(source) = project.source(entry.source)
                    {
                        let composition = editor
                            .source_composition(source)
                            .cropped(entry.crop())
                            .expect("validated proposal");
                        preview
                            .images
                            .compositions
                            .push((entry.source, composition));
                    }
                }
            }
            let check_loaded = !editor.interaction_blocked();
            for id in project.order.iter().copied() {
                if let Some(problem) = draft.problem(&inputs, id, check_loaded) {
                    preview.warnings.push((id, problem));
                }
            }
        }
        preview
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
                if let Some(draft) = tool.review.borrow().as_ref() {
                    let inputs = review_inputs(editor);
                    if draft.artwork_visible(canvas.ui)
                        && let Some(outline) = draft.outline(&inputs, layout)
                    {
                        crop_editor::paint_outline(canvas.ui, layout.canvas, outline, false, 64);
                    }
                }
                Some(layout)
            },
        )
    }

    fn action(&mut self, action: tool::Action, editor: &mut Editor, _cx: &UiContext<'_, Editor>) {
        match action {
            tool::Action::Primary if !editor.interaction_blocked() => {
                let Some(snapshot) = self.review.borrow().clone() else {
                    return;
                };
                if snapshot.entries.is_empty() {
                    editor.status = "Auto-crop has no proposals to apply".into();
                    return;
                }
                match apply_review(editor, &snapshot) {
                    Ok(()) => {
                        editor.fit_photo_view();
                        editor.status =
                            format!("Auto-crop applied to {} photos", snapshot.entries.len());
                        editor.end_tool();
                    }
                    Err(Rejection { source, error }) => {
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

fn apply_review(editor: &mut Editor, draft: &Draft) -> Result<(), Rejection> {
    let mut transaction = editor.edit_project("Auto-crop").map_err(Rejection::batch)?;
    let candidate = draft.candidate(&review_inputs(editor))?;
    transaction.replace_snapshot(candidate);
    editor.commit_project(transaction).map_err(Rejection::batch)
}

impl Draft {
    fn warning(&self, inputs: &Inputs<'_>) -> Option<&'static str> {
        if self.entries.is_empty() {
            Some("No crop proposals are available for this batch")
        } else if !self.valid(inputs) {
            Some("Crop batch is out of date; cancel and start again")
        } else {
            None
        }
    }

    fn controls(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        if !editor.document.project.input.capabilities().auto_crop {
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
        let editable = !editor.interaction_blocked() && self.valid(&review_inputs(editor));
        ui::ToolIdentity {
            icon: ui::icons::Icon::BatchCrop,
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
                        ui::ParameterRow::new(
                            "crop-margin",
                            "Margin",
                            0.0..=crate::auto_crop::MAX_MARGIN_PERCENT,
                        )
                        .default_value(crate::auto_crop::DEFAULT_MARGIN_PERCENT)
                        .unit("%")
                        .help(
                            "Trim each edge by a percentage of the detected frame's shorter side.",
                        )
                        .show(ui, &mut margin);
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
        let inputs = review_inputs(editor);
        if editor.interaction_blocked() || !self.valid(&inputs) {
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
                if !inputs.loaded_source_matches(entry.source, &entry.file) {
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

/// Admit the request immediately; cancelled or failed region preparation owns its own feedback.
pub(super) fn propose_photo_crop<'scope>(
    editor: &mut Editor,
    photo: SourceId,
    cx: &UiContext<'scope, Editor>,
) -> Option<impl Future<Output = Option<Result<Crop, String>>> + use<'scope>> {
    if editor.interaction_blocked()
        || editor.selected_area().is_none()
        || editor.document.visit.current_source != Some(photo)
    {
        return None;
    }

    let regions = editor.prepare_regions(cx)?;
    let cx = cx.clone();
    Some(async move {
        regions.await?;
        Some(cx.read(|editor| {
            let entry = Entry::propose(&review_inputs(editor), photo, editor.auto_crop_margin)
                .map_err(reason)?;
            Ok(entry.crop())
        }))
    })
}

pub(super) fn start_review(
    editor: &mut Editor,
    photos: Vec<SourceId>,
    scope: &'static str,
    cx: &UiContext<'_, Editor>,
) {
    if photos.is_empty() || !review_available(editor) || !editor.prepare_tool() {
        return;
    }
    let Some(regions) = editor.prepare_regions(cx) else {
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
            match capture_review(editor, photos, scope) {
                Some(review) => *state.borrow_mut() = Some(review),
                None => editor.end_tool(),
            }
        });
    }) else {
        return;
    };
    tool.preparation = Some(task);
    tool.install(editor);
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
    fn artwork_visible(&self, ui: &egui::Ui) -> bool {
        self.show_outline && !ui.input(|input| input.key_down(egui::Key::H))
    }

    fn outline(&self, inputs: &Inputs<'_>, layout: CanvasLayout) -> Option<[egui::Pos2; 4]> {
        if !self.belongs_to(inputs) {
            return None;
        }
        self.margin_entry(inputs)?.outline(layout)
    }

    fn margin_interaction(
        &mut self,
        editor: &mut Editor,
        ui: &egui::Ui,
        response: &egui::Response,
        layout: CanvasLayout,
    ) -> (bool, Option<egui::CursorIcon>) {
        let inputs = review_inputs(editor);
        if !ui.is_enabled()
            || !response.enabled()
            || !self.artwork_visible(ui)
            || editor.interaction_blocked()
            || ui.input(|input| input.key_down(egui::Key::Space))
            || !self.valid(&inputs)
        {
            editor.document.visit.auto_crop_view.drag = None;
            return (false, None);
        }
        let Some(entry) = self.margin_entry(&inputs) else {
            editor.document.visit.auto_crop_view.drag = None;
            return (false, None);
        };
        let Some(outline) = entry.outline(layout) else {
            editor.document.visit.auto_crop_view.drag = None;
            return (false, None);
        };

        let press = ui
            .input(|input| input.pointer.press_origin())
            .and_then(|point| hit(outline, point).map(|handle| (point, handle)));
        let drag = &mut editor.document.visit.auto_crop_view.drag;
        if response.drag_started_by(egui::PointerButton::Primary) {
            *drag = press.map(|(point, handle)| MarginDrag {
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
        let drag =
            drag.filter(|drag| drag.source == entry.source && drag.proposal == entry.proposal);
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
            editor.document.visit.auto_crop_view.drag = None;
        }
        (claimed, handle.map(|handle| cursor(outline, handle)))
    }
}

impl Entry {
    fn outline(&self, layout: CanvasLayout) -> Option<[egui::Pos2; 4]> {
        Some(
            self.crop()
                .coverage()
                .ok()?
                .corners()
                .map(|point| layout.screen_frame_point(layout.composition.source_to_frame(point))),
        )
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
