use super::document_scope::SamplingInputs;
use super::*;
use crate::source::SourceProvider;
use crate::{
    background, cancellation::Cancellation, project::SourceAnalysis, scan_regions::Areas,
    viewport::CanvasLayout,
};
use futures_channel::oneshot;
#[cfg(test)]
use std::sync::Mutex;
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};

use crate::roll_analysis::{self, FrameSamples, MAX_SAMPLES_PER_FRAME};

#[cfg(all(test, feature = "ocio-reference"))]
mod diagnostics;
#[cfg(test)]
mod tests;

#[derive(Clone, Copy)]
pub(super) enum Orientation {
    Left,
    Right,
    Horizontal,
    Vertical,
    Reset,
}

#[derive(Clone)]
pub(super) struct Request {
    pub document: Document,
    pub selected: SourceId,
    pub film: Option<Rect>,
    pub local_support: bool,
    pub picture: Option<Rect>,
    pub areas: Option<Approved>,
}

pub(super) mod fit;

#[cfg(test)]
mod acknowledgement_tests;
#[cfg(test)]
mod fit_tests;
#[cfg(test)]
mod interaction_tests;
#[cfg(test)]
mod operation_tests;
#[cfg(test)]
mod ownership_tests;
#[cfg(all(test, target_os = "macos"))]
mod preparation_async_tests;
#[cfg(all(test, target_os = "macos"))]
pub(super) mod preparation_tests;
#[cfg(test)]
pub(super) mod testing;
#[cfg(test)]
mod workflow_tests;
// Resolve the 0.5% picture tail and 1.5% film band with more than singleton ranks.
const MIN_PICTURE_SAMPLES: usize = 512;
const MIN_ROLL_FILM_SAMPLES: usize = 2048;
const MIN_BALANCE_SAMPLES: usize = 256;

fn require_picture(samples: &crate::scan::AnalysisSamples) -> Result<(), Error> {
    if samples.pixels.len() < MIN_PICTURE_SAMPLES {
        return Err(Error::Project(format!(
            "only {} of {} picture samples are eligible; need {MIN_PICTURE_SAMPLES}. Enlarge the picture area or check the capture",
            samples.pixels.len(),
            samples.attempted
        )));
    }
    Ok(())
}

fn require_statistics(
    film_count: usize,
    frames: &[roll_analysis::FrameStatistics],
) -> Result<(), Error> {
    if film_count < MIN_ROLL_FILM_SAMPLES {
        return Err(Error::UnusableSample(
            "too few eligible roll film samples for Dmin",
        ));
    }
    if frames.iter().any(|frame| {
        frame.balance_samples < MIN_BALANCE_SAMPLES || frame.exposure_samples < MIN_PICTURE_SAMPLES
    }) {
        return Err(Error::UnusableSample(
            "too few positive picture statistics; adjust the picture area or check the capture",
        ));
    }
    Ok(())
}

struct FrameMeasurements {
    floor: f64,
    film: crate::scan::AnalysisSamples,
    picture: crate::scan::AnalysisSamples,
    spatial: crate::gpu::analysis::CameraWindow,
}

fn restrict_to_common_floor(frames: &mut [FrameMeasurements]) -> Result<f64, Error> {
    let floor = frames.iter().map(|frame| frame.floor).fold(0., f64::max);
    for frame in frames {
        for samples in [&mut frame.film, &mut frame.picture] {
            let count = samples.pixels.len();
            samples.pixels.retain(|p| p.min_element() > floor);
            samples.rejected_floor += count - samples.pixels.len();
        }
        require_picture(&frame.picture)?;
    }
    Ok(floor)
}

pub(super) struct Outcome {
    document: Document,
    frames: usize,
    wb_rejections: Vec<(SourceId, String)>,
    support: Rect,
}

pub(super) fn rect([x, y, width, height]: [u32; 4]) -> Rect {
    Rect {
        x,
        y,
        width,
        height,
    }
}

// Source-scaled statistical inset, rounded inward to whole source pixels. This is
// sampling policy, not a displacement or an uncertainty claim about physical edges.
pub(super) fn sampling_inset(active: Rect) -> f64 {
    (2. * (f64::from(active.width) / 768.).max(f64::from(active.height) / 512.)).ceil()
}

fn array(r: Rect) -> [u32; 4] {
    [r.x, r.y, r.width, r.height]
}

// An editable statistics prior inside illumination support, not detected picture edges.
pub(super) fn suggest(support: Rect) -> Rect {
    let dx = support.width / 20;
    let dy = support.height / 20;
    Rect {
        x: support.x + dx,
        y: support.y + dy,
        width: support.width - 2 * dx,
        height: support.height - 2 * dy,
    }
}

fn picture_area(explicit: Option<Rect>, support: Option<Rect>) -> Result<Rect, Error> {
    explicit
        .or_else(|| support.map(suggest))
        .ok_or(Error::UnusableSample(
            "no roll illumination-area suggestion; use Edit picture area to select the film picture, excluding the holder",
        ))
}

