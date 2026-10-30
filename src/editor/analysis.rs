use super::*;

pub(super) const AREA_DISPLAY: picking::Display = picking::Display {
    label: "Picture area",
    view: picking::View::Native,
    area: true,
    lock_layer_selection: false,
    help: Some("Picture statistics only · source-aligned · not an output crop"),
};
use crate::roll_analysis::{self, FrameSamples, MAX_SAMPLES_PER_FRAME};

#[cfg(all(test, feature = "ocio-reference"))]
mod diagnostics;
#[cfg(test)]
mod lazy_support_tests;
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
    #[cfg(test)]
    pub revision: u64,
    pub document: Document,
    pub selected: SourceId,
    pub film: Option<Rect>,
    pub local_support: bool,
    pub picture: Option<Rect>,
    pub areas: Option<analysis_review::Approved>,
}

pub(super) mod fit;
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
    pub document: Document,
    pub frames: usize,
    pub support: Rect,
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

#[cfg(test)]
pub(super) fn develop_scan(
    developments: &source_loading::SourceLoader,
    document: &Document,
    cache: &Arc<crate::source::SourceCache>,
    expected: &Source,
    path: &std::path::Path,
    wb: [f64; 4],
    cancelled: impl Fn() -> bool,
) -> Result<Option<Scan>, Error> {
    let request = source::Request {
        input: document.project.input,
        reference: Arc::default(),
        path: path.to_owned(),
        wb: Some(wb),
        expected: Some(expected.clone()),
        sources: Some(developments.provider(Some(cache.clone()))),
        file: document.file_key(path),
        correction: None,
    };
    let result = request.load(developments, &cancelled);
    if cancelled() {
        return Ok(None);
    }
    result
        .and_then(|developed| developed.context.scan())
        .map(Some)
}

