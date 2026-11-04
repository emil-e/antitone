use super::*;
use crate::gpu::photograph::request::PhotographRequest;

/// An independently prepared source image and its presentation resources.
#[derive(Default)]
pub(in crate::editor) struct SourceView {
    pub ready: Option<Ready>,
    original: original_view::OriginalView,
    preview: Option<canvas_view::Preview>,
    #[cfg(test)]
    pub paint_observer: Option<Arc<SelectionTiming>>,
}

impl SourceView {
    pub fn source(&self) -> Option<source::Context> {
        let ready = self.ready.as_ref()?;
        Some(source::Context {
            source: ready.developed.clone(),
            reference: ready.reference.clone(),
            correction: None,
        })
    }
    #[cfg(test)]
    pub fn test_original(&self) -> &original_view::OriginalView {
        &self.original
    }
    #[cfg(test)]
    pub fn test_original_mut(&mut self) -> &mut original_view::OriginalView {
        &mut self.original
    }

    pub fn paint(
        &mut self,
        canvas: &mut Canvas<'_>,
        layout: CanvasLayout,
        editor: &mut Editor,
        services: Option<&source::Services>,
        cx: &UiContext<'_, Editor>,
    ) -> bool {
        let Some(ready) = &self.ready else {
            return false;
        };
        let preview = ready
            .developed
            .metadata()
            .source_preview(ready.developed.fixed_wb());
        let capture = match editor
            .source_preview_parameters(preview, editor.source_exposure)
            .and_then(PhotographRequest::inspection)
        {
            Ok(capture) => capture,
            Err(error) => {
                canvas
                    .ui
                    .label(format!("Source inspection unavailable: {error}"));
                return false;
            }
        };
        let Some(device) = services
            .map(|services| services.device.clone())
            .or_else(|| editor.document.visit.working_view.device().cloned())
        else {
            return false;
        };
        let Some(queue) = services
            .map(|services| services.queue.clone())
            .or_else(|| editor.document.visit.working_view.queue().cloned())
        else {
            return false;
        };
        let ppp = canvas.ui.ctx().pixels_per_point();
        let composition = layout.composition;
        let view = ViewRequest {
            area: layout.visible_area(),
            output: layout.output_size(layout.visible_area(), ppp),
            frame: None,
        };
        self.original.observe(
            OriginalPurpose::Tool,
            original_view::Demand {
                capture: capture.clone(),
                repairs: Default::default(),
                target: editor.document.target,
                view: Some(view),
                composition,
                source: self.source().expect("prepared source"),
            },
        );
        if !editor.busy
            && let Some(services) = services
        {
            self.original.request_with(
                services.clone(),
                editor.submissions.clone(),
                canvas.ui.ctx(),
                cx,
            );
        }
        let original = match self.original.rendered(OriginalPurpose::Tool) {
            Ok(image) => image,
            Err(error) => {
                editor.status = error.to_string();
                None
            }
        };
        let draw = layout.image.intersect(layout.canvas);
        let paint = if let Some(rendered) = original {
            self.preview = None;
            ready.proxy.release_output();
            let mapping = rendered.mapping(layout.frame_raster(ppp));
            Some(ImagePaint::rendered(
                rendered,
                mapping,
                editor.submissions.clone(),
                canvas.ui.ctx().clone(),
            ))
        } else {
            let frame = match preview_frame(composition, ready.proxy.size(), false) {
                Ok(frame) => frame,
                Err(error) => {
                    editor.status = error.to_string();
                    return false;
                }
            };
            let reused = self
                .preview
                .as_ref()
                .filter(|(proxy, previous, rendered)| {
                    proxy.same_source(&ready.proxy)
                        && *previous == frame
                        && rendered.capture == capture
                        && rendered.has_correction(None)
                        && rendered.target == editor.document.target
                })
                .map(|entry| entry.2.clone());
            let rendered = reused.or_else(|| {
                let kernels = editor.document.visit.working_view.kernels.get()?.clone();
                let spec = crate::gpu::view::request::SourceRequest::for_frame(frame).ok()?;
                let (source, processed) = ready
                    .proxy
                    .preview(&queue, &editor.submissions, kernels, &capture, None, spec)
                    .ok()?;
                let rendered = crate::gpu::view::RenderedView::from_processed(
                    &device,
                    source,
                    capture,
                    editor.document.target,
                    processed,
                    self.preview.as_ref().map(|entry| &entry.2),
                )
                .ok()?;
                self.preview = Some((ready.proxy.clone(), frame, rendered.clone()));
                Some(rendered)
            });
            rendered.map(|rendered| {
                let mapping = if draw == layout.image {
                    crate::geometry::TextureMapping::IDENTITY
                } else {
                    let extent = layout.image.size();
                    let origin = (draw.min - layout.image.min) / extent;
                    let max = (draw.max - layout.image.min) / extent;
                    crate::geometry::TextureMapping {
                        origin: [origin.x, origin.y],
                        x_axis: [max.x - origin.x, 0.],
                        y_axis: [0., max.y - origin.y],
                    }
                };
                ImagePaint::rendered(
                    rendered,
                    mapping,
                    editor.submissions.clone(),
                    canvas.ui.ctx().clone(),
                )
            })
        };
        let Some(paint) = paint else {
            canvas.ui.label("Preview unavailable");
            return false;
        };
        editor.canvas_frame.displayed =
            paint
                .displayed()
                .map(|(view, mapping)| canvas_view::Displayed {
                    view,
                    mapping,
                    size: [draw.width(), draw.height()].map(|v| (v * ppp).round().max(1.) as u32),
                });
        #[cfg(test)]
        let paint = SelectionPaint::new(paint, self.paint_observer.clone(), false);
        canvas
            .ui
            .painter()
            .with_clip_rect(layout.canvas)
            .add(egui_wgpu::Callback::new_paint_callback(draw, paint));
        true
    }
}
