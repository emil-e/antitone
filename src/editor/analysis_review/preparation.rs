//! Sequential area preparation: bounded evidence and optional exact scratch survive
//! native awaits, but a full developed source is held only while measuring one photo.
#[cfg(all(test, target_os = "macos"))]
mod tests;

use super::*;
use crate::editor::analysis_services::Cancellation;
use std::sync::Arc;

struct Preparation {
    request: Arc<analysis::Request>,
    code_scale: f64,
    files: Vec<thumbnails::FileKey>,
    entries: Vec<SourceAnalysis>,
    need_model: bool,
    captures: Vec<Capture>,
    measurements: measurements::Measurements,
    saved: Vec<Option<measurements::Measurement>>,
    groups: Vec<Vec<usize>>,
    models: Vec<RollModel>,
    fine: Option<([u32; 2], Vec<[u16; 3]>)>,
}

impl Preparation {
    fn new(
        request: Arc<analysis::Request>,
        device: &wgpu::Device,
        measurements: measurements::Measurements,
    ) -> Result<Self, Error> {
        request.document.validate_for_device(device)?;
        let project = &request.document.project;
        project.require_scan_sources()?;
        // One shared scale preserves cross-frame appearance while fitting shading headroom into evidence.
        let code_scale = project.order.iter().try_fold(1f64, |scale, id| {
            Ok::<_, Error>(scale.max(
                project.source(*id)?.development.input_floor
                    / crate::processing::CAMERA_INPUT_FLOOR,
            ))
        })?;
        let files = project
            .order
            .iter()
            .map(|id| {
                request
                    .document
                    .source_path(*id)
                    .map(|p| request.document.file_key(&p))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let entries: Vec<_> = project
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
        let saved = (0..project.order.len()).map(|_| None).collect();
        Ok(Self {
            request,
            code_scale,
            files,
            entries,
            need_model,
            captures: vec![],
            measurements,
            saved,
            groups: vec![],
            models: vec![],
            fine: None,
        })
    }

    fn verify_scan(&self, index: usize, scan: &Scan) -> Result<Source, Error> {
        let project = &self.request.document.project;
        let id = project.order[index];
        let path = self.request.document.source_path(id)?;
        let actual =
            document::source_metadata(&path, &scan.source().metadata(), scan.source().fixed_wb())?;
        document::verify_source(project.source(id)?, &actual)?;
        document::verify_scan(&actual, scan)?;
        Ok(actual)
    }

    fn coarse(
        &mut self,
        index: usize,
        scan: Option<Scan>,
        cancelled: impl Fn() -> bool,
    ) -> Result<bool, Error> {
        if let Some(scan) = scan {
            let actual = self.verify_scan(index, &scan)?;
            self.request
                .document
                .project
                .require_scan_source(&actual.development)?;
            let (w, h, rgb) = timed!(
                "coarse_preview",
                scan.measurement_preview(768, self.code_scale)
            )?;
            let rgb: Vec<_> = timed!(
                "coarse_pack",
                rgb.into_iter().map(|p| [p[0], p[1], p[2]]).collect()
            );
            let evidence = timed!("region_lines", Evidence::observe([w, h], &rgb))?;
            self.captures.push(timed!(
                "candidate_grouping",
                Capture::new(scan.area, [w, h], rgb, evidence)
            )?);
            if cancelled() {
                return Ok(false);
            }
            if !(self.entries[index].areas.opening_edited
                && self.entries[index].areas.picture_edited)
            {
                let (w, h) = scan.preview_dimensions(3072)?;
                if self.measurements.can_store([w, h]) {
                    let (size, rgb) = fine_measurement(&scan, self.code_scale)?;
                    self.saved[index] =
                        timed!("measurement_write", self.measurements.store(size, &rgb));
                }
            }
        }
        Ok(true)
    }

    fn fit(&mut self) -> Result<(), Error> {
        // Fit appearance in stable locator order; browsing and filmstrip order cannot select geometry.
        self.groups = timed!("source_grouping", {
            let mut groups: Vec<Vec<usize>> = vec![];
            for (i, c) in self.captures.iter().enumerate() {
                if let Some(group) = groups.iter_mut().find(|g| {
                    self.captures[g[0]].area == c.area && self.captures[g[0]].size == c.size
                }) {
                    group.push(i);
                } else {
                    groups.push(vec![i]);
                }
            }
            for group in &mut groups {
                group.sort_by_key(|&i| {
                    self.request
                        .document
                        .source_path(self.request.document.project.order[i])
                        .expect("verified locator")
                });
            }
            groups
        });
        // Captures are bounded RGB16 rasters, never leases for an entire roll of full RAWs.
        for group in &self.groups {
            let refs: Vec<_> = group.iter().map(|&i| &self.captures[i]).collect();
            self.models
                .push(timed!("roll_fit", RollModel::fit_refs(&refs))?);
        }
        Ok(())
    }

    fn needs_fine(&self, index: usize) -> bool {
        self.need_model
            && !(self.entries[index].areas.opening_edited
                && self.entries[index].areas.picture_edited)
    }

    fn read_fine(&mut self, index: usize) -> Result<bool, Error> {
        if !self.needs_fine(index) {
            return Ok(false);
        }
        self.fine = self.saved[index]
            .take()
            .and_then(|key| timed!("measurement_read", self.measurements.load(key)));
        Ok(self.fine.is_none())
    }

    fn refine(&mut self, index: usize, scan: Option<Scan>) -> Result<(), Error> {
        let project = &self.request.document.project;
        let id = project.order[index];
        if self.needs_fine(index) {
            let group = self
                .groups
                .iter()
                .position(|g| g.contains(&index))
                .expect("capture group");
            let within = self.groups[group].iter().position(|&i| i == index).unwrap();
            let ([w, h], rgb) = match self.fine.take() {
                Some(measurement) => measurement,
                None => {
                    let scan = scan.expect("fine source acquisition");
                    self.verify_scan(index, &scan)?;
                    fine_measurement(&scan, self.code_scale)?
                }
            };
            let previous = &self.entries[index].areas;
            let mut suggested = timed!(
                "fine_areas",
                self.models[group].areas(within, &self.captures[index], [w, h], &rgb)
            )?;
            if previous.opening_edited
                && let Some(q) = previous.opening
            {
                suggested.edit_opening(Areas::coverage(q)?);
                suggested.refit_picture(self.captures[index].area);
            }
            if previous.picture_edited
                && let Some(q) = previous.picture
            {
                suggested.edit_picture(Areas::coverage(q)?);
            }
            self.entries[index].areas = suggested;
            if self.entries[index].areas.opening.is_none()
                && let Some(field) = project.active_flat_field()
            {
                self.entries[index].areas.opening = Some(
                    Coverage::rectangle(analysis::rect(field.map.area()))
                        .corners()
                        .map(|p| p.0),
                );
            }
        }
        if let Some(explicit) = project.analysis_area {
            let areas = &mut self.entries[index].areas;
            areas.edit_picture(Coverage::rectangle(analysis::rect(explicit)));
            if areas.opening.is_none() {
                areas.opening = Some(
                    Coverage::rectangle(project.source(id)?.development.active_area.into())
                        .corners()
                        .map(|p| p.0),
                );
            }
        }
        Ok(())
    }

    fn apply_shared_picture_support(&mut self) -> Result<(), Error> {
        let project = &self.request.document.project;
        if project.analysis_area.is_some() {
            return Ok(());
        }
        let metadata = self.request.document.ready_spatial_metadata();
        for entry in &mut self.entries {
            if entry.areas.picture_edited {
                continue;
            }
            let Some(regions) = metadata
                .groups
                .iter()
                .flat_map(|group| &group.sources)
                .find(|row| row.input.source == entry.source)
            else {
                continue;
            };
            let active = regions.input.development.active_area;
            let inset = analysis::sampling_inset(active.into());
            if let Ok(quad) = regions.picture_sampling(inset) {
                entry.areas.picture = Some(quad);
                entry.areas.picture_geometry = None;
                entry.areas.picture_constraints.clear();
                entry.areas.picture_observed = [false; 4];
                entry.areas.picture_weak = [false; 4];
                entry.areas.validate(active.into())?;
            }
        }
        Ok(())
    }

    fn finish(mut self) -> Result<Approved, Error> {
        if self.need_model {
            for (group, model) in self.groups.iter().zip(&self.models) {
                let mut areas: Vec<_> = group
                    .iter()
                    .map(|&i| self.entries[i].areas.clone())
                    .collect();
                timed!("shared_refine", model.refine_areas(&mut areas))?;
                for (&index, areas) in group.iter().zip(areas) {
                    self.entries[index].areas = areas;
                }
            }
        }
        self.apply_shared_picture_support()?;
        Ok(Approved {
            entries: self.entries,
            files: self.files,
        })
    }
}

pub(in crate::editor) async fn prepare(
    request: analysis::Request,
    services: super::super::analysis_services::Services,
    cancelled: Cancellation,
    progress: impl FnMut(usize, usize),
) -> Result<Option<Approved>, Error> {
    prepare_staged(request, services, cancelled, progress, Default::default()).await
}

async fn prepare_staged(
    request: analysis::Request,
    services: super::super::analysis_services::Services,
    cancelled: Cancellation,
    mut progress: impl FnMut(usize, usize),
    measurements: measurements::Measurements,
) -> Result<Option<Approved>, Error> {
    let request = Arc::new(request);
    let input = request.clone();
    let device = services.device.clone();
    let Some(mut state) = services
        .cpu(&cancelled, move || {
            Preparation::new(input, &device, measurements)
        })
        .await?
    else {
        return Ok(None);
    };
    let total = request.document.project.order.len();
    for index in 0..total {
        if cancelled.is_cancelled() {
            return Ok(None);
        }
        progress(index, total * 2);
        if cancelled.is_cancelled() {
            return Ok(None);
        }
        let scan = if state.need_model {
            let Some(scan) = services
                .acquire(request.clone(), index, cancelled.clone())
                .await?
            else {
                return Ok(None);
            };
            Some(scan)
        } else {
            None
        };
        let retired = cancelled.clone();
        let Some((next, keep)) = services
            .cpu(&cancelled, move || {
                let keep = state.coarse(index, scan, || retired.is_cancelled())?;
                Ok((state, keep))
            })
            .await?
        else {
            return Ok(None);
        };
        state = next;
        if !keep {
            return Ok(None);
        }
    }
    let Some(mut state) = services
        .cpu(&cancelled, move || {
            state.fit()?;
            Ok(state)
        })
        .await?
    else {
        return Ok(None);
    };
    for index in 0..total {
        if cancelled.is_cancelled() {
            return Ok(None);
        }
        progress(total + index, total * 2);
        if cancelled.is_cancelled() {
            return Ok(None);
        }
        let Some((next, acquire)) = services
            .cpu(&cancelled, move || {
                let acquire = state.read_fine(index)?;
                Ok((state, acquire))
            })
            .await?
        else {
            return Ok(None);
        };
        state = next;
        let scan = if acquire {
            let Some(scan) = services
                .acquire(request.clone(), index, cancelled.clone())
                .await?
            else {
                return Ok(None);
            };
            Some(scan)
        } else {
            None
        };
        let Some(next) = services
            .cpu(&cancelled, move || {
                state.refine(index, scan)?;
                Ok(state)
            })
            .await?
        else {
            return Ok(None);
        };
        state = next;
    }
    let approved = services.cpu(&cancelled, move || state.finish()).await?;
    progress(total * 2, total * 2);
    Ok(approved.filter(|_| !cancelled.is_cancelled()))
}

// Algorithm/resource fixtures use the same stages synchronously so their scoped
// timing recorder observes every stage, including native acquisition and scratch I/O.
#[cfg(test)]
pub(super) fn prepare_with_measurements(
    request: &analysis::Request,
    generation: &AtomicU64,
    developments: &source_loading::SourceLoader,
    device: &wgpu::Device,
    mut progress: impl FnMut(usize, usize),
    measurements: measurements::Measurements,
    cache: &Arc<crate::source::SourceCache>,
) -> Result<Option<Approved>, Error> {
    let cancelled = || generation.load(Ordering::Relaxed) != request.revision;
    if cancelled() {
        return Ok(None);
    }
    let mut state = Preparation::new(Arc::new(request.clone()), device, measurements)?;
    let project = &request.document.project;
    let wb = project
        .sources
        .first()
        .expect("verified roll source")
        .development
        .decode
        .fixed_wb;
    let total = project.order.len();
    for (index, id) in project.order.iter().enumerate() {
        if cancelled() {
            return Ok(None);
        }
        progress(index, total * 2);
        if cancelled() {
            return Ok(None);
        }
        let scan = if state.need_model {
            let Some(scan) = timed!(
                "coarse_acquire",
                id.0,
                analysis::develop_scan(
                    developments,
                    &request.document,
                    cache,
                    project.source(*id)?,
                    &request.document.source_path(*id)?,
                    wb,
                    cancelled,
                )
            )?
            else {
                return Ok(None);
            };
            Some(scan)
        } else {
            None
        };
        if !state.coarse(index, scan, cancelled)? {
            return Ok(None);
        }
    }
    state.fit()?;
    for (index, id) in project.order.iter().enumerate() {
        if cancelled() {
            return Ok(None);
        }
        progress(total + index, total * 2);
        if cancelled() {
            return Ok(None);
        }
        let scan = if state.read_fine(index)? {
            let Some(scan) = timed!(
                "fine_acquire",
                id.0,
                analysis::develop_scan(
                    developments,
                    &request.document,
                    cache,
                    project.source(*id)?,
                    &request.document.source_path(*id)?,
                    wb,
                    cancelled,
                )
            )?
            else {
                return Ok(None);
            };
            Some(scan)
        } else {
            None
        };
        state.refine(index, scan)?;
    }
    let approved = state.finish()?;
    progress(total * 2, total * 2);
    Ok((!cancelled()).then_some(approved))
}
