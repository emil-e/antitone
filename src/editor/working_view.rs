//! Selected-view ownership: one neutral acquisition, latest-request coalescing, immutable frames.
use super::*;
use crate::frame_raster::FrameRaster;
#[cfg(test)]
use crate::gpu::test_inputs::AuthoredPhotograph;
use crate::gpu::view::RenderedView;
use crate::gpu::view::ViewKernels;
use crate::gpu::view::ViewSource;
use crate::gpu::view::request::SourceRequest;
use std::sync::{Mutex, OnceLock};

#[cfg(test)]
mod tests;
#[cfg(test)]
mod workflow_tests;

use crate::gpu::photograph::request::PhotographRequest;

/// One captured view request and the immutable pixels needed to produce it.
#[derive(Clone)]
pub(super) struct Demand {
    pub spec: SourceRequest,
    pub pixels: source::Context,
    pub capture: PhotographRequest,
}

pub(super) struct Calculation {
    pub image: Arc<crate::gpu::evaluation::ImageNode>,
    pub native: Arc<crate::gpu::evaluation::ImageNode>,
    pub plan: crate::acquisition::Plan,
}

impl Demand {
    #[cfg(test)]
    pub(super) fn processing(&self, limits: &wgpu::Limits) -> Result<AuthoredPhotograph, Error> {
        self.capture.processing(limits, &self.spec.repairs)
    }

    pub(super) fn same_calculation(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
            || self.pixels == other.pixels
                && self.spec == other.spec
                && self.capture == other.capture
    }

    #[cfg(test)]
    pub(super) fn fixture(spec: SourceRequest, pixels: source::Context) -> Self {
        let processing = spec.processing_fixture();
        Self {
            spec,
            pixels,
            capture: PhotographRequest::Fixture(processing),
        }
    }

    pub(super) fn same_source(&self, other: &Self) -> bool {
        self.pixels.source.identity() == other.pixels.source.identity()
            && self.capture.is_inspection() == other.capture.is_inspection()
    }

    pub(super) fn calculation(&self, source: &ViewSource) -> Result<Calculation, Error> {
        let limits = source.renderer.evaluation().device().limits();
        let scan = self.pixels.scan()?;
        let field = self
            .pixels
            .correction
            .as_ref()
            .map(|field| field.gpu.clone());
        if let Some(recipe) = self.capture.recipe() {
            let (photo, plan) = crate::gpu::photograph::PhotographGraph::preview(
                &limits,
                &scan,
                field,
                recipe,
                &self.spec.repairs,
                source.spec.frame(),
                Some(source.spec.plan.output()),
            )?;
            return Ok(Calculation {
                image: photo.processed,
                native: photo.neutral,
                plan,
            });
        }
        let spec = source.spec.clone().with_repairs(self.spec.repairs.clone());
        let native = self
            .capture
            .source_graphs(&scan, field, std::slice::from_ref(&spec))?
            .remove(0);
        let (image, plan) = self.capture.filtered(&limits, &spec, native.clone())?;
        Ok(Calculation {
            image,
            native,
            plan,
        })
    }

    pub(super) fn can_use(&self, source: &ViewSource) -> Result<bool, Error> {
        let Calculation { image, native, .. } = self.calculation(source)?;
        let Some(region) = image.region_for(&native, image.domain().finite()?)? else {
            return Ok(false);
        };
        Ok(source.supplied()?.covers(&native, region))
    }
}

/// Related supplied values, their complete processed calculations and captured provenance.
pub(super) struct Frame {
    pub source: Arc<ViewSource>,
    pub underlay: Option<Arc<ViewSource>>,
    pub request: Arc<Demand>,
    pub processed: Vec<Arc<crate::gpu::view::processing::ProcessedView>>,
}
impl Frame {
    pub(super) fn satisfies(&self, wanted: &Demand) -> bool {
        self.request.same_calculation(wanted) && self.source.spec.frame() == wanted.spec.frame()
    }

    #[cfg(test)]
    pub(super) fn processing(&self, limits: &wgpu::Limits) -> Result<AuthoredPhotograph, Error> {
        self.request.processing(limits)
    }

