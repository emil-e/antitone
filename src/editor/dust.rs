use super::*;
#[cfg(test)]
use crate::dust_repair::{
    AUTOMATIC_PADDING, CONTEXT_PADDING, MAX_MASK_PIXELS, MAX_MASK_WORK_PIXELS,
};
use crate::processing::InputSettings;
use crate::{
    dust_repair::{
        EvaluatedRepair, Repair, RepairId, RepairOrigin, RepairSet, SupportMask, donor_target,
        mask_within_budget, validate_offset as validate_dust_offset,
    },
    geometry::SourceRegion,
};
use std::collections::{BTreeSet, HashMap};

mod search;
pub(super) use search::Request as SearchRequest;

#[cfg(test)]
mod lifecycle_tests;

fn initial_brush_size(active: SourceRegion) -> u32 {
    ((active.width.min(active.height) as f64 * 0.01).round() as u32).clamp(4, 128)
}

pub(super) type Signature = super::processing::NeutralSourceSignature;

#[derive(Clone)]
pub(super) struct Request {
    pub request: u64,
    pub loaded: u64,
    pub source: SourceId,
    pub signature: Signature,
    pub repair: Repair,
    pub input: InputSettings,
    pub cancelled: Arc<std::sync::atomic::AtomicBool>,
}

pub(super) struct Response {
    pub request: u64,
    pub loaded: u64,
    pub source: SourceId,
    pub signature: Signature,
    pub repair: Repair,
    pub evaluated: Arc<EvaluatedRepair>,
}

fn repair_bound(repair: &EvaluatedRepair) -> DVec3 {
    repair.pixels().fold(DVec3::ZERO, |bound, (_, value)| {
        bound.max(DVec3::from_array(value.map(f64::from)).abs())
    })
}

struct Cached {
    loaded: u64,
    signature: Signature,
    repair: Repair,
    evaluated: Arc<EvaluatedRepair>,
    bound: DVec3,
}

struct Failed {
    loaded: u64,
    signature: Signature,
    repair: Repair,
}

struct Pending {
    evaluation: Request,
    task: Option<async_executor::Task<()>>,
}

struct EvaluationRetire(Arc<std::sync::atomic::AtomicBool>);
impl Drop for EvaluationRetire {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

pub(super) struct DonorRegion {
    pub support: SupportMask,
    pub target: SourceRegion,
    pub donor: SourceRegion,
}

#[derive(Clone, Copy)]
struct DonorDrag {
    source: [f64; 2],
    offset: [i32; 2],
}

#[derive(Clone, Copy)]
pub(super) struct DonorDraft {
    pub repair: RepairId,
    pub offset: [i32; 2],
    drag: Option<DonorDrag>,
}

#[derive(Clone)]
struct MaskDraft {
    target: Option<RepairId>,
    original_support: Option<SupportMask>,
    pixels: BTreeSet<[u32; 2]>,
    last: Option<[f64; 2]>,
    rejected: bool,
}

impl MaskDraft {
    fn new(target: Option<RepairId>, support: Option<&SupportMask>) -> Self {
        Self {
            target,
            original_support: support.cloned(),
            pixels: support
                .into_iter()
                .flat_map(SupportMask::selected_points)
                .collect(),
            last: None,
            rejected: false,
        }
    }

    fn support(&self) -> Result<SupportMask, Error> {
        // Unchanged support keeps its declared rectangle, including saved blank margins.
        if let Some(original) = &self.original_support
            && original.selected_points().count() == self.pixels.len()
            && original
                .selected_points()
                .all(|point| self.pixels.contains(&point))
        {
            return Ok(original.clone());
        }

        let min = [
            self.pixels.iter().map(|point| point[0]).min(),
            self.pixels.iter().map(|point| point[1]).min(),
        ];
        let max = [
            self.pixels.iter().map(|point| point[0]).max(),
            self.pixels.iter().map(|point| point[1]).max(),
        ];
        let (Some(min_x), Some(min_y), Some(max_x), Some(max_y)) = (min[0], min[1], max[0], max[1])
        else {
            return Err(Error::InvalidParameter("empty repair mask"));
        };
        let size = [max_x - min_x + 1, max_y - min_y + 1];
        if !mask_within_budget(size, self.pixels.len()) {
            return Err(Error::InvalidParameter("dust repair mask is too large"));
        }
        let mut bits = vec![false; (size[0] * size[1]) as usize];
        for [x, y] in &self.pixels {
            bits[((y - min_y) * size[0] + x - min_x) as usize] = true;
        }
        SupportMask::new([min_x, min_y], size, bits)
    }

    fn clear_stroke(&mut self) {
        self.last = None;
    }
}

pub(super) struct State {
    selected: HashMap<SourceId, RepairId>,
    cached: HashMap<(SourceId, RepairId), Cached>,
    preview_cached: HashMap<(SourceId, RepairId), Cached>,
    failed: HashMap<(SourceId, RepairId), Failed>,
    pending: Option<Pending>,
    mask_draft: Option<MaskDraft>,
    pub brush_size: u32,
    brush_source: Option<SourceId>,
    pub cleanup: bool,
    reveal_controls: bool,
    pub show_masks: bool,
    pub donor_draft: Option<DonorDraft>,
    pub render_error: Option<String>,
    searches: search::State,
    gpu: HashMap<RepairId, Arc<crate::gpu::repairs::Patch>>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            selected: HashMap::new(),
            cached: HashMap::new(),
            preview_cached: HashMap::new(),
            failed: HashMap::new(),
            pending: None,
            mask_draft: None,
            brush_size: 12,
            brush_source: None,
            cleanup: false,
            reveal_controls: false,
            show_masks: true,
            donor_draft: None,
            render_error: None,
            searches: search::State::default(),
            gpu: HashMap::new(),
        }
    }
}

