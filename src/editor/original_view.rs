//! Purpose-bound original-sample reconstruction, independent of normal working-view acceptance.
use super::*;
#[cfg(test)]
#[cfg(test)]
use crate::gpu::test_inputs::AuthoredPhotograph;
use std::sync::{Mutex, Weak};
#[cfg(test)]
mod tests;
#[cfg(test)]
mod workflow_tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OriginalPurpose {
    Reference,
    Tool,
}

#[derive(Clone, PartialEq)]
pub(super) struct Demand {
    pub capture: crate::gpu::photograph::request::PhotographRequest,
    pub repairs: crate::gpu::repairs::Snapshot,
    pub target: OutputTarget,
    pub view: Option<ViewRequest>,
    pub composition: Composition,
    pub source: source::Context,
}

#[cfg(test)]
impl Demand {
    pub fn fixture(
        input: AuthoredPhotograph,
        target: OutputTarget,
        view: Option<ViewRequest>,
        composition: Composition,
        source: source::Context,
    ) -> Self {
        Self {
            repairs: input.repairs.clone(),
            capture: crate::gpu::photograph::request::PhotographRequest::Fixture(input),
            target,
            view,
            composition,
            source,
        }
    }
}

pub(super) struct Request {
    pub demand: Arc<Demand>,
    completion: Completion,
}

enum Progress {
    Waiting,
    Running(ViewRequest),
    Ready(ViewRequest, Arc<GpuImage>),
    Failed,
}

#[derive(Clone)]
pub(super) struct Completion(Weak<Mutex<Progress>>);
impl std::fmt::Debug for Completion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("OriginalCompletion")
            .field(&self.0.as_ptr())
            .finish()
    }
}
impl PartialEq for Completion {
    fn eq(&self, other: &Self) -> bool {
        self.0.ptr_eq(&other.0)
    }
}
impl Eq for Completion {}
impl Completion {
    pub fn receive(&self, result: Result<Option<Arc<GpuImage>>, String>) -> Option<String> {
        let state = self.0.upgrade()?;
        let mut state = state.lock().unwrap();
        let Progress::Running(view) = &*state else {
            return None;
        };
        let view = *view;
        match result {
            Ok(Some(image)) => *state = Progress::Ready(view, image),
            Ok(None) => *state = Progress::Waiting,
            Err(error) => {
                *state = Progress::Failed;
                return Some(format!("Original view failed: {error}"));
            }
        }
        None
    }
    pub fn belongs_to(&self, owner: &OriginalView) -> bool {
        self.0.ptr_eq(&Arc::downgrade(&owner.progress))
    }
}
impl Request {
    pub async fn prepare(
        self,
        services: source::Services,
        submissions: crate::platform::SubmissionGate,
    ) -> Result<Option<Arc<crate::viewer::GpuImage>>, String> {
        let demand = self.demand;
        crate::background::run(move |stop| {
            if stop.is_cancelled() {
                return Ok(None);
            }
            let source = &demand.source;
            let scan = source.scan()?;
            let renderer = services.renderers.get(&services.device, &services.queue)?;
            let original = demand.capture.original(
                &services.device,
                &renderer,
                &scan,
                source
                    .correction
                    .as_ref()
                    .map(|correction| correction.gpu.clone()),
                &demand.repairs,
                demand.target,
            )?;
            crate::refinement::refine_definition(
                &renderer,
                &services.device,
                (&services.queue, &submissions),
                original,
                (demand.target, demand.view.expect("requested original view")),
                demand.capture.look_id().is_some(),
                || stop.is_cancelled(),
            )
        })
        .await
        .map_err(|_| "Original reconstruction worker panicked".to_owned())?
        .map_err(|error: crate::Error| error.to_string())
    }

    #[cfg(test)]
    pub fn completion(&self) -> Completion {
        self.completion.clone()
    }
    pub fn begin(&self) -> Option<Completion> {
        let state = self.completion.0.upgrade()?;
        let mut state = state.lock().unwrap();
        if !matches!(*state, Progress::Waiting) {
            return None;
        }
        *state = Progress::Running(self.demand.view?);
        Some(self.completion.clone())
    }
}