fn proposed_document(
    document: &Document,
    proposal: roll_analysis::Proposal,
    picture: Option<Rect>,
) -> Result<Document, Error> {
    if proposal.photos.len() != document.project.order.len() {
        return Err(Error::UnusableSample("analysis photo count changed"));
    }
    let mut candidate = document.clone();
    candidate.project.input = proposal.input;
    candidate.project.roll = proposal.roll;
    for (id, photo) in document.project.order.iter().zip(proposal.photos) {
        let source = candidate.project.source_mut(*id)?;
        let layers = source
            .adjustment_layers
            .get_or_insert_with(Default::default);
        layers.layers.clear();
        layers.add(
            "Global".into(),
            crate::adjustment_layers::AdjustmentLayer::global(photo),
        )?;
    }
    candidate.project.analysis_area = picture.map(array);
    Ok(candidate)
}

impl Editor {
    fn analysis_support(&self) -> Option<Rect> {
        self.document
            .correction
            .prepared()
            .map(|c| rect(c.settings.map.area()))
            .or_else(|| {
                self.document
                    .visit
                    .canvas
                    .ready
                    .as_ref()
                    .and_then(|r| r.analysis_support.flatten())
            })
    }

    pub(super) fn apply_roll_orientation(&mut self, action: Orientation) {
        let result = (|| -> Result<(), Error> {
            let mut draft = self.edit_project("Roll orientation")?;
            draft.update(|project| match action {
                Orientation::Left => project.orientation.turn_counterclockwise(),
                Orientation::Right => project.orientation.turn_clockwise(),
                Orientation::Horizontal => project.orientation.horizontal ^= true,
                Orientation::Vertical => project.orientation.vertical ^= true,
                Orientation::Reset => project.orientation = RollOrientation::default(),
            });
            self.commit_project(draft)
        })();
        if let Err(error) = result {
            self.status = format!("Orientation rejected: {error}");
            return;
        }
        self.document.visit.canvas.viewport.fit();
    }

    pub(super) fn roll_tool_shelf(&mut self, ui: &mut egui::Ui, cx: &UiContext<'_, Self>) {
        use ui::icons::Icon;
        use ui::{ToolButton, ToolShelf};
        let reviewing = self.document.analysis_review().is_some();
        let analyzing = self.document.analysis_operation().is_some();
        let capabilities = self.document.project.input.capabilities();
        let commands = !self.interaction_blocked()
            && self.selected_area().is_some()
            && !self.tool_interacting()
            && !self.dialogs.is_open();

        ToolShelf::show(ui, |ui| {
            for (icon, name, help, action) in [
                (
                    Icon::TurnLeft,
                    "Turn left",
                    "Turn the shared scan baseline 90° left.",
                    Orientation::Left,
                ),
                (
                    Icon::TurnRight,
                    "Turn right",
                    "Turn the shared scan baseline 90° right.",
                    Orientation::Right,
                ),
                (
                    Icon::FlipHorizontal,
                    "Flip horizontal",
                    "Flip all scans left/right.",
                    Orientation::Horizontal,
                ),
                (
                    Icon::FlipVertical,
                    "Flip vertical",
                    "Flip all scans top/bottom.",
                    Orientation::Vertical,
                ),
                (
                    Icon::Reset,
                    "Reset scan orientation",
                    "Reset the shared scan orientation.",
                    Orientation::Reset,
                ),
            ] {
                if ToolButton::command(icon, name, help)
                    .enabled(
                        commands
                            && (!matches!(action, Orientation::Reset)
                                || !self.document.project.orientation.is_identity()),
                    )
                    .show(ui)
                    .clicked()
                {
                    self.apply_roll_orientation(action);
                }
            }
            if capabilities.roll_analysis || reviewing {
                ui.separator();
                if ToolButton::tool(
                    Icon::Analyze,
                    "Analyze",
                    "Prepare and review sampling areas, then estimate the roll's starting grade.",
                    analyzing,
                )
                .enabled(self.tool_start_available() && self.selected_area().is_some())
                .show(ui)
                .clicked()
                {
                    if analyzing {
                        self.cancel_active_tool();
                    } else {
                        self.request_analysis(cx);
                    }
                }
            }
            if capabilities.auto_crop {
                let cropping = self
                    .current_tool()
                    .is_some_and(|tool| tool.is::<auto_crop::AutoCropTool>());
                if ToolButton::tool(
                    Icon::Crop,
                    "Auto-crop roll",
                    "Propose crop and straightening for the whole roll.",
                    cropping,
                )
                .enabled(cropping || self.auto_crop_entry_available())
                .show(ui)
                .clicked()
                {
                    if cropping {
                        self.cancel_active_tool();
                    } else {
                        self.review_crop_proposals(
                            self.document.project.order.clone(),
                            "Whole roll",
                            cx,
                        );
                    }
                }
            }
        });
    }

    pub(super) fn analysis_request(&mut self) -> Option<Request> {
        if self.interaction_blocked()
            || self.selected_area().is_none()
            || self.document.project.order.is_empty()
        {
            return None;
        }
        Some(Request {
            document: self.document.capture(),
            selected: self.document.visit.current_source?,
            film: self.analysis_support(),
            local_support: self
                .document
                .visit
                .canvas
                .ready
                .as_ref()
                .is_some_and(|ready| ready.analysis_support.is_none()),
            picture: self.document.state.project.analysis_area.map(rect),
            areas: None,
        })
    }