    pub(super) fn regrade(&self, request: Arc<Demand>) -> Result<Option<Self>, Error> {
        let sources: Vec<_> = std::iter::once(&self.source)
            .chain(self.underlay.as_ref())
            .collect();
        let calculations = sources
            .iter()
            .map(|source| request.calculation(source))
            .collect::<Result<Vec<_>, _>>()?;
        for (source, calculation) in sources.iter().zip(&calculations) {
            let region = calculation
                .image
                .region_for(&calculation.native, calculation.image.domain().finite()?)?
                .ok_or(Error::InvalidParameter("preview has no neutral dependency"))?;
            if !source.supplied()?.covers(&calculation.native, region) {
                return Ok(None);
            }
        }
        let device = self.source.renderer.evaluation().device();
        let processed = sources
            .into_iter()
            .zip(calculations)
            .enumerate()
            .map(|(index, (source, calculation))| {
                crate::gpu::view::processing::ProcessedView::prepare_calculation(
                    device,
                    source.acquired(),
                    calculation.image,
                    calculation.plan,
                    self.processed.get(index),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Some(Self {
            source: self.source.clone(),
            underlay: self.underlay.clone(),
            request,
            processed,
        }))
    }

    pub(super) fn supports(&self, wanted: &Demand) -> bool {
        // The producing request includes resolved arguments as well as intent: unchanged
        // recipes can acquire new repair values without changing their accepted instructions.
        if self.request.same_calculation(wanted) {
            return true;
        }
        std::iter::once(&self.source)
            .chain(self.underlay.as_ref())
            .all(|source| wanted.can_use(source).unwrap_or(false))
    }
}

#[derive(Default)]
struct MailboxState {
    demand: Option<Arc<Demand>>,
    completed: Option<Arc<Frame>>,
    camera: Option<crate::gpu::evaluation::ActiveInput>,
    pressure: Option<memory::Pressure>,
    pressure_watch: Option<memory::PressureWatch>,
}
#[derive(Default)]
pub(super) struct Mailbox(Mutex<MailboxState>, pub event_listener::Event);

impl Mailbox {
    fn pressure(&self, pressure: memory::Pressure) {
        let mut state = self.0.lock().unwrap();
        if state.pressure == Some(pressure) {
            return;
        }
        state.pressure = Some(pressure);
        if pressure != memory::Pressure::Normal {
            state.camera = None;
        }
        drop(state);
        self.1.notify(usize::MAX);
    }

    fn watch(self: &Arc<Self>) {
        let mut state = self.0.lock().unwrap();
        state
            .pressure
            .get_or_insert_with(crate::sys::memory_pressure);
        if state.pressure_watch.is_none() {
            let weak = Arc::downgrade(self);
            state.pressure_watch = Some(memory::PressureWatch::observe(move |pressure| {
                if let Some(mailbox) = weak.upgrade() {
                    mailbox.pressure(pressure);
                }
            }));
        }
    }
}

pub(super) struct Completion {
    owner: std::sync::Weak<Mailbox>,
    demand: Arc<Demand>,
    result: Result<Arc<Frame>, String>,
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
        demand.spec = spec;
        Arc::new(demand)
    }
}

pub(super) struct Acquisition {
    pub mailbox: Arc<Mailbox>,
    pub kernels: Arc<OnceLock<Arc<ViewKernels>>>,
}

#[derive(Default)]
pub(super) struct WorkingView {
    gpu: Option<(wgpu::Device, wgpu::Queue)>,
    intent: Option<(FrameRaster, PhotographRequest, OutputTarget)>,
    revision: u64,
    changed_at: Option<std::time::Instant>,
    mailbox: Arc<Mailbox>,
    pub(super) kernels: Arc<OnceLock<Arc<ViewKernels>>>,
    frame: Option<Arc<Frame>>,
    underlay_rendered: Option<Arc<RenderedView>>,
    wanted: Option<Arc<Demand>>,
    failed: Option<Arc<Demand>>,
    rendered: Option<Arc<RenderedView>>,
    reference: Option<Arc<RenderedView>>,
    pub displayed: Option<Arc<RenderedView>>,
    pub compare: bool,
    pub updating: bool,
    pub error: Option<String>,
}

impl WorkingView {
    pub(super) fn new(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        let mut state = Self::default();
        state.gpu = Some((device, queue));
        state
    }

