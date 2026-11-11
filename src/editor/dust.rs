use super::*;
#[cfg(test)]
use crate::dust_repair::{
    AUTOMATIC_PADDING, CONTEXT_PADDING, MAX_MASK_PIXELS, MAX_MASK_WORK_PIXELS,
};
use crate::{
    cancellation::Cancellation,
    dust_repair::{
        Repair, RepairId, RepairOrigin, RepairSet, SearchOutcome, SupportMask, donor_target,
        mask_within_budget, validate_offset as validate_dust_offset,
    },
    geometry::SourceRegion,
};
use std::{
    cell::{Cell, Ref, RefCell, RefMut},
    collections::{BTreeSet, HashMap},
    rc::{Rc, Weak},
};

#[cfg(test)]
mod search_testing;
#[cfg(test)]
mod search_tests;

#[cfg(test)]
mod lifecycle_tests;
#[cfg(test)]
pub(super) mod render_testing;
#[cfg(test)]
mod testing;

fn initial_brush_size(active: SourceRegion) -> u32 {
    ((active.width.min(active.height) as f64 * 0.01).round() as u32).clamp(4, 128)
}

#[cfg(test)]
pub(super) type Signature = crate::gpu::interpretation::InputTransform;

#[derive(Clone, Copy, PartialEq, Eq)]
enum RepairFeedback {
    Pending,
    Ready,
    Failed,
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
pub(super) struct MaskDraft {
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

#[derive(Default)]
struct Edit {
    mask_draft: Option<MaskDraft>,
    donor_draft: Option<DonorDraft>,
    show_masks: bool,
}

// Search continuations capture Edit, never this task owner.
#[derive(Default)]
struct Interaction {
    edit: Rc<RefCell<Edit>>,
    searches: RefCell<SearchState>,
}

pub(super) struct DustTool {
    interaction: Rc<Interaction>,
    reveal_controls: bool,
}

/// Photo-owned preferences and access to the current provisional instructions.
pub(super) struct PhotoState {
    pub brush_size: u32,
    brush_source: Option<SourceId>,
    preview: Weak<Interaction>,
}

impl Default for PhotoState {
    fn default() -> Self {
        Self {
            brush_size: 12,
            brush_source: None,
            preview: Weak::new(),
        }
    }
}

#[derive(Default)]
pub(super) struct State {
    selected: HashMap<SourceId, RepairId>,
    #[cfg(test)]
    search_worker: Option<search_testing::Search>,
}

impl State {
    fn removed(&mut self, source: SourceId, repair: RepairId) {
        if self.selected.get(&source) == Some(&repair) {
            self.selected.remove(&source);
        }
    }

    fn clear_repairs(&mut self, source: SourceId) {
        self.selected.remove(&source);
    }
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
    pub(super) fn dusting(&self) -> bool {
        self.document.visit.hosted_tool.is::<DustTool>()
    }

    fn dust_session(&self) -> Option<Rc<Interaction>> {
        self.document.visit.dust.preview.upgrade()
    }

    pub(super) fn dust_outlines(&self) -> bool {
        self.dust_session()
            .is_some_and(|session| session.edit.borrow().show_masks)
    }

    pub(super) fn dust_history_draft(&self) -> bool {
        self.dust_session().is_some_and(|session| {
            let edit = session.edit.borrow();
            edit.mask_draft.is_some()
                || edit.donor_draft.is_some()
                || !session.searches.borrow().is_empty()
        })
    }

    pub(super) fn dust_repairs(&self, source: SourceId) -> Option<&RepairSet> {
        self.document
            .state
            .project
            .source(source)
            .ok()
            .map(|source| &source.repairs)
            .filter(|set| !set.repairs().is_empty())
    }

    #[cfg(test)]
    pub(super) fn dust_repairs_mut(&mut self, source: SourceId) -> &mut RepairSet {
        &mut self
            .document
            .project
            .source_mut(source)
            .expect("accepted source")
            .repairs
    }

    fn dust_repair_feedback(
        &self,
        capture: &crate::image_recipe::ImageRecipe,
        repair: &Repair,
    ) -> RepairFeedback {
        let working = &self.document.visit.working_view;
        if working
            .failed_capture()
            .and_then(|capture| capture.recipe())
            .is_some_and(|failed| **failed == *capture)
        {
            return RepairFeedback::Failed;
        }
        if working
            .completed_capture()
            .and_then(|capture| capture.recipe())
            .is_some_and(|completed| {
                completed.neutral == capture.neutral && completed.repairs.contains(repair)
            })
        {
            return RepairFeedback::Ready;
        }
        RepairFeedback::Pending
    }

    pub(super) fn dust_render_state(&self) -> &'static str {
        if self.document.source {
            return "Repairs hidden";
        }
        let Some(source) = self.document.visit.current_source else {
            return "Repairs pending";
        };
        let Ok(capture) = self.positive_recipe(
            self.input(),
            self.document.project_snapshot().roll,
            self.photo_settings(),
        ) else {
            return "Repairs pending";
        };
        let mut pending = self.dust_pending_supports(source).next().is_some();
        for repair in capture.repairs.iter().filter(|repair| repair.enabled) {
            match self.dust_repair_feedback(&capture, repair) {
                RepairFeedback::Failed => return "Repair unavailable",
                RepairFeedback::Pending => pending = true,
                RepairFeedback::Ready => {}
            }
        }
        if pending {
            "Repairs pending"
        } else {
            "Repairs"
        }
    }

