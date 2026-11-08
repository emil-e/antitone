//! Captured roll sampling and fitting. Native awaits retain no blocking worker;
//! each source lease is released after bounded camera evidence is captured.
use crate::renderer::GpuRenderer;
#[cfg(all(test, target_os = "macos"))]
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

pub(super) struct Fit {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: Arc<GpuRenderer>,
    request: Arc<Request>,
    correction: Option<Arc<Correction>>,
    matrix: glam::DMat3,
    support: Option<Rect>,
    film: Option<Rect>,
    picture: Option<Rect>,
    owned: Vec<FrameMeasurements>,
}

impl Fit {
    pub(super) fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        renderer: Arc<GpuRenderer>,
        mut request: Request,
        input: source::Context,
    ) -> Result<Self, Error> {
        let scan = input.scan()?;
        if request.areas.is_none() && request.local_support && request.film.is_none() {
            request.film = scan.propose_flat_field_area()?;
        }
        request.document.validate_for_device(&device)?;
        analysis::verify(&request)?;
        let project = &request.document.project;
        if project.order.is_empty() || !project.order.contains(&request.selected) {
            return Err(Error::UnusableSample("no selected roll photo"));
        }
        if project.active_flat_field() != input.correction.as_ref().map(|c| &c.settings) {
            return Err(Error::UnusableSample(
                "analysis correction snapshot changed",
            ));
        }
        project.require_scan_sources()?;
        let matrix = glam::DMat3::from_cols_array_2d(
            &project
                .camera_basis()
                .ok_or(Error::UnusableSample("no roll camera basis"))?
                .camera_to_ap1,
        );
        let support = request.film;
        let owned = Vec::with_capacity(project.order.len());
        Ok(Self {
            device,
            queue,
            renderer,
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
        let actual =
            document::source_metadata(&path, &scan.source().metadata(), scan.source().fixed_wb())?;
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
        retained: Option<Arc<crate::scan::AnalysisInput>>,
        cancelled: impl Fn() -> bool,
    ) -> Result<bool, Error> {
        if cancelled() {
            return Ok(false);
        }
        let actual = self.verify_scan(index, &scan)?;
        self.request
            .document
            .project
            .require_scan_source(&actual.development)?;
        if let Some(correction) = &self.correction {
            correction.settings.require_source(&actual.development)?;
            scan.set_flat_field(Some(correction.settings.map.clone()))?;
        }
        let renderer = self.renderer.clone();
        let queue = self.queue.clone();
        let device = self.device.clone();
        let sampler = crate::gpu::analysis::CameraSampler::new(
            &device,
            &renderer,
            (&queue, &renderer.submissions),
            &scan,
        )?;
        let retained = retained
            .map(|input| {
                crate::gpu::analysis::CameraSampler::retained(
                    &device,
                    &renderer,
                    (&queue, &renderer.submissions),
                    input,
                    self.correction.as_ref().map(|c| c.settings.map.clone()),
                )
            })
            .transpose()?;
        self.sample_camera(
            index,
            scan.area,
            retained.as_ref().unwrap_or(&sampler),
            &sampler,
            cancelled,
        )
    }

    fn sample_camera(
        &mut self,
        index: usize,
        active: Rect,
        sampler: &crate::gpu::analysis::CameraSampler<'_>,
        originals: &crate::gpu::analysis::CameraSampler<'_>,
        cancelled: impl Fn() -> bool,
    ) -> Result<bool, Error> {
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
        let film_samples = sampler.sample(
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
        let picture_samples = sampler.sample(
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
        let area = match frame_areas {
            Some(areas) => crate::scan_regions::Areas::coverage(
                areas
                    .picture
                    .ok_or(Error::UnusableSample("no GI picture area"))?,
            )?
            .clipped_bounds(active)?,
            None => self.picture.expect("legacy picture geometry"),
        };
        let Some(spatial) = originals.spatial(
            area,
            |f| {
                frame_areas.is_none_or(|areas| areas.contains_footprint(f, active, true))
                    && on_film(f)
                    && inside_film(f, &picture_planes, 0.)
            },
            &cancelled,
        )?
        else {
            return Ok(false);
        };
        self.owned.push(FrameMeasurements {
            floor: sampler.effective_floor(),
            film,
            picture,
            spatial,
        });
        Ok(true)
    }

    fn finish(mut self, cancelled: impl Fn() -> bool) -> Result<Option<Outcome>, Error> {
        let device = &self.device;
        if cancelled() {
            return Ok(None);
        }
        let request = &self.request;
        let project = &request.document.project;
        let floor = restrict_to_common_floor(&mut self.owned)?;
        let frames: Vec<_> = self
            .owned
            .iter()
            .map(|frame| roll_analysis::SpatialFrameSamples {
                statistics: FrameSamples {
                    film: &frame.film.pixels,
                    picture: &frame.picture.pixels,
                },
                spatial: &frame.spatial,
            })
            .collect();
        let mut model = crate::gpu::analysis::ResponseModel::new(
            self.renderer.evaluation(),
            (&self.queue, &self.renderer.submissions),
        );
        let Some(proposal) = roll_analysis::estimate_spatial_with_model(
            &frames,
            floor,
            self.matrix,
            project.input,
            &mut model,
            &mut || cancelled(),
        )?
        else {
            return Ok(None);
        };
        require_statistics(
            self.owned.iter().map(|frame| frame.film.pixels.len()).sum(),
            &proposal.frames,
        )?;
        let wb_rejections: Vec<_> = proposal
            .frames
            .iter()
            .zip(&project.order)
            .filter_map(|(frame, id)| {
                frame
                    .grayness
                    .as_ref()
                    .and_then(|gi| gi.wb_rejection.as_ref())
                    .map(|reason| (*id, reason.clone()))
            })
            .collect();
        for (source, reason) in &wb_rejections {
            log::debug!(source = source.0, reason = reason.as_str(); "analysis_wb_abstained");
        }
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
        analysis::verify(request)?;
        candidate.validate_for_device(device)?;
        for id in &project.order {
            if cancelled() {
                return Ok(None);
            }
            let record = project.source(*id)?;
            let continuous = record.development.lens_correction.is_some();
            crate::gpu::parameters::CameraMatrix::require_source(self.matrix, continuous)?;
            if let Some(correction) = &self.correction {
                correction.gpu.validate_matrix(
                    self.matrix,
                    if continuous {
                        crate::processing::MAX_SOURCE_GAIN
                    } else {
                        1.
                    },
                )?;
                crate::gpu::source::SourceGeometry::new(record.development.active_area, [1, 1])?;
            }
        }
        if cancelled() {
            return Ok(None);
        }
        Ok(Some(Outcome {
            document: candidate,
            frames: frames.len(),
            wb_rejections,
            support: self.film.expect("film geometry"),
        }))
    }
}

impl Fit {
    pub(super) async fn run(
        mut self,
        sources: Arc<dyn SourceProvider>,
        sampling_inputs: SamplingInputs,
        exposures: Arc<crate::exposure_metadata::Store>,
        cancelled: Cancellation,
        mut progress: impl FnMut(usize, usize),
    ) -> Result<Option<Outcome>, Error> {
        let total = self.request.document.project.order.len();
        if self.needs_support() {
            progress(0, total);
            for index in 0..total {
                let Some(scan) = self
                    .request
                    .document
                    .acquire_source(
                        self.request.document.project.order[index],
                        sources.as_ref(),
                        &exposures,
                        &cancelled,
                    )
                    .await?
                else {
                    return Ok(None);
                };
                let Some(next) = background::compute(&cancelled, move || {
                    self.observe_support(index, scan)?;
                    Ok(self)
                })
                .await?
                else {
                    return Ok(None);
                };
                self = next;
                if !self.needs_support() {
                    break;
                }
            }
        }
        let Some(mut state) = background::compute(&cancelled, move || {
            self.geometry()?;
            Ok(self)
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
            let source = state.request.document.project.order[index];
            let retained = if state.request.areas.is_some() {
                sampling_inputs.get(source, &state.request.document.source_key(source)?)
            } else {
                None
            };
            let retired = cancelled.clone();
            // Spatial GI needs original sites beyond the retained sparse patches. The
            // source lease ends after this photograph's bounded camera evidence is copied.
            let Some(scan) = state
                .request
                .document
                .acquire_source(source, sources.as_ref(), &exposures, &cancelled)
                .await?
            else {
                return Ok(None);
            };
            let measured = background::compute(&cancelled, move || {
                let keep = state.sample(index, scan, retained, || retired.is_cancelled())?;
                Ok((state, keep))
            })
            .await?;
            let Some((next, keep)) = measured else {
                return Ok(None);
            };
            state = next;
            if !keep {
                return Ok(None);
            }
        }
        progress(total, total);
        let retired = cancelled.clone();
        Ok(
            background::compute(&cancelled, move || state.finish(|| retired.is_cancelled()))
                .await?
                .flatten(),
        )
    }
}
