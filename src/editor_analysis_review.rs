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
    pub reveal: bool,
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
                    reveal: true,
                });
                self.tab = 0;
                // Positive-only scopes have no data in the source sampling tool.
                self.scopes.visible = false;
                self.begin_pick(Pick::Analysis);
                self.refresh_review_region();
                self.viewport.fit();
                self.status="Review the suggested opening across the roll, then Analyze roll. Areas are statistics, not crops.".into();
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
        if review.picture
            && let Some(material) = self
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
                egui::Stroke::new(1., ui.visuals().weak_text_color())
            };
            painter.add(egui::Shape::closed_line(points.to_vec(), stroke));
            if active {
                for p in points {
                    painter.rect_filled(
                        egui::Rect::from_center_size(p, egui::vec2(6., 6.)),
                        0.,
                        stroke.color,
                    );
                }
            }
        }
    }
    pub(super) fn review_controls(&mut self, ui: &mut egui::Ui) {
        let Some(review) = &self.analysis_review else {
            return;
        };
        let count = review.approved.entries.len();
        let missing = review
            .approved
            .entries
            .iter()
            .filter(|e| !e.areas.ready())
            .count();
        let inspect = review
            .approved
            .entries
            .iter()
            .filter(|e| e.areas.needs_inspection())
            .count();
        let picture = review.picture;
        let replaces = review.replaces_photos;
        let detail = self
            .current_source
            .and_then(|id| self.review_areas(id))
            .map(|a| {
                let edited = if picture {
                    a.picture_edited
                } else {
                    a.opening_edited
                };
                if edited {
                    return if picture {
                        "Authored picture statistics area"
                    } else {
                        "Authored opening exception"
                    }
                    .into();
                }
                if !picture && a.opening_shared {
                    return if a.opening_exception {
                        "Roll-pooled opening with a photo-specific departure."
                    } else {
                        "Roll-pooled conservative sampling interior."
                    }
                    .into();
                }
                let observed = if picture {
                    a.picture_observed
                } else {
                    a.opening_observed
                }
                .iter()
                .filter(|&&v| v)
                .count();
                let weak = if picture {
                    a.picture_weak.iter().filter(|&&v| v).count()
                } else {
                    0
                };
                format!(
                    "{observed} fitted sides, {weak} weak transitions, {} coverage bounds.",
                    4 - observed - weak
                )
            });
        let mut next_picture = picture;
        ui.label(format!(
            "{} / {count} · {}",
            self.selected + 1,
            if picture {
                "Picture statistics"
            } else {
                "Holder opening"
            }
        ));
        ui.add_enabled_ui(self.analysis_pending.is_none(), |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.selectable_value(&mut next_picture, false, "Opening");
                ui.selectable_value(&mut next_picture, true, "Picture statistics");
            });
        });
        ui.small("Browse the filmstrip and edit photo exceptions. One confirmation covers the roll, not a crop or pixel-validity guarantee.");
        if replaces {
            presentation::notice_heading(
                ui,
                presentation::Severity::Warning,
                "Replace Photo adjustments",
            );
            ui.label("Analyze replaces Photo grading with estimated exposure and defaults for the other Photo controls.");
        } else {
            ui.label("Analyze initializes shared Roll settings and individual exposures.");
        }
        ui.small("Keeps gamma/curve, correction, crops and orientation. Cancel keeps accepted areas and grading.");
        if let Some(detail) = &detail {
            ui.small(detail);
        }
        if picture {
            ui.small("Tint: automatic film exclusions. Editing opening replaces that mask. Numerical pixel checks still run during Analyze.");
        }
        ui.small(format!("{inspect} photos have coverage bounds, opening exceptions or missing areas. Unflagged suggestions can also be wrong."));
        if missing > 0 {
            ui.small(format!(
                "{missing} photos need an opening/picture area before Analyze."
            ));
        }
        if next_picture != picture {
            self.stash_review_region();
            self.analysis_review.as_mut().unwrap().picture = next_picture;
            self.refresh_review_region();
        }
    }
}