    pub(super) fn reset_dust_repairs(&mut self, source: SourceId) {
        let result = (|| -> Result<(), Error> {
            let mut transaction = self.edit_photo("Reset Dust repairs")?;
            transaction.update(|project| {
                project
                    .source_mut(source)
                    .map(|source| source.repairs.clear())
            })?;
            self.commit_project(transaction)
        })();
        if let Err(error) = result {
            self.status = error.to_string();
            return;
        }
        self.document.dust.clear_repairs(source);
        if self.document.visit.current_source == Some(source)
            && let Some(session) = self.dust_session()
        {
            *session.searches.borrow_mut() = Default::default();
            let mut edit = session.edit.borrow_mut();
            edit.mask_draft = None;
            edit.donor_draft = None;
        }
    }

    fn invalidate_settled(&mut self) {
        self.invalidate_original();
    }

    pub(super) fn begin_dust_cleanup(&mut self) {
        if !self.document.project.input.ready() || !self.tool_start_available() {
            return;
        }
        if self.document.visit.dust.brush_source != self.document.visit.current_source
            && let (Some(source), Some(active)) =
                (self.document.visit.current_source, self.selected_area())
        {
            self.document.visit.dust.brush_size = initial_brush_size(active);
            self.document.visit.dust.brush_source = Some(source);
        }
        let tool = DustTool::new(true, true);
        let preview = Rc::downgrade(&tool.interaction);
        if self.activate_tool(tool::Placement::Photo, Box::new(tool)) {
            self.document.source = false;
            self.document.visit.dust.preview = preview;
        }
    }

    pub(super) fn restore_dust(&mut self, show_masks: bool) {
        let tool = DustTool::new(show_masks, false);
        let preview = Rc::downgrade(&tool.interaction);
        self.document
            .set_hosted_tool(tool::Placement::Photo, Box::new(tool));
        self.document.visit.dust.preview = preview;
    }

    pub(super) fn selected_dust_repair(&self) -> Option<RepairId> {
        let source = self.document.visit.current_source?;
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

    fn effective_dust_repairs(&self, source: SourceId) -> Vec<Repair> {
        let Ok(source) = self.document.project.source(source) else {
            return Vec::new();
        };
        self.dust_recipe_repairs(source)
            .unwrap_or_else(|| source.repairs.repairs().to_vec())
    }

    pub(super) fn dust_recipe_repairs(&self, source: &Source) -> Option<Vec<Repair>> {
        if self.document.visit.current_source != Some(source.id) {
            return None;
        }
        let session = self.dust_session()?;
        let edit = session.edit.borrow();
        let mask = edit.mask_draft.as_ref();
        let target = mask
            .filter(|draft| !draft.rejected)
            .and_then(|draft| draft.target);
        let support = mask
            .filter(|draft| draft.target.is_some() && !draft.rejected)
            .and_then(|draft| draft.support().ok());
        let donor = edit.donor_draft;
        Some(
            source
                .repairs
                .repairs()
                .iter()
                .filter_map(|repair| {
                    let mut repair = repair.clone();
                    if Some(repair.id) == target {
                        repair.support = support.clone()?;
                    }
                    if let Some(donor) = donor.filter(|donor| donor.repair == repair.id) {
                        repair.offset = donor.offset;
                    }
                    Some(repair)
                })
                .collect(),
        )
    }

    fn effective_dust_repair(&self, source: SourceId, id: RepairId) -> Option<Repair> {
        self.effective_dust_repairs(source)
            .into_iter()
            .find(|repair| repair.id == id)
    }

    #[cfg(test)]
    pub(super) fn dust_signature(&self) -> Option<Signature> {
        self.neutral_source_signature(&self.input()).ok()
    }
}

impl tool::Tool for DustTool {
    fn presentation(&self, _editor: &Editor) -> tool::Presentation {
        tool::Presentation {
            identity: presentation::ToolIdentity {
                icon: icons::Icon::Dust,
                title: "Dust cleanup",
            },
            primary: None,
            dismiss: "Done",
            warning: None,
            help: "Draw for a new heal · click an outline to select another heal.\nShift-drag adds to selected · Option-drag removes · Delete removes selected.\nDrag the source outline to move it · release commits changes · H hides outlines.",
            navigation_help: Some("Scroll changes brush size · Cmd/Ctrl-scroll or pinch zooms"),
            fit_label: "Fit",
        }
    }

