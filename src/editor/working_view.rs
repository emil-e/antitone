//! Selected image demand, latest-request coalescing, and coherent presentation.
use super::*;
use crate::evaluation;
use crate::frame_raster::FrameRaster;
use crate::photographic::graph::view_source::SourceRequest;
use crate::photographic::stages::{display_rendering as drt, look::Looks};
#[cfg(test)]
use crate::photographic::test_inputs::AuthoredPhotograph;
use crate::viewer::view::{Navigation, RenderedView, ViewImage, ViewKernels};
use std::sync::{Mutex, OnceLock};

#[cfg(test)]
mod tests;
#[cfg(test)]
mod workflow_tests;

use crate::photographic::graph::request::PhotographRequest;

/// One captured view request and the immutable pixels needed to produce it.
pub(super) struct Demand {
    pub frame: FrameRaster,
    pub pixels: source::Context,
    pub capture: PhotographRequest,
    pub target: OutputTarget,
    rendering: OnceLock<Result<Arc<crate::viewer::view::rendering::Request>, Error>>,
}
impl Clone for Demand {
    fn clone(&self) -> Self {
        Self {
            frame: self.frame,
            pixels: self.pixels.clone(),
            capture: self.capture.clone(),
            target: self.target,
            rendering: OnceLock::new(),
        }
    }
}

impl Demand {
    fn rendering(
        &self,
        context: evaluation::Context,
        looks: &Looks,
        transform: &drt::Target,
        kernels: Arc<ViewKernels>,
        previous: Option<&SourceRequest>,
    ) -> Result<Arc<crate::viewer::view::rendering::Request>, Error> {
        self.rendering
            .get_or_init(|| {
                crate::viewer::view::rendering::Request::photograph(
                    context,
                    kernels,
                    &self.pixels.source,
                    self.pixels
                        .reference
                        .as_ref()
                        .map(|reference| reference.ev()),
                    self.pixels.correction.gpu()?,
                    crate::viewer::view::rendering::Intent {
                        capture: &self.capture,
                        frame: self.frame,
                        looks,
                        transform,
                    },
                    previous,
                )
                .map(Arc::new)
            })
            .clone()
    }

    #[cfg(test)]
    pub(super) fn processing(&self, limits: &wgpu::Limits) -> Result<AuthoredPhotograph, Error> {
        self.capture.processing(limits)
    }

    pub(super) fn same_calculation(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
            || self.pixels == other.pixels
                && self.frame == other.frame
                && self.capture == other.capture
                && self.target == other.target
    }

    #[cfg(test)]
    pub(super) fn fixture(spec: SourceRequest, pixels: source::Context) -> Self {
        let processing = spec.processing_fixture();
        Self {
            frame: spec.frame(),
            pixels,
            capture: PhotographRequest::Fixture(processing),
            target: OutputTarget::Srgb,
            rendering: OnceLock::new(),
        }
    }

    pub(super) fn same_source(&self, other: &Self) -> bool {
        self.pixels.source.identity() == other.pixels.source.identity()
            && self.capture.is_inspection() == other.capture.is_inspection()
    }
}

/// A captured request's detail and navigation image products.
pub(super) struct Frame {
    sequence: u64,
    pub source: Arc<ViewImage>,
    pub underlay: Option<Arc<ViewImage>>,
    pub request: Arc<Demand>,
    // Support refresh can reauthor from an already warm frame without joining GPU startup.
    looks: Arc<Looks>,
    targets: Arc<drt::Targets>,
}
impl Frame {
    #[cfg(test)]
    pub(super) fn images(&self) -> impl Iterator<Item = &Arc<ViewImage>> {
        std::iter::once(&self.source).chain(self.underlay.iter())
    }

    fn from_images(
        request: Arc<Demand>,
        sequence: u64,
        images: Vec<Arc<ViewImage>>,
        looks: Arc<Looks>,
        targets: Arc<drt::Targets>,
    ) -> Self {
        let mut images = images.into_iter();
        let source = images.next().expect("detail image product");
        let underlay = images.next();
        Self {
            sequence,
            request,
            source,
            underlay,
            looks,
            targets,
        }
    }

    pub(super) fn satisfies(&self, wanted: &Demand) -> bool {
        self.request.same_calculation(wanted) && self.source.spec.frame() == wanted.frame
    }