    pub(super) fn for_visit(&self) -> Self {
        let mut next = Self::default();
        next.gpu = self.gpu.clone();
        next.kernels = self.kernels.clone();
        next.compare = self.compare;
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
        self.changed_at = Some(std::time::Instant::now());
        true
    }

    pub(super) fn revision(&self) -> Option<u64> {
        self.intent.as_ref().map(|_| self.revision)
    }

    #[cfg(all(test, feature = "ui-tests"))]
    pub(super) fn processing(&self) -> Option<AuthoredPhotograph> {
        self.intent.as_ref().map(|intent| {
            let repairs = self
                .wanted
                .as_ref()
                .map(|demand| demand.spec.repairs.clone())
                .unwrap_or_default();
            intent
                .1
                .processing(
                    &self.device().map(|d| d.limits()).unwrap_or_default(),
                    &repairs,
                )
                .expect("captured view input was admitted")
        })
    }

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
                        let repairs = self
                            .wanted
                            .as_ref()
                            .map(|s| s.spec.repairs.clone())
                            .unwrap_or_default();
                        rendered.spec.frame() == *frame
                            && rendered.spec.repairs == repairs
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

    pub(super) fn queue(&self) -> Option<&wgpu::Queue> {
        self.gpu.as_ref().map(|(_, queue)| queue)
    }

    pub(super) fn reference_ready(&self) -> bool {
        self.reference.is_some()
    }

    pub(super) fn hold(
        &mut self,
        target: OutputTarget,
        compare: bool,
    ) -> Option<Arc<RenderedView>> {
        self.displayed = self.wanted.as_ref().and_then(|wanted| {
            self.present(wanted.spec.frame(), self.displayed.as_ref(), compare)
                .ok()
                .flatten()
                .or_else(|| self.rendered.clone())
                .filter(|rendered| {
                    rendered.spec.identity == wanted.spec.identity && rendered.target == target
                })
        });
        self.updating = true;
        self.displayed.clone()
    }

    pub(super) fn clear(&mut self) {
        self.intent = None;
        self.changed_at = None;
        self.retire_support();
    }

    fn retire_support(&mut self) {
        let mut mailbox = self.mailbox.0.lock().unwrap();
        let retired = mailbox.demand.take();
        mailbox.completed = None;
        mailbox.camera = None;

        self.frame = None;
        self.underlay_rendered = None;
        self.wanted = None;
        self.failed = None;
        self.rendered = None;
        self.reference = None;
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
    pub(super) fn neutral_source(&self) -> Option<Arc<ViewSource>> {
        let source = self.source()?;
        let state = self.mailbox.0.lock().unwrap();
        ViewSource::cached_neutral(
            &source.renderer.evaluation().materializations(),
            source.renderer.clone(),
            self.kernels.get()?.clone(),
            &source.spec.clone().with_repairs(Default::default()),
            &state.demand.as_ref()?.pixels.scan().ok()?,
            state
                .demand
                .as_ref()?
                .pixels
                .correction
                .as_ref()
                .map(|c| c.gpu.clone()),
        )
    }

    pub(super) fn support_ready(&self) -> bool {
        self.frame
            .as_ref()
            .zip(self.wanted.as_ref())
            .is_some_and(|(frame, wanted)| frame.satisfies(wanted))
    }

    #[cfg(all(test, feature = "ui-tests"))]
    pub(super) fn current_frame(&self) -> Option<FrameRaster> {
        self.wanted.as_ref().map(|wanted| wanted.spec.frame())
    }

    pub(super) fn request(&mut self, mut demand: Demand) {
        demand.spec = demand
            .spec
            .with_identity(crate::gpu::source::identity::Identity::camera(
                demand.pixels.source.identity(),
            ));
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
            state.camera = None;
            state.completed = None;
            self.frame = None;
        }
        let demand = Arc::new(demand);
        self.wanted = Some(demand.clone());
        state.demand = Some(demand);
        drop(state);
        self.refresh_support();
        self.mailbox.1.notify(usize::MAX);
    }