    fn drawer(&mut self, ui: &mut egui::Ui, editor: &mut Editor, _cx: &UiContext<'_, Editor>) {
        self.interaction.searches.borrow_mut().prune();
        if std::mem::take(&mut self.reveal_controls) {
            ui.scroll_to_cursor(Some(egui::Align::Min));
        }
        let Some(source) = editor.document.visit.current_source else {
            return;
        };
        let repair_count = editor
            .dust_repairs(source)
            .map_or(0, |set| set.repairs().len());
        let searching = !self.interaction.searches.borrow().is_empty();
        let reset_enabled = !editor.interaction_blocked()
            && editor.selected_area().is_some()
            && self.donor().is_none()
            && (repair_count > 0 || searching);
        presentation::ToolIdentity {
            icon: icons::Icon::Dust,
            title: "Dust repair"
        }.inspector(
            ui,
            |ui| ui
                .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let reset = icons::button(
                        ui,
                        icons::Icon::Reset,
                        reset_enabled,
                        "Reset dust repairs",
                        "Reset dust repairs to defaults."
                    ).clicked();
                    ui.weak(format!("{repair_count} repairs"));
                    reset
                }).inner,
            |ui, reset| {
                ui.horizontal(|ui| {
                    ui.label("Brush size")
                        .on_hover_text("Diameter in source pixels.");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.weak(format!("{} px", editor.document.visit.dust.brush_size));
                    });
                });
                ui.scope(|ui| {
                    ui.spacing_mut().slider_width = ui.available_width();
                    ui.add(
                        egui::Slider::new(&mut editor.document.visit.dust.brush_size, 1..=128)
                            .show_value(false)
                            .step_by(1.),
                    );
                });
                ui.checkbox(
                    &mut self.interaction.edit.borrow_mut().show_masks,
                    "Show repair outlines",
                )
                .on_hover_text("Hold H for a momentary unobstructed preview.");
                let rejected = self.mask().is_some_and(|draft| draft.rejected);
                let guidance = egui::RichText::new(if rejected {
                    "Stroke too large · release to discard it and use smaller repairs."
                } else if editor.selected_dust_repair().is_some() {
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
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.weak("Finding donor…");
                    });
                } else if repair_count == 0 {
                    ui.small("Manual masks only · automatic proposals are not wired yet");
                }
                if reset {
                    editor.reset_dust_repairs(source);
                    editor.status = "Dust repairs reset".into();
                    editor.invalidate_settled();
                }
            },
        );
    }

    fn canvas(
        &mut self,
        viewer: &mut canvas::Viewer<'_>,
        editor: &mut Editor,
        cx: &UiContext<'_, Editor>,
    ) -> Option<crate::viewport::CanvasLayout> {
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
                let rect = canvas.response.rect;
                let composition = canvas.composition;
                let ppp = canvas.ui.ctx().pixels_per_point();
                let relayout =
                    |viewport: &mut Viewport| viewport.layout_composition(rect, composition, ppp);
                let layout = canvas.layout(&mut editor.active_canvas_mut().viewport, relayout);
                if canvas.ui.is_enabled() && editor.tab == 1 {
                    tool.adjust_dust_brush_from_scroll(editor, canvas.ui, layout);
                }
                let mut response = canvas.response.clone();
                if editor.tab == 1 {
                    navigation::consume_primary(&mut response);
                }
                let (layout, panning) = navigation::navigate_view(
                    &mut editor.active_canvas_mut().viewport,
                    canvas.ui,
                    &response,
                    layout,
                    relayout,
                );
                canvas.ui.add_enabled_ui(!panning, |ui| {
                    tool.dust_interaction(editor, ui, canvas.response, layout, cx);
                });
                let ready = canvas.paint_image(editor, layout, false, cx);
                tool.paint_dust(editor, canvas.ui, layout);
                if canvas.ui.is_enabled()
                    && ready
                    && !panning
                    && editor.tab == 1
                    && !editor.document.source
                {
                    let (point, modifying, space) = canvas.ui.input(|input| {
                        (
                            input.pointer.hover_pos(),
                            input.modifiers.alt || input.modifiers.shift,
                            input.key_down(egui::Key::Space),
                        )
                    });
                    if !space && point.is_some_and(|point| layout.image.contains(point)) {
                        canvas.ui.ctx().set_cursor_icon(
                            tool.dust_canvas_cursor(editor, layout, point, modifying),
                        );
                    }
                }
                Some(layout)
            },
        )
    }

    fn shortcuts(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        if ui.is_enabled()
            && !editor.interaction_blocked()
            && ui.input_mut(|input| {
                let delete = input.consume_key(egui::Modifiers::NONE, egui::Key::Delete);
                let backspace = input.consume_key(egui::Modifiers::NONE, egui::Key::Backspace);
                delete || backspace
            })
        {
            self.delete_selected_dust_repair(editor);
        }
    }

    fn action(&mut self, action: tool::Action, editor: &mut Editor, _cx: &UiContext<'_, Editor>) {
        if matches!(action, tool::Action::Dismiss) && !editor.interaction_blocked() {
            if !self.interaction.searches.borrow().is_empty() {
                editor.status = "Unfinished Dust strokes discarded".into();
            }
            editor.end_tool();
        }
    }
}

impl DustTool {
    fn new(show_masks: bool, reveal_controls: bool) -> Self {
        Self {
            interaction: Rc::new(Interaction {
                edit: Rc::new(RefCell::new(Edit {
                    show_masks,
                    ..Default::default()
                })),
                searches: RefCell::default(),
            }),
            reveal_controls,
        }
    }