    #[cfg(test)]
    pub(super) fn processing(&self, limits: &wgpu::Limits) -> Result<AuthoredPhotograph, Error> {
        self.request.processing(limits)
    }
}

#[derive(Default)]
struct MailboxState {
    sequence: u64,
    demand: Option<Arc<Demand>>,
    completed: Option<Arc<Frame>>,
    #[cfg(test)]
    underlay_gate: Option<futures_channel::oneshot::Receiver<()>>,
}
#[derive(Default)]
pub(super) struct Mailbox(Mutex<MailboxState>, pub event_listener::Event);

pub(super) struct Completion {
    owner: std::sync::Weak<Mailbox>,
    demand: Arc<Demand>,
    result: Result<Arc<Frame>, Error>,
}

#[cfg(test)]
impl Completion {
    fn fixture_demand(working: &WorkingView, spec: SourceRequest) -> Arc<Demand> {
        let mut demand = (**working
            .mailbox
            .0
            .lock()
            .unwrap()
            .demand
            .as_ref()
            .expect("fixture has a captured request"))
        .clone();
        demand.frame = spec.frame();
        demand.capture = PhotographRequest::Fixture(spec.processing_fixture());
        Arc::new(demand)
    }
}

pub(super) struct Acquisition {
    pub mailbox: Arc<Mailbox>,
    pub kernels: Arc<OnceLock<Arc<ViewKernels>>>,
}

#[derive(Default)]
pub(super) struct WorkingView {
    pub(super) task: Option<async_executor::Task<()>>,
    gpu: Option<(wgpu::Device, crate::gpu::submission::Queue)>,
    intent: Option<(FrameRaster, PhotographRequest, OutputTarget)>,
    revision: u64,
    #[cfg(test)]
    changed_at: Option<std::time::Instant>,
    mailbox: Arc<Mailbox>,
    pub(super) kernels: Arc<OnceLock<Arc<ViewKernels>>>,
    frame: Option<Arc<Frame>>,
    underlay_rendered: Option<Arc<RenderedView>>,
    wanted: Option<Arc<Demand>>,
    failed: Option<Arc<Demand>>,
    rendered: Option<Arc<RenderedView>>,
    navigation: Navigation,
    pub displayed: Option<Arc<RenderedView>>,
    pub updating: bool,
    pub error: Option<Error>,
    #[cfg(all(test, feature = "ui-tests"))]
    pub(super) fail_next: Option<Error>,
}

impl WorkingView {
    pub(super) fn new(device: wgpu::Device, queue: crate::gpu::submission::Queue) -> Self {
        let mut state = Self::default();
        state.gpu = Some((device, queue));
        state
    }

    pub(super) fn for_visit(&self) -> Self {
        let mut next = Self::default();
        next.gpu = self.gpu.clone();
        next.kernels = self.kernels.clone();
        next.revision = self.revision;
        next
    }

    pub(super) fn observe(
        &mut self,
        frame: FrameRaster,
        capture: PhotographRequest,
        target: OutputTarget,
    ) -> bool {
        let intent = (frame, capture, target);
        if self.intent.as_ref() == Some(&intent) {
            return false;
        }

        self.intent = Some(intent);
        self.revision += 1;
        #[cfg(test)]
        {
            self.changed_at = Some(std::time::Instant::now());
        }
        true
    }

    pub(super) fn revision(&self) -> Option<u64> {
        self.intent.as_ref().map(|_| self.revision)
    }

    #[cfg(all(test, feature = "ui-tests"))]
    pub(super) fn processing(&self) -> Option<AuthoredPhotograph> {
        self.intent.as_ref().map(|intent| {
            intent
                .1
                .processing(&self.device().map(|d| d.limits()).unwrap_or_default())
                .expect("captured view input was admitted")
        })
    }

    #[cfg(test)]
    pub(super) fn idle(&self) -> bool {
        !self.updating
            && !self.colour_in_flight()
            && self
                .changed_at
                .is_some_and(|changed_at| changed_at.elapsed() >= Duration::from_millis(250))
            && self
                .intent
                .as_ref()
                .is_some_and(|(frame, capture, target)| {
                    self.displayed.as_ref().is_some_and(|rendered| {
                        rendered.spec.frame() == *frame
                            && rendered.matches_capture(&rendered.spec, capture, *target)
                            && rendered.target == *target
                            && !rendered.in_flight()
                    })
                })
    }

