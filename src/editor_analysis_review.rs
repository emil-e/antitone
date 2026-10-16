use super::*;
use crate::{
    project::SourceAnalysis,
    scan_regions::{Areas, Capture, Evidence, RollModel},
};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone)]
pub(super) struct Approved {
    pub entries: Vec<SourceAnalysis>,
    pub files: Vec<thumbnails::FileKey>,
}
pub(super) struct Review {
    pub request: analysis::Request,
    pub approved: Approved,
    pub picture: bool,
    pub replaces_photos: bool,
    pub loaded: u64,
    pub return_tab: usize,
    pub return_scopes_visible: bool,
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

fn verify_file(request: &analysis::Request, approved: &Approved) -> Result<(), Error> {
    if approved.files.len() != request.document.project.order.len()
        || approved.entries.len() != approved.files.len()
    {
        return Err(Error::UnusableSample("incomplete analysis-area review"));
    }
    for (i, id) in request.document.project.order.iter().enumerate() {
        let path = request.document.source_path(*id)?;
        if thumbnails::FileKey::new(&path) != approved.files[i] {
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
        row.areas.validate(
            request
                .document
                .project
                .source(*id)?
                .development
                .active_area
                .into(),
        )?;
    }
    Ok(())
}
pub(super) fn verify(request: &analysis::Request) -> Result<(), Error> {
    if let Some(approved) = &request.areas {
        verify_file(request, approved)?;
    }
    Ok(())
}

pub(super) fn prepare(
    request: &analysis::Request,
    generation: &AtomicU64,
    developments: &development::Scheduler,
    mut progress: impl FnMut(usize, usize),
) -> Result<Option<Approved>, Error> {
    let cancelled = || generation.load(Ordering::Relaxed) != request.revision;
    if cancelled() {
        return Ok(None);
    }
    request.document.validate()?;
    let project = &request.document.project;
    let wb = project
        .development
        .as_ref()
        .ok_or(Error::UnusableSample("no roll development"))?
        .fixed_wb;
    let files = project
        .order
        .iter()
        .map(|id| {
            request
                .document
                .source_path(*id)
                .map(|p| thumbnails::FileKey::new(&p))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut entries: Vec<_> = project
        .order
        .iter()
        .map(|id| {
            project
                .analysis_regions
                .iter()
                .find(|v| v.source == *id)
                .cloned()
                .unwrap_or(SourceAnalysis {
                    source: *id,
                    areas: Areas::default(),
                })
        })
        .collect();
    let need_model = entries
        .iter()
        .any(|v| !v.areas.ready() || !v.areas.opening_edited && !v.areas.opening_shared);
    let mut captures = vec![];
    for (index, id) in project.order.iter().enumerate() {
        if cancelled() {
            return Ok(None);
        }
        progress(index, project.order.len() * 2);
        if need_model {
            let expected = project.source(*id)?;
            let path = request.document.source_path(*id)?;
            let Some(scan) = analysis::develop_scan(
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
            let (w, h, rgb) = scan.preview_source(768)?;
            let rgb: Vec<_> = rgb.into_iter().map(|p| [p[0], p[1], p[2]]).collect();
            let evidence = Evidence::observe([w, h], &rgb)?;
            captures.push(Capture::new(scan.area, [w, h], rgb, evidence)?);
        }
        if thumbnails::FileKey::new(&request.document.source_path(*id)?) != files[index] {
            return Err(Error::UnusableSample(
                "source changed while preparing analysis areas",
            ));
        }
    }
    // Fit appearance in stable locator order; browsing and filmstrip order cannot select geometry.
    let mut groups: Vec<Vec<usize>> = vec![];
    for (i, c) in captures.iter().enumerate() {
        if let Some(group) = groups
            .iter_mut()
            .find(|g| captures[g[0]].area == c.area && captures[g[0]].size == c.size)
        {
            group.push(i);
        } else {
            groups.push(vec![i]);
        }
    }
    let mut models = vec![];
    for group in &mut groups {
        group.sort_by_key(|&i| {
            request
                .document
                .source_path(project.order[i])
                .expect("verified locator")
        });
    }
    // Captures are bounded RGB16 rasters, never leases for an entire roll of full RAWs.
    for group in &groups {
        let refs: Vec<_> = group.iter().map(|&i| &captures[i]).collect();
        models.push(RollModel::fit_refs(&refs)?);
    }
    for (index, id) in project.order.iter().enumerate() {
        if cancelled() {
            return Ok(None);
        }
        progress(project.order.len() + index, project.order.len() * 2);
        if need_model
            && !(entries[index].areas.opening_edited && entries[index].areas.picture_edited)
        {
            let group = groups
                .iter()
                .position(|g| g.contains(&index))
                .expect("capture group");
            let within = groups[group].iter().position(|&i| i == index).unwrap();
            let expected = project.source(*id)?;
            let path = request.document.source_path(*id)?;
            let Some(scan) = analysis::develop_scan(
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
            let (w, h, rgb) = scan.preview_source(3072)?;
            let rgb: Vec<_> = rgb.into_iter().map(|p| [p[0], p[1], p[2]]).collect();
            let previous = &entries[index].areas;
            let mut suggested = models[group].areas(within, &captures[index], [w, h], &rgb)?;
            if previous.opening_edited
                && let Some(q) = previous.opening
            {
                suggested.edit_opening(Areas::coverage(q)?);
                suggested.refit_picture(captures[index].area);
            }
            if previous.picture_edited
                && let Some(q) = previous.picture
            {
                suggested.edit_picture(Areas::coverage(q)?);
            }
            entries[index].areas = suggested;
            if entries[index].areas.opening.is_none()
                && let Some(field) = project.active_flat_field()
            {
                entries[index].areas.opening = Some(
                    Coverage::rectangle(analysis::rect(field.map.area()))
                        .corners()
                        .map(|p| p.0),
                );
            }
        }
        if let Some(explicit) = project.analysis_area {
            let areas = &mut entries[index].areas;
            areas.edit_picture(Coverage::rectangle(analysis::rect(explicit)));
            if areas.opening.is_none() {
                areas.opening = Some(
                    Coverage::rectangle(project.source(*id)?.development.active_area.into())
                        .corners()
                        .map(|p| p.0),
                );
            }
        }
        if thumbnails::FileKey::new(&request.document.source_path(*id)?) != files[index] {
            return Err(Error::UnusableSample(
                "source changed while preparing analysis areas",
            ));
        }
    }
    if need_model {
        for (group, model) in groups.iter().zip(&models) {
            let mut areas: Vec<_> = group.iter().map(|&i| entries[i].areas.clone()).collect();
            model.refine_areas(&mut areas)?;
            for (&index, areas) in group.iter().zip(areas) {
                entries[index].areas = areas;
            }
        }
    }
    progress(project.order.len() * 2, project.order.len() * 2);
    if cancelled() {
        return Ok(None);
    }
    Ok(Some(Approved { entries, files }))
}

impl Editor {
    pub(super) fn prepare_analysis_review(&mut self, mut request: analysis::Request) {
        request.revision = self.analysis_generation.fetch_add(1, Ordering::Relaxed) + 1;
        let id = self.request_id();
        self.thumbnails.cancel();
        self.tasks.begin(Task::AnalysisAreas(id));
        self.analysis_preparing = Some(analysis::Pending {
            id,
            request: request.clone(),
        });
        self.busy = true;
        self.status = "Preparing per-photo analysis areas…".into();
        if self
            .commands
            .send(Command::AnalysisAreas(id, Box::new(request)))
            .is_err()
        {
            self.analysis_preparing = None;
            self.tasks.finish(TaskRequest::Analyze(id));
            self.busy = false;
            self.status = "RAW worker stopped; grade unchanged".into();
        }
    }
    pub(super) fn analysis_areas_result(
        &mut self,
        id: u64,
        result: Result<Option<Approved>, String>,
    ) {
        if !self.analysis_preparing.as_ref().is_some_and(|p| p.id == id) {
            return;
        }
        let pending = self.analysis_preparing.take().unwrap();
        self.tasks.finish(TaskRequest::Analyze(id));
        self.busy = false;
        self.sync_document();
        let request = pending.request;
        if self.analysis_generation.load(Ordering::Relaxed) != request.revision {
            self.status = "Analysis-area preparation cancelled; grade unchanged".into();
            return;
        }
        if self.id != request.loaded
            || self.pick.is_some()
            || self.crop_edit.is_some()
            || self.current_source != Some(request.selected)
            || self.document.project != request.document.project
            || self.document.path != request.document.path
        {
            self.status = "Analysis areas discarded: document or settings changed".into();
            return;
        }
        match result {
            Ok(Some(approved)) => {
                let replaces_photos = request.document.project.order.iter().any(|id| {
                    request
                        .document
                        .project
                        .source(*id)
                        .is_ok_and(|s| s.photo != PhotoSettings::default())
                });
                self.analysis_review = Some(Review {
                    request,
                    approved,
                    picture: false,
                    replaces_photos,
                    loaded: self.id,
                    return_tab: self.tab,
                    return_scopes_visible: self.scopes.visible,
                });
                self.tab = 0;
                // Positive-only scopes have no data in the source sampling tool.
                self.scopes.visible = false;
                self.begin_pick(Pick::Analysis);
                self.refresh_review_region();
                self.viewport.fit();
                self.status = "Review sampling areas".into();
            }
            Ok(None) => self.status = "Analysis-area preparation cancelled; grade unchanged".into(),
            Err(error) => {
                self.status = format!("Area preparation failed: {error}. Grade unchanged.")
            }
        }
    }
    pub(super) fn review_areas(&self, id: SourceId) -> Option<&Areas> {
        self.analysis_review
            .as_ref()?
            .approved
            .entries
            .iter()
            .find(|r| r.source == id)
            .map(|r| &r.areas)
    }
    pub(super) fn stash_review_region(&mut self) {
        if self.analysis_pending.is_some() {
            return;
        }
        let Some(review) = &mut self.analysis_review else {
            return;
        };
        let Some(entry) = review
            .approved
            .entries
            .iter_mut()
            .find(|r| Some(r.source) == self.current_source)
        else {
            return;
        };
        let Some(region) = self.region else {
            return;
        };
        let quad = region.corners().map(|p| p.0);
        let old = if review.picture {
            entry.areas.picture
        } else {
            entry.areas.opening
        };
        if old == Some(quad) {
            return;
        }
        if review.picture {
            entry.areas.edit_picture(region);
        } else {
            entry.areas.edit_opening(region);
            let active = review
                .request
                .document
                .project
                .source(entry.source)
                .expect("review source")
                .development
                .active_area
                .into();
            entry.areas.refit_picture(active);
        }
    }
    pub(super) fn refresh_review_region(&mut self) {
        self.region = self
            .analysis_review
            .as_ref()
            .and_then(|r| {
                self.current_source
                    .and_then(|id| r.approved.entries.iter().find(|e| e.source == id))
                    .and_then(|e| {
                        if r.picture {
                            e.areas.picture
                        } else {
                            e.areas.opening
                        }
                    })
            })
            .and_then(|q| Areas::coverage(q).ok());
        self.pick_drag = None;
    }
    pub(super) fn browse_analysis_review(&mut self, id: SourceId) {
        if self.analysis_review.is_none() || self.analysis_pending.is_some() {
            return;
        }
        self.stash_review_region();
        self.pick_roll_source(id);
        self.analysis_review.as_mut().unwrap().loaded = self.id;
        self.refresh_review_region();
    }
    pub(super) fn end_analysis_review(&mut self) {
        let Some(review) = self.analysis_review.take() else {
            return;
        };
        self.tab = review.return_tab;
        self.scopes.visible = review.return_scopes_visible;
        let notice = self.status.clone();
        self.finish_pick();
        self.status = notice;
    }
    pub(super) fn confirm_analysis_review(&mut self) {
        if self.busy || self.ready.is_none() || self.analysis_pending.is_some() {
            return;
        }
        self.stash_review_region();
        self.sync_document();
        let Some(review) = &self.analysis_review else {
            return;
        };
        if self.pick != Some(Pick::Analysis)
            || self.crop_edit.is_some()
            || self.id != review.loaded
            || self.current_source != self.document.project.order.get(self.selected).copied()
            || self.document.project != review.request.document.project
            || self.document.path != review.request.document.path
            || self.analysis_generation.load(Ordering::Relaxed) != review.request.revision
        {
            self.status = "Analysis review discarded: document or settings changed".into();
            self.end_analysis_review();
            return;
        }
        let mut request = review.request.clone();
        request.loaded = self.id;
        request.selected = self.current_source.expect("review source");
        request.areas = Some(review.approved.clone());
        if let Err(error) = verify(&request) {
            self.status = format!("Cannot analyze: {error}. Grade unchanged.");
            return;
        }
        self.start_analysis(request);
    }
    pub(super) fn review_area_at(
        &self,
        geometry: crate::viewport::CanvasLayout,
        point: egui::Pos2,
    ) -> Option<bool> {
        let review = self.analysis_review.as_ref()?;
        let areas = self.review_areas(self.current_source?)?;
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
                    let a = q[i];
                    let delta = q[(i + 1) % 4] - a;
                    let t = ((point - a).dot(delta) / delta.length_sq()).clamp(0., 1.);
                    point.distance(a + delta * t)
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
        geometry: crate::viewport::CanvasLayout,
        point: egui::Pos2,
    ) {
        self.stash_review_region();
        if let Some(picture) = self.review_area_at(geometry, point)
            && self
                .analysis_review
                .as_ref()
                .is_some_and(|r| r.picture != picture)
        {
            self.analysis_review.as_mut().unwrap().picture = picture;
            self.refresh_review_region();
        }
    }
    pub(super) fn paint_review_areas(
        &self,
        ui: &egui::Ui,
        geometry: crate::viewport::CanvasLayout,
        canvas: egui::Rect,
    ) {
        let Some(review) = &self.analysis_review else {
            return;
        };
        let other = self
            .current_source
            .and_then(|id| self.review_areas(id))
            .and_then(|a| if review.picture { a.opening } else { a.picture })
            .and_then(|q| Areas::coverage(q).ok());
        let painter = ui.painter().with_clip_rect(canvas);
        if let Some(material) = self
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
        for (coverage, active) in [(other, false), (self.region, true)] {
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