impl Editor {
    fn analysis_support(&self) -> Option<Rect> {
        self.document
            .correction
            .as_ref()
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

    pub(super) fn analysis_picture(&self) -> Option<Rect> {
        self.document
            .state
            .project
            .analysis_area
            .map(rect)
            .or_else(|| self.analysis_support().map(suggest))
    }

    pub(super) fn analysis_support_result(
        &mut self,
        request: u64,
        loaded: u64,
        result: Result<Option<Rect>, String>,
    ) {
        if self
            .document
            .visit
            .support
            .pending
            .as_ref()
            .map(|pending| (pending.id, pending.loaded))
            != Some((request, loaded))
            || loaded != self.id
            || self.active_pick() != Some(Pick::Analysis.display())
            || self.document_pending.is_some()
        {
            return;
        }
        self.document.visit.support = Default::default();
        let Some(ready) = &mut self.document.visit.canvas.ready else {
            return;
        };
        match result {
            Ok(support) => {
                ready.analysis_support = Some(support);
                // A delayed suggestion must never overwrite a rectangle already drawn.
                if self.pick_region().is_none() && self.document.visit.canvas.pick_drag.is_none() {
                    self.set_pick_region(support.map(suggest).map(Coverage::rectangle));
                }
            }
            Err(error) => self.status = error,
        }
    }

    pub(super) fn apply_roll_orientation(&mut self, action: Orientation) {
        let before = self.history_snapshot();
        match action {
            Orientation::Left => self.document.roll_orientation.turn_counterclockwise(),
            Orientation::Right => self.document.roll_orientation.turn_clockwise(),
            Orientation::Horizontal => self.document.roll_orientation.horizontal ^= true,
            Orientation::Vertical => self.document.roll_orientation.vertical ^= true,
            Orientation::Reset => self.document.roll_orientation = RollOrientation::default(),
        }
        self.document.visit.canvas.viewport.fit();
        self.record_edit(before, "Roll orientation", None);
    }

    pub(super) fn roll_tool_shelf(&mut self, ui: &mut egui::Ui, cx: &UiContext<'_, Self>) -> bool {
        use icons::Icon;
        use presentation::{ToolButton, ToolShelf};
        let reviewing = self.document.analysis_review().is_some();
        let analyzing = self.document.analysis_operation().is_some();
        let capabilities = self.document.input.capabilities();
        let commands = !self.busy
            && self.document.visit.canvas.ready.is_some()
            && self.active_pick().is_none()
            && self.document.crop_edit().is_none()
            && self.document.auto_crop_edit().is_none()
            && self.document.flat_edit().is_none()
            && self.document.mask_edit().is_none()
            && !self.document.visit.adjustment_layers.confirming()
            && !reviewing
            && self.document.removal_pending.is_none();
        let mut analyze = false;

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
                                || !self.document.roll_orientation.is_identity()),
                    )
                    .show(ui)
                    .clicked()
                {
                    self.control_command(editing::Command::Roll(action), cx);
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
                .enabled(self.tool_start_available() && self.document.visit.canvas.ready.is_some())
                .show(ui)
                .clicked()
                {
                    if analyzing {
                        self.cancel_active_tool();
                    } else {
                        analyze = true;
                    }
                }
            }
            if capabilities.auto_crop {
                let cropping = self.document.auto_crop_operation().is_some();
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
                        self.document.auto_crop_requested = Some(auto_crop::Action::Batch {
                            photos: self.document.project.order.clone(),
                            scope: "Whole roll",
                        });
                    }
                }
            }
        });
        analyze
    }

    pub(super) fn analysis_request(&mut self) -> Option<Request> {
        if self.busy
            || self.document.visit.canvas.ready.is_none()
            || self.document.project.order.is_empty()
        {
            return None;
        }
        self.sync_document();
        Some(Request {
            #[cfg(test)]
            revision: 0,
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
        self.sync_document();
        if cancelled {
            self.status = "Analysis cancelled".into();
            self.end_analysis_review(cx);
            return;
        }
        match result {
            Ok(Some(outcome)) => {
                if let Err(error) = analysis_review::verify(snapshot) {
                    self.status = format!("Analysis discarded: {error}; grade unchanged");
                    self.end_analysis_review(cx);
                    return;
                }
                let photo = match self
                    .validate_document_candidate(&outcome.document)
                    .and_then(|_| {
                        outcome.document.project.source(snapshot.selected).map(|s| {
                            s.adjustment_layers
                                .as_ref()
                                .and_then(|layers| layers.layers.last())
                                .map(|record| record.adjustment.photo)
                                .unwrap_or_default()
                        })
                    }) {
                    Ok(photo) => photo,
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
                let before = self.history_snapshot();
                self.document.install(outcome.document);
                self.document.visit.adjustment_layers = AdjustmentLayerEditor::default();
                self.load_input(self.document.state.project.input);
                self.document.roll = self.document.state.project.roll;
                self.document.visit.adjustment_layer_settings = photo;
                self.document.source = false;
                self.document.visit.working_view.clear();
                self.invalidate_original();
                self.record_edit(before, "Analyze roll", None);
                self.status = format!(
                    "Analyzed {} frames. Adjust normally; re-pick Dmin if needed.",
                    outcome.frames
                );
            }
            Ok(None) => self.status = "Analysis cancelled".into(),
            Err(error) => self.status = format!("Analysis failed: {error}. Grade unchanged."),
        }
        self.end_analysis_review(cx);
    }

    pub(super) fn apply_analysis_area(&mut self, cx: &UiContext<'_, Self>) {
        let Some(coverage) = self.pick_region() else {
            return;
        };
        let Some(area) = self
            .document
            .visit
            .current_source
            .and_then(|id| self.document.state.project.source(id).ok())
            .and_then(|source| coverage.exact_rectangle(source.development.active_area.into()))
        else {
            self.status = "Picture area must be a source-aligned rectangle".into();
            return;
        };
        self.sync_document();
        let mut candidate = self.document.capture();
        candidate.project.analysis_area = Some(array(area));
        match self.validate_document_candidate(&candidate) {
            Ok(()) => {
                let before = self.history_snapshot();
                self.document.install(candidate);
                self.record_edit(before, "Picture statistics area", None);
                self.status = "Picture statistics area updated; processing unchanged".into();
                self.finish_pick(cx);
            }
            Err(error) => self.status = format!("Area rejected: {error}"),
        }
    }
}