    pub(super) fn enabled(&self) -> bool {
        self.gpu.is_some()
    }

    pub(super) fn device(&self) -> Option<&wgpu::Device> {
        self.gpu.as_ref().map(|(device, _)| device)
    }

    pub(super) fn queue(&self) -> Option<&crate::gpu::submission::Queue> {
        self.gpu.as_ref().map(|(_, queue)| queue)
    }

    pub(super) fn hold(&mut self, target: OutputTarget) -> Option<Arc<RenderedView>> {
        self.displayed = self.wanted.clone().and_then(|wanted| {
            self.present(wanted.frame)
                .ok()
                .flatten()
                .or_else(|| self.rendered.clone())
                .filter(|rendered| {
                    rendered.spec.identity
                        == crate::acquisition::camera::identity::Identity::camera(
                            wanted.pixels.source.identity(),
                        )
                        && rendered.target == target
                })
        });
        self.updating = true;
        self.displayed.clone()
    }

    pub(super) fn clear(&mut self) {
        self.intent = None;
        #[cfg(test)]
        {
            self.changed_at = None;
        }
        self.retire_support();
    }

    fn retire_support(&mut self) {
        self.task = None;
        let mut mailbox = self.mailbox.0.lock().unwrap();
        let retired = mailbox.demand.take();
        mailbox.completed = None;

        self.frame = None;
        self.underlay_rendered = None;
        self.wanted = None;
        self.failed = None;
        self.rendered = None;
        self.navigation = Navigation::default();
        self.displayed = None;
        self.updating = false;
        self.error = None;
        drop(mailbox);
        drop(retired);
        self.mailbox.1.notify(usize::MAX);
    }

    #[cfg(all(test, feature = "ui-tests"))]
    pub(super) fn regional(&self) -> Option<&Arc<RenderedView>> {
        self.rendered.as_ref()
    }

    #[cfg(test)]
    pub(super) fn completed_capture(&self) -> Option<&PhotographRequest> {
        self.frame
            .as_ref()
            .filter(|_| self.support_ready())
            .map(|frame| &frame.request.capture)
    }

    #[cfg(test)]
    pub(super) fn support_ready(&self) -> bool {
        self.frame
            .as_ref()
            .zip(self.wanted.as_ref())
            .is_some_and(|(frame, wanted)| {
                frame.satisfies(wanted) && (frame.underlay.is_some() || self.task.is_none())
            })
    }

    #[cfg(all(test, feature = "ui-tests"))]
    pub(super) fn current_frame(&self) -> Option<FrameRaster> {
        self.wanted.as_ref().map(|wanted| wanted.frame)
    }

    pub(super) fn request(&mut self, demand: Demand) {
        if self
            .wanted
            .as_ref()
            .is_some_and(|wanted| wanted.capture.is_inspection() != demand.capture.is_inspection())
        {
            self.retire_support();
        }
        let mut state = self.mailbox.0.lock().unwrap();
        if state
            .demand
            .as_ref()
            .is_some_and(|current| current.same_calculation(&demand))
        {
            return;
        }
        self.failed = None;
        self.error = None;
        if state
            .demand
            .as_ref()
            .is_some_and(|old| !old.same_source(&demand))
        {
            state.completed = None;
            self.frame = None;
        }
        let demand = Arc::new(demand);
        log::debug!(
            width = demand.frame.output[0],
            height = demand.frame.output[1];
            "working_view_requested"
        );
        self.wanted = Some(demand.clone());
        state.sequence += 1;
        state.demand = Some(demand);
        drop(state);
        self.refresh_support();
        self.mailbox.1.notify(usize::MAX);
        #[cfg(all(test, feature = "ui-tests"))]
        if let Some(error) = self.fail_next.take() {
            self.acquisition_failed(error);
        }
    }

    pub(super) fn presentation_failed(&mut self, error: Error) {
        if self.error.as_ref() != Some(&error) {
            self.log_failure("presentation", "current", None, &error);
            if matches!(error, Error::GpuOutOfMemory(_))
                && let Some(frame) = &self.frame
            {
                frame.source.context.materializations().evict_optional();
            }
        }
        self.failed = self.wanted.clone();
        self.error = Some(error);
    }

