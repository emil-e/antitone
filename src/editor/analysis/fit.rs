//! Roll fitting uses shared original-camera evidence, independent of optional RAW retention.
use crate::{
    evaluation,
    photographic::graph::camera_response,
    photographic::stages::display_rendering as drt,
    rendering::OutputTarget,
    roll_analysis::gpu::{CameraSampler, ResponseModel},
};
#[cfg(all(test, target_os = "macos"))]
mod tests;

use super::*;
#[cfg(test)]
use crate::scan::DevelopedSource;

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
    pub correction: super::super::correction::Resources,
    pub sources: Arc<dyn SourceProvider>,
    pub sampling_inputs: SamplingInputs,
    pub initialization: Arc<crate::source::Initialization>,
}

struct Camera {
    pixels: Arc<crate::scan::AnalysisInput>,
    camera_to_ap1: glam::DMat3,
}

pub(super) struct Fit {
    evaluation: evaluation::Context,
    queue: crate::gpu::submission::Queue,
    request: Arc<Request>,
    correction: Option<Arc<Correction>>,
    cameras: Vec<Camera>,
    support: Option<Rect>,
    film: Option<Rect>,
    picture: Option<Rect>,
    owned: Vec<FrameMeasurements>,
}

impl Fit {
    pub(super) async fn acquire(
        evaluation: evaluation::Context,
        queue: crate::gpu::submission::Queue,
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
        let correction = futures_lite::future::or(
            async {
                cancelled.wait().await;
                Ok(None)
            },
            async { correction.acquire().await.map(Some).map_err(Error::Gpu) },
        )
        .await?;
        let Some(correction) = correction else {
            return Ok(None);
        };
        let project = &request.document.project;
        if project.order.is_empty() || !project.order.contains(&request.selected) {
            return Err(Error::UnusableSample("no selected roll photo"));
        }
        if project.active_flat_field().map(|settings| &settings.map)
            != correction.as_ref().map(|correction| correction.gpu.field())
        {
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
                let developed = acquired.prepared.source.clone();
                let Some(proposed) = background::compute(cancelled, move || {
                    crate::flat_field::reference::suggest_region(&developed)
                })
                .await?
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
        for &source in &project.order {
            let physical = request.document.source_input(source, &initialization)?;
            let Some(camera) = sampling_inputs
                .acquire(source, &physical, sources.as_ref(), cancelled)
                .await?
            else {
                return Ok(None);
            };
            cameras.push(Camera {
                camera_to_ap1: glam::DMat3::from_cols_array_2d(&camera.development().camera_to_ap1),
                pixels: camera.pixels(),
            });
        }

        let owned = Vec::with_capacity(project.order.len());
        request.document.capture_initialization(&initialization);
        analysis::verify(&request)?;
        Ok(Some(Self {
            evaluation,
            queue,
            request: Arc::new(request),
            correction,
            cameras,
            support,
            film: None,
            picture: None,
            owned,
        }))
    }

    #[cfg(test)]
    fn verify_source(&self, index: usize, source: &DevelopedSource) -> Result<(), Error> {
        let project = &self.request.document.project;
        let id = project.order[index];
        let actual = crate::project::SourceDevelopment::from_developed(source)?;
        crate::development_metadata::require_geometry(&project.source(id)?.development, &actual)
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
        source: &DevelopedSource,
        cancelled: impl Fn() -> bool,
    ) -> Result<Option<Arc<crate::scan::AnalysisInput>>, Error> {
        self.verify_source(index, source)?;
        Ok(crate::scan::AnalysisInput::capture(source, cancelled)?.map(Arc::new))
    }

    #[cfg(test)]
    fn sample(
        &mut self,
        index: usize,
        source: Arc<DevelopedSource>,
        retained: Option<Arc<crate::scan::AnalysisInput>>,
        cancelled: impl Fn() -> bool,
    ) -> Result<bool, Error> {
        let input = match retained {
            Some(input) => input,
            None => {
                let Some(input) = self.capture_input(index, &source, &cancelled)? else {
                    return Ok(false);
                };
                input
            }
        };
        drop(source);
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
        let evaluation = self.evaluation.clone();
        let queue = self.queue.clone();
        let sampler = CameraSampler::captured(
            &evaluation,
            &queue,
            input,
            self.correction.as_ref().map(|c| c.gpu.clone()),
        )?;
        self.sample_camera(index, active, &sampler, cancelled)
    }

    fn sample_camera(
        &mut self,
        index: usize,
        active: Rect,
        sampler: &CameraSampler<'_>,
        cancelled: impl Fn() -> bool,
    ) -> Result<bool, Error> {
        if let Some(picture) = self.picture {
            require_picture_bounds(picture, active)?;
        }
        let source = self.request.document.project.order[index];
        let frame_areas = self.request.areas.as_ref().map(|approved| {
            &approved
                .entry(source)
                .expect("verified reviewed photo")
                .areas
        });
        let regions = self
            .request
            .document
            .spatial_metadata
            .groups
            .iter()
            .flat_map(|group| &group.sources)
            .find(|row| row.input.source == source);
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
        self.owned.push(FrameMeasurements {
            camera_to_ap1: self.cameras[index].camera_to_ap1,
            film,
            picture,
        });
        Ok(true)
    }

    fn finish(
        self,
        classifier_drt: &drt::Target,
        cancelled: impl Fn() -> bool,
    ) -> Result<Option<Outcome>, Error> {
        if cancelled() {
            return Ok(None);
        }
        let request = &self.request;
        let project = &request.document.project;
        let frames: Vec<_> = self
            .owned
            .iter()
            .map(|frame| FrameSamples {
                camera_to_ap1: frame.camera_to_ap1,
                film: &frame.film.pixels,
                picture: &frame.picture.pixels,
            })
            .collect();
        let mut model = ResponseModel::new(&self.evaluation, &self.queue);
        let Some(mut proposal) =
            roll_analysis::estimate_anchor_spans(&frames, project.input, &mut model, &mut || {
                cancelled()
            })?
        else {
            return Ok(None);
        };
        // Validate capture support before semantic filtering, which may abstain on an individual photo.
        require_statistics(
            self.owned.iter().map(|frame| frame.film.pixels.len()).sum(),
            &proposal.frames,
        )?;
        if cancelled() {
            return Ok(None);
        }
        let load_started = std::time::Instant::now();
        let mut classifier = semantic::Classifier::load()?;
        let load_seconds = load_started.elapsed().as_secs_f64();
        let initial = proposal.roll;
        let negative = proposal.input.negative_input().expect("anchor proposal");
        let posts: Vec<_> = self
            .cameras
            .iter()
            .map(|camera| {
                Ok(crate::color::wb_matrix(initial.wb)?
                    * camera.camera_to_ap1
                    * initial.exposure.exp2())
            })
            .collect::<Result<_, Error>>()?;
        let started = std::time::Instant::now();
        if !roll_analysis::semantic_refit(
            &mut proposal,
            &frames,
            &mut model,
            &mut || cancelled(),
            |index, ids, stop| {
                self.semantic_sites(
                    index,
                    ids,
                    (negative, posts[index]),
                    classifier_drt,
                    &mut classifier,
                    stop,
                )
            },
        )? {
            return Ok(None);
        }
        log::info!(
            seconds = started.elapsed().as_secs_f64(),
            load_seconds,
            warmth = proposal.roll.wb.warmth,
            tint = proposal.roll.wb.tint;
            "analysis_semantic_wb_completed"
        );
        self.propose(proposal, cancelled)
    }

    fn semantic_sites(
        &self,
        index: usize,
        ids: &[usize],
        (negative, post): (crate::processing::NegativeInput, glam::DMat3),
        drt: &drt::Target,
        classifier: &mut semantic::Classifier,
        cancelled: &mut dyn FnMut() -> bool,
    ) -> Result<Option<Vec<bool>>, Error> {
        use crate::geometry::{Orientation, SourcePoint};
        if ids.len() < 256 {
            return Ok(Some(vec![true; ids.len()]));
        }
        let positions = &self.owned[index].picture.positions;
        let mut lo = [u32::MAX; 2];
        let mut hi = [0; 2];
        for &p in positions {
            for c in 0..2 {
                lo[c] = lo[c].min(p[c]);
                hi[c] = hi[c].max(p[c]);
            }
        }
        let bounds = Rect {
            x: lo[0],
            y: lo[1],
            width: hi[0] - lo[0] + 1,
            height: hi[1] - lo[1] + 1,
        };
        let project = &self.request.document.project;
        let photo = project.source(project.order[index])?;
        let sampler = CameraSampler::captured(
            &self.evaluation,
            &self.queue,
            self.cameras[index].pixels.clone(),
            self.correction.as_ref().map(|c| c.gpu.clone()),
        )?;
        let Some(preview) = sampler.preview_centres(bounds, &mut || cancelled())? else {
            return Ok(None);
        };
        if cancelled() {
            return Ok(None);
        }
        let size = preview.size.map(|n| n as u32);
        let endpoint =
            camera_response::classifier_image(size, &preview.camera, negative, post, drt)?;
        let Some(rgb) = read_classifier_rgb(&self.evaluation, &self.queue, endpoint, cancelled)?
        else {
            return Ok(None);
        };
        let [width, height] = size;
        let orientation = Orientation::new(
            Rect {
                x: 0,
                y: 0,
                width,
                height,
            },
            project.orientation,
            photo.orientation,
        )?;
        let [w, h] = orientation.size();
        let mut image = image::RgbImage::new(w, h);
        for (i, rgb) in rgb.into_iter().enumerate() {
            let point = SourcePoint([
                (i as u32 % width) as f64 + 0.5,
                (i as u32 / width) as f64 + 0.5,
            ]);
            let [x, y] = orientation.source_to_oriented(point).0;
            image.put_pixel(x as u32, y as u32, image::Rgb(rgb));
        }
        let sites: Vec<_> = ids
            .iter()
            .map(|&i| {
                let p = positions[i];
                let raw = std::array::from_fn(|c| {
                    let extent = preview.hi[c].saturating_sub(preview.lo[c]).max(1);
                    0.5 + (p[c] as f64 - preview.lo[c] as f64) / f64::from(extent)
                        * (preview.size[c] - 1) as f64
                });
                let [x, y] = orientation.source_to_oriented(SourcePoint(raw)).0;
                [x / f64::from(w), y / f64::from(h)]
            })
            .collect();
        classifier.keep(&image, &sites, cancelled)
    }

    fn propose(
        &self,
        proposal: roll_analysis::Proposal,
        cancelled: impl Fn() -> bool,
    ) -> Result<Option<Outcome>, Error> {
        let request = &self.request;
        let project = &request.document.project;
        if proposal.frames.len() != project.order.len() {
            return Err(Error::UnusableSample("analysis statistics count changed"));
        }
        if proposal
            .frames
            .iter()
            .any(|frame| frame.exposure_samples < MIN_PICTURE_SAMPLES)
        {
            return Err(Error::UnusableSample("too few positive exposure samples"));
        }
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
                .extend(approved.records());
        }
        analysis::verify(request)?;
        for (index, id) in project.order.iter().enumerate() {
            if cancelled() {
                return Ok(None);
            }
            let record = candidate.project.source(*id)?;
            crate::photographic::graph::admission::prepare(
                &self.evaluation.device().limits(),
                candidate.project.input,
                candidate.project.roll,
                &record.grade_snapshot()?,
                self.cameras[index].camera_to_ap1,
                record.development.active_area.into(),
            )?;
            if let Some(correction) = &self.correction {
                correction.gpu.validate_matrix(
                    self.cameras[index].camera_to_ap1,
                    crate::processing::MAX_SOURCE_GAIN,
                )?;
                crate::acquisition::camera::SourceGeometry::new(
                    record.development.active_area,
                    [1, 1],
                )?;
            }
        }
        if cancelled() {
            return Ok(None);
        }
        Ok(Some(Outcome {
            document: candidate,
            frames: self.owned.len(),
            wb_rejections,
            support: self.film.expect("film geometry"),
        }))
    }
}

