//! Roll fitting uses shared original-camera evidence, independent of optional RAW retention.
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

pub(super) struct Resources {
    pub correction: Option<Arc<Correction>>,
    pub sources: Arc<dyn SourceProvider>,
    pub sampling_inputs: SamplingInputs,
    pub initialization: Arc<crate::source::Initialization>,
}

pub(super) struct Fit {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: Arc<GpuRenderer>,
    request: Arc<Request>,
    correction: Option<Arc<Correction>>,
    matrix: glam::DMat3,
    cameras: Vec<Arc<crate::scan::AnalysisInput>>,
    support: Option<Rect>,
    film: Option<Rect>,
    picture: Option<Rect>,
    owned: Vec<FrameMeasurements>,
}

impl Fit {
    pub(super) async fn acquire(
        device: wgpu::Device,
        queue: wgpu::Queue,
        renderer: Arc<GpuRenderer>,
        mut request: Request,
        resources: Resources,
        cancelled: &Cancellation,
    ) -> Result<Option<Self>, Error> {
        let Resources {
            correction,
            sources,
            mut sampling_inputs,
            initialization,
        } = resources;
        let project = &request.document.project;
        if project.order.is_empty() || !project.order.contains(&request.selected) {
            return Err(Error::UnusableSample("no selected roll photo"));
        }
        if project.active_flat_field() != correction.as_ref().map(|c| &c.settings) {
            return Err(Error::UnusableSample(
                "analysis correction snapshot changed",
            ));
        }
        let mut support = request.film;
        if request.areas.is_none() && request.local_support && support.is_none() {
            for &source in &project.order {
                let physical = request.document.source_input(source, &initialization)?;
                let acquired = futures_lite::future::or(
                    async {
                        cancelled.wait().await;
                        Ok(None)
                    },
                    async {
                        physical
                            .source(sources.as_ref(), crate::source::Priority::Foreground)
                            .await
                            .map(Some)
                    },
                )
                .await?;
                let Some(acquired) = acquired else {
                    return Ok(None);
                };
                let scan = acquired.prepared.scan();
                let Some(proposed) =
                    background::compute(cancelled, move || scan.propose_flat_field_area()).await?
                else {
                    return Ok(None);
                };
                support = proposed;
                let Some(camera) = acquired.camera_samples(cancelled).await? else {
                    return Ok(None);
                };
                sampling_inputs.record(source, &camera);
                if support.is_some() {
                    break;
                }
            }
        }
        let mut cameras = Vec::with_capacity(project.order.len());
        let mut basis: Option<crate::project::ScanBasis> = None;
        for &source in &project.order {
            let physical = request.document.source_input(source, &initialization)?;
            let Some(camera) = sampling_inputs
                .acquire(source, &physical, sources.as_ref(), cancelled)
                .await?
            else {
                return Ok(None);
            };
            let shared = basis.get_or_insert_with(|| {
                crate::project::ScanBasis::from_development(camera.development())
            });
            shared.require_source(camera.development())?;
            cameras.push(camera.pixels());
        }
        let matrix = glam::DMat3::from_cols_array_2d(
            &basis.expect("nonempty captured roll").camera.camera_to_ap1,
        );
        let owned = Vec::with_capacity(project.order.len());
        request.document.capture_initialization(&initialization);
        analysis::verify(&request)?;
        Ok(Some(Self {
            device,
            queue,
            renderer,
            request: Arc::new(request),
            correction,
            matrix,
            cameras,
            support,
            film: None,
            picture: None,
            owned,
        }))
    }