    pub(super) fn acquisition(&self) -> Option<Acquisition> {
        let state = self.mailbox.0.lock().unwrap();
        let wanted = state.demand.as_ref()?;
        let missing_detail = self
            .frame
            .as_ref()
            .is_none_or(|frame| !frame.satisfies(wanted));
        let pending_underlay = wanted.rendering.get().is_some_and(|rendering| {
            rendering
                .as_ref()
                .is_ok_and(|rendering| rendering.underlay_pending())
        });
        ((missing_detail || pending_underlay)
            && self
                .failed
                .as_ref()
                .is_none_or(|failed| !failed.same_calculation(wanted)))
        .then(|| Acquisition {
            mailbox: self.mailbox.clone(),
            kernels: self.kernels.clone(),
        })
    }

    pub(super) fn owns(&self, owner: &std::sync::Weak<Mailbox>) -> bool {
        owner.ptr_eq(&Arc::downgrade(&self.mailbox))
    }

    pub(super) fn acquired(&mut self, completion: Completion) {
        let Completion {
            owner,
            demand,
            result,
        } = completion;
        if !self.owns(&owner) {
            if let Err(error) = &result {
                self.log_failure("acquisition", "retired_owner", Some(&demand), error);
            }
            return;
        }
        if self
            .mailbox
            .0
            .lock()
            .unwrap()
            .demand
            .as_ref()
            .is_none_or(|wanted| !wanted.same_calculation(&demand))
        {
            if let Err(error) = &result {
                self.log_failure("acquisition", "superseded_request", Some(&demand), error);
            }
            return;
        }
        log::debug!(succeeded = result.is_ok(); "working_view_acquired");
        match result {
            Ok(frame) if frame.satisfies(&demand) => {
                self.frame = Some(frame);
                self.failed = None;
                self.error = None;
            }
            Ok(_) => {
                self.log_failure(
                    "publication",
                    "current",
                    Some(&demand),
                    "acquired view does not satisfy requested source/layout",
                );
                self.failed = Some(demand);
                self.error = Some(Error::InvalidParameter(
                    "acquired view does not satisfy requested source/layout",
                ));
            }
            Err(error) => {
                self.log_failure("acquisition", "current", Some(&demand), &error);
                self.failed = Some(demand);
                self.error = Some(error);
            }
        }
        self.refresh_support();
    }

    pub(super) fn log_failure(
        &self,
        phase: &str,
        disposition: &str,
        request: Option<&Demand>,
        error: impl std::fmt::Display,
    ) {
        log::warn!(
            phase = phase,
            disposition = disposition,
            error = error.to_string(),
            request_frame:? = request.map(|demand| demand.frame),
            wanted_frame:? = self.wanted.as_ref().map(|demand| demand.frame),
            inspection:? = request.map(|demand| demand.capture.is_inspection()),
            pressure:? = crate::sys::memory_pressure();
            "working_view_failed"
        );
    }

    pub(super) fn acquisition_failed(&mut self, error: Error) {
        self.failed = self.mailbox.0.lock().unwrap().demand.clone();
        self.log_failure("preparation", "current", self.failed.as_deref(), &error);
        self.error = Some(error);
    }

    pub(super) fn refresh_support(&mut self) {
        let (frame, wanted, sequence) = {
            let state = self.mailbox.0.lock().unwrap();
            let (Some(frame), Some(wanted)) = (&state.completed, &state.demand) else {
                return;
            };
            let frame = self
                .frame
                .as_ref()
                .filter(|current| current.sequence > frame.sequence)
                .unwrap_or(frame);
            (frame.clone(), wanted.clone(), state.sequence)
        };
        if wanted.pixels.correction.pending() {
            return;
        }
        let rendering = wanted.rendering(
            frame.source.context.clone(),
            &frame.looks,
            frame.targets.get(wanted.target),
            frame.source.kernels().clone(),
            Some(&frame.source.spec),
        );
        match rendering {
            Ok(rendering) => {
                self.frame = Some(match rendering.ready() {
                    Some(images) => Arc::new(Frame::from_images(
                        wanted,
                        sequence,
                        images,
                        frame.looks.clone(),
                        frame.targets.clone(),
                    )),
                    None => frame,
                });
            }
            Err(error) => {
                self.log_failure("rendering", "current", Some(&wanted), &error);
                self.failed = Some(wanted);
                self.error = Some(error);
            }
        }
    }