    pub(super) fn acquisition(&self) -> Option<Acquisition> {
        let state = self.mailbox.0.lock().unwrap();
        let wanted = state.demand.as_ref()?;
        (self
            .frame
            .as_ref()
            .is_none_or(|frame| !frame.satisfies(wanted))
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
            return;
        }
        match result {
            Ok(frame) if frame.satisfies(&demand) => {
                self.frame = Some(frame);
                self.failed = None;
                self.error = None;
            }
            Ok(_) => {
                self.failed = Some(demand);
                self.error = Some("acquired view does not satisfy requested source/layout".into());
            }
            Err(error) => {
                self.failed = Some(demand);
                self.error = Some(error);
            }
        }
        self.refresh_support();
    }

    pub(super) fn acquisition_failed(&mut self, error: String) {
        self.failed = self.mailbox.0.lock().unwrap().demand.clone();
        self.error = Some(error);
    }

    pub(super) fn refresh_support(&mut self) {
        let (frame, wanted) = {
            let state = self.mailbox.0.lock().unwrap();
            let (Some(frame), Some(wanted)) = (&state.completed, &state.demand) else {
                return;
            };
            (frame.clone(), wanted.clone())
        };
        if frame.source.spec.identity != wanted.spec.identity
            || frame.request.capture.is_inspection() != wanted.capture.is_inspection()
            || frame.source.spec.frame().composition != wanted.spec.frame().composition
        {
            return;
        }
        let updated = if frame.request.same_calculation(&wanted) || !frame.supports(&wanted) {
            frame.clone()
        } else {
            match frame.regrade(wanted.clone()) {
                Ok(Some(updated)) => Arc::new(updated),
                Ok(None) => frame.clone(),
                Err(error) => {
                    self.error = Some(error.to_string());
                    frame.clone()
                }
            }
        };
        let mut state = self.mailbox.0.lock().unwrap();
        if state
            .completed
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, &frame))
            && state
                .demand
                .as_ref()
                .is_some_and(|current| current.same_calculation(&wanted))
        {
            state.completed = Some(updated.clone());
            self.frame = Some(updated);
        }
    }

    #[cfg(test)]
    fn source(&self) -> Option<&Arc<ViewSource>> {
        self.frame.as_ref().map(|frame| &frame.source)
    }
    #[cfg(test)]
    fn underlay(&self) -> Option<&Arc<ViewSource>> {
        self.frame
            .as_ref()
            .and_then(|frame| frame.underlay.as_ref())
    }

    #[cfg(test)]
    fn render(
        &mut self,
        input: &AuthoredPhotograph,
        target: OutputTarget,
        context: &egui::Context,
    ) -> Result<(), Error> {
        self.render_capture(&PhotographRequest::Fixture(input.clone()), target, context)
    }

    fn render_capture(
        &mut self,
        capture: &PhotographRequest,
        target: OutputTarget,
        context: &egui::Context,
    ) -> Result<(), Error> {
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
        let Some(mut frame) = self.frame.clone() else {
            self.updating = true;
            return Ok(());
        };
        if wanted.capture.is_inspection()
            && self.mailbox.0.lock().unwrap().demand.is_none()
            && frame.request.capture != *capture
        {
            let mut request = (*frame.request).clone();
            request.capture = capture.clone();
            if let Some(updated) = frame.regrade(Arc::new(request))? {
                frame = Arc::new(updated);
                self.frame = Some(frame.clone());
            }
        }
        if frame.processed.len() != 1 + usize::from(frame.underlay.is_some()) {
            let updated = frame
                .regrade(frame.request.clone())?
                .ok_or(Error::InvalidParameter(
                    "frame does not supply its captured calculation",
                ))?;
            frame = Arc::new(updated);
            self.frame = Some(frame.clone());
        }
        let capture = &frame.request.capture;
        let matches = |rendered: &RenderedView, index: usize| {
            rendered.capture == *capture
                && rendered.target == target
                && rendered.uses_processed(&frame.processed[index])
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
                |source: Arc<ViewSource>, index: usize, previous: Option<&Arc<RenderedView>>| {
                    if let Some(rendered) = previous.filter(|rendered| matches(rendered, index)) {
                        return Ok(rendered.clone());
                    }
                    RenderedView::from_processed(
                        self.device().unwrap(),
                        source,
                        capture.clone(),
                        target,
                        frame.processed[index].clone(),
                        previous,
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

    fn present(
        &self,
        frame: FrameRaster,
        previous: Option<&Arc<RenderedView>>,
        compare: bool,
    ) -> Result<Option<Arc<RenderedView>>, Error> {
        let is_current_source = |rendered: &RenderedView| {
            self.wanted.as_ref().is_some_and(|wanted| {
                wanted.spec.identity == rendered.spec.identity
                    && wanted.capture.is_inspection() == rendered.capture.is_inspection()
            })
        };
        if compare
            && self
                .wanted
                .as_ref()
                .is_some_and(|wanted| !wanted.capture.is_inspection())
            && let Some(reference) = self
                .reference
                .as_ref()
                .filter(|rendered| is_current_source(rendered) && rendered.spec.frame() == frame)
        {
            return Ok(Some(reference.clone()));
        }

        let Some(detail) = self
            .rendered
            .as_ref()
            .filter(|rendered| is_current_source(rendered))
        else {
            return Ok(None);
        };

        match &self.underlay_rendered {
            Some(underlay) => RenderedView::composite(
                self.device().unwrap(),
                detail.clone(),
                underlay.clone(),
                frame,
                previous,
            )
            .map(Some),
            None => Ok(Some(detail.clone())),
        }
    }
}

impl Drop for WorkingView {
    fn drop(&mut self) {
        let mut mailbox = self.mailbox.0.lock().unwrap();
        let retired = mailbox.demand.take();
        mailbox.completed = None;
        mailbox.camera = None;

        drop(mailbox);
        drop(retired);
    }
}

// Physical acquisition follows the latest captured demand while retaining one admission.
#[allow(clippy::too_many_arguments)]
pub(super) async fn acquire_images(
    mailbox: &Arc<Mailbox>,
    renderer: &Arc<crate::renderer::GpuRenderer>,
    kernels: &Arc<ViewKernels>,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    submissions: &crate::gpu::submission::SubmissionGate,
) -> Option<Completion> {
    mailbox.watch();
    loop {
        let demand = mailbox.0.lock().unwrap().demand.clone()?;
        let spec = &demand.spec;
        let cancelled = || {
            mailbox
                .0
                .lock()
                .unwrap()
                .demand
                .as_ref()
                .is_none_or(|latest| !demand.same_source(latest))
        };
        if cancelled() {
            return None;
        }
        let result = async {
            let scan = demand.pixels.scan()?;
            let specs = vec![spec.clone(), spec.underlay()?];
            let correction = demand
                .pixels
                .correction
                .as_ref()
                .map(|field| field.gpu.clone());
            let neutral_specs: Vec<_> = specs
                .iter()
                .cloned()
                .map(|spec| spec.with_repairs(Default::default()))
                .collect();
            let native_request =
                |root: Arc<crate::gpu::evaluation::ImageNode>,
                 region: crate::gpu::evaluation::CellRegion| {
                    let grid = root.domain().grid;
                    renderer.evaluation().request(
                        root,
                        crate::gpu::evaluation::Region {
                            bounds: region,
                            grid,
                        },
                        crate::gpu::evaluation::Budget {
                            tile: [region.width, region.height],
                            scratch_bytes: 512 << 20,
                        },
                    )
                };
            let (calculations, native_requests) = match &demand.capture {
                PhotographRequest::Recipe(recipe) | PhotographRequest::BeforeLooks(recipe) => specs
                    .iter()
                    .map(|spec| {
                        let (photograph, plan) = crate::gpu::photograph::PhotographGraph::preview(
                            &device.limits(),
                            &scan,
                            correction.clone(),
                            recipe,
                            &spec.repairs,
                            spec.frame(),
                            None,
                        )?;
                        let region = photograph
                            .processed
                            .region_for(
                                &photograph.neutral,
                                photograph.processed.domain().finite()?,
                            )?
                            .ok_or(Error::InvalidParameter("preview has no neutral dependency"))?;
                        let native = if spec.repairs.0.is_empty() {
                            photograph.neutral
                        } else {
                            crate::gpu::evaluation::reconstruct(
                                photograph.unrepaired,
                                photograph.neutral.domain(),
                            )?
                        };
                        Ok((
                            (photograph.processed, plan),
                            native_request(native, region)?,
                        ))
                    })
                    .collect::<Result<Vec<_>, Error>>()?
                    .into_iter()
                    .unzip(),
                _ => {
                    let native_roots =
                        demand
                            .capture
                            .source_graphs(&scan, correction.clone(), &specs)?;
                    let calculations = native_roots
                        .into_iter()
                        .zip(&specs)
                        .map(|(native, spec)| {
                            demand.capture.filtered(&device.limits(), spec, native)
                        })
                        .collect::<Result<Vec<_>, Error>>()?;
                    let requests = demand
                        .capture
                        .source_graphs(&scan, correction.clone(), &neutral_specs)?
                        .into_iter()
                        .zip(&neutral_specs)
                        .map(|(root, spec)| native_request(root, spec.input_region()?))
                        .collect::<Result<Vec<_>, Error>>()?;
                    (calculations, requests)
                }
            };
            let cache = renderer.evaluation().materializations();
            let source = crate::gpu::evaluation::camera_source(&scan)?;
            let prepare = {
                let state = mailbox.0.lock().unwrap();
                state.pressure == Some(memory::Pressure::Normal)
                    && state
                        .camera
                        .as_ref()
                        .is_none_or(|camera| !camera.matches(&source))
            };
            if prepare {
                let camera = renderer
                    .evaluation()
                    .prepare_input(queue, submissions, &cache, source, || {
                        cancelled()
                            || mailbox.0.lock().unwrap().pressure != Some(memory::Pressure::Normal)
                    })
                    .await?;
                if !cancelled() {
                    let mut state = mailbox.0.lock().unwrap();
                    if state.pressure == Some(memory::Pressure::Normal)
                        && state
                            .demand
                            .as_ref()
                            .is_some_and(|latest| demand.same_source(latest))
                    {
                        state.camera = camera;
                    }
                }
            }
            // Input storage survives parameter/layout demand. Start the image calculation
            // with the latest capture after that preparation, not an already superseded one.
            if mailbox
                .0
                .lock()
                .unwrap()
                .demand
                .as_ref()
                .is_none_or(|latest| latest.spec != *spec)
            {
                return Ok(None);
            }
            let Some(neutral) = ViewSource::acquire_images(
                device,
                queue,
                submissions,
                renderer.clone(),
                kernels.clone(),
                neutral_specs,
                native_requests,
                &cache,
                &cancelled,
            )
            .await?
            else {
                return Ok(None);
            };
            let previous = mailbox.0.lock().unwrap().completed.clone();
            let mut sources = Vec::new();
            for (index, (wanted, neutral)) in specs.into_iter().zip(neutral).enumerate() {
                let old = previous.as_ref().and_then(|frame| {
                    if index == 0 {
                        Some(frame.source.clone())
                    } else {
                        frame.underlay.clone()
                    }
                });
                let Some(source) = ViewSource::reconstruct_images(
                    device,
                    queue,
                    submissions,
                    neutral,
                    wanted.repairs,
                    old,
                    &cache,
                    cancelled,
                )
                .await?
                else {
                    return Ok(None);
                };
                sources.push(source);
            }
            let requests = calculations
                .iter()
                .map(|(root, _)| {
                    crate::gpu::view::processing::ProcessedView::request(
                        renderer.evaluation(),
                        root.clone(),
                    )
                })
                .collect::<Result<Vec<_>, Error>>()?;
            let futures =
                renderer
                    .evaluation()
                    .materialize_many(queue, submissions, &cache, requests)?;
            let mut pending = Box::pin(futures_util::future::try_join_all(futures));
            let values = futures_util::future::poll_fn(|cx| {
                if cancelled() {
                    return std::task::Poll::Ready(None);
                }
                std::future::Future::poll(pending.as_mut(), cx).map(Some)
            })
            .await;
            let Some(values) = values else {
                return Ok(None);
            };
            let values = match values {
                Ok(values) => values,
                Err(error) if matches!(error.as_ref(), Error::Cancelled) => return Ok(None),
                Err(error) => return Err(Error::Gpu(error.to_string())),
            };
            let processed = sources
                .iter()
                .zip(calculations)
                .zip(values)
                .map(|((source, (root, plan)), value)| {
                    crate::gpu::view::processing::ProcessedView::produced(
                        source.acquired(),
                        root,
                        plan,
                        value,
                    )
                })
                .collect::<Result<Vec<_>, Error>>()?;
            let underlay = sources.pop();
            let source = sources.pop().expect("detail and underlay");
            Ok(Some(Arc::new(Frame {
                source,
                underlay,
                request: demand.clone(),
                processed,
            })))
        }
        .await
        .map_err(|error: Error| error.to_string());
        if cancelled() {
            return None;
        }
        let mut state = mailbox.0.lock().unwrap();
        if let Ok(Some(frame)) = &result {
            state.completed = Some(frame.clone());
        }
        let latest = state.demand.as_ref()?;
        if latest.spec == *spec {
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
        // Finish one immutable image before starting the coalesced demand. Its photo-owned
        // presentation remains useful even when the exact-grade consumer has moved on.
    }
}

impl Editor {
    pub(super) fn comparing_original(&self) -> bool {
        self.document.visit.working_view.compare && !self.document.visit.masks.editing()
    }

    pub(super) fn admit_view_reference(&mut self, frame: FrameRaster) -> bool {
        let pixels = frame.output.iter().fold(16usize, |bytes, dimension| {
            bytes.saturating_mul(*dimension as usize)
        });
        if self.canvas_inspection() {
            self.discard_original();
            self.document.thumbnails.reserve_view(0);
            return false;
        }

        let reservation = if self.document.visit.canvas.original.refining()
            || self.document.visit.canvas.original.settled().is_none()
        {
            pixels.saturating_mul(2).saturating_add(3 << 20)
        } else {
            pixels
        };
        let allowed = if self.comparing_original() {
            self.document.thumbnails.reserve_view(0);
            true
        } else if self.document.visit.canvas.original.settled().is_some() {
            self.document.thumbnails.reserve_view(reservation)
        } else {
            self.document.thumbnails.reserve_view(0);
            false
        };
        if !allowed {
            self.discard_original();
            self.document.visit.working_view.reference = None;
        }

        allowed
    }

    pub(super) fn working_view(
        &mut self,
        frame: FrameRaster,
        (capture, repairs): (&PhotographRequest, &crate::gpu::repairs::Snapshot),
        image: super::canvas::ImageMode,
        context: &egui::Context,
        cx: &UiContext<'_, Self>,
    ) -> Result<Option<Arc<RenderedView>>, Error> {
        let target = self.document.target();
        let inspection = image.inspection(self.canvas_inspection());
        let compare = self.comparing_original();
        let previous = self.document.visit.working_view.displayed.take();
        if self
            .document
            .visit
            .working_view
            .wanted
            .as_ref()
            .is_some_and(|wanted| wanted.capture.is_inspection() != inspection)
        {
            self.document.visit.working_view.retire_support();
        }
        if !self.document.visit.working_view.enabled() {
            return Ok(None);
        }

        if self.interaction_blocked()
            && self.document_pending.is_none()
            && !image.is_sample()
            && !self.cropping()
        {
            let mut mailbox = self.document.visit.working_view.mailbox.0.lock().unwrap();
            mailbox.demand = None;
            drop(mailbox);

            if inspection && self.document.visit.working_view.wanted.is_some() {
                self.document
                    .visit
                    .working_view
                    .render_capture(capture, target, context)?;
            }

            self.document.visit.working_view.displayed = self
                .document
                .visit
                .working_view
                .present(frame, previous.as_ref(), compare)?
                .filter(|rendered| rendered.target == self.document.target());
            self.document.visit.working_view.updating =
                !self.document.visit.working_view.support_ready()
                    || self
                        .document
                        .visit
                        .working_view
                        .displayed
                        .as_ref()
                        .is_none_or(|rendered| {
                            rendered.spec.frame() != frame
                                || !rendered.matches_capture(
                                    &rendered.spec,
                                    capture,
                                    self.document.target(),
                                )
                        });
            return Ok(self.document.visit.working_view.displayed.clone());
        }

        if self.document_pending.is_some() {
            // Candidate preparation must not retire the accepted view before replacement succeeds.
            self.document.visit.working_view.displayed = previous;
            return Ok(self.document.visit.working_view.hold(target, compare));
        }

        if repairs.0.is_empty()
            && (image.is_sample() || self.fullscreen.is_none() && self.cropping())
        {
            self.document.visit.working_view.clear();
            return Ok(None);
        }

        if self.document.visit.canvas.ready.is_none() {
            self.document.visit.working_view.clear();
            return Ok(None);
        }

        let source_input = self.source_context().expect("ready working source");
        if inspection {
            let spec = SourceRequest::for_frame(frame)?;
            self.request_working(
                Demand {
                    spec,
                    pixels: source_input,
                    capture: capture.clone(),
                },
                cx,
            );
            self.document
                .visit
                .working_view
                .render_capture(capture, target, context)?;
            self.document.visit.working_view.reference = None;

            self.document.visit.working_view.displayed = self
                .document
                .visit
                .working_view
                .present(frame, previous.as_ref(), compare)?
                .filter(|rendered| rendered.target == self.document.target());
            self.document.visit.working_view.updating |= self
                .document
                .visit
                .working_view
                .displayed
                .as_ref()
                .is_none_or(|rendered| {
                    self.document
                        .visit
                        .working_view
                        .wanted
                        .as_ref()
                        .is_none_or(|wanted| {
                            !rendered.matches_capture(&wanted.spec, capture, self.document.target())
                        })
                });
            return Ok(self.document.visit.working_view.displayed.clone());
        }

        let spec = capture.source_request(frame, repairs.clone())?;
        self.request_working(
            Demand {
                spec,
                pixels: source_input,
                capture: capture.clone(),
            },
            cx,
        );
        self.document
            .visit
            .working_view
            .render_capture(capture, target, context)?;

        self.document.visit.working_view.reference = self
            .original_rendered(OriginalPurpose::Reference)?
            .filter(|rendered| rendered.spec.frame() == frame);

        self.document.visit.working_view.displayed = self
            .document
            .visit
            .working_view
            .present(frame, previous.as_ref(), compare)?
            .filter(|rendered| rendered.target == self.document.target());
        self.document.visit.working_view.updating |= self
            .document
            .visit
            .working_view
            .displayed
            .as_ref()
            .is_none_or(|rendered| {
                self.document
                    .visit
                    .working_view
                    .wanted
                    .as_ref()
                    .is_none_or(|wanted| {
                        !rendered.matches_capture(&wanted.spec, capture, self.document.target())
                    })
            });
        if self.document.visit.working_view.displayed.is_none()
            && self.document.visit.working_view.error.is_none()
        {
            context.request_repaint_after(Duration::from_millis(16));
        }

        Ok(self.document.visit.working_view.displayed.clone())
    }
}

#[derive(Default)]
pub(super) struct State {
    pub task: Option<async_executor::Task<()>>,
}

impl Acquisition {
    pub async fn prepare(
        self,
        services: source::Services,
        submissions: crate::platform::SubmissionGate,
    ) -> Result<Option<Completion>, String> {
        let kernels = self.kernels.clone();
        let prepared = crate::background::run(move |stop| {
            if stop.is_cancelled() {
                return Ok(None);
            }
            let renderer = services.renderers.get(&services.device, &services.queue)?;
            let pipelines = if let Some(pipelines) = kernels.get() {
                pipelines.clone()
            } else {
                let prepared =
                    pollster::block_on(crate::gpu::view::ViewKernels::new(&services.device))?;
                kernels.get_or_init(|| prepared).clone()
            };
            Ok::<_, crate::Error>(Some((services, renderer, pipelines)))
        })
        .await
        .map_err(|_| "Working-source preparation worker panicked".to_owned())?
        .map_err(|error| error.to_string())?;
        let Some((services, renderer, kernels)) = prepared else {
            return Ok(None);
        };
        Ok(acquire_images(
            &self.mailbox,
            &renderer,
            &kernels,
            &services.device,
            &services.queue,
            &submissions,
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
        if self.allow_close
            || self.document_pending.is_some()
            || self.document.visit.working.task.is_some()
        {
            return;
        }
        let Some(request) = self.document.visit.working_view.acquisition() else {
            return;
        };
        let Some(services) = self.source_services.clone() else {
            self.document
                .visit
                .working_view
                .acquisition_failed("Working-source services unavailable".into());
            return;
        };
        let owner = Arc::downgrade(&request.mailbox);
        let submissions = self.submissions.clone();
        let mailbox = request.mailbox.clone();
        let Some(task) = cx.spawn(move |cx| async move {
            let acquisition = request.prepare(services, submissions);
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
                editor.document.visit.working.task = None;
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
                .acquisition_failed("Application stopped".into());
            return;
        };
        self.document.visit.working.task = Some(task);
    }
}