    #[cfg(test)]
    fn verify_scan(&self, index: usize, scan: &Scan) -> Result<Source, Error> {
        let project = &self.request.document.project;
        let id = project.order[index];
        let path = self.request.document.source_path(id)?;
        let actual = document::source_metadata(
            &path,
            &scan.source().metadata(),
            scan.source().white_balance(),
        )?;
        crate::development_metadata::require_geometry(
            &project.source(id)?.development,
            &actual.development,
        )?;
        document::verify_scan(&actual, scan)?;
        Ok(actual)
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

    #[cfg(test)]
    fn capture_input(
        &self,
        index: usize,
        scan: &Scan,
        cancelled: impl Fn() -> bool,
    ) -> Result<Option<Arc<crate::scan::AnalysisInput>>, Error> {
        let actual = self.verify_scan(index, scan)?;
        crate::project::ScanBasis {
            camera: crate::project::CameraBasis {
                camera_model: actual.development.camera_model.clone(),
                camera_to_ap1: self.matrix.to_cols_array_2d(),
            },
        }
        .require_source(&actual.development)?;
        Ok(crate::scan::AnalysisInput::capture(scan, cancelled)?.map(Arc::new))
    }

    #[cfg(test)]
    fn sample(
        &mut self,
        index: usize,
        scan: Scan,
        retained: Option<Arc<crate::scan::AnalysisInput>>,
        cancelled: impl Fn() -> bool,
    ) -> Result<bool, Error> {
        let input = match retained {
            Some(input) => input,
            None => {
                let Some(input) = self.capture_input(index, &scan, &cancelled)? else {
                    return Ok(false);
                };
                input
            }
        };
        drop(scan);
        self.sample_input(index, input, cancelled)
    }

    fn sample_input(
        &mut self,
        index: usize,
        input: Arc<crate::scan::AnalysisInput>,
        cancelled: impl Fn() -> bool,
    ) -> Result<bool, Error> {
        if cancelled() {
            return Ok(false);
        }
        let active = input.area();
        if active
            != self
                .request
                .document
                .project
                .source(self.request.document.project.order[index])?
                .development
                .active_area
                .into()
        {
            return Err(Error::InvalidParameter(
                "captured camera grid differs from source",
            ));
        }
        let renderer = self.renderer.clone();
        let queue = self.queue.clone();
        let sampler = crate::gpu::analysis::CameraSampler::captured(
            &renderer,
            (&queue, &renderer.submissions),
            input,
            self.correction.as_ref().map(|c| c.gpu.clone()),
        )?;
        self.sample_camera(index, active, &sampler, cancelled)
    }

    fn sample_camera(
        &mut self,
        index: usize,
        active: Rect,
        sampler: &crate::gpu::analysis::CameraSampler<'_>,
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
        let Some(spatial) = sampler.window(
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
            .map(|frame| FrameSamples {
                film: &frame.film.pixels,
                picture: &frame.picture.pixels,
            })
            .collect();
        let mut model = crate::gpu::analysis::ResponseModel::new(
            self.renderer.evaluation(),
            (&self.queue, &self.renderer.submissions),
        );
        let Some(proposal) = roll_analysis::estimate_spatial_with_provider(
            &frames,
            floor,
            self.matrix,
            project.input,
            &mut model,
            &mut || cancelled(),
            |index, stopped| {
                self.owned[index].spatial.read(
                    &self.renderer,
                    (&self.queue, &self.renderer.submissions),
                    self.correction.as_ref().map(|c| c.gpu.clone()),
                    stopped,
                )
            },
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
            crate::gpu::parameters::CameraMatrix::require_source(self.matrix, true)?;
            if let Some(correction) = &self.correction {
                correction
                    .gpu
                    .validate_matrix(self.matrix, crate::processing::MAX_SOURCE_GAIN)?;
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
        self,
        cancelled: Cancellation,
        mut progress: impl FnMut(usize, usize),
    ) -> Result<Option<Outcome>, Error> {
        let total = self.cameras.len();
        let Some(mut state) = background::compute(&cancelled, move || {
            let mut state = self;
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
            let input = state.cameras[index].clone();
            let retired = cancelled.clone();
            let measured = background::compute(&cancelled, move || {
                let keep = state.sample_input(index, input, || retired.is_cancelled())?;
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
