use super::*;
use crate::{project::SourceAnalysis, scan_regions::Areas};
#[cfg(all(test, target_os = "macos"))]
mod performance;
pub(super) mod preparation;
#[cfg(all(test, target_os = "macos"))]
pub(super) mod preparation_tests;

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
    ) -> Option<futures_channel::oneshot::Receiver<analysis_workflow::Confirmation>> {
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
    ) -> Option<futures_channel::oneshot::Receiver<analysis_workflow::Confirmation>> {
        editor.sync_document();
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
        if editor.busy || editor.document.visit.canvas.ready.is_none() || self.fitting() {
            return;
        }
        editor.sync_document();
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