// Model input encoding is downstream of the graph's target-linear sRGB endpoint.
fn read_classifier_rgb(
    context: &evaluation::Context,
    queue: &crate::gpu::submission::Queue,
    image: evaluation::Value,
    cancelled: &mut dyn FnMut() -> bool,
) -> Result<Option<Vec<[u8; 3]>>, Error> {
    let domain = image.domain();
    let bounds = domain.finite()?;
    let width = bounds.width;
    let request = context.request(
        image,
        evaluation::Region {
            bounds,
            grid: domain.grid,
        },
        evaluation::Tiling { tile: [width, 64] },
    )?;
    let mut output = vec![[0; 3]; bounds.width as usize * bounds.height as usize];
    let stop = std::cell::RefCell::new(cancelled);
    let read = context.read(
        queue,
        &request,
        || (stop.borrow_mut())(),
        |block, pixels| {
            let block = block.source()?;
            for (i, p) in pixels.iter().enumerate() {
                if p[3] < 0. || !p.iter().all(|v| v.is_finite()) {
                    return Err(Error::NumericalOverflow);
                }
                let x = block.x as usize + i % block.width as usize;
                let y = block.y as usize + i / block.width as usize;
                for (c, &value) in p[..3].iter().enumerate() {
                    output[y * width as usize + x][c] = (255.
                        * OutputTarget::Srgb.encode(f64::from(value.clamp(0., 1.)))?)
                    .round() as u8;
                }
            }
            Ok(())
        },
    )?;
    Ok(read.map(|_| output))
}

impl Fit {
    pub(super) async fn run(
        self,
        classifier_drt: drt::Target,
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
            let input = state.cameras[index].pixels.clone();
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
        Ok(background::compute(&cancelled, move || {
            state.finish(&classifier_drt, || retired.is_cancelled())
        })
        .await?
        .flatten())
    }
}