    #[cfg(test)]
    fn source(&self) -> Option<&Arc<ViewImage>> {
        self.frame.as_ref().map(|frame| &frame.source)
    }
    #[cfg(test)]
    fn underlay(&self) -> Option<&Arc<ViewImage>> {
        self.frame
            .as_ref()
            .and_then(|frame| frame.underlay.as_ref())
    }

    fn render(&mut self, context: &egui::Context) -> Result<(), Error> {
        if self.colour_in_flight() {
            self.device()
                .unwrap()
                .poll(wgpu::PollType::Poll)
                .map_err(|error| Error::Gpu(error.to_string()))?;
            if self.colour_in_flight() {
                context.request_repaint_after(Duration::from_millis(8));
            }
        }
        let wanted = self
            .wanted
            .as_ref()
            .expect("requested physical view")
            .clone();
        if self
            .failed
            .as_ref()
            .is_some_and(|failed| failed.same_calculation(&wanted))
            && let Some(error) = &self.error
        {
            return Err(error.clone());
        }
        let Some(frame) = self.frame.clone() else {
            self.updating = true;
            return Ok(());
        };
        let capture = &frame.request.capture;
        let target = frame.request.target;
        let matches = |rendered: &RenderedView, index: usize| {
            let source = if index == 0 {
                &frame.source
            } else {
                frame.underlay.as_ref().unwrap()
            };
            // Reusing calculated pixels does not refresh the rendered value's source metadata.
            rendered.spec == source.spec
                && rendered.capture == *capture
                && rendered.target == target
                && rendered.uses_image(source)
        };
        let changed = self
            .rendered
            .as_ref()
            .is_none_or(|rendered| !matches(rendered, 0))
            || frame.underlay.is_some()
                && self
                    .underlay_rendered
                    .as_ref()
                    .is_none_or(|rendered| !matches(rendered, 1));
        if changed {
            if self.colour_in_flight() {
                context.request_repaint_after(Duration::from_millis(8));
                self.updating = true;
                return Ok(());
            }
            let render_source =
                |source: Arc<ViewImage>, index: usize, previous: Option<&Arc<RenderedView>>| {
                    if let Some(rendered) = previous.filter(|rendered| matches(rendered, index)) {
                        return Ok(rendered.clone());
                    }
                    RenderedView::from_image(
                        self.device().unwrap(),
                        source,
                        capture.clone(),
                        target,
                    )
                };
            let detail = render_source(frame.source.clone(), 0, self.rendered.as_ref())?;
            let underlay = frame
                .underlay
                .as_ref()
                .map(|underlay| {
                    if Arc::ptr_eq(underlay, &frame.source) {
                        Ok(detail.clone())
                    } else {
                        render_source(underlay.clone(), 1, self.underlay_rendered.as_ref())
                    }
                })
                .transpose()?;
            self.rendered = Some(detail);
            self.underlay_rendered = underlay;
        }
        self.updating = !frame.satisfies(&wanted);
        if self.failed.is_none() {
            self.error = None;
        }
        Ok(())
    }

    fn colour_in_flight(&self) -> bool {
        [&self.rendered, &self.underlay_rendered]
            .into_iter()
            .flatten()
            .any(|rendered| rendered.in_flight())
    }

    fn present(&mut self, frame: FrameRaster) -> Result<Option<Arc<RenderedView>>, Error> {
        let is_current_source = |rendered: &RenderedView| {
            self.wanted.as_ref().is_some_and(|wanted| {
                crate::acquisition::camera::identity::Identity::camera(
                    wanted.pixels.source.identity(),
                ) == rendered.spec.identity
                    && wanted.capture.is_inspection() == rendered.capture.is_inspection()
            })
        };
        let Some(detail) = self
            .rendered
            .as_ref()
            .filter(|rendered| is_current_source(rendered))
        else {
            return Ok(None);
        };

        self.navigation
            .present(
                &self.gpu.as_ref().unwrap().0,
                detail,
                self.underlay_rendered.as_ref(),
                frame,
            )
            .map(Some)
    }
}

