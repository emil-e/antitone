//! Captured roll sampling and fitting. Native awaits retain no blocking worker;
//! each measured source is released before only bounded statistics advance.
use super::super::analysis_services::{Cancellation, Services};
#[cfg(all(test, target_os = "macos"))]
#[path = "editor_analysis_fit_tests.rs"]
mod tests;

use super::*;

fn inside_film(footprint: Rect, planes: &[crate::spatial_metadata::Plane], inset: f64) -> bool {
    let corners = [
        [f64::from(footprint.x) + 0.5, f64::from(footprint.y) + 0.5],
        [
            f64::from(footprint.x + footprint.width) - 0.5,
            f64::from(footprint.y + footprint.height) - 0.5,
        ],
    ];
    // A linear half-plane reaches its minimum at the corner selected by its normal's signs.
    // Test the entire pixel-centre footprint, not merely the retained centre sample.
    planes.iter().all(|&[a, b, c]| {
        let x = corners[usize::from(a < 0.)][0];
        let y = corners[usize::from(b < 0.)][1];
        a * x + b * y >= c + inset
    })
}

struct Fit {
    request: Arc<Request>,
    correction: Option<Arc<Correction>>,
    matrix: glam::DMat3,
    support: Option<Rect>,
    film: Option<Rect>,
    picture: Option<Rect>,
    owned: Vec<FrameMeasurements>,
}

impl Fit {
    fn new(
        device: &wgpu::Device,
        mut request: Request,
        input: source::Context,
    ) -> Result<Self, Error> {
        let scan = input.scan()?;
        if request.areas.is_none() && request.local_support && request.film.is_none() {
            request.film = scan.propose_flat_field_area()?;
        }
        request.document.validate_for_device(device)?;
        analysis_review::verify(&request)?;
        let project = &request.document.project;
        if project.order.is_empty() || !project.order.contains(&request.selected) {
            return Err(Error::UnusableSample("no selected roll photo"));
        }
        if project.active_flat_field() != input.correction.as_ref().map(|c| &c.settings) {
            return Err(Error::UnusableSample(
                "analysis correction snapshot changed",
            ));
        }
        project
            .development
            .as_ref()
            .ok_or(Error::UnusableSample("no roll development"))?;
        let matrix = glam::DMat3::from_cols_array_2d(
            &project
                .camera_basis
                .as_ref()
                .ok_or(Error::UnusableSample("no roll camera basis"))?
                .camera_to_ap1,
        );
        let support = request.film;
        let owned = Vec::with_capacity(project.order.len());
        Ok(Self {
            request: Arc::new(request),
            correction: input.correction,
            matrix,
            support,
            film: None,
            picture: None,
            owned,
        })
    }

    fn needs_support(&self) -> bool {
        self.request.areas.is_none() && self.support.is_none()
    }

    fn verify_scan(&self, index: usize, scan: &Scan) -> Result<Source, Error> {
        let project = &self.request.document.project;
        let id = project.order[index];
        let path = self.request.document.source_path(id)?;
        let actual = document::source_metadata(&path, &scan.source().metadata())?;
        document::verify_source(project.source(id)?, &actual)?;
        document::verify_scan(&actual, scan)?;
        Ok(actual)
    }

    fn observe_support(&mut self, index: usize, scan: Scan) -> Result<(), Error> {
        self.verify_scan(index, &scan)?;
        self.support = scan.propose_flat_field_area()?;
        Ok(())
    }

    fn geometry(&mut self) -> Result<(), Error> {
        self.picture = if self.request.areas.is_some() {
            None
        } else {
            Some(picture_area(self.request.picture, self.support)?)
        };
        // Explicit picture geometry can still be used without a local illumination hint.
        // Never invent whole-camera geometry for dense-tail/balance/exposure statistics.
        self.film = Some(self.support.unwrap_or_else(|| {
            rect(
                self.request
                    .document
                    .project
                    .source(self.request.selected)
                    .expect("selected roll source")
                    .development
                    .active_area,
            )
        }));
        Ok(())
    }