pub(super) struct OriginalView {
    pub wanted: Option<Arc<Demand>>,
    purpose: Option<OriginalPurpose>,
    progress: Arc<Mutex<Progress>>,
    pub task: Option<async_executor::Task<()>>,
    rendered: Option<Arc<crate::gpu::view::RenderedView>>,
    pub last_edit: std::time::Instant,
}
impl Default for OriginalView {
    fn default() -> Self {
        Self {
            wanted: None,
            purpose: None,
            progress: Arc::new(Mutex::new(Progress::Waiting)),
            task: None,
            rendered: None,
            last_edit: std::time::Instant::now(),
        }
    }
}
impl OriginalView {
    #[cfg(test)]
    pub fn continuation(&self) -> Completion {
        Completion(Arc::downgrade(&self.progress))
    }
    #[cfg(test)]
    pub fn start_fixture(&self, view: ViewRequest) -> Completion {
        *self.progress.lock().unwrap() = Progress::Running(view);
        self.continuation()
    }
    #[cfg(test)]
    pub fn settle_fixture(&self, view: ViewRequest, image: Arc<GpuImage>) {
        self.start_fixture(view).receive(Ok(Some(image)));
    }
    pub fn settled(&self) -> Option<(ViewRequest, Arc<GpuImage>)> {
        match &*self.progress.lock().unwrap() {
            Progress::Ready(view, image) => Some((*view, image.clone())),
            _ => None,
        }
    }
    pub fn refining(&self) -> bool {
        matches!(*self.progress.lock().unwrap(), Progress::Running(_))
    }
    pub fn failed(&self) -> bool {
        matches!(*self.progress.lock().unwrap(), Progress::Failed)
    }
    fn retire_pending(&mut self) {
        self.task = None;
        self.progress = Arc::new(Mutex::new(Progress::Waiting));
    }
    pub fn invalidate(&mut self) {
        self.retire_pending();
        self.wanted = None;
        self.purpose = None;
        self.rendered = None;
    }
    pub fn discard(&mut self) {
        if self.wanted.is_some() || self.settled().is_some() || self.refining() {
            self.invalidate();
        } else {
            self.cancel_pending();
        }
    }
    pub fn cancel_pending(&mut self) {
        self.task = None;
        if self.refining() {
            self.retire_pending();
        }
    }
    pub fn observe(&mut self, purpose: OriginalPurpose, demand: Demand) {
        if self.purpose != Some(purpose) || self.wanted.as_deref() != Some(&demand) {
            self.invalidate();
            self.purpose = Some(purpose);
            self.wanted = Some(Arc::new(demand));
            self.last_edit = std::time::Instant::now();
        }
    }
    pub fn leave(&mut self) {
        if self.purpose == Some(OriginalPurpose::Tool)
            || (self.purpose.is_some() && self.settled().is_none())
        {
            self.invalidate();
        } else {
            self.cancel_pending();
        }
    }
    pub fn idle(&self) -> bool {
        !self.refining()
            && (self.wanted.is_none() || self.settled().is_some() || self.failed())
            && self.last_edit.elapsed() >= Duration::from_millis(250)
    }
    pub fn request(&self) -> Option<Request> {
        let demand = self.wanted.as_ref()?;
        demand.view?;
        if !matches!(*self.progress.lock().unwrap(), Progress::Waiting)
            || self.last_edit.elapsed() < Duration::from_millis(250)
        {
            return None;
        }
        Some(Request {
            demand: demand.clone(),
            completion: Completion(Arc::downgrade(&self.progress)),
        })
    }
}

impl OriginalView {
    pub(super) fn request_with(
        &mut self,
        services: source::Services,
        submissions: crate::platform::SubmissionGate,
        context: &egui::Context,
        cx: &UiContext<'_, Editor>,
    ) {
        if let Some(request) = self.request() {
            let Some(completion) = request.begin() else {
                return;
            };
            let stopped = completion.clone();
            self.task = cx.spawn(move |cx| async move {
                let result = request.prepare(services, submissions).await;
                cx.update(|editor| {
                    if let Some(error) = completion.receive(result) {
                        editor.status = error;
                    }
                });
            });
            if self.task.is_none() {
                stopped.receive(Err("Application stopped".into()));
            }
        }
        if self.refining() || self.settled().is_none() && !self.failed() {
            context.request_repaint_after(Duration::from_millis(50));
        }
    }