impl Drop for WorkingView {
    fn drop(&mut self) {
        let mut mailbox = self.mailbox.0.lock().unwrap();
        let retired = mailbox.demand.take();
        mailbox.completed = None;

        drop(mailbox);
        drop(retired);
        self.mailbox.1.notify(usize::MAX);
    }
}

async fn packed_view_input(
    demand: &Demand,
    context: &evaluation::Context,
    queue: &crate::gpu::submission::Queue,
) -> Result<Option<evaluation::Image<evaluation::ImageLayout>>, Error> {
    use evaluation::{ImageBacking, ImageFormat, Region, Tiling};

    let developed = &demand.pixels.source;
    let visible = demand.frame.bounds()?;
    let cells =
        |area: crate::geometry::SourceRegion| u64::from(area.width) * u64::from(area.height);
    let source_cells = cells(developed.area());
    // Only broad, substantial views benefit from paying for a full original upload.
    let bytes = source_cells * ImageFormat::Rgba16Uint.pixel_bytes();
    if !(32 << 20..=512 << 20).contains(&bytes) || cells(visible) * 2 < source_cells {
        return Ok(None);
    }
    let source = crate::acquisition::camera::camera_source(developed)?;
    if source
        .source()
        .is_none_or(|source| source.storage_format() != ImageFormat::Rgba16Uint)
    {
        return Ok(None);
    }
    let domain = source.domain();
    let evaluation = context
        .request(
            source,
            Region {
                bounds: domain.finite()?,
                grid: domain.grid,
            },
            Tiling { tile: [512, 128] },
        )?
        .without_retention();
    context
        .materialize(
            queue,
            &context.materializations(),
            evaluation,
            ImageFormat::Rgba16Uint,
            ImageBacking::Texture,
        )?
        .await
        .map(Some)
        .map_err(|error| (*error).clone())
}

