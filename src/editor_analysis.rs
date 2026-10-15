use super::*;
use crate::roll_analysis::{self, FrameSamples, MAX_SAMPLES_PER_FRAME};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(test)]
#[path = "editor_analysis_diagnostics.rs"]
mod diagnostics;
#[cfg(test)]
#[path = "editor_lazy_support_tests.rs"]
mod lazy_support_tests;
#[cfg(test)]
#[path = "editor_analysis_tests.rs"]
mod tests;

#[derive(Clone)]
pub(super) struct Request {
    pub loaded: u64,
    pub revision: u64,
    pub document: Document,
    pub selected: SourceId,
    pub film: Option<Rect>,
    pub local_support: bool,
    pub picture: Option<Rect>,
}
pub(super) struct Pending {
    pub id: u64,
    pub request: Request,
}
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
    if frames.iter().any(|f| {
        f.balance_samples < MIN_BALANCE_SAMPLES || f.exposure_samples < MIN_PICTURE_SAMPLES
    }) {
        return Err(Error::UnusableSample(
            "too few positive picture statistics; adjust the picture area or check the capture",
        ));
    }
    Ok(())
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
    explicit.or_else(|| support.map(suggest)).ok_or(Error::UnusableSample(
        "no roll illumination-area suggestion; use Edit picture area to select the film picture, excluding the holder"
    ))
}

pub(super) fn run(
    device: &wgpu::Device,
    request: &Request,
    generation: &AtomicU64,
    correction: Option<&Arc<Correction>>,
    developments: &development::Scheduler,
    mut progress: impl FnMut(usize, usize),
) -> Result<Option<Outcome>, Error> {
    let cancelled = || generation.load(Ordering::Relaxed) != request.revision;
    if cancelled() {
        return Ok(None);
    }
    request.document.validate()?;
    let project = &request.document.project;
    if project.order.is_empty() || !project.order.contains(&request.selected) {
        return Err(Error::UnusableSample("no selected roll photo"));
    }
    if project.active_flat_field() != correction.map(|c| &c.settings) {
        return Err(Error::UnusableSample(
            "analysis correction snapshot changed",
        ));
    }
    let wb = project
        .development
        .as_ref()
        .ok_or(Error::UnusableSample("no roll development"))?
        .fixed_wb;
    let matrix = glam::DMat3::from_cols_array_2d(
        &project
            .camera_basis
            .as_ref()
            .ok_or(Error::UnusableSample("no roll camera basis"))?
            .camera_to_ap1,
    );
    let mut support = request.film;
    if support.is_none() {
        progress(0, project.order.len());
        for id in &project.order {
            if cancelled() {
                return Ok(None);
            }
            let expected = project.source(*id)?;
            let path = request.document.source_path(*id)?;
            let Some(scan) = develop_scan(
                developments,
                &request.document,
                expected,
                &path,
                wb,
                cancelled,
            )?
            else {
                return Ok(None);
            };
            let actual = document::source_metadata(&path, &scan.source().metadata())?;
            document::verify_source(expected, &actual)?;
            document::verify_scan(&actual, &scan)?;
            if let Some(area) = scan.propose_flat_field_area()? {
                support = Some(area);
                break;
            }
        }
    }
    let picture = picture_area(request.picture, support)?;
    // Explicit picture geometry can still be used without a local illumination hint.
    // Never invent whole-camera geometry for dense-tail/balance/exposure statistics.
    let film = support.unwrap_or_else(|| {
        rect(
            project
                .source(request.selected)
                .expect("selected roll source")
                .development
                .active_area,
        )
    });
    let mut owned = Vec::with_capacity(project.order.len());
    let mut floor = None;
    for (index, id) in project.order.iter().enumerate() {
        if cancelled() {
            return Ok(None);
        }
        progress(index, project.order.len());
        let expected = project.source(*id)?;
        let path = request.document.source_path(*id)?;
        let Some(mut scan) = develop_scan(
            developments,
            &request.document,
            expected,
            &path,
            wb,
            cancelled,
        )?
        else {
            return Ok(None);
        };
        let actual = document::source_metadata(&path, &scan.source().metadata())?;
        document::verify_source(expected, &actual)?;
        document::verify_scan(&actual, &scan)?;
        project.require_camera_basis(&actual.development)?;
        if let Some(correction) = correction {
            correction.settings.require_source(&actual.development)?;
            scan.set_flat_field(Some(correction.settings.map.clone()))?;
        }
        if floor.is_some_and(|value| value != scan.effective_floor()) {
            return Err(Error::UnusableSample(
                "inconsistent analysis precision floor",
            ));
        }
        floor = Some(scan.effective_floor());
        let Some(film) = scan.analysis_samples(film, MAX_SAMPLES_PER_FRAME, cancelled)? else {
            return Ok(None);
        };
        let Some(picture) = scan.analysis_samples(picture, MAX_SAMPLES_PER_FRAME, cancelled)?
        else {
            return Ok(None);
        };
        require_picture(&picture)?;
        owned.push((film.pixels, picture.pixels));
        // Each temporary RAW is released here; only bounded statistics survive.
    }
    progress(project.order.len(), project.order.len());
    let frames: Vec<_> = owned
        .iter()
        .map(|(film, picture)| FrameSamples { film, picture })
        .collect();
    let Some(proposal) =
        roll_analysis::estimate(&frames, floor.unwrap(), matrix, project.roll, cancelled)?
    else {
        return Ok(None);
    };
    require_statistics(
        owned.iter().map(|(film, _)| film.len()).sum(),
        &proposal.frames,
    )?;
    let mut candidate = request.document.clone();
    candidate.project.dmin = Some(proposal.dmin.to_array());
    candidate.project.dmax = Some(proposal.dmax.to_array());
    candidate.project.input_floor = Some(crate::processing::CAMERA_INPUT_FLOOR);
    candidate.project.roll = proposal.roll;
    for (id, photo) in project.order.iter().zip(&proposal.photos) {
        candidate.project.source_mut(*id)?.photo = *photo;
    }
    candidate.project.analysis_area = Some(array(picture));
    candidate.validate()?; // Includes effective-floor GPU constants for every Photo grade.
    for id in &project.order {
        if cancelled() {
            return Ok(None);
        }
        let record = project.source(*id)?;
        let mut input = crate::gpu::ProcessingSource::new(device, &[[0; 4]], matrix)?;
        if let Some(correction) = correction {
            input.set_flat_field(
                Some(correction.gpu.clone()),
                crate::gpu::SourceGeometry::new(record.development.active_area, [1, 1])?,
            )?;
        }
    }
    if cancelled() {
        return Ok(None);
    }
    Ok(Some(Outcome {
        document: candidate,
        frames: frames.len(),
        support: film,
    }))
}

