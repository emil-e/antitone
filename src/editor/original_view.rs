//! Purpose-bound original-sample reconstruction, independent of normal working-view acceptance.
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OriginalPurpose {
    Reference,
    Tool,
}
pub(super) struct Request {
    pub revision: u64,
    pub generation: Arc<AtomicU64>,
    pub input: ProcessingInput,
    pub target: OutputTarget,
    pub view: ViewRequest,
    pub source: source::Context,
}

pub(super) type Signature = (
    ProcessingSignature,
    OutputTarget,
    Option<ViewRequest>,
    Composition,
);

pub(super) struct OriginalView {
    pub generation: Arc<AtomicU64>,
    pub signature: Option<Signature>,
    purpose: Option<OriginalPurpose>,
    pub settled: Option<(u64, ViewRequest, Arc<GpuImage>)>,
    rendered: Option<Arc<crate::gpu::view::RenderedView>>,
    pub refining: Option<(u64, ViewRequest)>,
    pub last_edit: std::time::Instant,
    pub refinement_error: Option<u64>,
}

impl Default for OriginalView {
    fn default() -> Self {
        Self::new(Arc::default())
    }
}

impl Drop for OriginalView {
    fn drop(&mut self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
    }
}

impl OriginalView {
    pub fn new(generation: Arc<AtomicU64>) -> Self {
        Self {
            generation,
            signature: None,
            purpose: None,
            settled: None,
            rendered: None,
            refining: None,
            last_edit: std::time::Instant::now(),
            refinement_error: None,
        }
    }

    pub fn revision(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }

    pub fn invalidate(&mut self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.signature = None;
        self.purpose = None;
        self.settled = None;
        self.rendered = None;
        self.refinement_error = None;
        self.refining = None;
        // Physical admission belongs to the running service, not this logical demand.
    }

    pub fn discard(&mut self) {
        if self.signature.is_some() || self.settled.is_some() {
            self.invalidate();
        } else {
            self.cancel_pending();
        }
    }

    pub fn cancel_pending(&mut self) {
        if self
            .refining
            .is_some_and(|(revision, _)| revision == self.revision())
        {
            let revision = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
            if let Some((accepted, _, _)) = &mut self.settled {
                *accepted = revision;
            }
        }
        self.refining = None;
    }

    pub fn observe(&mut self, purpose: OriginalPurpose, signature: Signature) {
        if self.purpose != Some(purpose) || self.signature.as_ref() != Some(&signature) {
            self.invalidate();
            self.purpose = Some(purpose);
            self.signature = Some(signature);
            self.last_edit = std::time::Instant::now();
        }
    }

    pub fn leave(&mut self) {
        if self.purpose == Some(OriginalPurpose::Tool)
            || (self.purpose.is_some() && self.settled.is_none())
        {
            self.invalidate();
        } else {
            self.cancel_pending();
        }
    }

    pub fn idle(&self) -> bool {
        self.refining.is_none()
            && (self.signature.is_none()
                || self.settled.is_some()
                || self.refinement_error == Some(self.revision()))
            && self.last_edit.elapsed() >= Duration::from_millis(250)
    }

    pub fn request(
        &self,
        input: &ProcessingInput,
        target: OutputTarget,
        source: source::Context,
    ) -> Option<Request> {
        let view = self.signature.as_ref()?.2?;
        let revision = self.revision();
        if self.refining.is_some()
            || self.settled.is_some()
            || self.refinement_error == Some(revision)
            || self.last_edit.elapsed() < Duration::from_millis(250)
        {
            return None;
        }
        Some(Request {
            revision,
            generation: self.generation.clone(),
            input: input.clone(),
            target,
            view,
            source,
        })
    }

    pub fn receive(
        &mut self,
        revision: u64,
        result: Result<Option<Arc<GpuImage>>, String>,
    ) -> Option<String> {
        let (_, view) = self.refining.filter(|(running, _)| *running == revision)?;
        self.refining = None;
        if self.revision() != revision {
            return None;
        }
        match result {
            Ok(Some(image)) => self.settled = Some((revision, view, image)),
            Ok(None) => {}
            Err(error) => {
                self.refinement_error = Some(revision);
                return Some(format!("Original view failed: {error}"));
            }
        }
        None
    }
}

impl Editor {
    /// Both comparison and precision tools present the same accepted reconstruction product.
    pub(super) fn original_rendered(
        &mut self,
        purpose: OriginalPurpose,
        input: &ProcessingInput,
        matrix: glam::DMat3,
    ) -> Result<Option<Arc<crate::gpu::view::RenderedView>>, Error> {
        use crate::{
            frame_raster::FrameRaster,
            gpu::view::{RenderedView, ViewSourceSpec},
        };
        let Some((revision, view, image)) = self.document.visit.grade.original.settled.as_ref()
        else {
            return Ok(None);
        };
        if self.document.visit.grade.original.purpose != Some(purpose)
            || *revision != self.document.visit.grade.original.revision()
            || self
                .document
                .visit
                .grade
                .original
                .signature
                .as_ref()
                .is_none_or(|signature| {
                    signature.0 != input.signature()
                        || signature.1 != self.document.target
                        || signature.2 != Some(*view)
                })
        {
            return Ok(None);
        }
        // A rectangular tool request is sampled in native axes, even when the canvas has
        // quarter-turns or reflections. A straightened request already carries its affine frame.
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
        let field = if self.document.flat_edit.is_some() {
            None
        } else {
            self.document
                .correction
                .as_ref()
                .map(|correction| correction.settings.map.clone())
        };
        let Some(ready) = self.document.visit.ready.as_ref() else {
            return Ok(None);
        };
        let identity = crate::gpu::execution::source::Identity::camera(ready.developed.identity());
        let spec = if input.pipeline().is_none() {
            ViewSourceSpec::inspection(self.id, frame, matrix, field)?
        } else {
            let settings = match self.document.input.calibration() {
                Some(base) => self
                    .document
                    .input
                    .with_calibration(self.processing_calibration(base)?),
                None => self.document.input,
            };
            let source = self.document.state.project.source(
                self.document
                    .visit
                    .current_source
                    .ok_or(Error::UnusableSample("original source unavailable"))?,
            )?;
            ViewSourceSpec::new(
                self.id,
                frame,
                settings,
                self.document.positive_matrix(source, settings)?,
                field,
            )?
        }
        .with_identity(identity)
        .with_repairs(input.repairs.clone())
        .with_pipeline(input.pixel_pipeline()?)?;
        if let Some(rendered) = &self.document.visit.grade.original.rendered
            && Arc::ptr_eq(&rendered.image, image)
            && rendered.matches(&spec, &input.signature(), self.document.target)
        {
            return Ok(Some(rendered.clone()));
        }
        let rendered =
            RenderedView::reference(image.clone(), spec, input.signature(), self.document.target);
        self.document.visit.grade.original.rendered = Some(rendered.clone());
        Ok(Some(rendered))
    }
}