    fn sample(
        &mut self,
        index: usize,
        mut scan: Scan,
        cancelled: impl Fn() -> bool,
    ) -> Result<bool, Error> {
        if cancelled() {
            return Ok(false);
        }
        let actual = self.verify_scan(index, &scan)?;
        self.request
            .document
            .project
            .require_camera_basis(&actual.development)?;
        if let Some(correction) = &self.correction {
            correction.settings.require_source(&actual.development)?;
            scan.set_flat_field(Some(correction.settings.map.clone()))?;
        }
        let active = scan.area;
        let frame_areas = self.request.areas.as_ref().map(|a| &a.entries[index].areas);
        let regions = self
            .request
            .document
            .spatial_metadata
            .groups
            .iter()
            .flat_map(|group| &group.sources)
            .find(|row| row.input.source == self.request.document.project.order[index]);
        let film_planes: Vec<_> = regions
            .and_then(|row| row.film.as_ref())
            .into_iter()
            .flat_map(|film| &film.boundaries)
            .filter_map(|boundary| match boundary {
                crate::spatial_metadata::Boundary::Measured { plane } => Some(*plane),
                _ => None,
            })
            .collect();
        let picture_planes: Vec<_> = regions
            .and_then(|row| row.picture.as_ref())
            .into_iter()
            .flat_map(|picture| &picture.boundaries)
            .filter_map(crate::spatial_metadata::Boundary::plane)
            .collect();
        let holder_planes: Vec<_> = regions
            .and_then(|row| row.holder_opening.as_ref())
            .into_iter()
            .flat_map(|holder| &holder.boundaries)
            .filter_map(crate::spatial_metadata::Boundary::plane)
            .collect();
        let inset = sampling_inset(active);
        let on_film = |f| {
            inside_film(f, &film_planes, inset)
                && inside_film(f, &holder_planes, inset)
                && regions
                    .and_then(|row| row.material.as_ref())
                    .is_none_or(|m| m.contains_footprint(f, active, inset))
        };
        // Physical film support supplements even authored areas. It must not resize the
        // canonical grid, alter accepted geometry, or turn unknown/camera limits into edges.
        let film_samples = scan.analysis_samples_in(
            if frame_areas.is_some() {
                active
            } else {
                self.film.expect("film geometry")
            },
            MAX_SAMPLES_PER_FRAME,
            |f| {
                frame_areas.is_none_or(|areas| areas.contains_footprint(f, active, false))
                    && on_film(f)
            },
            &cancelled,
        )?;
        let Some(film) = film_samples else {
            return Ok(false);
        };
        let picture_samples = scan.analysis_samples_in(
            if frame_areas.is_some() {
                active
            } else {
                self.picture.expect("legacy picture geometry")
            },
            MAX_SAMPLES_PER_FRAME,
            |f| {
                frame_areas.is_none_or(|areas| areas.contains_footprint(f, active, true))
                    && on_film(f)
                    && inside_film(f, &picture_planes, 0.)
            },
            &cancelled,
        )?;
        let Some(picture) = picture_samples else {
            return Ok(false);
        };
        require_picture(&picture)?;
        self.owned.push(FrameMeasurements {
            floor: scan.effective_floor(),
            film,
            picture,
        });
        Ok(true)
    }

    fn finish(
        mut self,
        device: &wgpu::Device,
        cancelled: impl Fn() -> bool,
    ) -> Result<Option<Outcome>, Error> {
        if cancelled() {
            return Ok(None);
        }
        let request = &self.request;
        let project = &request.document.project;
        let floor = restrict_to_common_floor(&mut self.owned)?;
        let frames: Vec<_> = self
            .owned
            .iter()
            .map(|frame| FrameSamples {
                film: &frame.film.pixels,
                picture: &frame.picture.pixels,
            })
            .collect();
        let Some(proposal) =
            roll_analysis::estimate(&frames, floor, self.matrix, project.roll, &cancelled)?
        else {
            return Ok(None);
        };
        require_statistics(
            self.owned.iter().map(|frame| frame.film.pixels.len()).sum(),
            &proposal.frames,
        )?;
        let mut candidate = proposed_document(&request.document, proposal, self.picture)?;
        if let Some(approved) = &request.areas {
            candidate
                .project
                .analysis_regions
                .retain(|e| !project.order.contains(&e.source));
            candidate
                .project
                .analysis_regions
                .extend(approved.entries.clone());
        }
        analysis_review::verify(request)?;
        candidate.validate_for_device(device)?;
        for id in &project.order {
            if cancelled() {
                return Ok(None);
            }
            let record = project.source(*id)?;
            let mut input = if record.development.lens_correction.is_some() {
                crate::gpu::ProcessingSource::new_continuous(device, &[[0.; 4]], self.matrix)?
            } else {
                crate::gpu::ProcessingSource::new(device, &[[0; 4]], self.matrix)?
            };
            if let Some(correction) = &self.correction {
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
            support: self.film.expect("film geometry"),
        }))
    }
}

pub(in crate::editor) async fn run(
    request: Request,
    input: source::Context,
    services: Services,
    cancelled: Cancellation,
    mut progress: impl FnMut(usize, usize),
) -> Result<Option<Outcome>, Error> {
    let device = services.device.clone();
    let Some(mut state) = services
        .cpu(&cancelled, move || Fit::new(&device, request, input))
        .await?
    else {
        return Ok(None);
    };
    let total = state.request.document.project.order.len();
    if state.needs_support() {
        progress(0, total);
        for index in 0..total {
            if cancelled.is_cancelled() {
                return Ok(None);
            }
            let Some(scan) = services
                .acquire(state.request.clone(), index, cancelled.clone())
                .await?
            else {
                return Ok(None);
            };
            let Some(next) = services
                .cpu(&cancelled, move || {
                    state.observe_support(index, scan)?;
                    Ok(state)
                })
                .await?
            else {
                return Ok(None);
            };
            state = next;
            if !state.needs_support() {
                break;
            }
        }
    }
    let Some(mut state) = services
        .cpu(&cancelled, move || {
            state.geometry()?;
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
        progress(index, total);
        let Some(scan) = services
            .acquire(state.request.clone(), index, cancelled.clone())
            .await?
        else {
            return Ok(None);
        };
        let retired = cancelled.clone();
        let Some((next, keep)) = services
            .cpu(&cancelled, move || {
                let keep = state.sample(index, scan, || retired.is_cancelled())?;
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
    progress(total, total);
    let device = services.device.clone();
    let retired = cancelled.clone();
    Ok(services
        .cpu(&cancelled, move || {
            state.finish(&device, || retired.is_cancelled())
        })
        .await?
        .flatten())
}
