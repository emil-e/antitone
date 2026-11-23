use super::{Editor, Ready, Viewport, original_view::OriginalView};
use crate::{
    Error,
    application::UiContext,
    frame_raster::FrameRaster,
    gpu::{
        flat::GpuFlatField,
        photograph::request::PhotographRequest,
        view::{RenderedView, ViewImage, ViewKernels},
    },
    rendering::OutputTarget,
    viewer::GpuProxy,
};
use futures_util::FutureExt;
use std::sync::Arc;

pub(super) struct PreviewDemand {
    pub proxy: GpuProxy,
    pub frame: FrameRaster,
    pub capture: PhotographRequest,
    pub field: Option<Arc<GpuFlatField>>,
    pub target: OutputTarget,
}

impl PreviewDemand {
    fn matches(&self, other: &Self) -> bool {
        self.proxy.same_source(&other.proxy)
            && self.frame == other.frame
            && self.capture == other.capture
            && self.field.as_ref().map(|field| field.field())
                == other.field.as_ref().map(|field| field.field())
            && self.target == other.target
    }

    fn present(&self, image: Arc<ViewImage>) -> Result<Arc<RenderedView>, Error> {
        let device = image.context.device().clone();
        RenderedView::from_image(&device, image, self.capture.clone(), self.target)
    }
}

/// The fallback owns only its current request. Replacing it drops any pending production wait.
#[derive(Default)]
pub(super) struct Preview(Option<(PreviewDemand, PreviewState)>);

enum PreviewState {
    Pending(async_executor::Task<Result<Option<Arc<ViewImage>>, Error>>),
    Ready(Arc<RenderedView>),
    Unavailable,
    Failed(Error),
}

impl Preview {
    pub fn clear(&mut self) {
        self.0 = None;
    }

    pub fn pending(&self) -> bool {
        matches!(&self.0, Some((_, PreviewState::Pending(_))))
    }

    pub fn rendered(&self) -> Option<&Arc<RenderedView>> {
        match &self.0 {
            Some((_, PreviewState::Ready(view))) => Some(view),
            _ => None,
        }
    }

    pub fn error(&self) -> Option<&Error> {
        match &self.0 {
            Some((_, PreviewState::Failed(error))) => Some(error),
            _ => None,
        }
    }

    #[cfg(test)]
    pub fn demand(&self) -> Option<&PreviewDemand> {
        self.0.as_ref().map(|(demand, _)| demand)
    }

    pub fn render(
        &mut self,
        demand: PreviewDemand,
        kernels: Option<Arc<ViewKernels>>,
        gpu: &crate::gpu::submission::Queue,
        context: &egui::Context,
        cx: &UiContext<'_, Editor>,
    ) -> Option<Arc<RenderedView>> {
        if !self
            .0
            .as_ref()
            .is_some_and(|(current, _)| current.matches(&demand))
        {
            self.clear();
        }
        if kernels.is_none() && self.pending() {
            self.clear();
        }
        if self.0.is_none() {
            let state =
                PreviewState::settle(Self::start(&demand, kernels?, gpu, context, cx), context);
            self.0 = Some((demand, state));
        }
        if let Some((demand, state)) = &mut self.0
            && let PreviewState::Pending(task) = state
            && task.is_finished()
        {
            let result = task.now_or_never().expect("finished proxy production");
            let result = result.and_then(|image| match image {
                Some(image) => demand.present(image).map(PreviewState::Ready),
                None => Ok(PreviewState::Unavailable),
            });
            *state = PreviewState::settle(result, context);
        }
        self.rendered().cloned()
    }

    fn start(
        demand: &PreviewDemand,
        kernels: Arc<ViewKernels>,
        queue: &crate::gpu::submission::Queue,
        context: &egui::Context,
        cx: &UiContext<'_, Editor>,
    ) -> Result<PreviewState, Error> {
        let spec = demand.capture.source_request(demand.frame)?;
        let request = demand.proxy.preview(
            kernels,
            &demand.capture,
            demand.field.clone(),
            spec,
            demand.target,
        )?;
        if let Some(mut images) = request.ready() {
            return demand.present(images.remove(0)).map(PreviewState::Ready);
        }

        let (queue, context) = (queue.clone(), context.clone());
        let task = cx.spawn(move |_| async move {
            let result = request
                .produce(&queue, || false)
                .await
                .map(|images| images.map(|mut images| images.remove(0)));
            context.request_repaint();
            result
        });
        Ok(task.map_or(PreviewState::Unavailable, PreviewState::Pending))
    }
}

impl PreviewState {
    fn settle(result: Result<Self, Error>, context: &egui::Context) -> Self {
        result.unwrap_or_else(|error| {
            log::warn!(error = error.to_string(); "proxy_preview_failed");
            context.request_repaint();
            Self::Failed(error)
        })
    }
}

/// The photograph and sampling actually passed to this canvas's paint callback.
pub(super) struct Displayed {
    pub view: Arc<crate::gpu::view::RenderedView>,
    pub mapping: crate::geometry::TextureMapping,
    pub size: [u32; 2],
}

/// Image and interaction state selected by the shared canvas and viewer controls.
#[derive(Default)]
pub(super) struct CanvasView {
    pub viewport: Viewport,
    pub(super) input: CanvasInput,
}

#[derive(Default)]
pub(super) struct CanvasInput {
    pub ready: Option<Ready>,
    pub last_layout: Option<(crate::viewport::CanvasLayout, f32)>,
    pub preview: Preview,
    pub selection_timing: Option<Arc<super::SelectionTiming>>,
    pub original: OriginalView,
}

impl CanvasView {
    pub fn retire_input(&mut self) {
        self.input = Default::default();
    }
}

impl std::ops::Deref for CanvasView {
    type Target = CanvasInput;
    fn deref(&self) -> &Self::Target {
        &self.input
    }
}
impl std::ops::DerefMut for CanvasView {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.input
    }
}

impl super::Editor {
    pub(super) fn active_canvas(&self) -> &CanvasView {
        &self.document.visit.canvas
    }

    pub(super) fn active_canvas_mut(&mut self) -> &mut CanvasView {
        &mut self.document.visit.canvas
    }

    /// Fit the photograph's view, not the viewport of a temporary tool.
    pub(super) fn fit_photo_view(&mut self) {
        self.active_canvas_mut().viewport.fit();
    }

    pub(super) fn canvas_inspection(&self) -> bool {
        self.document.source
    }
}