// Physical acquisition follows the latest captured demand while retaining one admission.
pub(super) async fn acquire_images(
    mailbox: &Arc<Mailbox>,
    context: &evaluation::Context,
    looks: &Arc<Looks>,
    targets: &Arc<drt::Targets>,
    kernels: &Arc<ViewKernels>,
    queue: &crate::gpu::submission::Queue,
) -> Option<Completion> {
    loop {
        let (demand, sequence) = {
            let state = mailbox.0.lock().unwrap();
            (state.demand.clone()?, state.sequence)
        };
        let frame = demand.frame;
        let started = std::time::Instant::now();
        let trace = |phase: &str| {
            log::debug!(
                phase = phase,
                seconds = started.elapsed().as_secs_f64(),
                width = frame.output[0],
                height = frame.output[1],
                retained_bytes = context.retained_bytes(),
                cache_limit = context.cache_policy().generated_bytes;
                "working_view_phase"
            );
        };
        trace("acquisition_started");
        let cancelled = || {
            mailbox
                .0
                .lock()
                .unwrap()
                .demand
                .as_ref()
                .is_none_or(|latest| !demand.same_source(latest) || latest.frame != frame)
        };
        if cancelled() {
            return None;
        }
        let changed = mailbox.1.listen();
        if cancelled() {
            return None;
        }
        let correction = futures_lite::future::or(
            async { Some(demand.pixels.correction.clone().acquire().await) },
            async {
                changed.await;
                None
            },
        )
        .await;
        let Some(correction) = correction else {
            continue;
        };
        let result = async {
            correction.map_err(Error::Gpu)?;
            let previous = mailbox
                .0
                .lock()
                .unwrap()
                .completed
                .as_ref()
                .map(|frame| frame.source.spec.clone());
            let rendering = demand.rendering(
                context.clone(),
                looks,
                targets.get(demand.target),
                kernels.clone(),
                previous.as_ref(),
            )?;
            trace("graph_described");
            // Keep the uploaded source live through both consumers without retaining another
            // full-frame image between visits.
            let mut packed_input = None;
            if previous.is_none() && rendering.detail_pending() {
                let changed = mailbox.1.listen();
                if cancelled() {
                    return Ok(None);
                }
                let uploaded = futures_lite::future::or(
                    async { Some(packed_view_input(&demand, context, queue).await) },
                    async { changed.await; None },
                ).await;
                match uploaded {
                    Some(Ok(Some(input))) => {
                        packed_input = Some(input);
                        rendering.replan_pending();
                        trace("packed_input_ready");
                    }
                    Some(Err(error)) => log::debug!(error = error.to_string().as_str(); "working_packed_input_unavailable"),
                    _ => {}
                }
                if cancelled() {
                    return Ok(None);
                }
            }
            let Some(images) = rendering.produce(queue, &cancelled).await? else {
                return Ok(None);
            };
            let make_frame = |images| {
                Arc::new(Frame::from_images(
                    demand.clone(),
                    sequence,
                    images,
                    looks.clone(),
                    targets.clone(),
                ))
            };
            trace("detail_ready");
            if !rendering.underlay_pending() {
                trace("processed_ready");
                return Ok(Some(make_frame(images)));
            }

            let detail = make_frame(images);
            {
                let mut state = mailbox.0.lock().unwrap();
                if state
                    .demand
                    .as_ref()
                    .is_some_and(|latest| demand.same_source(latest) && latest.frame == frame)
                    && state
                        .completed
                        .as_ref()
                        .is_none_or(|current| current.sequence <= sequence)
                {
                    state.completed = Some(detail.clone());
                }
            }
            mailbox.1.notify(usize::MAX);
            #[cfg(test)]
            {
                let gate = mailbox.0.lock().unwrap().underlay_gate.take();
                if let Some(gate) = gate {
                    let _ = gate.await;
                }
            }

            // Once detail can draw, obsolete navigation work should not delay a new capture.
            let obsolete = || {
                mailbox
                    .0
                    .lock()
                    .unwrap()
                    .demand
                    .as_ref()
                    .is_none_or(|latest| !demand.same_calculation(latest))
            };
            if !obsolete() {
                if packed_input.is_some() {
                    rendering.replan_underlay();
                }
                if let Err(error) = rendering.produce_underlay(queue, obsolete).await {
                    log::warn!(error = error.to_string().as_str(); "working_underlay_unavailable");
                }
            }
            drop(packed_input);
            trace("processed_ready");
            Ok(Some(rendering.ready().map_or(detail, make_frame)))
        }
        .await;
        trace("acquisition_finished");
        if cancelled() {
            return None;
        }
        if matches!(&result, Err(Error::GpuOutOfMemory(_))) {
            context.materializations().evict_optional();
        }
        let mut state = mailbox.0.lock().unwrap();
        if let Ok(Some(frame)) = &result
            && state
                .completed
                .as_ref()
                .is_none_or(|current| current.sequence <= sequence)
        {
            state.completed = Some(frame.clone());
        }
        let latest = state.demand.as_ref()?;
        if latest.frame == frame {
            drop(state);
            return match result {
                Ok(Some(frame)) => Some(Completion {
                    owner: Arc::downgrade(mailbox),
                    demand: demand.clone(),
                    result: Ok(frame),
                }),
                Ok(None) => None,
                Err(error) => Some(Completion {
                    owner: Arc::downgrade(mailbox),
                    demand: demand.clone(),
                    result: Err(error),
                }),
            };
        }
        drop(state);
        mailbox.1.notify(usize::MAX);
        // Same-layout captures can publish useful progress; obsolete sampling layouts retire.
    }
}

impl Editor {
    pub(super) fn preview_failure_status(&self) -> Option<(ui::ContextualStatus, &Error)> {
        let working = &self.document.visit.working_view;
        let error = working.error.as_ref().or_else(|| {
            // An optional fallback failure does not fail the primary view still being produced.
            if working.updating {
                None
            } else {
                self.active_canvas().preview.error()
            }
        })?;
        let preview = if self.canvas_frame.displayed.is_some() {
            "Preview not updated — showing previous image"
        } else {
            "Preview unavailable"
        };
        let reason = match error {
            Error::GpuOutOfMemory(_) => "not enough graphics memory",
            _ => "calculation failed",
        };
        Some((
            ui::ContextualStatus::new(ui::StatusLevel::Error, format!("{preview}: {reason}")),
            error,
        ))
    }