fn develop_scan(
    developments: &development::Scheduler,
    document: &Document,
    expected: &Source,
    path: &std::path::Path,
    wb: [f64; 4],
    cancelled: impl Fn() -> bool,
) -> Result<Option<Scan>, Error> {
    let cache = LoadCache::new(path, Some(expected), Some(wb), document);
    let result = developments.load(
        development::Request {
            path: path.to_owned(),
            wb: Some(wb),
            expected: Some(expected.clone()),
            basis: document.project.camera_basis.clone(),
            file: cache.file,
            key: cache.key,
            store: Some(cache.store),
            parent: cache.parent,
            retain: false,
        },
        development::Priority::Foreground,
        &cancelled,
    );
    if cancelled() {
        return Ok(None);
    }
    result.map(Scan::from_source).map(Some)
}

impl Editor {
    fn analysis_support(&self) -> Option<Rect> {
        self.correction
            .as_ref()
            .map(|c| rect(c.settings.map.area()))
            .or_else(|| {
                self.ready
                    .as_ref()
                    .and_then(|r| r.analysis_support.flatten())
            })
    }
    pub(super) fn analysis_picture(&self) -> Option<Rect> {
        self.document
            .project
            .analysis_area
            .map(rect)
            .or_else(|| self.analysis_support().map(suggest))
    }
    pub(super) fn request_analysis_support(&mut self) {
        self.region = self.analysis_picture().map(Coverage::rectangle);
        if self.region.is_some()
            || self
                .ready
                .as_ref()
                .is_none_or(|ready| ready.analysis_support.is_some())
        {
            return;
        }
        let request = self.request_id();
        if self
            .commands
            .send(Command::AnalysisSupport(request, self.id))
            .is_ok()
        {
            self.analysis_area_pending = Some(request);
        } else {
            self.status = "RAW worker stopped".into();
        }
    }
    pub(super) fn analysis_support_result(
        &mut self,
        request: u64,
        loaded: u64,
        result: Result<Option<Rect>, String>,
    ) {
        if self.analysis_area_pending != Some(request)
            || loaded != self.id
            || self.pick != Some(Pick::Analysis)
            || self.document_pending.is_some()
        {
            return;
        }
        self.analysis_area_pending = None;
        let Some(ready) = &mut self.ready else {
            return;
        };
        match result {
            Ok(support) => {
                ready.analysis_support = Some(support);
                // A delayed suggestion must never overwrite a rectangle already drawn.
                if self.region.is_none() && self.pick_drag.is_none() {
                    self.region = support.map(suggest).map(Coverage::rectangle);
                }
            }
            Err(error) => self.status = error,
        }
    }
    pub(super) fn analysis_controls(&mut self, ui: &mut egui::Ui) -> bool {
        use presentation::TextAction;
        let mut analyze = false;
        presentation::section(
            ui,
            "roll-analysis",
            "Roll analysis",
            presentation::SectionHeader {
                summary: "Shared roll initialization",
                changed: false,
                reset: None,
                default_open: None,
            },
            |ui| {
                if self.analysis_pending.is_some() {
                    let view = self
                        .tasks
                        .view()
                        .expect("pending analysis has presentation");
                    view.inline(ui);
                } else {
                    ui.add_enabled_ui(
                        !self.busy
                            && self.ready.is_some()
                            && self.pick.is_none()
                            && self.crop_edit.is_none(),
                        |ui| {
                            ui.horizontal_wrapped(|ui| {
                                if TextAction::new("Analyze roll").primary().show(ui).clicked() {
                                    analyze = true;
                                }
                                if TextAction::new("Edit picture area…").show(ui).clicked() {
                                    self.begin_pick(Pick::Analysis);
                                }
                            });
                        },
                    );
                    if self.document.project.analysis_area.is_some() && TextAction::new("Reset picture area").enabled(!self.busy && self.pick.is_none() && self.crop_edit.is_none()).help("Discard the accepted statistics rectangle; next Analyze seeks a roll-supported suggestion. Processing stays unchanged.").show(ui).clicked() {
                        self.reset_analysis_area();
                    }
                    if self.analysis_picture().is_none() {
                        ui.small("Opening suggestions are calculated when needed. Analyze searches for support; or draw the picture area explicitly.");
                    }
                }
                ui.small("Estimate RGB references, shared colour balance and individual exposures. Keeps gamma/curve, correction and composition. Picture area controls statistics, not cropping.");
            },
        );
        analyze
    }
    pub(super) fn reset_analysis_area(&mut self) {
        self.document.project.analysis_area = None;
        self.analysis_area_pending = None;
        if let Some(ready) = &mut self.ready {
            ready.analysis_support = Some(None);
        }
        self.status = "Picture area reset; next Analyze searches the roll. Grade unchanged.".into();
    }