    fn mask(&self) -> Option<Ref<'_, MaskDraft>> {
        Ref::filter_map(self.interaction.edit.borrow(), |edit| {
            edit.mask_draft.as_ref()
        })
        .ok()
    }
    fn mask_mut(&mut self) -> Option<RefMut<'_, MaskDraft>> {
        RefMut::filter_map(self.interaction.edit.borrow_mut(), |edit| {
            edit.mask_draft.as_mut()
        })
        .ok()
    }
    fn set_mask(&mut self, value: Option<MaskDraft>) {
        self.interaction.edit.borrow_mut().mask_draft = value;
    }
    fn donor(&self) -> Option<DonorDraft> {
        self.interaction.edit.borrow().donor_draft
    }
    fn donor_mut(&mut self) -> Option<RefMut<'_, DonorDraft>> {
        RefMut::filter_map(self.interaction.edit.borrow_mut(), |edit| {
            edit.donor_draft.as_mut()
        })
        .ok()
    }
    fn set_donor(&mut self, value: Option<DonorDraft>) {
        self.interaction.edit.borrow_mut().donor_draft = value;
    }
    fn take_donor(&mut self) -> Option<DonorDraft> {
        self.interaction.edit.borrow_mut().donor_draft.take()
    }

    fn dust_interaction(
        &mut self,
        editor: &mut Editor,
        ui: &egui::Ui,
        response: &egui::Response,
        geometry: crate::viewport::CanvasLayout,
        cx: &UiContext<'_, Editor>,
    ) {
        if editor.tab != 1 || editor.fullscreen.is_some() || editor.document.source {
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
        let allowed = !editor.interaction_blocked()
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
                && self.mask().is_none()
                && !self.dust_donor_dragging()
                && let Some(point) = start.filter(|p| geometry.image.contains(*p))
            {
                if modifies.alt || modifies.shift {
                    if let Some(id) = editor.selected_dust_repair() {
                        self.begin_dust_mask(editor, Some(id));
                    }
                } else if let Some(id) = self.dust_repair_at(editor, geometry, point) {
                    self.select_dust_repair(editor, id);
                } else if self.dust_donor_at(editor, geometry, point) {
                    if started {
                        self.begin_dust_donor_drag(editor, geometry, point);
                    }
                } else {
                    self.begin_dust_mask(editor, None);
                }
                if self.mask().is_some() {
                    self.begin_dust_stroke();
                    self.paint_dust_stroke(editor, geometry, point, modifies.alt);
                }
            }
            if response.dragged_by(egui::PointerButton::Primary)
                && let Some(point) = response.interact_pointer_pos()
            {
                if self.dust_donor_dragging() {
                    self.drag_dust_donor(editor, geometry, point);
                } else if self.mask().is_some() {
                    self.paint_dust_stroke(editor, geometry, point, modifies.alt);
                }
            }
            if clicked {
                self.commit_dust_mask(editor, cx);
            }
        }
        if response.drag_stopped_by(egui::PointerButton::Primary) {
            self.end_dust_stroke();
            self.end_dust_donor_drag(editor);
            self.commit_dust_mask(editor, cx);
        }
    }

    fn begin_dust_mask(&mut self, editor: &mut Editor, target: Option<RepairId>) {
        if let (Some(source), Some(repair)) = (editor.document.visit.current_source, target)
            && editor.dust_target_pending(source, repair)
        {
            editor.status = "Wait for this repair's donor search before editing its mask".into();
            return;
        }
        self.interaction.edit.borrow_mut().show_masks = true;
        self.set_donor(None);
        let support = target.and_then(|id| {
            let source = editor.document.visit.current_source?;
            editor
                .dust_repairs(source)?
                .repairs()
                .iter()
                .find(|repair| repair.id == id)
                .map(|repair| repair.support.clone())
        });
        self.set_mask(Some(MaskDraft::new(target, support.as_ref())));
    }

    fn cancel_dust_mask(&mut self) {
        self.set_mask(None);
        self.set_donor(None);
    }

    fn commit_dust_mask(&mut self, editor: &mut Editor, cx: &UiContext<'_, Editor>) {
        let Some((target, empty)) = self
            .mask()
            .map(|draft| (draft.target, draft.pixels.is_empty()))
        else {
            return;
        };
        if empty {
            self.cancel_dust_mask();
            if target.is_some() {
                self.delete_selected_dust_repair(editor);
            }
        } else {
            self.apply_dust_mask(editor, cx);
        }
    }

    fn adjust_dust_brush_from_scroll(
        &mut self,
        editor: &mut Editor,
        ui: &egui::Ui,
        geometry: crate::viewport::CanvasLayout,
    ) {
        if editor.document.source {
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
        editor.document.visit.dust.brush_size = if delta > 0. {
            editor
                .document
                .visit
                .dust
                .brush_size
                .saturating_add(step)
                .min(128)
        } else {
            editor
                .document
                .visit
                .dust
                .brush_size
                .saturating_sub(step)
                .max(1)
        };
    }

    fn apply_dust_mask(&mut self, editor: &mut Editor, cx: &UiContext<'_, Editor>) {
        let Some(source) = editor.document.visit.current_source else {
            self.cancel_dust_mask();
            return;
        };
        if self.mask().is_some_and(|draft| draft.rejected) {
            self.cancel_dust_mask();
            editor.status =
                "Dust stroke rejected: repair budget exceeded; no changes applied".into();
            return;
        }
        let Some((target, support)) = self.mask().map(|draft| (draft.target, draft.support()))
        else {
            return;
        };
        let support = match support {
            Ok(support) => support,
            Err(error) => {
                editor.status = format!("Dust mask rejected: {error}");
                self.cancel_dust_mask();
                return;
            }
        };
        let previous = target.and_then(|id| {
            editor
                .dust_repairs(source)?
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
        self.cancel_dust_mask();
        editor.status = "Finding a donor…".into();
        self.find_dust_donor(editor, source, previous, support, cx);
        editor.invalidate_settled();
    }

    fn dust_source_position(
        &self,
        editor: &Editor,
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
        let active = editor.selected_area()?;
        (source[0] >= f64::from(active.x)
            && source[1] >= f64::from(active.y)
            && source[0] < f64::from(active.x + active.width)
            && source[1] < f64::from(active.y + active.height))
        .then_some(source)
    }

    fn begin_dust_stroke(&mut self) {
        if let Some(mut draft) = self.mask_mut() {
            draft.clear_stroke();
        }
    }

    fn paint_dust_stroke(
        &mut self,
        editor: &mut Editor,
        geometry: crate::viewport::CanvasLayout,
        point: egui::Pos2,
        temporary_erase: bool,
    ) {
        let Some(position) = self.dust_source_position(editor, geometry, point) else {
            self.end_dust_stroke();
            return;
        };
        let Some(source) = editor.document.visit.current_source else {
            return;
        };
        if self.mask().is_some_and(|draft| draft.rejected) {
            return;
        }
        let Some(target) = self.mask().map(|draft| draft.target) else {
            return;
        };
        let blocked = editor
            .dust_repairs(source)
            .into_iter()
            .flat_map(RepairSet::repairs)
            .filter(|repair| Some(repair.id) != target)
            .map(|repair| repair.support.clone())
            .chain(editor.dust_pending_supports(source))
            .collect::<Vec<_>>();
        let active = editor.selected_area().expect("selected photo geometry");
        let erase = temporary_erase;
        let brush_size = editor.document.visit.dust.brush_size;
        let result = brush_stroke(
            &mut self.mask_mut().unwrap(),
            position,
            brush_size,
            erase,
            active,
            &blocked,
        );
        match result {
            Ok((_, true)) => editor.status = "Dust masks cannot overlap another repair".into(),
            Ok(_) => {}
            Err(_) => {
                self.mask_mut().unwrap().rejected = true;
                editor.status = "Dust stroke too large; release to discard it".into();
            }
        }
    }

    fn end_dust_stroke(&mut self) {
        if let Some(mut draft) = self.mask_mut() {
            draft.clear_stroke();
        }
    }

    fn dust_canvas_cursor(
        &self,
        editor: &Editor,
        geometry: crate::viewport::CanvasLayout,
        point: Option<egui::Pos2>,
        modifying: bool,
    ) -> egui::CursorIcon {
        if self.dust_donor_dragging() {
            egui::CursorIcon::Grabbing
        } else if modifying || self.mask().is_some() {
            egui::CursorIcon::Crosshair
        } else if point.is_some_and(|p| self.dust_repair_at(editor, geometry, p).is_some()) {
            egui::CursorIcon::PointingHand
        } else if point.is_some_and(|p| self.dust_donor_at(editor, geometry, p)) {
            egui::CursorIcon::Grab
        } else {
            egui::CursorIcon::Crosshair
        }
    }

    fn dust_repair_at(
        &self,
        editor: &Editor,
        geometry: crate::viewport::CanvasLayout,
        point: egui::Pos2,
    ) -> Option<RepairId> {
        if editor.interaction_blocked()
            || editor.tab != 1
            || editor.document.source
            || !self.interaction.edit.borrow().show_masks
            || editor.picking()
            || self.donor().is_some()
        {
            return None;
        }
        let source_point = self.dust_source_position(editor, geometry, point)?;
        let pixel = [
            source_point[0].floor() as u32,
            source_point[1].floor() as u32,
        ];
        let source = editor.document.visit.current_source?;
        let set = editor.dust_repairs(source)?;
        let selected = editor.selected_dust_repair();
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

    fn select_dust_repair(&mut self, editor: &mut Editor, id: RepairId) {
        if let Some(source) = editor.document.visit.current_source
            && editor
                .dust_repairs(source)
                .is_some_and(|set| set.repairs().iter().any(|repair| repair.id == id))
        {
            self.interaction
                .searches
                .borrow_mut()
                .manual_selection(source);
            editor.document.dust.selected.insert(source, id);
        }
    }

    fn delete_selected_dust_repair(&mut self, editor: &mut Editor) {
        let Some(source) = editor.document.visit.current_source else {
            return;
        };
        let Some(id) = editor.selected_dust_repair() else {
            return;
        };
        self.cancel_dust_mask();
        let result = (|| -> Result<(), Error> {
            let mut transaction = editor.edit_photo("Delete Dust repair")?;
            transaction.update(|project| project.source_mut(source)?.repairs.remove(id))?;
            editor.commit_project(transaction)
        })();
        if result.is_ok() {
            editor.document.dust.removed(source, id);
            self.interaction
                .searches
                .borrow_mut()
                .cancel_target(source, id);
            editor.status = "Dust repair deleted".into();
            editor.invalidate_settled();
        }
    }

    fn dust_donor_regions(&self, editor: &Editor) -> Vec<DonorRegion> {
        self.dust_donor_regions_inner(editor).unwrap_or_default()
    }

    fn dust_donor_regions_inner(&self, editor: &Editor) -> Option<Vec<DonorRegion>> {
        let source = editor.document.visit.current_source?;
        let repair_id = self
            .donor()
            .map(|draft| draft.repair)
            .or_else(|| self.mask().and_then(|draft| draft.target))
            .or_else(|| {
                self.mask()
                    .is_none()
                    .then(|| editor.selected_dust_repair())
                    .flatten()
            })?;
        if editor.dust_target_pending(source, repair_id) {
            return None;
        }
        let effective = editor.effective_dust_repair(source, repair_id)?;
        let active = editor.selected_area()?;
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

    fn dust_donor_at(
        &self,
        editor: &Editor,
        geometry: crate::viewport::CanvasLayout,
        point: egui::Pos2,
    ) -> bool {
        self.dust_donor_hit(editor, geometry, point).is_some()
    }

    fn dust_donor_hit(
        &self,
        editor: &Editor,
        geometry: crate::viewport::CanvasLayout,
        point: egui::Pos2,
    ) -> Option<DonorRegion> {
        if !self.interaction.edit.borrow().show_masks {
            return None;
        }
        let position = self.dust_source_position(editor, geometry, point)?;
        self.dust_donor_regions(editor).into_iter().find(|region| {
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

    fn begin_dust_donor_drag(
        &mut self,
        editor: &mut Editor,
        geometry: crate::viewport::CanvasLayout,
        point: egui::Pos2,
    ) {
        let Some(region) = self.dust_donor_hit(editor, geometry, point) else {
            return;
        };
        let Some(source_point) = self.dust_source_position(editor, geometry, point) else {
            return;
        };
        if self.donor().is_none() {
            let Some(repair) = editor.selected_dust_repair() else {
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
            self.set_donor(Some(DonorDraft {
                repair,
                offset,
                drag: None,
            }));
        }
        if let Some(mut draft) = self.donor_mut() {
            draft.drag = Some(DonorDrag {
                source: source_point,
                offset: draft.offset,
            });
        }
    }

    fn drag_dust_donor(
        &mut self,
        editor: &mut Editor,
        geometry: crate::viewport::CanvasLayout,
        point: egui::Pos2,
    ) {
        let Some(position) = self.dust_source_position(editor, geometry, point) else {
            return;
        };
        let Some(active) = editor.selected_area() else {
            return;
        };
        let Some(support) = editor
            .document
            .visit
            .current_source
            .and_then(|source| {
                editor
                    .selected_dust_repair()
                    .and_then(|id| editor.effective_dust_repair(source, id))
            })
            .map(|r| r.support)
        else {
            return;
        };
        let Ok(target) = donor_target(&support, active) else {
            return;
        };
        let Some(mut draft) = self.donor_mut() else {
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
            editor.invalidate_settled();
        }
    }

    fn dust_donor_dragging(&self) -> bool {
        self.donor().is_some_and(|draft| draft.drag.is_some())
    }

    fn end_dust_donor_drag(&mut self, editor: &mut Editor) {
        let Some(draft) = self.take_donor() else {
            return;
        };
        if draft.drag.is_none() {
            return;
        }
        let Some(source) = editor.document.visit.current_source else {
            return;
        };
        let Some((support, accepted_offset)) = editor
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
        let mut transaction = match editor.edit_photo("Dust donor") {
            Ok(edit) => edit,
            Err(error) => {
                editor.status = error.to_string();
                return;
            }
        };
        let result = editor
            .selected_area()
            .ok_or(Error::InvalidParameter("manual donor unavailable"))
            .and_then(|active| validate_dust_offset(draft.offset, &support, active))
            .and_then(|()| {
                transaction.update(|project| {
                    project
                        .source_mut(source)?
                        .repairs
                        .set_offset(draft.repair, draft.offset)
                })
            });
        match result {
            Ok(()) => {
                if let Err(error) = editor.commit_project(transaction) {
                    editor.status = error.to_string();
                    return;
                }
                editor.status = "Dust source updated".into();
            }
            Err(error) => {
                editor.status = format!("Dust source rejected: {error}");
            }
        }
        editor.invalidate_settled();
    }

    fn paint_dust(&self, editor: &Editor, ui: &egui::Ui, geometry: crate::viewport::CanvasLayout) {
        let Some(source) = editor.document.visit.current_source else {
            return;
        };
        if editor.tab != 1 || editor.fullscreen.is_some() || editor.canvas_inspection() {
            return;
        }
        let painter = ui.painter().with_clip_rect(geometry.canvas);
        let overlays_hidden = ui.input(|input| input.key_down(egui::Key::H));
        let show_masks = self.interaction.edit.borrow().show_masks;
        if show_masks && !overlays_hidden {
            self.paint_repair_outlines(editor, source, ui, geometry, &painter);
        }
        if !overlays_hidden {
            self.paint_mask_outline(ui, geometry, &painter);
        }
        self.paint_brush_cursor(editor, ui, geometry, &painter);
        if show_masks && !overlays_hidden {
            self.paint_donor_guides(editor, geometry, &painter);
        }
    }

    fn paint_repair_outlines(
        &self,
        editor: &Editor,
        source: SourceId,
        ui: &egui::Ui,
        geometry: crate::viewport::CanvasLayout,
        painter: &egui::Painter,
    ) {
        let set = editor.dust_repairs(source);
        let selected = editor.selected_dust_repair();
        let capture = editor
            .positive_recipe(
                editor.input(),
                editor.document.project_snapshot().roll,
                editor.photo_settings(),
            )
            .ok();
        let mask_target = self.mask().and_then(|draft| draft.target);
        for repair in set.into_iter().flat_map(RepairSet::repairs) {
            if mask_target == Some(repair.id) || editor.dust_target_pending(source, repair.id) {
                continue;
            }
            let feedback = capture.as_ref().map_or(RepairFeedback::Pending, |capture| {
                editor.dust_repair_feedback(capture, repair)
            });
            let active = selected == Some(repair.id);
            let color = if mask_target.is_some() {
                egui::Color32::from_white_alpha(45)
            } else if !repair.enabled {
                egui::Color32::from_white_alpha(65)
            } else if feedback == RepairFeedback::Failed {
                ui.visuals().error_fg_color.linear_multiply(0.7)
            } else if feedback == RepairFeedback::Pending {
                egui::Color32::from_rgb(235, 180, 80).linear_multiply(0.7)
            } else if active {
                egui::Color32::from_rgb(100, 220, 240)
            } else {
                egui::Color32::from_white_alpha(85)
            };
            paint_support_outline(
                painter,
                geometry,
                &repair.support,
                [0., 0.],
                color,
                if active { 1.5 } else { 1. },
            );
        }
        for support in editor.dust_pending_supports(source) {
            paint_support_outline(
                painter,
                geometry,
                &support,
                [0., 0.],
                ui.visuals().weak_text_color(),
                1.5,
            );
        }
    }

    fn paint_mask_outline(
        &self,
        ui: &egui::Ui,
        geometry: crate::viewport::CanvasLayout,
        painter: &egui::Painter,
    ) {
        if let Some(mask) = self.mask().and_then(|draft| draft.support().ok()) {
            paint_support_outline(
                painter,
                geometry,
                &mask,
                [0., 0.],
                if self.mask().is_some_and(|draft| draft.rejected) {
                    ui.visuals().error_fg_color
                } else {
                    egui::Color32::from_rgb(100, 220, 240)
                },
                1.,
            );
        }
    }

    fn paint_brush_cursor(
        &self,
        editor: &Editor,
        ui: &egui::Ui,
        geometry: crate::viewport::CanvasLayout,
        painter: &egui::Painter,
    ) {
        let (point, add, erase, hide_cursor) = ui.ctx().input(|input| {
            (
                input.pointer.hover_pos(),
                input.modifiers.shift,
                input.modifiers.alt,
                input.key_down(egui::Key::H) || input.key_down(egui::Key::Space),
            )
        });
        if !editor.document.source
            && !self.dust_donor_dragging()
            && !hide_cursor
            && let Some(point) = point
            && self.dust_canvas_cursor(editor, geometry, Some(point), add || erase)
                == egui::CursorIcon::Crosshair
            && let Some(source_point) = self.dust_source_position(editor, geometry, point)
        {
            let center = screen_source_point(geometry, source_point);
            let radius =
                (editor.document.visit.dust.brush_size as f32 * 0.5 * geometry.scale).max(2.);
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
                    if self.mask().is_some_and(|draft| draft.rejected) {
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
    }

    fn paint_donor_guides(
        &self,
        editor: &Editor,
        geometry: crate::viewport::CanvasLayout,
        painter: &egui::Painter,
    ) {
        for region in self.dust_donor_regions(editor) {
            let (target, donor, support) = (region.target, region.donor, region.support);
            let offset = [
                f64::from(donor.x) - f64::from(target.x),
                f64::from(donor.y) - f64::from(target.y),
            ];
            paint_support_outline(
                painter,
                geometry,
                &support,
                [0., 0.],
                egui::Color32::WHITE,
                2.,
            );
            paint_support_outline(
                painter,
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

/// Released support remains visible and blocks overlapping edits while its task runs.
#[derive(Clone)]
pub(super) struct PendingStroke {
    pub source: SourceId,
    pub support: SupportMask,
    target: Option<RepairId>,
    released: std::time::Instant,
    select: Cell<bool>,
}

struct SearchInput {
    pub support: SupportMask,
    pub previous_offset: Option<[i32; 2]>,
    pub recipe: crate::image_recipe::ImageRecipe,
    pub context: source::Context,
    pub services: Option<source::Services>,
    #[cfg(test)]
    pub worker: Option<search_testing::Search>,
}

type Applied = Result<Option<RepairId>, String>;

struct PendingSearch {
    stroke: Rc<PendingStroke>,
    task: async_executor::Task<Applied>,
}

#[derive(Default)]
struct SearchState {
    tasks: Vec<PendingSearch>,
}

impl SearchState {
    fn pending(&self) -> impl Iterator<Item = &PendingSearch> {
        self.tasks
            .iter()
            .filter(|pending| !pending.task.is_finished())
    }

    fn is_empty(&self) -> bool {
        self.pending().next().is_none()
    }

    // Outcomes are applied before task completion; pruning only releases finished handles.
    fn prune(&mut self) {
        self.tasks.retain(|pending| !pending.task.is_finished());
    }

    fn supports(&self, source: SourceId) -> impl Iterator<Item = &SupportMask> {
        self.pending()
            .filter(move |pending| pending.stroke.source == source)
            .map(|pending| &pending.stroke.support)
    }

    fn has_target(&self, source: SourceId, repair: RepairId) -> bool {
        self.pending()
            .any(|pending| pending.stroke.source == source && pending.stroke.target == Some(repair))
    }

    fn cancel_target(&mut self, source: SourceId, repair: RepairId) {
        self.tasks.retain(|pending| {
            pending.stroke.source != source || pending.stroke.target != Some(repair)
        });
    }

    fn manual_selection(&self, source: SourceId) {
        for pending in self
            .pending()
            .filter(|pending| pending.stroke.source == source)
        {
            pending.stroke.select.set(false);
        }
    }
}

async fn apply_stroke(
    input: SearchInput,
    stroke: Rc<PendingStroke>,
    active: SourceRegion,
    editing: Rc<RefCell<Edit>>,
    cx: UiContext<'_, Editor>,
) -> Applied {
    let choice = search(input).await;
    let select = stroke.select.get() && editing.borrow().mask_draft.is_none();
    cx.update(|editor| {
        let result = match choice {
            Ok(SearchOutcome::Candidate(offset)) => {
                validate_dust_offset(offset, &stroke.support, active)
                    .map_err(|error| error.to_string())
                    .and_then(|()| editor.accept_donor(&stroke, offset, select))
                    .map(Some)
            }
            Ok(SearchOutcome::Impossible) => {
                editor.status =
                    "Dust stroke rejected: no clean donor placement; no changes applied".into();
                Ok(None)
            }
            Ok(SearchOutcome::Cancelled) => Ok(None),
            Err(error) => Err(error),
        };
        if let Err(error) = &result {
            editor.status = format!("Dust stroke not accepted: {error}");
        }
        result
    })
    .unwrap_or(Ok(None))
}

impl Editor {
    #[cfg(test)]
    pub(super) fn pending_dust_strokes(&self) -> Vec<PendingStroke> {
        self.dust_session().map_or_else(Vec::new, |session| {
            session
                .searches
                .borrow()
                .pending()
                .map(|pending| (*pending.stroke).clone())
                .collect()
        })
    }

    pub(super) fn dust_search_pending(&self) -> bool {
        self.dust_session()
            .is_some_and(|session| !session.searches.borrow().is_empty())
    }

    fn dust_target_pending(&self, source: SourceId, repair: RepairId) -> bool {
        self.dust_session()
            .is_some_and(|session| session.searches.borrow().has_target(source, repair))
    }

    fn dust_pending_supports(&self, source: SourceId) -> impl Iterator<Item = SupportMask> {
        self.dust_session()
            .map_or_else(Vec::new, |session| {
                session
                    .searches
                    .borrow()
                    .supports(source)
                    .cloned()
                    .collect()
            })
            .into_iter()
    }
}

impl DustTool {
    fn find_dust_donor(
        &mut self,
        editor: &mut Editor,
        source: SourceId,
        previous: Option<Repair>,
        support: SupportMask,
        cx: &UiContext<'_, Editor>,
    ) {
        let stroke = Rc::new(PendingStroke {
            released: std::time::Instant::now(),
            source,
            target: previous.as_ref().map(|repair| repair.id),
            support: support.clone(),
            select: Cell::new(true),
        });
        log::info!(pixels = support.selected_points().count(), width = support.size()[0], height = support.size()[1]; "dust_stroke_released");
        let source_context = match editor.capture_source_context() {
            Ok(input) => input,
            Err(error) => {
                editor.status = error;
                return;
            }
        };
        let recipe = match editor.positive_recipe(
            editor.input(),
            editor.document.project_snapshot().roll,
            editor.photo_settings(),
        ) {
            Ok(recipe) => recipe,
            Err(error) => {
                editor.status = error.to_string();
                return;
            }
        };
        let services = editor.source_services.clone();
        #[cfg(test)]
        let worker = editor.document.dust.search_worker.clone();
        let active = editor
            .selected_area()
            .expect("released stroke has source geometry");
        if previous.is_none() {
            editor.document.dust.selected.remove(&source);
        }
        {
            let mut searches = self.interaction.searches.borrow_mut();
            searches.prune();
            searches.manual_selection(source);
        }
        let model = stroke.clone();
        let editing = self.interaction.edit.clone();
        match cx.spawn(move |cx| async move {
            let context = match source_context.await {
                Ok(context) => context,
                Err(error) => {
                    cx.update(|editor| {
                        editor.status = format!("Dust stroke not accepted: {error}");
                    });
                    return Err(error);
                }
            };
            log::info!(seconds = stroke.released.elapsed().as_secs_f64(); "dust_source_context_ready");
            let input = SearchInput {
                support,
                previous_offset: previous.as_ref().map(|repair| repair.offset),
                recipe,
                context,
                services,
                #[cfg(test)]
                worker,
            };
            apply_stroke(input, stroke, active, editing, cx).await
        }) {
            Some(task) => self
                .interaction
                .searches
                .borrow_mut()
                .tasks
                .push(PendingSearch {
                    stroke: model,
                    task,
                }),
            None => editor.status = "Dust stroke not accepted: Application stopped".into(),
        }
    }
}

impl Editor {
    fn accept_donor(
        &mut self,
        stroke: &PendingStroke,
        offset: [i32; 2],
        select: bool,
    ) -> Result<RepairId, String> {
        let source = stroke.source;
        let mut transaction = self
            .edit_photo("Dust mask")
            .map_err(|error| error.to_string())?;
        let repair = transaction
            .update(|project| -> Result<RepairId, Error> {
                let repairs = &mut project.source_mut(source)?.repairs;
                match stroke.target {
                    Some(target) => {
                        repairs.set_support(target, stroke.support.clone())?;
                        repairs.set_offset(target, offset)?;
                        Ok(target)
                    }
                    None => repairs.insert(stroke.support.clone(), RepairOrigin::Manual, offset),
                }
            })
            .map_err(|error| error.to_string())?;
        self.commit_project(transaction)
            .map_err(|error| error.to_string())?;
        if select {
            self.document.dust.selected.insert(source, repair);
        }
        log::info!(repair = repair.0, repairs = self.dust_repairs(source).unwrap().repairs().len(), seconds = stroke.released.elapsed().as_secs_f64(); "dust_donor_accepted");
        self.status = "Donor selected; evaluating repair…".into();
        self.invalidate_settled();
        Ok(repair)
    }
}

async fn search(input: SearchInput) -> Result<SearchOutcome, String> {
    crate::background::run(move |cancelled| {
        #[cfg(test)]
        let input = {
            let mut input = input;
            if let Some(worker) = input.worker.take() {
                return worker.run(input, cancelled);
            }
            input
        };
        search_pixels(input, &cancelled).map_err(|error| error.to_string())
    })
    .await
    .map_err(|_| "Dust search worker panicked".to_owned())?
}

fn search_pixels(
    input: SearchInput,
    cancelled: &Cancellation,
) -> Result<SearchOutcome, crate::Error> {
    let stopped = || cancelled.is_cancelled();
    if stopped() {
        return Ok(SearchOutcome::Cancelled);
    }
    let scan = input.context.scan()?;
    let (scan, recipe) = crate::photo_preparation::native_image(&scan, &input.recipe)?;
    let services = input
        .services
        .as_ref()
        .ok_or_else(|| crate::Error::Gpu("repair image-query services unavailable".into()))?;
    let renderer = services.renderers.get(&services.device, &services.queue)?;
    let mut source = crate::gpu::repair_source::Source::from_recipe(
        &services.device,
        &renderer,
        (&services.queue, services.renderers.submissions()),
        &scan,
        &recipe,
    )?;

    crate::dust_repair::search_image(
        &input.support,
        input.previous_offset,
        scan.area,
        &mut source,
        std::time::Instant::now() + crate::dust_repair::SEARCH_BUDGET,
        crate::dust_repair::SEARCH_ACCEPTABLE_SCORE,
        &mut crate::dust_repair::Execution {
            cancelled: &stopped,
        },
    )
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
        let first_source = editor.document.visit.current_source.unwrap();
        let mut second_record = editor
            .document
            .project
            .source(first_source)
            .unwrap()
            .clone();
        second_record.path = "second.ARW".into();
        let second_source = editor
            .document
            .state
            .project
            .add_source(second_record)
            .unwrap();
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