    pub(super) fn working_view(
        &mut self,
        frame: FrameRaster,
        capture: &PhotographRequest,
        context: &egui::Context,
        cx: &UiContext<'_, Self>,
    ) -> Result<Option<Arc<RenderedView>>, Error> {
        let target = self.document.target();
        if !self.document.visit.working_view.enabled() {
            return Ok(None);
        }
        let Some(source_input) = self.source_context() else {
            self.document.visit.working_view.clear();
            return Ok(None);
        };
        self.document
            .visit
            .working_view
            .observe(frame, capture.clone(), target);

        self.request_working(
            Demand {
                frame,
                pixels: source_input,
                capture: capture.clone(),
                target,
                rendering: OnceLock::new(),
            },
            cx,
        );
        let working = &mut self.document.visit.working_view;
        working.render(context)?;
        working.displayed = working
            .present(frame)?
            .filter(|rendered| rendered.target == target);
        working.updating |= working.displayed.as_ref().is_none_or(|rendered| {
            working.wanted.as_ref().is_none_or(|wanted| {
                !(rendered.spec.frame() == wanted.frame
                    && rendered.matches_capture(&rendered.spec, capture, target))
            })
        });
        if working.displayed.is_none() && working.error.is_none() {
            context.request_repaint_after(Duration::from_millis(16));
        }

        Ok(working.displayed.clone())
    }
}

impl Acquisition {
    pub async fn prepare(self, services: source::Services) -> Result<Option<Completion>, Error> {
        let kernels = self.kernels.clone();
        let prepared = crate::background::run(move |stop| {
            if stop.is_cancelled() {
                return Ok(None);
            }
            let targets = services.display.ready()?;
            let pipelines = if let Some(pipelines) = kernels.get() {
                pipelines.clone()
            } else {
                let prepared =
                    pollster::block_on(crate::viewer::view::ViewKernels::new(&services.device))?;
                kernels.get_or_init(|| prepared).clone()
            };
            Ok::<_, crate::Error>(Some((services, targets, pipelines)))
        })
        .await
        .map_err(|_| Error::Gpu("Working-source preparation worker panicked".into()))??;
        let Some((services, targets, kernels)) = prepared else {
            return Ok(None);
        };
        Ok(acquire_images(
            &self.mailbox,
            &services.evaluation,
            &services.looks,
            &targets,
            &kernels,
            &services.queue,
        )
        .await)
    }
}

impl Editor {
    pub(super) fn request_working(
        &mut self,
        demand: Demand,
        cx: &crate::application::UiContext<'_, Self>,
    ) {
        self.document.visit.working_view.request(demand);
        self.acquire_working(cx);
    }

    pub(super) fn acquire_working(&mut self, cx: &crate::application::UiContext<'_, Self>) {
        if self.allow_close {
            return;
        }
        if self.document.visit.working_view.wanted.is_none() {
            return;
        }
        let Some(services) = self.source_services.clone() else {
            self.document
                .visit
                .working_view
                .acquisition_failed(Error::Gpu("Working-source services unavailable".into()));
            return;
        };
        let Some(request) = self.document.visit.working_view.acquisition() else {
            return;
        };
        if self.document.visit.working_view.task.is_some() {
            return;
        }
        let owner = Arc::downgrade(&request.mailbox);
        let mailbox = request.mailbox.clone();
        let Some(task) = cx.spawn(move |cx| async move {
            let acquisition = request.prepare(services);
            let support = async {
                loop {
                    let changed = mailbox.1.listen();
                    cx.update(|editor| {
                        if editor.document.visit.working_view.owns(&owner) {
                            editor.document.visit.working_view.refresh_support();
                        }
                    });
                    changed.await;
                }
            };
            let result = futures_lite::future::or(acquisition, support).await;
            cx.update(|editor| {
                if !editor.document.visit.working_view.owns(&owner) {
                    return;
                }
                editor.document.visit.working_view.task = None;
                match result {
                    Ok(Some(completion)) => editor.document.visit.working_view.acquired(completion),
                    Ok(None) => {}
                    Err(error) => editor.document.visit.working_view.acquisition_failed(error),
                }
                editor.acquire_working(&cx);
            });
        }) else {
            self.document
                .visit
                .working_view
                .acquisition_failed(Error::Gpu("Application stopped".into()));
            return;
        };
        self.document.visit.working_view.task = Some(task);
    }
}