    #[cfg(test)]
    pub(super) fn begin_analysis(&mut self, cx: &UiContext<'_, Self>) {
        let Some(request) = self.analysis_request() else {
            return;
        };
        self.start_analysis(request, cx);
    }

    pub(super) fn accept_analysis(
        &mut self,
        snapshot: &Request,
        result: Result<Option<Box<Outcome>>, String>,
        cancelled: bool,
        cx: &UiContext<'_, Self>,
    ) {
        if cancelled {
            self.status = "Analysis cancelled".into();
            self.end_analysis_review(cx);
            return;
        }
        match result {
            Ok(Some(outcome)) => {
                if let Err(error) = verify(snapshot) {
                    self.status = format!("Analysis discarded: {error}; grade unchanged");
                    self.end_analysis_review(cx);
                    return;
                }
                let mut transaction = match self.edit_project("Analyze roll") {
                    Ok(edit) => edit,
                    Err(error) => {
                        self.status = format!("Analysis rejected: {error}");
                        self.end_analysis_review(cx);
                        return;
                    }
                };
                if snapshot.areas.is_none()
                    && let Some(ready) = &mut self.document.visit.canvas.ready
                {
                    ready.analysis_support = Some(Some(outcome.support));
                }
                transaction.replace_snapshot(outcome.document.project.snapshot());
                if let Err(error) = self.commit_project(transaction) {
                    self.status = format!("Analysis rejected: {error}");
                    self.end_analysis_review(cx);
                    return;
                }
                self.document.install(outcome.document);
                self.document.visit.adjustment_layers = AdjustmentLayerEditor::default();
                self.document.source = false;
                self.document.visit.working_view.clear();
                self.invalidate_original();
                self.status = format!(
                    "Analyzed {} frames. Adjust normally; re-pick Dmin if needed.",
                    outcome.frames
                );
                if !outcome.wb_rejections.is_empty() {
                    self.status.push_str(&format!(
                        " Shared WB used {}/{} frames.",
                        outcome.frames - outcome.wb_rejections.len(),
                        outcome.frames
                    ));
                }
            }
            Ok(None) => self.status = "Analysis cancelled".into(),
            Err(error) => self.status = format!("Analysis failed: {error}. Grade unchanged."),
        }
        self.end_analysis_review(cx);
    }
}

pub(super) struct AnalyzeTool {
    pub operation: Operation,
    pub review: Option<Review>,
    pub(super) drag: Option<PickDrag>,
    pub(super) viewport: Viewport,
    pub(super) visit: Option<u64>,
    fullscreen_viewport: Option<Viewport>,
}