    /// Present a completed original-sample reconstruction without another image calculation.
    pub(super) fn rendered(
        &mut self,
        purpose: OriginalPurpose,
    ) -> Result<Option<Arc<crate::gpu::view::RenderedView>>, Error> {
        use crate::frame_raster::FrameRaster;
        use crate::gpu::view::RenderedView;
        use crate::gpu::view::request::SourceRequest;
        let owner = &*self;
        let Some((view, image)) = owner.settled() else {
            return Ok(None);
        };
        if owner.purpose != Some(purpose) {
            return Ok(None);
        }
        let Some(wanted) = owner.wanted.clone() else {
            return Ok(None);
        };
        // Rectangular tools sample native axes; an affine request carries its own grid.
        let frame = match view.frame {
            Some(frame) => frame,
            None => {
                let composition =
                    Composition::new(view.area, Default::default(), Default::default())?;
                let mut frame = FrameRaster::full(composition)?;
                frame.output = view.output;
                frame
            }
        };
        let identity =
            crate::gpu::source::identity::Identity::camera(wanted.source.source.identity());
        let spec = match &wanted.capture {
            crate::gpu::photograph::request::PhotographRequest::Recipe(recipe) => {
                SourceRequest::for_recipe(frame, recipe)?
            }
            crate::gpu::photograph::request::PhotographRequest::Inspection(_) => {
                SourceRequest::for_frame(frame)?
            }
            #[cfg(test)]
            crate::gpu::photograph::request::PhotographRequest::Fixture(input)
                if input.grade().is_some() =>
            {
                SourceRequest::for_processing(frame, input)?
            }
            #[cfg(test)]
            crate::gpu::photograph::request::PhotographRequest::Fixture(_) => {
                SourceRequest::for_frame(frame)?
            }
        }
        .with_identity(identity)
        .with_repairs(wanted.repairs.clone());
        if let Some(rendered) = &owner.rendered
            && Arc::ptr_eq(&rendered.image, &image)
            && rendered.capture == wanted.capture
            && rendered.target == wanted.target
            && rendered.spec.frame() == frame
        {
            return Ok(Some(rendered.clone()));
        }
        let rendered = RenderedView::reference(image, spec, wanted.capture.clone(), wanted.target);
        self.rendered = Some(rendered.clone());
        Ok(Some(rendered))
    }
}

impl Editor {
    pub(super) fn original_rendered(
        &mut self,
        purpose: OriginalPurpose,
    ) -> Result<Option<Arc<crate::gpu::view::RenderedView>>, Error> {
        self.active_canvas_mut().grade.original.rendered(purpose)
    }

    pub(super) fn invalidate_original(&mut self) {
        self.document.visit.canvas.grade.original.invalidate();
    }
    pub(super) fn discard_original(&mut self) {
        self.document.visit.canvas.grade.original.discard();
    }
    pub(super) fn cancel_original_pending(&mut self) {
        self.document.visit.canvas.grade.original.cancel_pending();
    }
    pub(super) fn leave_original(&mut self) {
        self.active_canvas_mut().grade.original.leave();
    }

    pub(super) fn observe_original(
        &mut self,
        purpose: Option<OriginalPurpose>,
        demand: Option<Demand>,
    ) {
        match (purpose, demand) {
            (Some(purpose), Some(demand)) => self
                .active_canvas_mut()
                .grade
                .original
                .observe(purpose, demand),
            _ => self.leave_original(),
        }
    }

    pub(super) fn request_original(&mut self, context: &egui::Context, cx: &UiContext<'_, Self>) {
        if let Some(request) = self.active_canvas().grade.original.request() {
            let Some(completion) = request.begin() else {
                return;
            };
            let Some(services) = self.source_services.clone() else {
                if let Some(error) =
                    completion.receive(Err("Original reconstruction services unavailable".into()))
                {
                    self.status = error;
                }
                return;
            };
            let submissions = self.submissions.clone();
            let completion_on_shutdown = completion.clone();
            let Some(task) = cx.spawn(move |cx| async move {
                let result = request.prepare(services, submissions).await;
                cx.update(|editor| {
                    let visible = completion.belongs_to(&editor.active_canvas().grade.original);
                    if let Some(error) = completion.receive(result)
                        && visible
                    {
                        editor.status = error;
                    }
                });
            }) else {
                if let Some(error) =
                    completion_on_shutdown.receive(Err("Application stopped".into()))
                {
                    self.status = error;
                }
                return;
            };
            self.active_canvas_mut().grade.original.task = Some(task);
        }
        let original = &self.active_canvas().grade.original;
        if original.refining() || original.settled().is_none() && !original.failed() {
            context.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }
}
