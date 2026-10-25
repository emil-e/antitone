//! Selected-view ownership: one neutral acquisition, latest-request coalescing, immutable frames.
use super::*;
use crate::processing::InputSettings;
use crate::{
    frame_raster::FrameRaster,
    gpu::view::{RenderedView, ViewKernels, ViewSource, ViewSourceSpec},
};
use std::sync::{Mutex, OnceLock};

#[cfg(test)]
#[path = "editor_working_view_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "editor_working_workflow_tests.rs"]
mod workflow_tests;

#[derive(Default)]
struct MailboxState {
    wanted: Option<ViewSourceSpec>,
    input: Option<source::Context>,
    underlay: Option<Arc<ViewSource>>,
    neutral_detail: Option<Arc<ViewSource>>,
    neutral_underlay: Option<Arc<ViewSource>>,
    repaired_detail: Option<Arc<ViewSource>>,
}

#[derive(Default)]
pub(super) struct Mailbox(Mutex<MailboxState>, pub event_listener::Event);

pub(super) struct Completion {
    spec: ViewSourceSpec,
    result: Result<Arc<ViewSource>, String>,
}

pub(super) struct Acquisition {
    pub mailbox: Arc<Mailbox>,
    pub kernels: Arc<OnceLock<Arc<ViewKernels>>>,
}

#[derive(Default)]
pub(super) struct WorkingView {
    gpu: Option<(wgpu::Device, wgpu::Queue)>,
    intent: Option<(u64, FrameRaster, ProcessingSignature, OutputTarget)>,
    revision: u64,
    changed_at: Option<std::time::Instant>,
    mailbox: Arc<Mailbox>,
    pub(super) kernels: Arc<OnceLock<Arc<ViewKernels>>>,
    source: Option<Arc<ViewSource>>,
    underlay: Option<Arc<ViewSource>>,
    underlay_rendered: Option<Arc<RenderedView>>,
    wanted: Option<ViewSourceSpec>,
    failed: Option<ViewSourceSpec>,
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