impl AnalyzeTool {
    pub fn new(operation: Operation) -> Self {
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
        matches!(self.operation.phase, Phase::Fitting)
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
            identity: ui::ToolIdentity {
                icon: ui::icons::Icon::Analyze,
                title: "Analyze",
            },
            primary: Some(tool::Primary {
                label: "Analyze roll",
                enabled: !editor.interaction_blocked()
                    && !self.fitting()
                    && self.review.as_ref().is_some_and(|review| {
                        review
                            .approved
                            .entries
                            .iter()
                            .all(|entry| entry.areas.ready())
                    })
                    && editor.selected_area().is_some(),
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
                    && !editor.interaction_blocked()
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
            Phase::Regions => {
                editor.cancel_spatial();
                editor.status = "Analysis cancelled; areas and grade unchanged".into();
                editor.end_tool();
            }
            Phase::Review => {
                editor.status = "Analysis review cancelled; areas and grade unchanged".into();
                editor.end_tool();
            }
            Phase::Preparing | Phase::Fitting => {
                self.operation.cancelled.cancel();

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
    pub fn analysis_operation(&self) -> Option<&Operation> {
        Some(&self.analyze_tool()?.operation)
    }
    pub fn analysis_operation_mut(&mut self) -> Option<&mut Operation> {
        Some(&mut self.analyze_tool_mut()?.operation)
    }
    pub fn set_analysis_operation(&mut self, operation: Option<Operation>) {
        if let Some(operation) = operation {
            self.set_hosted_tool(
                tool::Placement::Document,
                Box::new(AnalyzeTool::new(operation)),
            );
        } else if self.hosted_tool.is::<AnalyzeTool>() {
            self.hosted_tool = tool::Slot::Empty;
        }
    }
    pub fn analysis_review(&self) -> Option<&Review> {
        self.analyze_tool()?.review.as_ref()
    }
    #[cfg(test)]
    pub fn analysis_review_mut(&mut self) -> Option<&mut Review> {
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

#[derive(Clone)]
pub(super) struct Approved {
    pub entries: Vec<SourceAnalysis>,
    pub files: Vec<thumbnails::FileKey>,
}

pub(super) struct Review {
    pub document: Document,
    pub approved: Approved,
    pub picture: bool,
    pub replaces_photos: bool,
}

fn review_label(canvas: egui::Rect, q: [egui::Pos2; 4], picture: bool) -> egui::Rect {
    let p = q[0] + egui::vec2(5., 4.);
    let p = egui::pos2(
        p.x.clamp(
            canvas.left() + 4.,
            (canvas.right() - 54.).max(canvas.left() + 4.),
        ),
        p.y.clamp(
            canvas.top() + 4.,
            (canvas.bottom() - 32.).max(canvas.top() + 4.),
        ) + if picture { 16. } else { 0. },
    );
    egui::Rect::from_min_size(p, egui::vec2(50., 14.))
}

fn verify_file(document: &Document, approved: &Approved) -> Result<(), Error> {
    if approved.files.len() != document.project.order.len()
        || approved.entries.len() != approved.files.len()
    {
        return Err(Error::UnusableSample("incomplete analysis-area review"));
    }
    for (i, id) in document.project.order.iter().enumerate() {
        let path = document.source_path(*id)?;
        if document.file_key(&path) != approved.files[i] {
            return Err(Error::UnusableSample(
                "source changed since analysis-area review",
            ));
        }
        let row = &approved.entries[i];
        if row.source != *id || !row.areas.ready() {
            return Err(Error::UnusableSample(
                "missing or unordered reviewed analysis areas",
            ));
        }
        row.areas
            .validate(document.project.source(*id)?.development.active_area.into())?;
    }
    Ok(())
}

pub(super) fn verify(request: &analysis::Request) -> Result<(), Error> {
    verify_areas(&request.document, request.areas.as_ref())
}

fn verify_areas(document: &Document, approved: Option<&Approved>) -> Result<(), Error> {
    if !document.spatial_metadata.groups.is_empty()
        && document.ready_spatial_metadata() != *document.spatial_metadata
    {
        return Err(Error::UnusableSample(
            "supporting spatial inputs changed since analysis preparation",
        ));
    }
    if let Some(approved) = approved {
        verify_file(document, approved)?;
    }
    Ok(())
}

impl Editor {
    pub(super) fn open_analysis_review(
        &mut self,
        request: &analysis::Request,
        result: Result<Option<Approved>, String>,
    ) -> Option<futures_channel::oneshot::Receiver<Confirmation>> {
        self.with_analyze_tool(|tool, editor| tool.open_analysis_review(editor, request, result))
            .flatten()
    }
    pub(super) fn close_analysis(&mut self) {
        if self.document.hosted_tool.is::<AnalyzeTool>() {
            self.end_tool();
        }
    }
    pub(super) fn end_analysis_review(&mut self, _cx: &UiContext<'_, Self>) {
        self.close_analysis();
    }
    #[cfg(test)]
    pub(super) fn confirm_analysis_review(&mut self, cx: &UiContext<'_, Self>) {
        self.with_analyze_tool(|tool, editor| tool.confirm_analysis_review(editor, cx));
    }
    #[cfg(test)]
    pub(super) fn review_areas(&self, id: SourceId) -> Option<&Areas> {
        self.document.analyze_tool()?.review_areas(id)
    }
    #[cfg(test)]
    pub(super) fn review_area_at(
        &self,
        geometry: crate::viewport::CanvasLayout,
        point: egui::Pos2,
    ) -> Option<bool> {
        self.document
            .analyze_tool()?
            .review_area_at(self, geometry, point)
    }
    #[cfg(test)]
    pub(super) fn select_review_area(
        &mut self,
        geometry: crate::viewport::CanvasLayout,
        point: egui::Pos2,
    ) {
        self.with_analyze_tool(|tool, editor| tool.select_review_area(editor, geometry, point));
    }
}

impl AnalyzeTool {
    pub(super) fn open_analysis_review(
        &mut self,
        editor: &mut Editor,
        request: &analysis::Request,
        result: Result<Option<Approved>, String>,
    ) -> Option<futures_channel::oneshot::Receiver<Confirmation>> {
        match result {
            Ok(Some(approved)) => {
                let replaces_photos = request.document.project.order.iter().any(|id| {
                    request.document.project.source(*id).is_ok_and(|s| {
                        s.grade_snapshot().is_ok_and(|snapshot| {
                            !snapshot.layers().is_empty()
                                && snapshot.layers()
                                    != [crate::adjustment_layers::AdjustmentLayer::global(
                                        AdjustmentLayerSettings::default(),
                                    )]
                        })
                    })
                });
                self.review = Some(Review {
                    document: request.document.clone(),
                    approved,
                    picture: false,
                    replaces_photos,
                });
                editor.tab = 0;
                self.visit = Some(editor.document.visit.view);
                self.drag = None;
                self.viewport.fit();
                editor.status = "Review sampling areas".into();
                let (decision, result) = futures_channel::oneshot::channel();
                self.operation.decision = Some(decision);
                return Some(result);
            }
            Ok(None) => {
                editor.status = "Analysis-area preparation cancelled; grade unchanged".into()
            }
            Err(error) => {
                editor.status = format!("Area preparation failed: {error}. Grade unchanged.")
            }
        }
        None
    }

    pub(super) fn review_areas(&self, id: SourceId) -> Option<&Areas> {
        self.review
            .as_ref()?
            .approved
            .entries
            .iter()
            .find(|r| r.source == id)
            .map(|r| &r.areas)
    }

    pub(super) fn selected_region(&self, id: SourceId) -> Option<Coverage> {
        let review = self.review.as_ref()?;
        let areas = self.review_areas(id)?;
        let quad = if review.picture {
            areas.picture
        } else {
            areas.opening
        }?;
        Areas::coverage(quad).ok()
    }

    pub(super) fn edit_region(&mut self, source: SourceId, region: Coverage) {
        if self.fitting() {
            return;
        }
        let Some(review) = self.review.as_mut() else {
            return;
        };
        let Some(entry) = review
            .approved
            .entries
            .iter_mut()
            .find(|row| row.source == source)
        else {
            return;
        };
        let old = if review.picture {
            entry.areas.picture
        } else {
            entry.areas.opening
        };
        if old == Some(region.corners().map(|point| point.0)) {
            return;
        }
        if review.picture {
            entry.areas.edit_picture(region);
        } else {
            entry.areas.edit_opening(region);
            let active = review
                .document
                .project
                .source(source)
                .expect("review source")
                .development
                .active_area
                .into();
            entry.areas.refit_picture(active);
        }
    }

    pub(super) fn confirm_analysis_review(
        &mut self,
        editor: &mut Editor,
        _cx: &UiContext<'_, Editor>,
    ) {
        if editor.interaction_blocked() || editor.selected_area().is_none() || self.fitting() {
            return;
        }
        let Some(review) = &self.review.as_ref() else {
            return;
        };
        if let Err(error) = verify_areas(&review.document, Some(&review.approved)) {
            editor.status = format!("Cannot analyze: {error}. Grade unchanged.");
            return;
        }
        self.submit_analysis_review(editor, review.approved.clone());
    }

    pub(super) fn review_area_at(
        &self,
        editor: &Editor,
        geometry: crate::viewport::CanvasLayout,
        point: egui::Pos2,
    ) -> Option<bool> {
        let review = self.review.as_ref()?;
        let areas = self.review_areas(editor.document.visit.current_source?)?;
        for (picture, quad) in [(false, areas.opening), (true, areas.picture)] {
            if let Some(quad) = quad
                && let Ok(coverage) = Areas::coverage(quad)
            {
                let q = coverage
                    .corners()
                    .map(|p| geometry.screen_frame_point(geometry.composition.source_to_frame(p)));
                if review_label(geometry.canvas, q, picture).contains(point) {
                    return Some(picture);
                }
            }
        }
        if areas.opening.is_none() {
            return Some(false);
        }
        if areas.picture.is_none() {
            return Some(true);
        }
        let mut nearest = None;
        for (picture, quad) in [(false, areas.opening), (true, areas.picture)] {
            let coverage = Areas::coverage(quad?).ok()?;
            let q = coverage
                .corners()
                .map(|p| geometry.screen_frame_point(geometry.composition.source_to_frame(p)));
            let distance = (0..4)
                .map(|i| {
                    let edge_start = q[i];
                    let edge = q[(i + 1) % 4] - edge_start;
                    let t = ((point - edge_start).dot(edge) / edge.length_sq()).clamp(0., 1.);
                    point.distance(edge_start + edge * t)
                })
                .min_by(f32::total_cmp)
                .unwrap();
            if distance <= 8.
                && nearest.is_none_or(|(best, selected): (f32, bool)| {
                    distance < best - 0.01
                        || (distance - best).abs() <= 0.01
                            && picture == review.picture
                            && selected != review.picture
                })
            {
                nearest = Some((distance, picture));
            }
        }
        if let Some((_, picture)) = nearest {
            return Some(picture);
        }
        for (picture, quad) in [(true, areas.picture), (false, areas.opening)] {
            let q = Areas::coverage(quad?)
                .ok()?
                .corners()
                .map(|p| geometry.screen_frame_point(geometry.composition.source_to_frame(p)));
            let cross = (0..4)
                .map(|i| (q[(i + 1) % 4] - q[i]).rot90().dot(point - q[i]))
                .collect::<Vec<_>>();
            if cross.iter().all(|v| *v >= 0.) || cross.iter().all(|v| *v <= 0.) {
                return Some(picture);
            }
        }
        Some(review.picture)
    }

    pub(super) fn select_review_area(
        &mut self,
        editor: &mut Editor,
        geometry: crate::viewport::CanvasLayout,
        point: egui::Pos2,
    ) {
        if let Some(picture) = self.review_area_at(editor, geometry, point)
            && self.review.as_ref().is_some_and(|r| r.picture != picture)
        {
            self.review.as_mut().unwrap().picture = picture;
            self.drag = None;
        }
    }

    pub(super) fn paint_review_areas(
        &self,
        editor: &Editor,
        ui: &egui::Ui,
        geometry: crate::viewport::CanvasLayout,
        canvas: egui::Rect,
    ) {
        let Some(review) = &self.review.as_ref() else {
            return;
        };
        let other = editor
            .document
            .visit
            .current_source
            .and_then(|id| self.review_areas(id))
            .and_then(|a| if review.picture { a.opening } else { a.picture })
            .and_then(|q| Areas::coverage(q).ok());
        let painter = ui.painter().with_clip_rect(canvas);
        if let Some(material) = editor
            .document
            .visit
            .current_source
            .and_then(|id| self.review_areas(id))
            .and_then(|a| a.material.as_ref())
        {
            let area = geometry.composition.area();
            // Nearest-centre membership uses Voronoi bounds, not centre-to-centre cells.
            let cell = |axis: &[u32], index: usize, extent: u32| {
                [
                    if index == 0 {
                        0.
                    } else {
                        f64::from(axis[index - 1] + axis[index]) / 2. + 0.5
                    },
                    if index + 1 == axis.len() {
                        f64::from(extent)
                    } else {
                        f64::from(axis[index] + axis[index + 1]) / 2. + 0.5
                    },
                ]
            };
            for (y, _) in material.ys.iter().enumerate() {
                for (x, _) in material.xs.iter().enumerate() {
                    if material.labels.as_bytes()[y * material.xs.len() + x] == b'F' {
                        continue;
                    }
                    let [l, r] = cell(&material.xs, x, material.size[0]);
                    let [t, b] = cell(&material.ys, y, material.size[1]);
                    let points = [[l, t], [r, t], [r, b], [l, b]].map(|p| {
                        geometry.screen_point(egui::pos2(
                            (f64::from(area.x)
                                + p[0] * f64::from(area.width) / f64::from(material.size[0]))
                                as f32,
                            (f64::from(area.y)
                                + p[1] * f64::from(area.height) / f64::from(material.size[1]))
                                as f32,
                        ))
                    });
                    painter.add(egui::Shape::convex_polygon(
                        points.to_vec(),
                        ui.visuals().weak_text_color().gamma_multiply(0.3),
                        egui::Stroke::NONE,
                    ));
                }
            }
        }
        for (coverage, active) in [
            (other, false),
            (
                editor
                    .document
                    .visit
                    .current_source
                    .and_then(|id| self.selected_region(id)),
                true,
            ),
        ] {
            let Some(coverage) = coverage else {
                continue;
            };
            let points = coverage
                .corners()
                .map(|p| geometry.screen_point(egui::pos2(p.0[0] as f32, p.0[1] as f32)));
            let stroke = if active {
                ui.visuals().selection.stroke
            } else {
                egui::Stroke::new(1., ui.visuals().text_color())
            };
            painter.add(egui::Shape::closed_line(
                points.to_vec(),
                egui::Stroke::new(3., egui::Color32::from_black_alpha(180)),
            ));
            painter.add(egui::Shape::closed_line(points.to_vec(), stroke));
            for p in points {
                painter.rect_filled(
                    egui::Rect::from_center_size(p, egui::vec2(8., 8.)),
                    0.,
                    egui::Color32::from_black_alpha(180),
                );
                painter.rect_filled(
                    egui::Rect::from_center_size(p, egui::vec2(6., 6.)),
                    0.,
                    stroke.color,
                );
            }
            let picture = if active {
                review.picture
            } else {
                !review.picture
            };
            let label = if picture { "Picture" } else { "Opening" };
            let point = review_label(canvas, points, picture).min;
            painter.text(
                point + egui::vec2(1., 1.),
                egui::Align2::LEFT_TOP,
                label,
                egui::FontId::proportional(11.),
                egui::Color32::BLACK,
            );
            painter.text(
                point,
                egui::Align2::LEFT_TOP,
                label,
                egui::FontId::proportional(11.),
                stroke.color,
            );
        }
    }

    pub(super) fn review_warning(&self, id: SourceId) -> Option<&'static str> {
        let areas = self.review_areas(id)?;
        match (areas.opening.is_some(), areas.picture.is_some()) {
            (false, false) => Some("Opening and picture sampling areas are missing"),
            (false, true) => Some("Opening sampling area is missing"),
            (true, false) => Some("Picture sampling area is missing"),
            _ => None,
        }
    }
}

fn review_windows(
    request: &analysis::Request,
    regions: &crate::spatial_metadata::SourceRegions,
) -> Result<Areas, Error> {
    let project = &request.document.project;
    let source = regions.input.source;
    let active = regions.input.development.active_area.into();
    let previous = project
        .analysis_regions
        .iter()
        .find(|row| row.source == source);
    let mut areas = match previous {
        Some(row)
            if row.areas.ready() && (row.areas.opening_edited || row.areas.opening_shared) =>
        {
            row.areas.clone()
        }
        _ => {
            let mut suggested = regions.sampling_areas();
            if let Some(previous) = previous {
                if previous.areas.opening_edited
                    && let Some(opening) = previous.areas.opening
                {
                    suggested.edit_opening(Areas::coverage(opening)?);
                    suggested.refit_picture(active);
                }
                if previous.areas.picture_edited
                    && let Some(picture) = previous.areas.picture
                {
                    suggested.edit_picture(Areas::coverage(picture)?);
                }
            }
            suggested
        }
    };
    if areas.opening.is_none()
        && let Some(field) = project.active_flat_field()
    {
        areas.opening = Some(
            Coverage::rectangle(analysis::rect(field.map.area()))
                .corners()
                .map(|p| p.0),
        );
    }
    if let Some(explicit) = project.analysis_area {
        areas.edit_picture(Coverage::rectangle(analysis::rect(explicit)));
        if areas.opening.is_none() {
            areas.opening = Some(Coverage::rectangle(active).corners().map(|p| p.0));
        }
    } else if !areas.picture_edited
        && let Ok(picture) = regions.picture_sampling(analysis::sampling_inset(active))
    {
        areas.picture = Some(picture);
        areas.picture_geometry = None;
        areas.picture_constraints.clear();
        areas.picture_observed = [false; 4];
        areas.picture_weak = [false; 4];
    }
    areas.validate(active)?;
    Ok(areas)
}

fn assemble_review(
    request: &analysis::Request,
    cancelled: impl Fn() -> bool,
    mut progress: impl FnMut(usize, usize),
) -> Result<Option<Approved>, Error> {
    if cancelled() {
        return Ok(None);
    }
    let order = &request.document.project.order;
    let metadata = &request.document.spatial_metadata;
    let mut entries = Vec::with_capacity(order.len());
    let mut files = Vec::with_capacity(order.len());
    for (index, &source) in order.iter().enumerate() {
        progress(index, order.len());
        if cancelled() {
            return Ok(None);
        }
        let regions = metadata
            .groups
            .iter()
            .flat_map(|group| &group.sources)
            .find(|row| row.input.source == source)
            .ok_or(Error::UnusableSample("shared area detection is not ready"))?;
        entries.push(SourceAnalysis {
            source,
            areas: review_windows(request, regions)?,
        });
        files.push(
            request
                .document
                .file_key(&request.document.source_path(source)?),
        );
    }
    progress(order.len(), order.len());
    Ok((!cancelled()).then_some(Approved { entries, files }))
}

async fn prepare_review(
    request: analysis::Request,
    cancelled: Cancellation,
    progress: impl FnMut(usize, usize) + Send + 'static,
) -> Result<Option<Approved>, Error> {
    let stopped = cancelled.clone();
    background::compute(&cancelled, move || {
        assemble_review(&request, || stopped.is_cancelled(), progress)
    })
    .await
    .map(Option::flatten)
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Phase {
    Regions,
    Preparing,
    Review,
    Fitting,
}

pub(super) struct Operation {
    pub phase: Phase,

    pub cancelled: Cancellation,
    #[cfg(test)]
    pub activity: Option<ActivityReporter>,
    pub decision: Option<oneshot::Sender<Confirmation>>,
    pub task: Option<async_executor::Task<()>>,
}

impl Operation {
    pub(super) fn new(phase: Phase) -> Self {
        Self {
            phase,

            cancelled: Cancellation::default(),
            #[cfg(test)]
            activity: None,
            decision: None,
            task: None,
        }
    }
}

impl Drop for Operation {
    fn drop(&mut self) {
        self.cancelled.cancel();
    }
}

struct FitInput {
    activity: Activity,
    correction: Option<Arc<Correction>>,
    sources: Option<Arc<dyn SourceProvider>>,
    sampling_inputs: SamplingInputs,
    initialization: Arc<crate::source::Initialization>,
}

pub(super) struct Confirmation {
    selected: super::SourceId,
    areas: Approved,
    fit: FitInput,
}

async fn sampling_review(
    request: analysis::Request,
    activity: Activity,
    cancelled: Cancellation,
    cx: UiContext<'_, Editor>,
) {
    let produced = activity.reporter();
    let result = async {
        #[cfg(test)]
        if let Some(work) = cx
            .update(|editor| editor.document.analysis_preparation_work.take())
            .flatten()
        {
            return work(request.clone(), cancelled.clone()).await;
        }
        prepare_review(request.clone(), cancelled.clone(), move |done, total| {
            produced.progress(done, total)
        })
        .await
        .map_err(|error| error.to_string())
    }
    .await;
    drop(activity);
    let decision = cx
        .update(|editor| {
            editor.finish_analysis_work();
            if cancelled.is_cancelled() {
                editor.status = "Analysis-area preparation cancelled; grade unchanged".into();
                return None;
            }
            editor.open_analysis_review(&request, result)
        })
        .flatten();
    await_review(request, decision, cancelled, &cx).await;
}

async fn await_review(
    mut request: analysis::Request,
    decision: Option<oneshot::Receiver<Confirmation>>,
    cancelled: Cancellation,
    cx: &UiContext<'_, Editor>,
) {
    if let Some(decision) = decision
        && let Ok(confirmation) = decision.await
    {
        request.selected = confirmation.selected;
        request.areas = Some(confirmation.areas);
        fit_and_accept(request, confirmation.fit, cancelled, cx).await;
        return;
    }
    cx.update(|editor| editor.document.set_analysis_operation(None));
}

async fn fit_and_accept(
    request: analysis::Request,
    fit: FitInput,
    cancelled: Cancellation,
    cx: &UiContext<'_, Editor>,
) {
    let FitInput {
        activity,
        correction,
        sources,
        sampling_inputs,
        initialization,
    } = fit;
    let produced = activity.reporter();
    let result = async {
        #[cfg(test)]
        if let Some(work) = cx
            .update(|editor| editor.document.analysis_fit_work.take())
            .flatten()
        {
            return work(request.clone(), cancelled.clone()).await;
        }
        let sources = sources.ok_or("Source provider unavailable")?;
        let (device, queue, renderers) = cx
            .read(|editor| {
                editor.source_services.as_ref().map(|source| {
                    (
                        source.device.clone(),
                        source.queue.clone(),
                        source.renderers.clone(),
                    )
                })
            })
            .ok_or("GPU resources unavailable")?;
        let captured = request.clone();
        let resources = fit::Resources {
            correction,
            sources,
            sampling_inputs,
            initialization,
        };
        let renderer = background::compute(&cancelled, move || {
            renderers
                .get(&device, &queue)
                .map(|renderer| (device, queue, renderer))
        })
        .await
        .map_err(|error| error.to_string())?;
        let Some((device, queue, renderer)) = renderer else {
            return Ok(None);
        };
        let state = fit::Fit::acquire(device, queue, renderer, captured, resources, &cancelled)
            .await
            .map_err(|error| error.to_string())?;
        let Some(state) = state else {
            return Ok(None);
        };
        state
            .run(cancelled.clone(), move |done, total| {
                produced.progress(done, total)
            })
            .await
            .map_err(|error| error.to_string())
            .map(|outcome| outcome.map(Box::new))
    }
    .await;
    drop(activity);
    cx.update(|editor| {
        editor.finish_analysis_work();
        editor.accept_analysis(&request, result, cancelled.is_cancelled(), cx);
        editor.acquire_working(cx);
        editor.document.set_analysis_operation(None);
    });
}

impl Editor {
    pub(super) fn analysis_preparing(&self) -> bool {
        self.document
            .analysis_operation()
            .is_some_and(|op| matches!(op.phase, Phase::Preparing))
    }

    pub(super) fn analysis_fitting(&self) -> bool {
        self.document
            .analysis_operation()
            .is_some_and(|op| matches!(op.phase, Phase::Fitting))
    }

    fn begin_analysis_preparation(&mut self) -> Activity {
        let cancelled = self
            .document
            .analysis_operation()
            .unwrap()
            .cancelled
            .clone();
        let activity = self.activities.start(
            ActivityOptions::new("Preparing analysis areas…")
                .modal()
                .cancellable("Cancel analysis", "Cancelling analysis…", cancelled),
        );
        #[cfg(test)]
        {
            self.document.analysis_operation_mut().unwrap().activity = Some(activity.reporter());
        }
        self.document.analysis_operation_mut().unwrap().phase = Phase::Preparing;
        self.document.thumbnails.cancel();

        self.status = "Preparing per-photo analysis areas…".into();
        activity
    }

    fn finish_analysis_work(&mut self) {
        if let Some(op) = self.document.analysis_operation_mut() {
            op.phase = Phase::Review;
            #[cfg(test)]
            {
                op.activity = None;
            }
        }
    }

    pub(super) fn request_analysis(&mut self, cx: &UiContext<'_, Self>) {
        if !self.prepare_tool() {
            return;
        }
        let Some(mut request) = self.analysis_request() else {
            return;
        };
        let operation = Operation::new(Phase::Regions);
        let Some(regions) = self.prepare_analysis_regions(cx) else {
            return;
        };

        let cancelled = operation.cancelled.clone();
        let retire = cancelled.cancel_on_drop();
        self.document.set_analysis_operation(Some(operation));
        let task = cx.spawn(move |cx| async move {
            let _retire = retire;
            if regions.await.is_none() {
                cx.update(|editor| editor.document.set_analysis_operation(None));
                return;
            }
            let prepared = cx
                .update(|editor| {
                    super::document::rebase_project(
                        &mut request.document.project,
                        request.document.path.as_deref(),
                        editor.document.path.as_deref(),
                    );
                    request.document = editor.document.capture();
                    Some(editor.begin_analysis_preparation())
                })
                .flatten();
            if let Some(activity) = prepared {
                sampling_review(request, activity, cancelled, cx.clone()).await;
            } else {
                cx.update(|editor| editor.document.set_analysis_operation(None));
            }
        });
        if let Some(task) = task {
            self.document.analysis_operation_mut().unwrap().task = Some(task);
        } else {
            self.document.set_analysis_operation(None);
            self.cancel_spatial();
        }
    }

    #[cfg(test)]
    fn begin_analysis_fit(&mut self, total: usize) -> Option<FitInput> {
        self.with_analyze_tool(|tool, editor| tool.begin_analysis_fit(editor, total))
            .flatten()
    }

    pub(super) fn cancel_analysis(&mut self) {
        self.with_analyze_tool(|tool, editor| tool.cancel(editor));
    }
}

impl AnalyzeTool {
    fn begin_analysis_fit(&mut self, editor: &mut Editor, total: usize) -> Option<FitInput> {
        let activity =
            editor
                .activities
                .start(ActivityOptions::new("Analyzing roll…").modal().cancellable(
                    "Cancel analysis",
                    "Cancelling analysis…",
                    self.operation.cancelled.clone(),
                ));
        #[cfg(test)]
        {
            self.operation.activity = Some(activity.reporter());
        }
        self.operation.phase = Phase::Fitting;
        editor.document.thumbnails.cancel();
        editor.cancel_original_pending();
        activity.progress(0, total);
        editor.status = "Analyzing roll…".into();
        Some(FitInput {
            activity,
            correction: editor.document.correction.prepared().cloned(),
            sources: editor.source_provider(),
            sampling_inputs: editor.document.sampling_inputs.clone(),
            initialization: editor.document.initialization.clone(),
        })
    }

    pub(super) fn submit_analysis_review(&mut self, editor: &mut Editor, areas: Approved) {
        if self.operation.decision.is_none() {
            return;
        }
        let Some(fit) = self.begin_analysis_fit(editor, editor.document.project.order.len()) else {
            return;
        };
        let decision = self.operation.decision.take().unwrap();
        let _ = decision.send(Confirmation {
            selected: editor.document.visit.current_source.expect("review source"),
            areas,
            fit,
        });
    }
}