    fn analysis_request(&mut self) -> Option<Request> {
        if self.busy || self.pick.is_some() || self.crop_edit.is_some() || self.ready.is_none() {
            return None;
        }
        self.sync_document();
        Some(Request {
            loaded: self.id,
            revision: self.analysis_generation.load(Ordering::Relaxed),
            document: self.document.clone(),
            selected: self.current_source?,
            film: self.analysis_support(),
            local_support: self
                .ready
                .as_ref()
                .is_some_and(|ready| ready.analysis_support.is_none()),
            picture: self.document.project.analysis_area.map(rect),
        })
    }

    pub(super) fn request_analysis(&mut self) {
        if self.analysis_confirmation.is_some() {
            return;
        }
        let Some(request) = self.analysis_request() else {
            return;
        };
        let adjusted = request
            .document
            .project
            .order
            .iter()
            .try_fold(false, |adjusted, id| {
                request
                    .document
                    .project
                    .source(*id)
                    .map(|source| adjusted || source.photo != PhotoSettings::default())
            });
        match adjusted {
            Ok(true) => self.analysis_confirmation = Some(request),
            Ok(false) => self.start_analysis(request),
            Err(error) => self.status = format!("Analysis failed: {error}. Grade unchanged."),
        }
    }

    fn finish_analysis_confirmation(&mut self, accept: bool) {
        let Some(request) = self.analysis_confirmation.take() else {
            return;
        };
        if !accept {
            self.status = "Analysis cancelled".into();
            return;
        }
        self.sync_document();
        if self.busy
            || self.ready.is_none()
            || self.pick.is_some()
            || self.crop_edit.is_some()
            || self.analysis_generation.load(Ordering::Relaxed) != request.revision
            || self.id != request.loaded
            || self.current_source != Some(request.selected)
            || self.document.path != request.document.path
            || self.document.project != request.document.project
        {
            self.status = "Analysis confirmation discarded: document or settings changed".into();
            return;
        }
        self.start_analysis(request);
    }