    pub(super) fn observe(
        &mut self,
        authority: u64,
        frame: FrameRaster,
        processing: ProcessingSignature,
        target: OutputTarget,
    ) -> bool {
        let intent = (authority, frame, processing, target);
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
    pub(super) fn processing(&self) -> Option<&ProcessingSignature> {
        self.intent.as_ref().map(|intent| &intent.2)
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
                .is_some_and(|(authority, frame, processing, target)| {
                    self.displayed.as_ref().is_some_and(|rendered| {
                        rendered.spec.authority == *authority
                            && rendered.spec.frame == *frame
                            && rendered.processing == *processing
                            && rendered.target == *target
                            && !rendered.in_flight()
                    })
                })
    }

    pub(super) fn authority(&self) -> Option<u64> {
        self.wanted.as_ref().map(|wanted| wanted.authority)
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
        authority: u64,
        target: OutputTarget,
    ) -> Option<Arc<RenderedView>> {
        self.displayed = self
            .wanted
            .as_ref()
            .and_then(|wanted| {
                self.present(wanted.frame, self.displayed.as_ref())
                    .ok()
                    .flatten()
            })
            .or_else(|| self.rendered.clone())
            .filter(|rendered| rendered.spec.authority == authority && rendered.target == target);
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
        let retired = mailbox.input.take();
        mailbox.wanted = None;
        mailbox.underlay = None;
        mailbox.neutral_detail = None;
        mailbox.neutral_underlay = None;
        mailbox.repaired_detail = None;

        self.source = None;
        self.underlay = None;
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
    }

    #[cfg(all(test, feature = "ui-tests"))]
    pub(super) fn regional(&self) -> Option<&Arc<RenderedView>> {
        self.rendered.as_ref()
    }

    #[cfg(all(test, feature = "ui-tests"))]
    pub(super) fn neutral_source(&self) -> Option<Arc<ViewSource>> {
        self.mailbox.0.lock().unwrap().neutral_detail.clone()
    }

    pub(super) fn support_ready(&self) -> bool {
        self.source
            .as_ref()
            .zip(self.wanted.as_ref())
            .is_some_and(|(source, wanted)| source.spec == *wanted)
    }

    pub(super) fn current_frame(&self) -> Option<FrameRaster> {
        self.wanted.as_ref().map(|wanted| wanted.frame)
    }

    pub(super) fn request(&mut self, spec: ViewSourceSpec, input: source::Context) {
        if self
            .wanted
            .as_ref()
            .is_some_and(|wanted| wanted.is_inspection() != spec.is_inspection())
        {
            self.retire_support();
        }

        let mut mailbox = self.mailbox.0.lock().unwrap();
        if self.wanted.as_ref() != Some(&spec) {
            self.failed = None;
            self.error = None;
        }

        if mailbox.underlay.as_ref().is_some_and(|underlay| {
            !underlay.spec.same_source(&spec)
                || underlay.spec.frame.composition != spec.frame.composition
        }) {
            mailbox.underlay = None;
        }
        self.underlay = mailbox.underlay.clone();
        let retired = mailbox.input.replace(input);
        mailbox.wanted = Some(spec.clone());

        self.wanted = Some(spec);
        drop(mailbox);
        drop(retired);
    }

    pub(super) fn acquisition(&self, authority: u64) -> Option<Acquisition> {
        let state = self.mailbox.0.lock().unwrap();
        let wanted = state.wanted.as_ref()?;
        (wanted.authority == authority
            && state.input.is_some()
            && self
                .source
                .as_ref()
                .is_none_or(|source| source.spec != *wanted)
            && self.failed.as_ref() != Some(wanted))
        .then(|| Acquisition {
            mailbox: self.mailbox.clone(),
            kernels: self.kernels.clone(),
        })
    }

    pub(super) fn acquired(&mut self, completion: Completion) {
        let Completion { spec, result } = completion;
        if self.wanted.as_ref() != Some(&spec)
            || self.mailbox.0.lock().unwrap().wanted.as_ref() != Some(&spec)
        {
            return;
        }
        match result {
            Ok(source) => {
                self.source = Some(source);
                self.failed = None;
                self.error = None;
            }
            Err(error) => {
                self.failed = Some(spec);
                self.error = Some(error);
            }
        }
        self.refresh_support();
    }

    pub(super) fn acquisition_failed(&mut self, error: String) {
        self.failed = self.wanted.clone();
        self.error = Some(error);
    }

    pub(super) fn refresh_support(&mut self) {
        self.underlay = self.mailbox.0.lock().unwrap().underlay.clone();
    }

    fn render(
        &mut self,
        settings: &InputSettings,
        roll: RollSettings,
        snapshot: PhotoGradeSnapshot,
        input: &ProcessingInput,
        target: OutputTarget,
        context: &egui::Context,
    ) -> Result<(), Error> {
        self.render_with(input, target, context, |device, source, previous| {
            let mut source_input = input.clone();
            source_input.repairs = source.spec.repairs.clone();
            RenderedView::new(
                device,
                source,
                settings,
                roll,
                snapshot.clone(),
                &source_input,
                target,
                previous,
            )
        })
    }

    fn render_inspection(
        &mut self,
        input: &ProcessingInput,
        target: OutputTarget,
        context: &egui::Context,
    ) -> Result<(), Error> {
        self.render_with(input, target, context, |device, source, previous| {
            RenderedView::inspection(device, source, input, target, previous)
        })
    }

    fn render_with(
        &mut self,
        input: &ProcessingInput,
        target: OutputTarget,
        context: &egui::Context,
        create: impl Fn(
            &wgpu::Device,
            Arc<ViewSource>,
            Option<&Arc<RenderedView>>,
        ) -> Result<Arc<RenderedView>, Error>,
    ) -> Result<(), Error> {
        let signature_for = |source: &ViewSource| {
            let mut source_input = input.clone();
            source_input.repairs = source.spec.repairs.clone();
            source_input.signature()
        };

        if self.colour_in_flight() {
            self.device()
                .unwrap()
                .poll(wgpu::PollType::Poll)
                .map_err(|error| Error::Gpu(error.to_string()))?;
            if self.colour_in_flight() {
                context.request_repaint_after(Duration::from_millis(8));
            }
        }

        let wanted = self.wanted.as_ref().expect("requested physical view");
        let detail = self
            .source
            .as_ref()
            .filter(|source| source.spec.same_source(wanted));
        let coverage = self
            .underlay
            .as_ref()
            .filter(|source| source.spec.same_source(wanted));
        let source = detail
            .filter(|source| source.spec.repairs == wanted.repairs)
            .or(coverage.filter(|source| source.spec.repairs == wanted.repairs))
            .or(detail)
            .or(coverage);
        let Some(source) = source else {
            // Upstream calibration/correction changes cannot grade an incompatible neutral source.
            self.rendered = self.rendered.take().filter(|rendered| {
                rendered.spec.authority == wanted.authority && rendered.target == target
            });
            self.updating = true;
            return Ok(());
        };

        let underlay = self.underlay.as_ref().filter(|underlay| {
            underlay.spec.same_source(wanted) && underlay.spec.repairs == source.spec.repairs
        });
        if self
            .rendered
            .as_ref()
            .is_none_or(|rendered| !rendered.matches(&source.spec, &signature_for(source), target))
            || underlay.is_some_and(|underlay| {
                self.underlay_rendered.as_ref().is_none_or(|rendered| {
                    !rendered.matches(&underlay.spec, &signature_for(underlay), target)
                })
            })
        {
            if self.colour_in_flight() {
                // Keep at most one submitted colour result. Repaint consumes the newest controls,
                // not a FIFO of intermediate edits, after this immutable version completes.
                context.request_repaint_after(Duration::from_millis(8));
                self.updating = true;
                return Ok(());
            }

            let render_source = |source: Arc<ViewSource>, previous: Option<&Arc<RenderedView>>| {
                if let Some(rendered) = previous.filter(|rendered| {
                    rendered.matches(&source.spec, &signature_for(&source), target)
                }) {
                    return Ok(rendered.clone());
                }
                create(self.device().unwrap(), source, previous)
            };
            let rendered_detail = render_source(source.clone(), self.rendered.as_ref())?;
            let rendered_underlay = underlay
                .map(|underlay| {
                    if Arc::ptr_eq(underlay, source) {
                        Ok(rendered_detail.clone())
                    } else {
                        render_source(underlay.clone(), self.underlay_rendered.as_ref())
                    }
                })
                .transpose()?;
            self.rendered = Some(rendered_detail);
            self.underlay_rendered = rendered_underlay;
        }

        self.updating = source.spec != *wanted;
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
    ) -> Result<Option<Arc<RenderedView>>, Error> {
        if self.compare
            && self
                .wanted
                .as_ref()
                .is_some_and(|wanted| !wanted.is_inspection())
            && let Some(reference) = self
                .reference
                .as_ref()
                .filter(|rendered| rendered.spec.frame == frame)
        {
            return Ok(Some(reference.clone()));
        }

        let Some(detail) = &self.rendered else {
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
        let retired = mailbox.input.take();
        mailbox.wanted = None;
        mailbox.underlay = None;
        mailbox.neutral_detail = None;
        mailbox.neutral_underlay = None;
        mailbox.repaired_detail = None;
        drop(mailbox);
        drop(retired);
    }
}

// Physical acquisition follows the latest captured demand while retaining one admission.
#[allow(clippy::too_many_arguments)]
pub(super) fn acquire(
    mailbox: &Mailbox,
    renderers: &crate::renderer::RendererCache,
    kernels: &OnceLock<Arc<ViewKernels>>,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    submissions: &crate::gpu::SubmissionGate,
    source_epoch: &std::sync::atomic::AtomicU64,
    retired: &std::sync::atomic::AtomicBool,
) -> Option<Completion> {
    use std::sync::atomic::Ordering;
    loop {
        if retired.load(Ordering::Relaxed) {
            return None;
        }
        let (spec, input) = {
            let state = mailbox.0.lock().unwrap();
            let demand = state.wanted.as_ref().zip(state.input.as_ref())?;
            (demand.0.clone(), demand.1.clone())
        };

        if source_epoch.load(Ordering::Relaxed) != spec.authority {
            let state = mailbox.0.lock().unwrap();
            if state.wanted.as_ref() != Some(&spec) {
                continue;
            }
            return None;
        }

        let scan = match input.scan() {
            Ok(scan) => scan,
            Err(error) => {
                let state = mailbox.0.lock().unwrap();
                if state.wanted.as_ref() != Some(&spec) {
                    continue;
                }
                return Some(Completion {
                    spec,
                    result: Err(error.to_string()),
                });
            }
        };
        let correction = input.correction.as_ref();
        let needs_underlay = mailbox
            .0
            .lock()
            .unwrap()
            .underlay
            .as_ref()
            .is_none_or(|underlay| {
                !underlay.spec.same_source(&spec)
                    || underlay.spec.frame.composition != spec.frame.composition
                    || underlay.spec.repairs != spec.repairs
            });
        let cancelled = || {
            retired.load(Ordering::Relaxed)
                || source_epoch.load(std::sync::atomic::Ordering::Relaxed) != spec.authority
                || mailbox
                    .0
                    .lock()
                    .unwrap()
                    .wanted
                    .as_ref()
                    .is_none_or(|wanted| {
                        if needs_underlay {
                            !wanted.same_source(&spec)
                                || wanted.frame.composition != spec.frame.composition
                                || wanted.repairs != spec.repairs
                        } else {
                            wanted != &spec
                        }
                    })
        };

        let result =
            (|| {
                let renderer = renderers.get(device, queue)?;
                // Compile off-thread, then publish immutable pipelines independently of
                // acquisition. Painting reads only an already-published handle.
                let kernels = if let Some(kernels) = kernels.get() {
                    kernels.clone()
                } else {
                    let prepared = pollster::block_on(ViewKernels::new(device))?;
                    kernels.get_or_init(|| prepared).clone()
                };

                let specs = if needs_underlay {
                    vec![spec.clone(), spec.underlay()?]
                } else {
                    vec![spec.clone()]
                };
                let mut neutral_sources = Vec::new();
                let mut missing_specs = Vec::new();
                {
                    let state = mailbox.0.lock().unwrap();
                    for wanted in &specs {
                        let neutral = wanted.clone().with_repairs(Default::default());
                        let cached = [&state.neutral_detail, &state.neutral_underlay]
                            .into_iter()
                            .flatten()
                            .find(|source| source.spec == neutral)
                            .cloned();
                        if cached.is_none() {
                            missing_specs.push(neutral);
                        }
                        neutral_sources.push(cached);
                    }
                }

                if !missing_specs.is_empty() {
                    let Some(acquired) =
                        ViewSource::acquire_many(
                            device,
                            queue,
                            submissions,
                            renderer,
                            kernels.clone(),
                            &scan,
                            correction.map(|correction| correction.gpu.clone()),
                            missing_specs,
                            || {
                                retired.load(Ordering::Relaxed)
                                    || source_epoch.load(std::sync::atomic::Ordering::Relaxed)
                                        != spec.authority
                                    || mailbox.0.lock().unwrap().wanted.as_ref().is_none_or(
                                        |wanted| {
                                            !wanted.same_source(&spec)
                                                || wanted.frame.composition
                                                    != spec.frame.composition
                                        },
                                    )
                            },
                        )?
                    else {
                        return Ok(None);
                    };

                    let mut acquired = acquired.into_iter();
                    for neutral_source in &mut neutral_sources {
                        if neutral_source.is_none() {
                            *neutral_source = acquired.next();
                        }
                    }

                    let mut state = mailbox.0.lock().unwrap();
                    if !retired.load(Ordering::Relaxed)
                        && state.wanted.as_ref().is_some_and(|wanted| {
                            wanted.same_source(&spec)
                                && wanted.frame.composition == spec.frame.composition
                        })
                    {
                        state.neutral_detail = neutral_sources[0].clone();
                        if needs_underlay {
                            state.neutral_underlay = neutral_sources[1].clone();
                        }
                    }
                }

                let mut views = Vec::new();
                for (index, (wanted, neutral_source)) in
                    specs.into_iter().zip(neutral_sources).enumerate()
                {
                    let previous = {
                        let state = mailbox.0.lock().unwrap();
                        if index == 0 {
                            state.repaired_detail.clone()
                        } else {
                            state.underlay.clone()
                        }
                    };
                    let Some(view) = ViewSource::reconstruct_repairs(
                        device,
                        queue,
                        submissions,
                        &scan,
                        correction.map(|correction| correction.gpu.clone()),
                        neutral_source.expect("acquired neutral physical view"),
                        wanted.repairs,
                        previous,
                        cancelled,
                    )?
                    else {
                        return Ok(None);
                    };
                    views.push(view);
                }
                Ok(Some(views))
            })()
            .map_err(|error: Error| error.to_string());

        if retired.load(Ordering::Relaxed) {
            return None;
        }
        let mut state = mailbox.0.lock().unwrap();
        let result = result.map(|views| {
            views.map(|mut views| {
                if needs_underlay
                    && state.wanted.as_ref().is_some_and(|wanted| {
                        wanted.same_source(&spec)
                            && wanted.frame.composition == spec.frame.composition
                            && wanted.repairs == spec.repairs
                    })
                {
                    state.underlay = views.pop();
                }
                let detail = views.remove(0);
                if state
                    .wanted
                    .as_ref()
                    .is_some_and(|wanted| wanted.same_source(&spec))
                {
                    state.repaired_detail = Some(detail.clone());
                }
                detail
            })
        });

        if state.wanted.as_ref() == Some(&spec) {
            drop(state);
            return match result {
                Ok(Some(view)) => Some(Completion {
                    spec,
                    result: Ok(view),
                }),
                Ok(None) => None,
                Err(error) => Some(Completion {
                    spec,
                    result: Err(error),
                }),
            };
        }

        drop(state);
        mailbox.1.notify(usize::MAX);
        // A changed view replaces the pending request, never queues another acquisition worker.
    }
}

impl Editor {
    pub(super) fn admit_view_reference(&mut self, frame: FrameRaster) -> bool {
        let pixels = frame.output.iter().fold(16usize, |bytes, dimension| {
            bytes.saturating_mul(*dimension as usize)
        });
        if self.source {
            self.discard_original();
            self.thumbnails.reserve_view(0);
            return false;
        }

        let reservation = if self.original.refining.is_some() || self.original.settled.is_none() {
            pixels.saturating_mul(2).saturating_add(3 << 20)
        } else {
            pixels
        };
        let allowed = if self.working.compare {
            self.thumbnails.reserve_view(0);
            true
        } else if self.original.settled.is_some() {
            self.thumbnails.reserve_view(reservation)
        } else {
            self.thumbnails.reserve_view(0);
            false
        };
        if !allowed {
            self.discard_original();
            self.working.reference = None;
        }

        allowed
    }

    pub(super) fn working_view(
        &mut self,
        frame: FrameRaster,
        input: &ProcessingInput,
        context: &egui::Context,
        cx: &UiContext<'_, Self>,
    ) -> Result<Option<Arc<RenderedView>>, Error> {
        let previous = self.working.displayed.take();
        if self
            .working
            .wanted
            .as_ref()
            .is_some_and(|wanted| wanted.is_inspection() != self.source)
        {
            self.working.retire_support();
        }
        if !self.working.enabled() {
            return Ok(None);
        }

        if self.busy
            && self.document_pending.is_none()
            && self.pick.is_none()
            && self.document.visit.crop_edit.is_none()
            && self.flat_edit.is_none()
        {
            let mut mailbox = self.working.mailbox.0.lock().unwrap();
            mailbox.wanted = None;
            drop(mailbox);

            if self.source && self.working.wanted.is_some() {
                self.working
                    .render_inspection(input, self.target, context)?;
            }

            self.working.displayed =
                self.working
                    .present(frame, previous.as_ref())?
                    .filter(|rendered| {
                        rendered.spec.authority == self.id && rendered.target == self.target
                    });
            self.working.updating = !self.working.support_ready()
                || self.working.displayed.as_ref().is_none_or(|rendered| {
                    rendered.spec.frame != frame || rendered.processing != input.signature()
                });
            return Ok(self.working.displayed.clone());
        }

        if self.document_pending.is_some() {
            // Candidate preparation must not retire the accepted view before replacement succeeds.
            self.working.displayed = previous;
            return Ok(self.working.hold(self.id, self.target));
        }

        if self.pending_preview.is_some()
            || (input.repairs.0.is_empty()
                && (self.pick.is_some()
                    || self.document.pick_return.is_some()
                    || self.document.visit.crop_edit.is_some()
                    || self.flat_edit.is_some()))
        {
            self.working.clear();
            return Ok(None);
        }

        let Some(ready) = &self.document.visit.ready else {
            self.working.clear();
            return Ok(None);
        };

        let source_input = self.source_context().expect("ready working source");
        if self.source {
            let matrix = glam::DMat3::from_cols_array_2d(&ready.record.development.camera_to_ap1);
            let spec = ViewSourceSpec::inspection(
                self.id,
                frame,
                matrix,
                self.correction
                    .as_ref()
                    .map(|correction| correction.gpu.field().clone()),
            )?;
            self.request_working(spec, source_input, cx);
            self.working
                .render_inspection(input, self.target, context)?;
            self.working.reference = None;

            self.working.displayed =
                self.working
                    .present(frame, previous.as_ref())?
                    .filter(|rendered| {
                        rendered.spec.authority == self.id && rendered.target == self.target
                    });
            self.working.updating |= self.working.displayed.as_ref().is_none_or(|rendered| {
                self.working
                    .wanted
                    .as_ref()
                    .is_none_or(|wanted| !rendered.matches(wanted, &input.signature(), self.target))
            });
            return Ok(self.working.displayed.clone());
        }

        let base_input = self.input();
        if base_input.is_negative() && base_input.calibration().is_none() {
            self.working.clear();
            return Ok(None);
        }
        let settings = match base_input.calibration() {
            Some(base) => base_input.with_calibration(self.processing_calibration(base)?),
            None => base_input,
        };
        let matrix = glam::DMat3::from_cols_array_2d(&ready.record.development.camera_to_ap1);
        let snapshot = match input.grade() {
            Some(grade) => grade.snapshot().clone(),
            None => PhotoGradeSnapshot::new(self.photo, vec![])?,
        };
        let spec = ViewSourceSpec::new(
            self.id,
            frame,
            settings,
            matrix,
            self.correction
                .as_ref()
                .map(|correction| correction.gpu.field().clone()),
        )?
        .with_repairs(input.repairs.clone());
        self.request_working(spec, source_input, cx);
        self.working
            .render(&settings, self.roll, snapshot, input, self.target, context)?;

        self.working.reference = self
            .original_rendered(OriginalPurpose::Reference, input, matrix)?
            .filter(|rendered| rendered.spec.frame == frame);

        self.working.displayed =
            self.working
                .present(frame, previous.as_ref())?
                .filter(|rendered| {
                    rendered.spec.authority == self.id && rendered.target == self.target
                });
        self.working.updating |= self.working.displayed.as_ref().is_none_or(|rendered| {
            self.working
                .wanted
                .as_ref()
                .is_none_or(|wanted| !rendered.matches(wanted, &input.signature(), self.target))
        });
        if self.working.displayed.is_none() && self.working.error.is_none() {
            context.request_repaint_after(Duration::from_millis(16));
        }

        Ok(self.working.displayed.clone())
    }
}