impl Drop for State {
    fn drop(&mut self) {
        if let Some(pending) = &self.pending {
            pending
                .evaluation
                .cancelled
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

impl State {
    pub(super) fn attach_task(&mut self, request: u64, task: async_executor::Task<()>) {
        if let Some(pending) = self
            .pending
            .as_mut()
            .filter(|p| p.evaluation.request == request)
        {
            pending.task = Some(task);
        }
    }

    pub(super) fn history_draft(&self) -> bool {
        self.mask_draft.is_some() || self.donor_draft.is_some() || !self.searches.is_empty()
    }

    #[cfg(test)]
    pub(super) fn pending_request(&self) -> Option<u64> {
        self.pending
            .as_ref()
            .map(|pending| pending.evaluation.request)
    }

    #[cfg(test)]
    pub(super) fn has_evaluation(&self, source: SourceId, repair: RepairId) -> bool {
        self.cached.contains_key(&(source, repair))
    }

    #[cfg(all(test, feature = "ui-tests"))]
    pub(super) fn has_failed_evaluation(&self, source: SourceId, repair: RepairId) -> bool {
        self.failed.contains_key(&(source, repair))
    }

    pub(super) fn release_gpu(&mut self) {
        self.gpu.clear();
        self.render_error = None;
    }

    fn repair_changed(&mut self, source: SourceId, repair: RepairId, accepted: &RepairSet) {
        self.failed.remove(&(source, repair));
        if let Some(preview) = self.preview_cached.remove(&(source, repair))
            && accepted.repairs().contains(&preview.repair)
        {
            self.cached.insert((source, repair), preview);
        }
    }

    fn removed(&mut self, source: SourceId, repair: RepairId) {
        if let Some(pending) = &self.pending
            && pending.evaluation.source == source
            && pending.evaluation.repair.id == repair
        {
            pending
                .evaluation
                .cancelled
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
        if self.selected.get(&source) == Some(&repair) {
            self.selected.remove(&source);
        }
        self.cached.remove(&(source, repair));
        self.gpu.remove(&repair);
        self.preview_cached.remove(&(source, repair));
        self.failed.remove(&(source, repair));
        self.searches.cancel_target(source, repair);
    }

    fn clear_repairs(&mut self, source: SourceId) {
        self.selected.remove(&source);
        self.cached.retain(|(candidate, _), _| *candidate != source);
        self.preview_cached
            .retain(|(candidate, _), _| *candidate != source);
        self.failed.retain(|(candidate, _), _| *candidate != source);
        self.searches.cancel_source(source);
        if let Some(pending) = self
            .pending
            .take_if(|pending| pending.evaluation.source == source)
        {
            pending
                .evaluation
                .cancelled
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.mask_draft = None;
        self.donor_draft = None;
        self.gpu.clear();
        self.render_error = None;
    }

    #[cfg(all(test, feature = "ui-tests"))]
    pub fn evaluations(
        &self,
        source: SourceId,
        loaded: u64,
        signature: Signature,
        repairs: &[Repair],
    ) -> Vec<Arc<EvaluatedRepair>> {
        repairs
            .iter()
            .filter(|repair| repair.enabled)
            .filter_map(|repair| {
                self.preview_cached
                    .get(&(source, repair.id))
                    .filter(|cached| cached.repair == *repair)
                    .or_else(|| {
                        self.cached
                            .get(&(source, repair.id))
                            .filter(|cached| cached.repair == *repair)
                    })
            })
            .filter(|cached| cached.loaded == loaded && cached.signature == signature)
            .map(|cached| cached.evaluated.clone())
            .collect()
    }

    fn next_stale(
        &self,
        source: SourceId,
        loaded: u64,
        signature: Signature,
        repairs: &[Repair],
    ) -> Option<Repair> {
        if self.pending.is_some() {
            return None;
        }
        let repair = repairs
            .iter()
            .filter(|repair| repair.enabled)
            .find(|repair| {
                let current = |record_loaded, record_signature, record: &Repair| {
                    record_loaded == loaded && record_signature == signature && record == *repair
                };
                let cached = self
                    .preview_cached
                    .get(&(source, repair.id))
                    .filter(|cached| current(cached.loaded, cached.signature, &cached.repair))
                    .or_else(|| {
                        self.cached.get(&(source, repair.id)).filter(|cached| {
                            current(cached.loaded, cached.signature, &cached.repair)
                        })
                    });
                cached.is_none()
                    && !self.failed.get(&(source, repair.id)).is_some_and(|failed| {
                        current(failed.loaded, failed.signature, &failed.repair)
                    })
            })?
            .clone();
        Some(repair)
    }

    fn start(&mut self, request: &Request) {
        if let Some(pending) = &self.pending {
            pending
                .evaluation
                .cancelled
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.pending = Some(Pending {
            evaluation: request.clone(),
            task: None,
        });
    }

    fn accept(&mut self, response: Response, expected: Option<&Repair>, accepted: bool) -> bool {
        let Some(pending) = self
            .pending
            .take_if(|p| p.evaluation.request == response.request)
        else {
            return false;
        };
        if pending
            .evaluation
            .cancelled
            .load(std::sync::atomic::Ordering::Relaxed)
            || pending.evaluation.request != response.request
            || pending.evaluation.loaded != response.loaded
            || pending.evaluation.source != response.source
            || pending.evaluation.signature != response.signature
            || pending.evaluation.repair != response.repair
            || expected != Some(&pending.evaluation.repair)
        {
            return false;
        }
        self.failed.remove(&(response.source, response.repair.id));
        let cache = if accepted {
            &mut self.cached
        } else {
            &mut self.preview_cached
        };
        cache.insert(
            (response.source, response.repair.id),
            Cached {
                loaded: response.loaded,
                signature: response.signature,
                repair: response.repair,
                bound: repair_bound(&response.evaluated),
                evaluated: response.evaluated,
            },
        );
        true
    }

    fn fail(&mut self, request: u64) -> bool {
        let Some(pending) = self
            .pending
            .take_if(|pending| pending.evaluation.request == request)
        else {
            return false;
        };
        if pending
            .evaluation
            .cancelled
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return false;
        }
        self.failed.insert(
            (pending.evaluation.source, pending.evaluation.repair.id),
            Failed {
                loaded: pending.evaluation.loaded,
                signature: pending.evaluation.signature,
                repair: pending.evaluation.repair,
            },
        );
        true
    }
}

#[cfg(test)]
pub(super) fn evaluate(scan: &Scan, request: &Request) -> Result<Response, Error> {
    evaluate_with(scan, request, &|| false).map(|r| r.expect("uncancelled test evaluation"))
}

pub(super) fn evaluate_with(
    scan: &Scan,
    request: &Request,
    cancelled: &dyn Fn() -> bool,
) -> Result<Option<Response>, Error> {
    if cancelled() {
        return Ok(None);
    }
    let source = crate::scan::ImageSource::new(scan, request.input)?;
    let Some(evaluated) = crate::repair_source::evaluate_with(
        &source,
        &request.repair,
        &mut crate::dust_repair::Execution { cancelled },
    )?
    else {
        return Ok(None);
    };
    Ok(Some(Response {
        request: request.request,
        loaded: request.loaded,
        source: request.source,
        signature: request.signature,
        repair: request.repair.clone(),
        evaluated: Arc::new(evaluated),
    }))
}

fn brush_dab(
    draft: &mut MaskDraft,
    center: [f64; 2],
    size: u32,
    erase: bool,
    active: SourceRegion,
    blocked: &[SupportMask],
) -> Result<(bool, bool), Error> {
    let radius = (f64::from(size) * 0.5).max(0.75);
    let x0 = (center[0] - radius).floor() as i64;
    let y0 = (center[1] - radius).floor() as i64;
    let x1 = (center[0] + radius).ceil() as i64;
    let y1 = (center[1] + radius).ceil() as i64;
    let active_x0 = i64::from(active.x);
    let active_y0 = i64::from(active.y);
    let active_x1 = active_x0 + i64::from(active.width);
    let active_y1 = active_y0 + i64::from(active.height);
    let mut candidates = BTreeSet::new();
    let mut touched_blocked = false;
    for y in y0.max(active_y0)..y1.min(active_y1) {
        for x in x0.max(active_x0)..x1.min(active_x1) {
            let dx = x as f64 + 0.5 - center[0];
            let dy = y as f64 + 0.5 - center[1];
            if dx * dx + dy * dy > radius * radius {
                continue;
            }
            let point = [x as u32, y as u32];
            if !erase && blocked.iter().any(|support| support.contains(point)) {
                touched_blocked = true;
            } else {
                candidates.insert(point);
            }
        }
    }
    if erase {
        let before = draft.pixels.len();
        for point in candidates {
            draft.pixels.remove(&point);
        }
        return Ok((draft.pixels.len() != before, false));
    }
    let added = candidates
        .iter()
        .filter(|point| !draft.pixels.contains(*point))
        .count();
    if added == 0 {
        return Ok((false, touched_blocked));
    }
    let min_x = draft
        .pixels
        .iter()
        .chain(candidates.iter())
        .map(|point| point[0])
        .min()
        .unwrap();
    let min_y = draft
        .pixels
        .iter()
        .chain(candidates.iter())
        .map(|point| point[1])
        .min()
        .unwrap();
    let max_x = draft
        .pixels
        .iter()
        .chain(candidates.iter())
        .map(|point| point[0])
        .max()
        .unwrap();
    let max_y = draft
        .pixels
        .iter()
        .chain(candidates.iter())
        .map(|point| point[1])
        .max()
        .unwrap();
    if !mask_within_budget(
        [max_x - min_x + 1, max_y - min_y + 1],
        draft.pixels.len() + added,
    ) {
        return Err(Error::InvalidParameter("dust repair mask is too large"));
    }
    draft.pixels.extend(candidates);
    Ok((true, touched_blocked))
}

fn brush_stroke(
    draft: &mut MaskDraft,
    point: [f64; 2],
    size: u32,
    erase: bool,
    active: SourceRegion,
    blocked: &[SupportMask],
) -> Result<(bool, bool), Error> {
    let start = draft.last.unwrap_or(point);
    draft.last = Some(point);
    let distance = ((point[0] - start[0]).powi(2) + (point[1] - start[1]).powi(2)).sqrt();
    let steps = (distance / (f64::from(size) * 0.25).max(1.)).ceil() as usize;
    let mut changed = false;
    let mut touched_blocked = false;
    for step in 0..=steps.max(1) {
        let t = step as f64 / steps.max(1) as f64;
        let center = [
            start[0] + (point[0] - start[0]) * t,
            start[1] + (point[1] - start[1]) * t,
        ];
        let result = brush_dab(draft, center, size, erase, active, blocked)?;
        changed |= result.0;
        touched_blocked |= result.1;
    }
    Ok((changed, touched_blocked))
}

#[cfg(test)]
fn screen_polygon(geometry: crate::viewport::CanvasLayout, coverage: Coverage) -> Vec<egui::Pos2> {
    coverage
        .corners()
        .into_iter()
        .map(|point| geometry.screen_frame_point(geometry.composition.source_to_frame(point)))
        .collect()
}

fn support_boundary_segments(support: &SupportMask) -> Vec<[[f64; 2]; 2]> {
    let origin = support.origin();
    let size = support.size();
    let selected = |x: i64, y: i64| {
        x >= 0
            && y >= 0
            && x < i64::from(size[0])
            && y < i64::from(size[1])
            && support.contains([origin[0] + x as u32, origin[1] + y as u32])
    };
    let mut segments = Vec::new();
    for y in 0..=size[1] {
        let mut x = 0;
        while x < size[0] {
            while x < size[0]
                && selected(i64::from(x), i64::from(y) - 1) == selected(i64::from(x), i64::from(y))
            {
                x += 1;
            }
            let start = x;
            while x < size[0]
                && selected(i64::from(x), i64::from(y) - 1) != selected(i64::from(x), i64::from(y))
            {
                x += 1;
            }
            if start < x {
                segments.push([
                    [f64::from(origin[0] + start), f64::from(origin[1] + y)],
                    [f64::from(origin[0] + x), f64::from(origin[1] + y)],
                ]);
            }
        }
    }
    for x in 0..=size[0] {
        let mut y = 0;
        while y < size[1] {
            while y < size[1]
                && selected(i64::from(x) - 1, i64::from(y)) == selected(i64::from(x), i64::from(y))
            {
                y += 1;
            }
            let start = y;
            while y < size[1]
                && selected(i64::from(x) - 1, i64::from(y)) != selected(i64::from(x), i64::from(y))
            {
                y += 1;
            }
            if start < y {
                segments.push([
                    [f64::from(origin[0] + x), f64::from(origin[1] + start)],
                    [f64::from(origin[0] + x), f64::from(origin[1] + y)],
                ]);
            }
        }
    }
    segments
}

fn screen_source_point(geometry: crate::viewport::CanvasLayout, point: [f64; 2]) -> egui::Pos2 {
    geometry.screen_frame_point(
        geometry
            .composition
            .source_to_frame(crate::geometry::SourcePoint(point)),
    )
}

fn paint_support_outline(
    painter: &egui::Painter,
    geometry: crate::viewport::CanvasLayout,
    support: &SupportMask,
    offset: [f64; 2],
    color: egui::Color32,
    width: f32,
) {
    for [mut start, mut end] in support_boundary_segments(support) {
        start[0] += offset[0];
        start[1] += offset[1];
        end[0] += offset[0];
        end[1] += offset[1];
        let points = [
            screen_source_point(geometry, start),
            screen_source_point(geometry, end),
        ];
        painter.line_segment(
            points,
            egui::Stroke::new(
                width + 1.,
                egui::Color32::from_black_alpha((u16::from(color.a()) * 2 / 3) as u8),
            ),
        );
        painter.line_segment(points, egui::Stroke::new(width, color));
    }
}

fn support_anchor(support: &SupportMask) -> [f64; 2] {
    let origin = support.origin();
    let size = support.size();
    let center = [
        f64::from(origin[0]) + f64::from(size[0]) * 0.5,
        f64::from(origin[1]) + f64::from(size[1]) * 0.5,
    ];
    support
        .selected_points()
        .min_by(|a, b| {
            let distance = |point: [u32; 2]| {
                (f64::from(point[0]) + 0.5 - center[0]).powi(2)
                    + (f64::from(point[1]) + 0.5 - center[1]).powi(2)
            };
            distance(*a).total_cmp(&distance(*b))
        })
        .map_or(center, |point| {
            [f64::from(point[0]) + 0.5, f64::from(point[1]) + 0.5]
        })
}

fn distance_to_segment(point: egui::Pos2, start: egui::Pos2, end: egui::Pos2) -> f32 {
    let segment = end - start;
    let length_squared = segment.length_sq();
    if length_squared == 0. {
        return point.distance(start);
    }
    let along = ((point - start).dot(segment) / length_squared).clamp(0., 1.);
    point.distance(start + along * segment)
}

fn repair_outline_hit(
    geometry: crate::viewport::CanvasLayout,
    support: &SupportMask,
    point: egui::Pos2,
) -> bool {
    let [x, y] = support.origin();
    let [width, height] = support.size();
    // Hovering needs this test every frame; reject distant masks before tracing their edges.
    if !geometry
        .screen_region(SourceRegion {
            x,
            y,
            width,
            height,
        })
        .expand(9.)
        .contains(point)
    {
        return false;
    }
    support_boundary_segments(support)
        .into_iter()
        .any(|segment| {
            distance_to_segment(
                point,
                screen_source_point(geometry, segment[0]),
                screen_source_point(geometry, segment[1]),
            ) <= 8.
        })
}

impl Editor {
    pub(super) fn dust_repairs(&self, source: SourceId) -> Option<&RepairSet> {
        self.document
            .project
            .source(source)
            .ok()
            .map(|source| &source.repairs)
            .filter(|set| !set.repairs().is_empty())
    }

    pub(super) fn dust_repairs_mut(&mut self, source: SourceId) -> &mut RepairSet {
        &mut self
            .document
            .project
            .source_mut(source)
            .expect("accepted source")
            .repairs
    }

    pub(super) fn dust_render_state(&self) -> &'static str {
        if self.source {
            return "Repairs hidden";
        }
        if self.document.dust.render_error.is_some() {
            return "Repair unavailable";
        }

        let Some(source) = self.current_source else {
            return "Repairs pending";
        };
        let Some(signature) = self.dust_signature() else {
            return "Repairs pending";
        };

        let repairs = self.effective_dust_repairs(source);
        let mut pending = self
            .document
            .dust
            .searches
            .supports(source)
            .next()
            .is_some();
        for repair in repairs.iter().filter(|repair| repair.enabled) {
            if self
                .document
                .dust
                .failed
                .get(&(source, repair.id))
                .is_some_and(|failed| {
                    failed.loaded == self.id
                        && failed.signature == signature
                        && failed.repair == *repair
                })
            {
                return "Repair unavailable";
            }

            let matches = |cached: &&Cached| {
                cached.loaded == self.id
                    && cached.signature == signature
                    && cached.repair == *repair
            };
            if self
                .document
                .dust
                .cached
                .get(&(source, repair.id))
                .filter(matches)
                .is_none()
                && self
                    .document
                    .dust
                    .preview_cached
                    .get(&(source, repair.id))
                    .filter(matches)
                    .is_none()
            {
                pending = true;
            }
        }

        if pending {
            "Repairs pending"
        } else {
            "Repairs"
        }
    }

    fn dust_repair_changed(&mut self, source: SourceId, id: RepairId) {
        let accepted = &self
            .document
            .state
            .project
            .source(source)
            .expect("accepted source")
            .repairs;
        self.document.dust.repair_changed(source, id, accepted);
    }

    pub(super) fn reset_dust_repairs(&mut self, source: SourceId) {
        let before = self.history_snapshot();
        self.dust_repairs_mut(source).clear();
        self.document.dust.clear_repairs(source);
        self.record_edit(before, "Reset Dust repairs", Some(source));
    }

    pub(super) fn dust_controls(&mut self, ui: &mut egui::Ui, cx: &UiContext<'_, Self>) {
        use presentation::ToolIdentity;
        if !self.document.dust.cleanup {
            return;
        }
        if std::mem::take(&mut self.document.dust.reveal_controls) {
            ui.scroll_to_cursor(Some(egui::Align::Min));
        }
        let Some(source) = self.current_source else {
            return;
        };
        let repair_count = self
            .dust_repairs(source)
            .map_or(0, |set| set.repairs().len());
        let searching = self
            .document
            .dust
            .searches
            .supports(source)
            .next()
            .is_some();
        let available = !self.busy
            && self.document.visit.ready.is_some()
            && self.pick.is_none()
            && self.document.dust.donor_draft.is_none();
        let mut reset_repairs = false;
        ToolIdentity {
            icon: icons::Icon::Dust,
            title: "Dust repair",
        }
        .inspector(
            ui,
            |ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let reset = presentation::reset_button(
                        ui,
                        available && (repair_count > 0 || searching),
                        "dust repairs",
                    );
                    ui.weak(format!("{repair_count} repairs"));
                    reset
                })
                .inner
            },
            |ui, reset| {
                ui.horizontal(|ui| {
                    ui.label("Brush size")
                        .on_hover_text("Diameter in source pixels.");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.weak(format!("{} px", self.document.dust.brush_size));
                    });
                });
                ui.scope(|ui| {
                    ui.spacing_mut().slider_width = ui.available_width();
                    ui.add(
                        egui::Slider::new(&mut self.document.dust.brush_size, 1..=128)
                            .show_value(false)
                            .step_by(1.),
                    );
                });
                if let Some(error) = &self.document.dust.render_error {
                    ui.small("A repair is outside the current grade's GPU range.")
                        .on_hover_text(error);
                }
                ui.checkbox(&mut self.document.dust.show_masks, "Show repair outlines")
                    .on_hover_text("Hold H for a momentary unobstructed preview.");
                let rejected = self.document
                    .dust
                    .mask_draft
                    .as_ref()
                    .is_some_and(|draft| draft.rejected);
                let guidance = egui::RichText::new(if rejected {
                    "Stroke too large · release to discard it and use smaller repairs."
                } else if self.selected_dust_repair().is_some()
                {
                    "Draw for a new heal · Shift adds to selected · Option removes · drag its source."
                } else {
                    "Draw for a new heal · click an outline to select it · Delete removes selected."
                });
                ui.small(if rejected {
                    guidance.color(ui.visuals().error_fg_color)
                } else {
                    guidance
                });
                if searching {
                    ui.horizontal(|ui| { ui.spinner(); ui.weak("Finding donor…"); });
                } else if repair_count == 0 {
                    ui.small("Manual masks only · automatic proposals are not wired yet");
                }
                reset_repairs = reset.clicked();
            },
        );
        if reset_repairs {
            self.control_command(editing::Command::ResetRepairs(source), cx);
            self.status = "Dust repairs reset".into();
            self.invalidate_settled();
        }
    }

    fn invalidate_settled(&mut self) {
        self.invalidate_original();
    }

    pub(super) fn dust_interaction(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        geometry: crate::viewport::CanvasLayout,
        canvas_input: bool,
        panning: bool,
    ) {
        if !self.document.dust.cleanup || self.tab != 1 || self.fullscreen.is_some() || self.source
        {
            return;
        }
        let modifies = ui.input(|input| {
            input
                .raw
                .events
                .iter()
                .find_map(|event| match event {
                    egui::Event::PointerButton {
                        button: egui::PointerButton::Primary,
                        modifiers,
                        ..
                    } => Some(*modifiers),
                    _ => None,
                })
                .unwrap_or(input.modifiers)
        });
        let allowed = canvas_input
            && !panning
            && !self.busy
            && self.pick.is_none()
            && self.document.visit.crop_edit.is_none()
            && self.flat_edit.is_none()
            && self.document.analysis_review.is_none()
            && ui.is_enabled()
            && !ui.input(|i| i.key_down(egui::Key::Space));
        if allowed {
            let started = response.drag_started_by(egui::PointerButton::Primary);
            let clicked = response.clicked_by(egui::PointerButton::Primary);
            let start = started
                .then(|| ui.input(|i| i.pointer.press_origin()))
                .flatten()
                .or_else(|| response.interact_pointer_pos());
            if (started || clicked)
                && self.document.dust.mask_draft.is_none()
                && !self.dust_donor_dragging()
                && let Some(point) = start.filter(|p| geometry.image.contains(*p))
            {
                if modifies.alt || modifies.shift {
                    if let Some(id) = self.selected_dust_repair() {
                        self.begin_dust_mask(Some(id));
                    }
                } else if let Some(id) = self.dust_repair_at(geometry, point) {
                    self.select_dust_repair(id);
                } else if self.dust_donor_at(geometry, point) {
                    if started {
                        self.begin_dust_donor_drag(geometry, point);
                    }
                } else {
                    self.begin_dust_mask(None);
                }
                if self.document.dust.mask_draft.is_some() {
                    self.begin_dust_stroke();
                    self.paint_dust_stroke(geometry, point, modifies.alt);
                }
            }
            if response.dragged_by(egui::PointerButton::Primary)
                && let Some(point) = response.interact_pointer_pos()
            {
                if self.dust_donor_dragging() {
                    self.drag_dust_donor(geometry, point);
                } else if self.document.dust.mask_draft.is_some() {
                    self.paint_dust_stroke(geometry, point, modifies.alt);
                }
            }
            if clicked {
                self.commit_dust_mask();
            }
        }
        if response.drag_stopped_by(egui::PointerButton::Primary) {
            self.end_dust_stroke();
            self.end_dust_donor_drag();
            self.commit_dust_mask();
        }
    }

    pub(super) fn begin_dust_cleanup(&mut self) {
        if self.document.auto_crop_edit.is_some() {
            return;
        }
        self.finish_adjustment_layer_interaction();
        if self.document.dust.brush_source != self.current_source
            && let (Some(source), Some(active)) = (
                self.current_source,
                self.document.visit.ready.as_ref().map(|ready| ready.area),
            )
        {
            self.document.dust.brush_size = initial_brush_size(active);
            self.document.dust.brush_source = Some(source);
        }
        self.document.dust.cleanup = true;
        self.document.dust.reveal_controls = true;
        self.document.dust.show_masks = true;
    }

    pub(super) fn finish_dust_cleanup(&mut self) {
        self.cancel_dust_mask();
        self.document.dust.cleanup = false;
        self.document.dust.show_masks = false;
        self.document.dust.donor_draft = None;
    }

    pub(super) fn begin_dust_mask(&mut self, target: Option<RepairId>) {
        if let (Some(source), Some(repair)) = (self.current_source, target)
            && self.document.dust.searches.has_target(source, repair)
        {
            self.status = "Wait for this repair's donor search before editing its mask".into();
            return;
        }
        self.document.dust.cleanup = true;
        self.document.dust.show_masks = true;
        self.document.dust.donor_draft = None;
        let support = target.and_then(|id| {
            let source = self.current_source?;
            self.dust_repairs(source)?
                .repairs()
                .iter()
                .find(|repair| repair.id == id)
                .map(|repair| repair.support.clone())
        });
        self.document.dust.mask_draft = Some(MaskDraft::new(target, support.as_ref()));
    }

    pub(super) fn cancel_dust_mask(&mut self) {
        self.document.dust.mask_draft = None;
        self.document.dust.donor_draft = None;
    }

    pub(super) fn selected_dust_repair(&self) -> Option<RepairId> {
        let source = self.current_source?;
        self.document
            .dust
            .selected
            .get(&source)
            .copied()
            .filter(|id| {
                self.dust_repairs(source)
                    .is_some_and(|set| set.repairs().iter().any(|repair| repair.id == *id))
            })
    }

    pub(super) fn commit_dust_mask(&mut self) {
        let Some(draft) = self.document.dust.mask_draft.as_ref() else {
            return;
        };
        let target = draft.target;
        if draft.pixels.is_empty() {
            self.cancel_dust_mask();
            if target.is_some() {
                self.delete_selected_dust_repair();
            }
        } else {
            self.apply_dust_mask();
        }
    }

    pub(super) fn adjust_dust_brush_from_scroll(
        &mut self,
        ui: &egui::Ui,
        geometry: crate::viewport::CanvasLayout,
    ) {
        if !self.document.dust.cleanup || self.source {
            return;
        }
        let delta = ui.input_mut(|input| {
            if input
                .pointer
                .hover_pos()
                .is_none_or(|point| !geometry.image.contains(point))
            {
                return 0.;
            }
            let delta = input
                .raw
                .events
                .iter()
                .filter_map(|event| match event {
                    egui::Event::MouseWheel {
                        delta, modifiers, ..
                    } if !modifiers.command && !modifiers.ctrl => Some(delta.x + delta.y),
                    _ => None,
                })
                .sum();
            // Egui smooths wheel input across later frames. Cleanup owns an ordinary
            // wheel gesture for brush sizing, so suppress both its first pan and tail.
            input.smooth_scroll_delta = egui::Vec2::ZERO;
            delta
        });
        if delta.abs() < f32::EPSILON {
            return;
        }
        let step = ((delta.abs() / 12.).round() as u32).max(1);
        self.document.dust.brush_size = if delta > 0. {
            self.document.dust.brush_size.saturating_add(step).min(128)
        } else {
            self.document.dust.brush_size.saturating_sub(step).max(1)
        };
    }

    pub(super) fn apply_dust_mask(&mut self) {
        let Some(source) = self.current_source else {
            self.cancel_dust_mask();
            return;
        };
        let Some(draft) = &self.document.dust.mask_draft else {
            return;
        };
        if draft.rejected {
            self.cancel_dust_mask();
            self.status = "Dust stroke rejected: repair budget exceeded; no changes applied".into();
            return;
        }
        let target = draft.target;
        let support = match draft.support() {
            Ok(support) => support,
            Err(error) => {
                self.status = format!("Dust mask rejected: {error}");
                self.cancel_dust_mask();
                return;
            }
        };
        let previous = target.and_then(|id| {
            self.dust_repairs(source)?
                .repairs()
                .iter()
                .find(|r| r.id == id)
                .cloned()
        });
        if let Some(previous) = &previous
            && previous.support == support
        {
            self.cancel_dust_mask();
            return;
        }
        if target.is_some() && previous.is_none() {
            self.cancel_dust_mask();
            return;
        }
        self.queue_dust_search(source, previous, support);
        self.cancel_dust_mask();
        self.status = "Finding a donor…".into();
        self.invalidate_settled();
    }

    fn dust_source_position(
        &self,
        geometry: crate::viewport::CanvasLayout,
        point: egui::Pos2,
    ) -> Option<[f64; 2]> {
        if !geometry.image.contains(point) {
            return None;
        }
        let source = geometry
            .composition
            .frame_to_source(geometry.frame_point(point))
            .0;
        let active = self.document.visit.ready.as_ref()?.area;
        (source[0] >= f64::from(active.x)
            && source[1] >= f64::from(active.y)
            && source[0] < f64::from(active.x + active.width)
            && source[1] < f64::from(active.y + active.height))
        .then_some(source)
    }

    pub(super) fn begin_dust_stroke(&mut self) {
        if let Some(draft) = &mut self.document.dust.mask_draft {
            draft.clear_stroke();
        }
    }

    pub(super) fn paint_dust_stroke(
        &mut self,
        geometry: crate::viewport::CanvasLayout,
        point: egui::Pos2,
        temporary_erase: bool,
    ) {
        let Some(position) = self.dust_source_position(geometry, point) else {
            self.end_dust_stroke();
            return;
        };
        let Some(source) = self.current_source else {
            return;
        };
        if self
            .document
            .dust
            .mask_draft
            .as_ref()
            .is_some_and(|draft| draft.rejected)
        {
            return;
        }
        let Some(target) = self
            .document
            .dust
            .mask_draft
            .as_ref()
            .map(|draft| draft.target)
        else {
            return;
        };
        let blocked = self
            .dust_repairs(source)
            .into_iter()
            .flat_map(RepairSet::repairs)
            .filter(|repair| Some(repair.id) != target)
            .map(|repair| repair.support.clone())
            .chain(self.document.dust.searches.supports(source).cloned())
            .collect::<Vec<_>>();
        let active = self.document.visit.ready.as_ref().unwrap().area;
        let erase = temporary_erase;
        let result = brush_stroke(
            self.document.dust.mask_draft.as_mut().unwrap(),
            position,
            self.document.dust.brush_size,
            erase,
            active,
            &blocked,
        );
        match result {
            Ok((_, true)) => self.status = "Dust masks cannot overlap another repair".into(),
            Ok(_) => {}
            Err(_) => {
                self.document.dust.mask_draft.as_mut().unwrap().rejected = true;
                self.status = "Dust stroke too large; release to discard it".into();
            }
        }
    }

    pub(super) fn end_dust_stroke(&mut self) {
        if let Some(draft) = &mut self.document.dust.mask_draft {
            draft.clear_stroke();
        }
    }

    pub(super) fn dust_canvas_cursor(
        &self,
        geometry: crate::viewport::CanvasLayout,
        point: Option<egui::Pos2>,
        modifying: bool,
    ) -> egui::CursorIcon {
        if self.dust_donor_dragging() {
            egui::CursorIcon::Grabbing
        } else if modifying || self.document.dust.mask_draft.is_some() {
            egui::CursorIcon::Crosshair
        } else if point.is_some_and(|p| self.dust_repair_at(geometry, p).is_some()) {
            egui::CursorIcon::PointingHand
        } else if point.is_some_and(|p| self.dust_donor_at(geometry, p)) {
            egui::CursorIcon::Grab
        } else {
            egui::CursorIcon::Crosshair
        }
    }

    pub(super) fn dust_repair_at(
        &self,
        geometry: crate::viewport::CanvasLayout,
        point: egui::Pos2,
    ) -> Option<RepairId> {
        if self.busy
            || self.tab != 1
            || self.source
            || !self.document.dust.show_masks
            || self.pick.is_some()
            || self.document.dust.donor_draft.is_some()
        {
            return None;
        }
        let source_point = self.dust_source_position(geometry, point)?;
        let pixel = [
            source_point[0].floor() as u32,
            source_point[1].floor() as u32,
        ];
        let source = self.current_source?;
        let set = self.dust_repairs(source)?;
        let selected = self.selected_dust_repair();
        set.repairs()
            .iter()
            .filter(|repair| Some(repair.id) == selected)
            .chain(
                set.repairs()
                    .iter()
                    .rev()
                    .filter(|repair| Some(repair.id) != selected),
            )
            .find(|repair| {
                repair.support.contains(pixel)
                    || repair_outline_hit(geometry, &repair.support, point)
            })
            .map(|repair| repair.id)
    }

    pub(super) fn select_dust_repair(&mut self, id: RepairId) {
        if let Some(source) = self.current_source
            && self
                .dust_repairs(source)
                .is_some_and(|set| set.repairs().iter().any(|repair| repair.id == id))
        {
            self.document.dust.searches.manual_selection(source);
            self.document.dust.selected.insert(source, id);
        }
    }

    pub(super) fn delete_selected_dust_repair(&mut self) {
        let Some(source) = self.current_source else {
            return;
        };
        let Some(id) = self.selected_dust_repair() else {
            return;
        };
        self.cancel_dust_mask();
        let before = self.history_snapshot();
        if self.dust_repairs_mut(source).remove(id).is_ok() {
            self.record_edit(before, "Delete Dust repair", Some(source));
            self.document.dust.removed(source, id);
            self.status = "Dust repair deleted".into();
            self.invalidate_settled();
        }
    }

    pub(super) fn dust_donor_regions(&self) -> Vec<DonorRegion> {
        self.dust_donor_regions_inner().unwrap_or_default()
    }

    fn dust_donor_regions_inner(&self) -> Option<Vec<DonorRegion>> {
        let source = self.current_source?;
        let repair_id = self
            .document
            .dust
            .donor_draft
            .map(|draft| draft.repair)
            .or_else(|| {
                self.document
                    .dust
                    .mask_draft
                    .as_ref()
                    .and_then(|draft| draft.target)
            })
            .or_else(|| {
                (self.document.dust.cleanup && self.document.dust.mask_draft.is_none())
                    .then(|| self.selected_dust_repair())
                    .flatten()
            })?;
        if self.document.dust.searches.has_target(source, repair_id) {
            return None;
        }
        let effective = self.effective_dust_repair(source, repair_id)?;
        let active = self.document.visit.ready.as_ref()?.area;
        let target = donor_target(&effective.support, active).ok()?;
        let donor = SourceRegion {
            x: u32::try_from(i64::from(target.x) + i64::from(effective.offset[0])).ok()?,
            y: u32::try_from(i64::from(target.y) + i64::from(effective.offset[1])).ok()?,
            width: target.width,
            height: target.height,
        };
        Some(vec![DonorRegion {
            support: effective.support,
            target,
            donor,
        }])
    }

    pub(super) fn dust_donor_at(
        &self,
        geometry: crate::viewport::CanvasLayout,
        point: egui::Pos2,
    ) -> bool {
        self.dust_donor_hit(geometry, point).is_some()
    }

    fn dust_donor_hit(
        &self,
        geometry: crate::viewport::CanvasLayout,
        point: egui::Pos2,
    ) -> Option<DonorRegion> {
        if !self.document.dust.show_masks {
            return None;
        }
        let position = self.dust_source_position(geometry, point)?;
        self.dust_donor_regions().into_iter().find(|region| {
            let offset = [
                f64::from(region.donor.x) - f64::from(region.target.x),
                f64::from(region.donor.y) - f64::from(region.target.y),
            ];
            let target = [position[0] - offset[0], position[1] - offset[1]];
            let inside = target[0] >= 0.
                && target[1] >= 0.
                && region
                    .support
                    .contains([target[0].floor() as u32, target[1].floor() as u32]);
            let anchor = support_anchor(&region.support);
            inside
                || screen_source_point(geometry, [anchor[0] + offset[0], anchor[1] + offset[1]])
                    .distance(point)
                    <= 10.
        })
    }

    pub(super) fn begin_dust_donor_drag(
        &mut self,
        geometry: crate::viewport::CanvasLayout,
        point: egui::Pos2,
    ) {
        let Some(region) = self.dust_donor_hit(geometry, point) else {
            return;
        };
        let Some(source_point) = self.dust_source_position(geometry, point) else {
            return;
        };
        if self.document.dust.donor_draft.is_none() {
            let Some(repair) = self.selected_dust_repair() else {
                return;
            };
            let (target, donor) = (region.target, region.donor);
            let (Ok(offset_x), Ok(offset_y)) = (
                i32::try_from(i64::from(donor.x) - i64::from(target.x)),
                i32::try_from(i64::from(donor.y) - i64::from(target.y)),
            ) else {
                return;
            };
            let offset = [offset_x, offset_y];
            self.document.dust.donor_draft = Some(DonorDraft {
                repair,
                offset,
                drag: None,
            });
        }
        if let Some(draft) = &mut self.document.dust.donor_draft {
            draft.drag = Some(DonorDrag {
                source: source_point,
                offset: draft.offset,
            });
        }
    }

    pub(super) fn drag_dust_donor(
        &mut self,
        geometry: crate::viewport::CanvasLayout,
        point: egui::Pos2,
    ) {
        let Some(position) = self.dust_source_position(geometry, point) else {
            return;
        };
        let Some(active) = self.document.visit.ready.as_ref().map(|ready| ready.area) else {
            return;
        };
        let Some(support) = self
            .current_source
            .and_then(|source| {
                self.selected_dust_repair()
                    .and_then(|id| self.effective_dust_repair(source, id))
            })
            .map(|r| r.support)
        else {
            return;
        };
        let Ok(target) = donor_target(&support, active) else {
            return;
        };
        let Some(draft) = &mut self.document.dust.donor_draft else {
            return;
        };
        let Some(drag) = draft.drag else {
            return;
        };
        let requested = [
            i64::from(drag.offset[0]) + (position[0] - drag.source[0]).round() as i64,
            i64::from(drag.offset[1]) + (position[1] - drag.source[1]).round() as i64,
        ];
        let minimum = [
            i64::from(active.x) - i64::from(target.x),
            i64::from(active.y) - i64::from(target.y),
        ];
        let maximum = [
            i64::from(active.x + active.width) - i64::from(target.x + target.width),
            i64::from(active.y + active.height) - i64::from(target.y + target.height),
        ];
        let offset = [
            requested[0].clamp(minimum[0], maximum[0]) as i32,
            requested[1].clamp(minimum[1], maximum[1]) as i32,
        ];
        if draft.offset != offset {
            draft.offset = offset;
            self.invalidate_settled();
        }
    }

    pub(super) fn dust_donor_dragging(&self) -> bool {
        self.document
            .dust
            .donor_draft
            .is_some_and(|draft| draft.drag.is_some())
    }

    pub(super) fn end_dust_donor_drag(&mut self) {
        let Some(draft) = self.document.dust.donor_draft.take() else {
            return;
        };
        if draft.drag.is_none() {
            return;
        }
        let Some(source) = self.current_source else {
            return;
        };
        let Some((support, accepted_offset)) = self
            .dust_repairs(source)
            .and_then(|set| {
                set.repairs()
                    .iter()
                    .find(|repair| repair.id == draft.repair)
            })
            .map(|repair| (repair.support.clone(), repair.offset))
        else {
            return;
        };
        if draft.offset == accepted_offset {
            return;
        }
        let before = self.history_snapshot();
        let result = self
            .document
            .visit
            .ready
            .as_ref()
            .map(|ready| ready.area)
            .ok_or(Error::InvalidParameter("manual donor unavailable"))
            .and_then(|active| validate_dust_offset(draft.offset, &support, active))
            .and_then(|()| {
                self.dust_repairs_mut(source)
                    .set_offset(draft.repair, draft.offset)
            });
        match result {
            Ok(()) => {
                self.dust_repair_changed(source, draft.repair);
                self.record_edit(before, "Dust donor", Some(source));
                self.status = "Dust source updated".into();
            }
            Err(error) => {
                self.status = format!("Dust source rejected: {error}");
            }
        }
        self.invalidate_settled();
    }

    fn effective_dust_repairs(&self, source: SourceId) -> Vec<Repair> {
        let target = self
            .document
            .dust
            .mask_draft
            .as_ref()
            .filter(|draft| !draft.rejected)
            .and_then(|draft| draft.target);
        let support = self
            .document
            .dust
            .mask_draft
            .as_ref()
            .filter(|draft| draft.target.is_some() && !draft.rejected)
            .and_then(|draft| draft.support().ok());
        self.dust_repairs(source)
            .into_iter()
            .flat_map(RepairSet::repairs)
            .filter_map(|repair| {
                let mut repair = repair.clone();
                if Some(repair.id) == target {
                    repair.support = support.clone()?;
                }
                if let Some(donor) = self
                    .document
                    .dust
                    .donor_draft
                    .filter(|donor| donor.repair == repair.id)
                {
                    repair.offset = donor.offset;
                }
                Some(repair)
            })
            .collect()
    }

    fn effective_dust_repair(&self, source: SourceId, id: RepairId) -> Option<Repair> {
        self.effective_dust_repairs(source)
            .into_iter()
            .find(|repair| repair.id == id)
    }

    pub(super) fn dust_signature(&self) -> Option<Signature> {
        self.neutral_source_signature(&self.input()).ok()
    }

    pub(super) fn dust_validate_processing(
        &self,
        input: impl Into<InputSettings>,
        roll: RollSettings,
        photo: AdjustmentLayerSettings,
    ) -> Result<(), Error> {
        let input = input.into();
        self.dust_validate_snapshot(input, roll, &self.dust_grade_snapshot(photo)?)
    }

    pub(super) fn dust_validate_snapshot(
        &self,
        input: impl Into<InputSettings>,
        roll: RollSettings,
        snapshot: &AdjustmentLayerSnapshot,
    ) -> Result<(), Error> {
        let input = input.into();
        if self.document.dust.gpu.is_empty() {
            return Ok(());
        }
        let Some(source) = self.current_source else {
            return Ok(());
        };
        let signature = self.neutral_source_signature(&input)?;
        let mut bound = DVec3::ZERO;
        for repair in self
            .effective_dust_repairs(source)
            .iter()
            .filter(|r| r.enabled)
        {
            let matches = |c: &&Cached| {
                c.loaded == self.id && c.signature == signature && c.repair == *repair
            };
            if let Some(cached) = self
                .document
                .dust
                .preview_cached
                .get(&(source, repair.id))
                .filter(matches)
                .or_else(|| {
                    self.document
                        .dust
                        .cached
                        .get(&(source, repair.id))
                        .filter(matches)
                })
                && self
                    .document
                    .dust
                    .gpu
                    .get(&repair.id)
                    .is_some_and(|p| Arc::ptr_eq(&p.evaluated, &cached.evaluated))
            {
                bound = bound.max(cached.bound);
            }
        }
        self.validate_repaired_source(&input, roll, snapshot, bound)
    }

    fn dust_grade_snapshot(
        &self,
        photo: AdjustmentLayerSettings,
    ) -> Result<AdjustmentLayerSnapshot, Error> {
        match self.current_source {
            Some(id) => self.adjustment_layer_snapshot(self.document.project.source(id)?, photo),
            None => AdjustmentLayerSnapshot::new(Vec::new()),
        }
    }

    pub(super) fn dust_gpu_snapshot(
        &mut self,
        grade: Option<&AdjustmentLayerSnapshot>,
    ) -> Result<crate::gpu::repairs::Snapshot, Error> {
        let Some(signature) = self.dust_signature() else {
            return Ok(Default::default());
        };
        let Some(source) = self.current_source else {
            return Ok(Default::default());
        };
        let Some(device) = self.working.device().cloned() else {
            return Ok(Default::default());
        };
        self.document.dust.render_error = None;
        let input = self.input();
        let snapshot = match grade {
            Some(grade) => grade.clone(),
            None => self.dust_grade_snapshot(self.adjustment_layer_settings)?,
        };
        let repairs = self.effective_dust_repairs(source);
        let candidates = repairs
            .iter()
            .filter(|r| r.enabled)
            .filter_map(|repair| {
                let current = |c: &&Cached| {
                    c.loaded == self.id && c.signature == signature && c.repair == *repair
                };
                self.document
                    .dust
                    .preview_cached
                    .get(&(source, repair.id))
                    .filter(current)
                    .or_else(|| {
                        self.document
                            .dust
                            .cached
                            .get(&(source, repair.id))
                            .filter(current)
                    })
                    .map(|c| (repair.id, c))
            })
            .collect::<Vec<_>>();
        let bound = candidates
            .iter()
            .fold(DVec3::ZERO, |bound, (_, c)| bound.max(c.bound));
        let all_valid = self
            .validate_repaired_source(&input, self.roll, &snapshot, bound)
            .is_ok();
        let mut admitted_bound = DVec3::ZERO;
        let mut patches = Vec::new();
        for (id, cached) in candidates {
            let bound = admitted_bound.max(cached.bound);
            if !all_valid
                && let Err(error) =
                    self.validate_repaired_source(&input, self.roll, &snapshot, bound)
            {
                self.document.dust.render_error = Some(error.to_string());
                continue;
            }
            let patch = match self
                .document
                .dust
                .gpu
                .get(&id)
                .filter(|p| Arc::ptr_eq(&p.evaluated, &cached.evaluated))
            {
                Some(patch) => patch.clone(),
                None => {
                    let patch = crate::gpu::repairs::Patch::new(&device, cached.evaluated.clone())?;
                    self.document.dust.gpu.insert(id, patch.clone());
                    patch
                }
            };
            admitted_bound = bound;
            patches.push(patch);
        }
        self.document
            .dust
            .gpu
            .retain(|_, patch| patches.iter().any(|p| Arc::ptr_eq(p, patch)));
        Ok(crate::gpu::repairs::Snapshot(patches))
    }

    pub(super) fn schedule_dust(&mut self, signature: Signature, cx: &UiContext<'_, Self>) {
        self.start_dust_searches(cx);
        if self.busy
            || self.document_pending.is_some()
            || self.source_epoch.load(std::sync::atomic::Ordering::Relaxed) != self.id
        {
            return;
        }
        let Some(source) = self.current_source else {
            return;
        };
        let repairs = self.effective_dust_repairs(source);
        if let Some(pending) = &self.document.dust.pending
            && (pending.evaluation.source != source
                || pending.evaluation.loaded != self.id
                || pending.evaluation.signature != signature
                || !repairs.contains(&pending.evaluation.repair))
        {
            // Retain admission until its reply; obsolete work is not a failed heal.
            pending
                .evaluation
                .cancelled
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
        let Some(repair) = self
            .document
            .dust
            .next_stale(source, self.id, signature, &repairs)
        else {
            return;
        };
        let Some(input) = self.source_context() else {
            return;
        };
        let request = Request {
            request: self.request_id(),
            loaded: self.id,
            source,
            signature,
            repair,
            input: self.input(),
            cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };
        self.document.dust.start(&request);
        self.status = "Evaluating full-resolution dust repair…".into();
        let id = request.request;
        let retire = EvaluationRetire(request.cancelled.clone());
        let services = self.dust_services.clone();
        let epoch = self.source_epoch.clone();
        let Some(task) = cx.spawn(move |cx| async move {
            let _retire = retire;
            let result = match services {
                Some(services) => services.evaluate(request, input, epoch).await,
                None => Err("Dust services unavailable".into()),
            };
            cx.update(|editor| {
                editor.receive_dust(id, result);
                if let Some(signature) = editor.dust_signature() {
                    editor.schedule_dust(signature, &cx);
                }
            });
        }) else {
            self.receive_dust(id, Err("Application stopped".into()));
            return;
        };
        self.document.dust.attach_task(id, task);
    }

    pub(super) fn receive_dust(&mut self, request: u64, result: Result<Option<Response>, String>) {
        match result {
            Ok(Some(response)) => {
                let current = response.loaded == self.id
                    && self.source_epoch.load(std::sync::atomic::Ordering::Relaxed)
                        == response.loaded
                    && Some(response.source) == self.current_source
                    && Some(response.signature) == self.dust_signature();
                if !current {
                    self.document
                        .dust
                        .pending
                        .take_if(|p| p.evaluation.request == request);
                    return;
                }
                let expected = self.effective_dust_repair(response.source, response.repair.id);
                let accepted = expected.as_ref().is_some_and(|repair| {
                    self.dust_repairs(response.source)
                        .is_some_and(|set| set.repairs().contains(repair))
                });
                if self
                    .document
                    .dust
                    .accept(response, expected.as_ref(), accepted)
                {
                    self.status = "Dust repair ready".into();
                    self.invalidate_settled();
                }
            }
            Ok(None) => {
                self.document
                    .dust
                    .pending
                    .take_if(|p| p.evaluation.request == request);
            }
            Err(error) => {
                if self.document.dust.fail(request) {
                    self.status = format!("Dust repair failed: {error}");
                }
            }
        }
    }

    #[cfg(all(test, feature = "ui-tests"))]
    pub(super) fn dust_evaluations(&self, signature: Signature) -> Vec<Arc<EvaluatedRepair>> {
        self.current_source.map_or_else(Vec::new, |source| {
            let repairs = self.effective_dust_repairs(source);
            self.document
                .dust
                .evaluations(source, self.id, signature, &repairs)
        })
    }

    pub(super) fn paint_dust(&self, ui: &egui::Ui, geometry: crate::viewport::CanvasLayout) {
        let Some(source) = self.current_source else {
            return;
        };
        if self.tab != 1 || self.fullscreen.is_some() || self.source {
            return;
        }
        let set = self.dust_repairs(source);
        let selected = self.selected_dust_repair();
        let signature = self.dust_signature();
        let mask_target = self
            .document
            .dust
            .mask_draft
            .as_ref()
            .and_then(|draft| draft.target);
        let painter = ui.painter().with_clip_rect(geometry.canvas);
        let overlays_hidden = ui.ctx().input(|input| input.key_down(egui::Key::H));
        if self.document.dust.show_masks && !overlays_hidden {
            for repair in set.into_iter().flat_map(RepairSet::repairs) {
                if mask_target == Some(repair.id)
                    || self.document.dust.searches.has_target(source, repair.id)
                {
                    continue;
                }
                let pending = self.document.dust.pending.as_ref().is_some_and(|pending| {
                    pending.evaluation.loaded == self.id
                        && pending.evaluation.source == source
                        && Some(pending.evaluation.signature) == signature
                        && pending.evaluation.repair == *repair
                });
                let failed = self
                    .document
                    .dust
                    .failed
                    .get(&(source, repair.id))
                    .is_some_and(|failed| {
                        failed.loaded == self.id
                            && Some(failed.signature) == signature
                            && failed.repair == *repair
                    });
                let active = selected == Some(repair.id);
                let color = if mask_target.is_some() {
                    egui::Color32::from_white_alpha(45)
                } else if !repair.enabled {
                    egui::Color32::from_white_alpha(65)
                } else if failed {
                    ui.visuals().error_fg_color.linear_multiply(0.7)
                } else if pending {
                    egui::Color32::from_rgb(235, 180, 80).linear_multiply(0.7)
                } else if active {
                    egui::Color32::from_rgb(100, 220, 240)
                } else {
                    egui::Color32::from_white_alpha(85)
                };
                paint_support_outline(
                    &painter,
                    geometry,
                    &repair.support,
                    [0., 0.],
                    color,
                    if active { 1.5 } else { 1. },
                );
            }
        }
        if self.document.dust.show_masks && !overlays_hidden {
            for support in self.document.dust.searches.supports(source) {
                paint_support_outline(
                    &painter,
                    geometry,
                    support,
                    [0., 0.],
                    ui.visuals().weak_text_color(),
                    1.5,
                );
            }
        }
        if !overlays_hidden
            && let Some(mask) = self
                .document
                .dust
                .mask_draft
                .as_ref()
                .and_then(|draft| draft.support().ok())
        {
            paint_support_outline(
                &painter,
                geometry,
                &mask,
                [0., 0.],
                if self
                    .document
                    .dust
                    .mask_draft
                    .as_ref()
                    .is_some_and(|draft| draft.rejected)
                {
                    ui.visuals().error_fg_color
                } else {
                    egui::Color32::from_rgb(100, 220, 240)
                },
                1.,
            );
        }
        let (point, add, erase, hide_cursor) = ui.ctx().input(|input| {
            (
                input.pointer.hover_pos(),
                input.modifiers.shift,
                input.modifiers.alt,
                input.key_down(egui::Key::H) || input.key_down(egui::Key::Space),
            )
        });
        if self.document.dust.cleanup
            && !self.source
            && !self.dust_donor_dragging()
            && !hide_cursor
            && let Some(point) = point
            && self.dust_canvas_cursor(geometry, Some(point), add || erase)
                == egui::CursorIcon::Crosshair
            && let Some(source_point) = self.dust_source_position(geometry, point)
        {
            let center = screen_source_point(geometry, source_point);
            let radius = (self.document.dust.brush_size as f32 * 0.5 * geometry.scale).max(2.);
            painter.circle_stroke(
                center,
                radius + 0.5,
                egui::Stroke::new(2., egui::Color32::from_black_alpha(170)),
            );
            painter.circle_stroke(
                center,
                radius,
                egui::Stroke::new(
                    1.5,
                    if self
                        .document
                        .dust
                        .mask_draft
                        .as_ref()
                        .is_some_and(|draft| draft.rejected)
                    {
                        ui.visuals().error_fg_color
                    } else if erase {
                        egui::Color32::from_rgb(240, 120, 100)
                    } else if add {
                        egui::Color32::from_rgb(100, 220, 240)
                    } else {
                        egui::Color32::WHITE
                    },
                ),
            );
        }
        if overlays_hidden || !self.document.dust.show_masks {
            return;
        }
        for region in self.dust_donor_regions() {
            let (target, donor, support) = (region.target, region.donor, region.support);
            let offset = [
                f64::from(donor.x) - f64::from(target.x),
                f64::from(donor.y) - f64::from(target.y),
            ];
            paint_support_outline(
                &painter,
                geometry,
                &support,
                [0., 0.],
                egui::Color32::WHITE,
                2.,
            );
            paint_support_outline(
                &painter,
                geometry,
                &support,
                offset,
                egui::Color32::from_rgb(100, 220, 240),
                2.,
            );
            let target_anchor = support_anchor(&support);
            let donor_anchor = [target_anchor[0] + offset[0], target_anchor[1] + offset[1]];
            painter.line_segment(
                [
                    screen_source_point(geometry, target_anchor),
                    screen_source_point(geometry, donor_anchor),
                ],
                egui::Stroke::new(1., egui::Color32::from_gray(220)),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_brush_tracks_the_negative_short_side_with_practical_bounds() {
        let area = |width, height| SourceRegion {
            x: 0,
            y: 0,
            width,
            height,
        };
        assert_eq!(initial_brush_size(area(240, 176)), 4);
        assert_eq!(initial_brush_size(area(6_000, 4_000)), 40);
        assert_eq!(initial_brush_size(area(40_000, 30_000)), 128);
    }

    #[test]
    fn brush_dab_becomes_an_authoritative_nonrectangular_source_mask() {
        let active = SourceRegion {
            x: 100,
            y: 200,
            width: 40,
            height: 30,
        };
        let mut draft = MaskDraft::new(None, None);
        assert!(
            brush_dab(&mut draft, [110., 208.], 6, false, active, &[])
                .unwrap()
                .0
        );
        let support = draft.support().unwrap();
        assert_eq!(support.origin(), [107, 205]);
        assert_eq!(support.size(), [6, 6]);
        assert!(support.contains([110, 208]));
        assert!(!support.contains([107, 205]));
    }

    #[test]
    fn support_boundary_follows_the_mask_instead_of_its_bounding_box() {
        let support = SupportMask::new([10, 20], [2, 2], vec![true, true, true, false]).unwrap();
        let segments = support_boundary_segments(&support);
        assert!(segments.contains(&[[11., 21.], [12., 21.]]));
        assert!(segments.contains(&[[11., 21.], [11., 22.]]));
        assert!(!segments.contains(&[[10., 22.], [12., 22.]]));
    }

    #[test]
    fn brush_strokes_are_continuous_erasable_and_skip_other_repairs() {
        let active = SourceRegion {
            x: 0,
            y: 0,
            width: 100,
            height: 100,
        };
        let mut draft = MaskDraft::new(None, None);
        brush_stroke(&mut draft, [10.5, 10.5], 4, false, active, &[]).unwrap();
        brush_stroke(&mut draft, [30.5, 10.5], 4, false, active, &[]).unwrap();
        for x in 10..=30 {
            assert!(draft.pixels.contains(&[x, 10]));
        }
        draft.clear_stroke();
        brush_stroke(&mut draft, [20.5, 10.5], 4, true, active, &[]).unwrap();
        assert!(!draft.pixels.contains(&[20, 10]));

        let blocked = SupportMask::new([40, 10], [1, 1], vec![true]).unwrap();
        draft.clear_stroke();
        let (_, touched_blocked) =
            brush_stroke(&mut draft, [40.5, 10.5], 1, false, active, &[blocked]).unwrap();
        assert!(touched_blocked);
        assert!(!draft.pixels.contains(&[40, 10]));
    }

    #[test]
    fn committed_repair_sets_are_independent_per_source() {
        let mut editor = super::super::tests::fixture();
        super::super::tests::add_ready_fixture(&mut editor);
        let first_source = editor.current_source.unwrap();
        let mut second_record = editor
            .document
            .project
            .source(first_source)
            .unwrap()
            .clone();
        second_record.path = "second.ARW".into();
        let second_source = editor.document.project.add_source(second_record).unwrap();
        let mask = || SupportMask::new([20, 20], [3, 3], vec![true; 9]).unwrap();
        let first = editor
            .dust_repairs_mut(first_source)
            .insert(mask(), RepairOrigin::Manual, [4, 0])
            .unwrap();
        let second = editor
            .dust_repairs_mut(second_source)
            .insert(mask(), RepairOrigin::Detected, [4, -2])
            .unwrap();
        assert_eq!(first, RepairId(1));
        assert_eq!(second, RepairId(1));
        editor
            .dust_repairs_mut(first_source)
            .set_enabled(first, false)
            .unwrap();
        assert!(!editor.dust_repairs(first_source).unwrap().repairs()[0].enabled);
        assert!(editor.dust_repairs(second_source).unwrap().repairs()[0].enabled);
    }

    #[test]
    fn unchanged_failure_is_not_retried_or_allowed_to_block_later_repairs() {
        let mut state = State::default();
        let source = SourceId(1);
        let mut committed = RepairSet::default();
        for origin in [[20, 20], [30, 30]] {
            committed
                .insert(
                    SupportMask::new(origin, [2, 2], vec![true; 4]).unwrap(),
                    RepairOrigin::Manual,
                    [4, 0],
                )
                .unwrap();
        }
        let first = committed.repairs()[0].clone();
        state.start(&Request {
            cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            request: 4,
            loaded: 8,
            source,
            signature: [3; ProcessingParameters::BYTE_LEN].into(),
            repair: first,
            input: Calibration {
                dmin: DVec3::splat(0.5),
                dmax: DVec3::splat(0.01),
                input_floor: crate::processing::CAMERA_INPUT_FLOOR,
            }
            .into(),
        });
        assert!(state.fail(4));
        let repairs = committed.repairs().to_vec();
        assert_eq!(
            state
                .next_stale(
                    source,
                    8,
                    [3; ProcessingParameters::BYTE_LEN].into(),
                    &repairs
                )
                .unwrap()
                .id,
            RepairId(2)
        );
        assert_eq!(
            state
                .next_stale(
                    source,
                    8,
                    [4; ProcessingParameters::BYTE_LEN].into(),
                    &repairs
                )
                .unwrap()
                .id,
            RepairId(1)
        );
    }

    #[test]
    fn straightened_overlay_keeps_source_rectangle_as_a_quadrilateral() {
        let area = SourceRegion {
            x: 0,
            y: 0,
            width: 100,
            height: 80,
        };
        let composition = crate::geometry::Composition::with_crop(
            area,
            Default::default(),
            Default::default(),
            Some(crate::crop::Crop {
                center: [50., 40.],
                size: [60, 40],
                angle: 0.2,
            }),
        )
        .unwrap();
        let geometry = crate::viewport::CanvasLayout {
            canvas: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(100., 80.)),
            image: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(60., 40.)),
            area: crate::raw::Rect {
                x: 0,
                y: 0,
                width: 100,
                height: 80,
            },
            composition,
            scale: 1.,
        };
        let points = screen_polygon(
            geometry,
            Coverage::rectangle(SourceRegion {
                x: 20,
                y: 20,
                width: 10,
                height: 5,
            }),
        );
        assert_ne!(points[0].y, points[1].y);
        assert_ne!(points[1].x, points[2].x);
    }
}