    pub(super) fn analysis_dialog(&mut self, context: &egui::Context, escape_available: bool) {
        use presentation::{DecisionDialog, DialogChoice};
        if self.analysis_confirmation.is_none() {
            return;
        }
        let choice = DecisionDialog::new(
            "Replace Photo adjustments?",
            "Analyze roll will initialize shared Roll settings and replace existing Photo adjustments. Exposure is estimated; Photo white balance and other grading controls return to their defaults.",
            "Analyze roll",
        ).warning().detail("Crop, orientation and flat-field correction are kept.")
            .show(context, egui::Id::new("confirm-roll-analysis"), escape_available);
        if let Some(choice) = choice {
            self.finish_analysis_confirmation(choice == DialogChoice::Confirm);
        }
    }

    #[cfg(test)]
    pub(super) fn begin_analysis(&mut self) {
        let Some(request) = self.analysis_request() else {
            return;
        };
        self.start_analysis(request);
    }

    fn start_analysis(&mut self, mut request: Request) {
        request.revision = self.analysis_generation.fetch_add(1, Ordering::Relaxed) + 1;
        let id = self.request_id();
        self.thumbnails.cancel();
        self.original.cancel_pending();
        self.tasks.begin(Task::Analyze(id));
        self.tasks.report(
            TaskRequest::Analyze(id),
            TaskProgress::Counted {
                completed: 0,
                total: request.document.project.order.len(),
            },
        );
        self.analysis_pending = Some(Pending {
            id,
            request: request.clone(),
        });
        self.busy = true;
        self.status = "Analyzing roll…".into();
        if self
            .commands
            .send(Command::Analyze(id, Box::new(request)))
            .is_err()
        {
            self.analysis_pending = None;
            self.tasks.finish(TaskRequest::Analyze(id));
            self.busy = false;
            self.status = "RAW worker stopped".into();
        }
    }
    pub(super) fn cancel_analysis(&mut self) {
        self.analysis_generation.fetch_add(1, Ordering::Relaxed);
        if let Some(pending) = &self.analysis_pending {
            self.tasks.cancelling(TaskRequest::Analyze(pending.id));
        }
        self.status = "Cancelling analysis…".into();
    }
    pub(super) fn analysis_result(
        &mut self,
        id: u64,
        result: Result<Option<Box<Outcome>>, String>,
    ) {
        if !self.analysis_pending.as_ref().is_some_and(|p| p.id == id) {
            return;
        }
        let pending = self.analysis_pending.take().unwrap();
        self.tasks.finish(TaskRequest::Analyze(id));
        self.busy = false;
        self.sync_document();
        let snapshot = &pending.request;
        if self.analysis_generation.load(Ordering::Relaxed) != snapshot.revision {
            self.status = "Analysis cancelled".into();
            return;
        }
        if self.id != snapshot.loaded
            || self.current_source != Some(snapshot.selected)
            || self.document.path != snapshot.document.path
            || self.document.project != snapshot.document.project
        {
            self.status = "Analysis discarded: document or settings changed".into();
            return;
        }
        match result {
            Ok(Some(outcome)) => {
                let photo = match outcome.document.validate().and_then(|_| {
                    outcome
                        .document
                        .project
                        .source(snapshot.selected)
                        .map(|s| s.photo)
                }) {
                    Ok(photo) => photo,
                    Err(error) => {
                        self.status = format!("Analysis rejected: {error}");
                        return;
                    }
                };
                if let Some(ready) = &mut self.ready {
                    ready.analysis_support = Some(Some(outcome.support));
                }
                self.document = outcome.document;
                self.dmin = self.document.project.dmin.map(DVec3::from_array);
                self.dmax = self.document.project.dmax.map(DVec3::from_array);
                self.floor = self.document.project.input_floor.unwrap();
                self.roll = self.document.project.roll;
                self.photo = photo;
                self.source = false;
                self.original.invalidate();
                self.status = format!(
                    "Analyzed {} frames. Adjust normally; re-pick Dmin if needed.",
                    outcome.frames
                );
            }
            Ok(None) => self.status = "Analysis cancelled".into(),
            Err(error) => self.status = format!("Analysis failed: {error}. Grade unchanged."),
        }
    }
    pub(super) fn apply_analysis_area(&mut self) {
        let Some(coverage) = self.region else {
            return;
        };
        let Some(area) = self
            .current_source
            .and_then(|id| self.document.project.source(id).ok())
            .and_then(|source| coverage.exact_rectangle(source.development.active_area.into()))
        else {
            self.status = "Picture area must be a source-aligned rectangle".into();
            return;
        };
        self.sync_document();
        let mut candidate = self.document.clone();
        candidate.project.analysis_area = Some(array(area));
        match candidate.validate() {
            Ok(()) => {
                self.document = candidate;
                self.status = "Picture statistics area updated; processing unchanged".into();
                self.finish_pick();
            }
            Err(error) => self.status = format!("Area rejected: {error}"),
        }
    }
}
