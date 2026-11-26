use super::*;
use crate::{photographic::graph::request::PhotographRequest, viewport::CanvasLayout};

#[cfg(test)]
mod immutable_source_tests;
#[cfg(test)]
mod tests;

#[derive(Default)]
pub(super) struct Frame {
    pub input_id: Option<egui::Id>,
    pub layout: Option<(CanvasLayout, f32)>,
    pub displayed: Option<canvas_view::Displayed>,
    original_requested: bool,
}

pub(super) struct Viewer<'a> {
    pub ui: &'a mut egui::Ui,
    zoom: Option<f32>,
    fullscreen: bool,
}

impl Viewer<'_> {
    pub fn show<T: tool::Tool + ?Sized>(
        &mut self,
        tool: &mut T,
        editor: &mut Editor,
        cx: &UiContext<'_, Editor>,
        controls: impl FnOnce(&mut Editor, &mut egui::Ui, bool),
        paint: impl FnOnce(
            &mut T,
            &mut Surface<'_>,
            &mut Editor,
            &UiContext<'_, Editor>,
        ) -> Option<CanvasLayout>,
    ) -> Option<CanvasLayout> {
        let bar = editor.viewer_bar(self.ui, controls);
        self.fullscreen |= bar.fullscreen;
        if bar.fit {
            tool.viewport(editor).fit();
        }
        if let Some(action) = bar.action {
            tool.action(action, editor, cx);
        }
        if !editor.has_hosted_tool() {
            let fullscreen = editor.fullscreen.is_some();
            ui::viewer_surface(self.ui, fullscreen, |ui| {
                editor.viewer_canvas(ui, bar.zoom.or(self.zoom), cx)
            });
            return None;
        }
        let zoom = bar.zoom.or(self.zoom);
        let fullscreen = editor.fullscreen.is_some();
        editor.canvas_frame = Frame::default();
        ui::viewer_surface(self.ui, fullscreen, |ui| {
            let size = ui
                .available_rect_before_wrap()
                .intersect(ui.clip_rect())
                .size()
                .max(egui::vec2(1., 1.));
            let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
            let selected = editor.selected_view().map(|(area, proxy, pending)| {
                (editor.normal_canvas_composition(area), proxy, pending)
            });
            let enabled = editor.canvas_input_allowed(ui, rect);
            editor.canvas_frame.input_id = enabled.then_some(response.id);
            let layout = ui
                .add_enabled_ui(enabled, |ui| {
                    paint(
                        tool,
                        &mut Surface {
                            ui,
                            response: &response,
                            zoom_action: zoom,
                            selected,
                        },
                        editor,
                        cx,
                    )
                })
                .inner;
            if let Some(layout) = layout {
                editor.canvas_frame.layout = Some((layout, ui.ctx().pixels_per_point()));
            } else if let Some(task) = editor.activities.view() {
                task.viewer(ui, rect);
            }
            layout
        })
    }
}

/// A canvas surface; its owner chooses which image to compose on it.
pub(super) struct Surface<'a> {
    pub ui: &'a mut egui::Ui,
    pub response: &'a egui::Response,
    pub zoom_action: Option<f32>,
    selected: Option<(Composition, crate::viewer::GpuProxy, bool)>,
}

impl Surface<'_> {
    pub fn selected(&mut self) -> Option<Canvas<'_>> {
        let (composition, proxy, pending) = self.selected.clone()?;
        Some(Canvas {
            ui: self.ui,
            response: self.response,
            composition,
            zoom_action: self.zoom_action,
            proxy,
            pending,
        })
    }

    pub fn image(
        &mut self,
        composition: Composition,
        proxy: crate::viewer::GpuProxy,
    ) -> Canvas<'_> {
        Canvas {
            ui: self.ui,
            response: self.response,
            composition,
            zoom_action: self.zoom_action,
            proxy,
            pending: false,
        }
    }

    pub fn photograph(
        &mut self,
        editor: &mut Editor,
        cx: &UiContext<'_, Editor>,
    ) -> Option<CanvasLayout> {
        let mut image = self.selected()?;
        let ppp = image.ui.ctx().pixels_per_point();
        let layout = editor.canvas_layout(
            image.response.rect,
            image.composition,
            ppp,
            image.zoom_action,
        );
        let (layout, _) = editor.canvas_navigation(image.ui, image.response, layout, true);
        image.paint_image(editor, layout, false, cx);
        Some(layout)
    }
}

/// Photographic endpoint requested by a canvas consumer.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ImageMode {
    Photograph,
    Crop,
    SourceSample,
}

impl ImageMode {
    pub fn is_sample(self) -> bool {
        self == Self::SourceSample
    }
    pub fn inspection(self, normal: bool) -> bool {
        match self {
            Self::Photograph | Self::Crop => normal,
            Self::SourceSample => true,
        }
    }
}

/// The selected image and native canvas interaction for one immediate-mode pass.
pub(super) struct Canvas<'a> {
    pub ui: &'a mut egui::Ui,
    pub response: &'a egui::Response,
    pub composition: Composition,
    pub zoom_action: Option<f32>,
    pub proxy: crate::viewer::GpuProxy,
    pub pending: bool,
}

impl Canvas<'_> {
    pub fn layout(
        &self,
        viewport: &mut Viewport,
        mut relayout: impl FnMut(&mut Viewport) -> CanvasLayout,
    ) -> CanvasLayout {
        let mut layout = relayout(viewport);
        if let Some(zoom) = self.zoom_action {
            let ppp = self.ui.ctx().pixels_per_point();
            viewport.zoom_at(
                layout,
                ppp,
                if zoom > 0. {
                    zoom
                } else {
                    layout.scale * ppp * -zoom
                },
                self.response.rect.center(),
            );
            layout = relayout(viewport);
        }
        layout
    }

    pub fn paint_image(
        &mut self,
        editor: &mut Editor,
        layout: CanvasLayout,
        source_outside_frame: bool,
        cx: &UiContext<'_, Editor>,
    ) -> bool {
        let image = if source_outside_frame {
            ImageMode::Crop
        } else {
            ImageMode::Photograph
        };
        self.paint(editor, layout, image, cx)
    }

    pub fn paint_source_sample(
        &mut self,
        editor: &mut Editor,
        layout: CanvasLayout,
        cx: &UiContext<'_, Editor>,
    ) -> bool {
        self.paint(editor, layout, ImageMode::SourceSample, cx)
    }

    fn paint(
        &mut self,
        editor: &mut Editor,
        layout: CanvasLayout,
        image: ImageMode,
        cx: &UiContext<'_, Editor>,
    ) -> bool {
        editor.document.visit.canvas.last_layout = Some((layout, self.ui.ctx().pixels_per_point()));
        if let Some((draw, paint)) =
            editor.canvas_image(self.ui, layout, self.proxy.clone(), self.pending, image, cx)
        {
            self.ui.painter().with_clip_rect(self.response.rect).add(
                egui_wgpu::Callback::new_paint_callback(
                    draw,
                    SelectionPaint::new(
                        paint,
                        editor.active_canvas().selection_timing.clone(),
                        self.pending,
                    ),
                ),
            );
            true
        } else {
            false
        }
    }
}
fn preview_frame(
    composition: Composition,
    proxy_size: [u32; 2],
    native_axes: bool,
) -> Result<crate::frame_raster::FrameRaster, Error> {
    let composition = if native_axes {
        // Crop artwork supplies the affine mapping; retain the source outside the draft frame.
        Composition::new(
            composition.area(),
            RollOrientation::default(),
            PhotoOrientation::default(),
        )?
    } else {
        composition
    };
    let area = composition.area();
    let scale = (f64::from(proxy_size[0]) / f64::from(area.width))
        .min(f64::from(proxy_size[1]) / f64::from(area.height));
    let mut frame = crate::frame_raster::FrameRaster::full(composition)?;
    frame.output = composition
        .size()
        .map(|v| (f64::from(v) * scale).round().max(1.) as u32);
    Ok(frame)
}

fn preview_mapping(image: egui::Rect, draw: egui::Rect) -> crate::geometry::TextureMapping {
    use crate::geometry::TextureMapping;
    if draw == image {
        return TextureMapping::IDENTITY;
    }
    let extent = image.size();
    let origin = (draw.min - image.min) / extent;
    let max = (draw.max - image.min) / extent;
    TextureMapping {
        origin: [origin.x, origin.y],
        x_axis: [max.x - origin.x, 0.],
        y_axis: [0., max.y - origin.y],
    }
}

impl Editor {
    pub(super) fn refresh_canvas_preview(&mut self) {
        if self.document.visit.canvas.ready.is_none() {
            self.document.visit.working_view.clear();
        }
        self.seed_thumbnail();
        self.validate_pending_preview();
    }

    pub(super) fn show_viewer(
        &mut self,
        ui: &mut egui::Ui,
        menu_zoom: Option<f32>,
        cx: &UiContext<'_, Self>,
    ) -> super::thumbnails::ThumbnailProvider {
        self.canvas_frame.original_requested = false;
        if self.has_hosted_tool() {
            let mut viewer = Viewer {
                ui,
                zoom: menu_zoom,
                fullscreen: false,
            };
            self.with_current_tool(|tool, editor| tool.canvas(&mut viewer, editor, cx));
            if viewer.fullscreen {
                self.toggle_fullscreen(viewer.ui.ctx());
            }
        } else {
            let bar = self.viewer_bar(ui, Editor::photograph_controls);
            if bar.fullscreen {
                self.toggle_fullscreen(ui.ctx());
            }
            let fullscreen = self.fullscreen.is_some();
            if self.showing_lightbox() {
                self.observe_original(None);
                return ui::viewer_surface(ui, fullscreen, |ui| self.show_lightbox(ui, cx));
            }
            if bar.fit {
                self.document.visit.canvas.viewport.fit();
            }
            if let Some(action) = bar.action {
                match action {
                    tool::Action::Primary => self.apply_pick(cx),
                    tool::Action::Dismiss => self.finish_pick(cx),
                }
            }
            ui::viewer_surface(ui, fullscreen, |ui| {
                self.viewer_canvas(ui, bar.zoom.or(menu_zoom), cx)
            });
        }
        if !self.canvas_frame.original_requested {
            self.observe_original(None);
        }
        super::thumbnails::ThumbnailProvider::default()
    }

    #[cfg(test)]
    pub(super) fn hosted_viewer(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        zoom_action: Option<f32>,
        enabled: bool,
        cx: &UiContext<'_, Self>,
    ) -> Option<CanvasLayout> {
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(response.rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        if !enabled || !ui.is_enabled() {
            child.disable();
        }
        let mut viewer = Viewer {
            ui: &mut child,
            zoom: zoom_action,
            fullscreen: false,
        };
        self.with_current_tool(|tool, editor| tool.canvas(&mut viewer, editor, cx))
            .flatten()
    }

    /// The accepted source framing before any active crop draft, picker or review override.
    pub(super) fn source_composition(&self, source: &Source) -> Composition {
        let photo_orientation = if self.document.visit.current_source == Some(source.id) {
            self.photo_orientation()
        } else {
            source.orientation
        };
        let crop = if self.document.visit.current_source == Some(source.id) {
            self.photo_crop()
        } else {
            source.crop
        };
        Composition::with_crop(
            source.development.active_area.into(),
            self.document.project.orientation,
            photo_orientation,
            crop,
        )
        .expect("accepted source composition")
    }

    pub(super) fn normal_canvas_composition(&self, area: Rect) -> Composition {
        Composition::with_crop(
            area,
            self.document.project.orientation,
            self.photo_orientation(),
            self.photo_crop(),
        )
        .expect("accepted crop belongs to the loaded source")
    }

    #[cfg(test)]
    pub(super) fn canvas_composition(&mut self, area: Rect) -> Composition {
        let normal = self.normal_canvas_composition(area);
        if self.current_tool().is_none() {
            normal
        } else {
            self.with_current_tool(|tool, _| tool.composition(normal))
                .unwrap()
        }
    }

    fn canvas_layout(
        &mut self,
        canvas: egui::Rect,
        composition: Composition,
        pixels_per_point: f32,
        zoom_action: Option<f32>,
    ) -> CanvasLayout {
        let viewport = &mut self.active_canvas_mut().viewport;
        let mut layout = viewport.layout_composition(canvas, composition, pixels_per_point);
        if let Some(zoom) = zoom_action {
            viewport.zoom_at(
                layout,
                pixels_per_point,
                if zoom > 0. {
                    zoom
                } else {
                    layout.scale * pixels_per_point * -zoom
                },
                canvas.center(),
            );
            layout = viewport.layout_composition(canvas, composition, pixels_per_point);
        }
        layout
    }

    fn canvas_navigation(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        geometry: CanvasLayout,
        canvas_input: bool,
    ) -> (CanvasLayout, bool) {
        let pixels_per_point = ui.ctx().pixels_per_point();
        let (geometry, panning) = if !canvas_input {
            (geometry, false)
        } else {
            self.navigate(ui, response, geometry, pixels_per_point)
        };
        self.active_canvas_mut().last_layout = Some((geometry, pixels_per_point));
        (geometry, panning)
    }

    fn canvas_calculation(&mut self, image: ImageMode) -> Result<PhotographRequest, Error> {
        if image.inspection(self.canvas_inspection()) {
            return self
                .source_parameters(self.source_exposure)
                .and_then(PhotographRequest::inspection);
        }
        let recipe = Arc::new(self.view_recipe(
            self.input(),
            self.document.project_snapshot().roll,
            self.photo_settings(),
        )?);
        let capture = PhotographRequest::Recipe(recipe);
        let limits = self
            .document
            .visit
            .working_view
            .device()
            .map(|device| device.limits())
            .unwrap_or_default();
        capture.validate(&limits)?;
        Ok(capture)
    }

    fn canvas_image(
        &mut self,
        ui: &mut egui::Ui,
        geometry: CanvasLayout,
        proxy: crate::viewer::GpuProxy,
        pending: bool,
        image: ImageMode,
        cx: &UiContext<'_, Self>,
    ) -> Option<(egui::Rect, ImagePaint)> {
        let pixels_per_point = ui.ctx().pixels_per_point();
        let canvas = geometry.canvas;
        let source_outside_frame = image == ImageMode::Crop;
        let inspection = image.inspection(self.canvas_inspection());
        let composition = geometry.composition;
        // The camera proxy covers the full source, not the crop's bounding region.
        let rect = geometry.image.intersect(canvas);
        // An editing frame may be offscreen at explicit zoom; no refinement raster is requested.
        let physical_frame = if source_outside_frame {
            preview_frame(composition, proxy.size(), true).ok()?
        } else {
            geometry.physical_frame_raster(pixels_per_point)
        };
        let normal = !source_outside_frame && !image.is_sample();
        let working = normal && self.document.visit.working_view.enabled();
        let view = if working {
            Some(ViewRequest {
                area: physical_frame.bounds().expect("validated physical view"),
                output: physical_frame.output,
                frame: Some(physical_frame),
            })
        } else {
            (!source_outside_frame).then(|| ViewRequest {
                area: geometry.visible_area(),
                output: if composition.is_exact() {
                    geometry.output_size(geometry.visible_area(), pixels_per_point)
                } else {
                    geometry.frame_raster(pixels_per_point).output
                },
                frame: (!composition.is_exact()).then(|| geometry.frame_raster(pixels_per_point)),
            })
        };
        let capture = match self.canvas_calculation(image) {
            Ok(capture) => capture,
            Err(error) => {
                self.document.visit.working_view.clear();
                self.active_canvas_mut().preview.clear();
                let subject = if inspection {
                    "Source inspection"
                } else {
                    "Positive preview"
                };
                ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(canvas)
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                )
                .label(format!("{subject} unavailable: {error}"));
                return None;
            }
        };
        let target = self.document.target();
        let precision = !source_outside_frame && !pending && image.is_sample();
        let demand =
            self.source_context()
                .filter(|_| precision)
                .map(|source| original_view::Demand {
                    capture: capture.clone(),
                    target,
                    view,
                    composition,
                    source,
                });
        self.canvas_frame.original_requested = demand.is_some();
        self.observe_original(demand);
        let rendered = match self.working_view(physical_frame, &capture, ui.ctx(), cx) {
            Ok(view) => view,
            Err(error) => {
                self.document.visit.working_view.presentation_failed(error);
                self.document.visit.working_view.hold(target)
            }
        };
        if precision {
            self.request_original(ui.ctx(), cx);
        }
        let full_rect = geometry.screen_region(composition.area());
        let display_rect = if source_outside_frame {
            full_rect
        } else {
            geometry.image
        };
        let draw = if source_outside_frame {
            display_rect.intersect(canvas)
        } else if working {
            geometry.physical_draw(pixels_per_point)
        } else {
            rect
        };
        let rendered = rendered.filter(|rendered| {
            !source_outside_frame
                || (rendered.spec.frame().composition == physical_frame.composition
                    && rendered.spec.frame().min == physical_frame.min
                    && rendered.spec.frame().max == physical_frame.max
                    && rendered.capture == capture)
        });
        let tool = if precision {
            match self.original_rendered() {
                Ok(view) => view,
                Err(error) => {
                    self.status = error.to_string();
                    None
                }
            }
        } else {
            None
        };
        let paint = if let Some(rendered) = tool.or(rendered) {
            self.active_canvas_mut().preview.clear();
            let mapping = if source_outside_frame {
                geometry.uv(draw, composition.area())
            } else {
                rendered.mapping(if working {
                    physical_frame
                } else {
                    geometry.frame_raster(pixels_per_point)
                })
            };
            self.rendered_paint(rendered, mapping, ui.ctx().clone())
        } else {
            self.preview_paint(
                &proxy,
                geometry,
                &capture,
                source_outside_frame,
                ui.ctx(),
                cx,
            )
        };
        let Some(paint) = paint else {
            let working = &self.document.visit.working_view;
            ui.label(
                if self.active_canvas().preview.pending()
                    || (working.updating && working.error.is_none())
                {
                    "Loading preview…"
                } else {
                    "Preview unavailable"
                },
            );
            return None;
        };
        self.canvas_frame.displayed =
            paint
                .displayed()
                .map(|(view, mapping)| canvas_view::Displayed {
                    view,
                    mapping,
                    size: [draw.width(), draw.height()]
                        .map(|v| (v * pixels_per_point).round().max(1.) as u32),
                });
        Some((draw, paint))
    }

    fn rendered_paint(
        &mut self,
        rendered: Arc<crate::viewer::view::RenderedView>,
        mapping: crate::geometry::TextureMapping,
        context: egui::Context,
    ) -> Option<ImagePaint> {
        let presentation = match &self.presentation {
            Ok(presentation) => presentation,
            Err(error) => {
                self.document
                    .visit
                    .working_view
                    .presentation_failed(error.clone());
                return None;
            }
        };
        let queue = self.document.visit.working_view.queue()?.clone();
        match ImagePaint::rendered(
            presentation,
            rendered.clone(),
            mapping,
            queue.clone(),
            context.clone(),
        ) {
            Ok(paint) => Some(paint),
            Err(error) => {
                self.document.visit.working_view.presentation_failed(error);
                ImagePaint::retained(presentation, rendered, queue, context).ok()
            }
        }
    }

    /// Grade the camera proxy through the physical-view renderer. Crop editing retains the
    /// whole source and maps it beneath the draft frame; other previews compose before grading.
    fn preview_paint(
        &mut self,
        proxy: &crate::viewer::GpuProxy,
        geometry: CanvasLayout,
        capture: &crate::photographic::graph::request::PhotographRequest,
        source_outside_frame: bool,
        context: &egui::Context,
        cx: &UiContext<'_, Self>,
    ) -> Option<ImagePaint> {
        let correction = self.correction();
        if correction.pending() {
            self.active_canvas_mut().preview.clear();
            return None;
        }
        let working = &self.document.visit.working_view;
        let queue = working.queue()?.clone();
        let kernels = working
            .kernels
            .get()
            .filter(|_| !matches!(&working.error, Some(Error::GpuOutOfMemory(_))))
            .cloned();
        let frame = preview_frame(geometry.composition, proxy.size(), source_outside_frame).ok()?;
        let demand = canvas_view::PreviewDemand {
            proxy: proxy.clone(),
            frame,
            capture: capture.clone(),
            field: correction
                .prepared()
                .map(|correction| correction.gpu.clone()),
            target: self.document.target(),
        };
        let rendered = self
            .active_canvas_mut()
            .preview
            .render(demand, kernels, &queue, context, cx)?;
        let mapping = if source_outside_frame {
            let area = frame.composition.area();
            let draw = geometry.screen_region(area).intersect(geometry.canvas);
            geometry.uv(draw, area)
        } else {
            preview_mapping(geometry.image, geometry.image.intersect(geometry.canvas))
        };
        self.rendered_paint(rendered, mapping, context.clone())
    }

    pub(super) fn viewer_canvas(
        &mut self,
        ui: &mut egui::Ui,
        zoom_action: Option<f32>,
        cx: &UiContext<'_, Self>,
    ) {
        ui.push_id("viewer-canvas", |ui| {
            self.canvas_contents(ui, zoom_action, cx)
        });
    }

    fn canvas_contents(
        &mut self,
        ui: &mut egui::Ui,
        zoom_action: Option<f32>,
        cx: &UiContext<'_, Self>,
    ) {
        self.canvas_frame = Frame::default();
        let available = ui
            .available_rect_before_wrap()
            .intersect(ui.clip_rect())
            .size()
            .max(egui::vec2(1., 1.));
        let (canvas, response) = ui.allocate_exact_size(available, egui::Sense::click_and_drag());
        let selected = self.selected_view();
        let pixels_per_point = ui.ctx().pixels_per_point();
        let canvas_input = self.canvas_input_allowed(ui, canvas);
        self.canvas_frame.input_id = canvas_input.then_some(response.id);
        let composition = selected
            .as_ref()
            .map(|(area, _, _)| self.normal_canvas_composition(*area));
        let Some((_, proxy, pending)) = selected else {
            if let Some(view) = self.activities.view() {
                view.viewer(ui, canvas);
            } else {
                ui.painter().text(
                    canvas.center(),
                    egui::Align2::CENTER_CENTER,
                    if self.interaction_blocked() {
                        "Working…"
                    } else if self.document.visit.source.pending() {
                        "Loading photo…"
                    } else if self.document.visit.current_source.is_some() {
                        "Photo unavailable"
                    } else {
                        "Add scans to begin"
                    },
                    egui::FontId::proportional(14.),
                    ui.visuals().text_color(),
                );
            }
            return;
        };
        let composition = composition.expect("selected image composition");
        let geometry = self.canvas_layout(canvas, composition, pixels_per_point, zoom_action);
        let (geometry, _) = self.canvas_navigation(ui, &response, geometry, canvas_input);
        self.canvas_frame.layout = Some((geometry, pixels_per_point));
        let image = self.canvas_image(ui, geometry, proxy, pending, ImageMode::Photograph, cx);
        if let Some((draw, paint)) = image {
            ui.painter()
                .with_clip_rect(canvas)
                .add(egui_wgpu::Callback::new_paint_callback(
                    draw,
                    SelectionPaint::new(
                        paint,
                        self.active_canvas().selection_timing.clone(),
                        pending,
                    ),
                ));
        }
    }
}

/// An independently prepared source image and its presentation resources.
#[derive(Default)]
pub(super) struct SourceView {
    pub ready: Option<Ready>,
    original: original_view::OriginalView,
    preview: canvas_view::Preview,
    #[cfg(test)]
    pub paint_observer: Option<Arc<SelectionTiming>>,
}

impl SourceView {
    pub fn source(&self) -> Option<source::Context> {
        let ready = self.ready.as_ref()?;
        Some(source::Context {
            source: ready.developed.clone(),
            reference: ready.reference.clone(),
            correction: Default::default(),
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
            .source_preview(ready.developed.white_balance());
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
        let ppp = canvas.ui.ctx().pixels_per_point();
        let composition = layout.composition;
        let view = ViewRequest {
            area: layout.visible_area(),
            output: layout.output_size(layout.visible_area(), ppp),
            frame: None,
        };
        self.original.observe(original_view::Demand {
            capture: capture.clone(),
            target: editor.document.target(),
            view: Some(view),
            composition,
            source: self.source().expect("prepared source"),
        });
        if let Some(services) = services {
            self.original
                .request_with(services.clone(), canvas.ui.ctx(), cx);
        }
        let original = match self.original.rendered() {
            Ok(image) => image,
            Err(error) => {
                editor.status = error.to_string();
                None
            }
        };
        let draw = layout.image.intersect(layout.canvas);
        let presentation = match &editor.presentation {
            Ok(presentation) => presentation,
            Err(error) => {
                editor.status = error.to_string();
                canvas.ui.label("Preview unavailable");
                return false;
            }
        };
        let visible = if let Some(rendered) = original {
            self.preview.clear();
            let mapping = rendered.mapping(layout.frame_raster(ppp));
            Some((rendered, mapping))
        } else {
            let frame = match preview_frame(composition, ready.proxy.size(), false) {
                Ok(frame) => frame,
                Err(error) => {
                    editor.status = error.to_string();
                    return false;
                }
            };
            let working = &editor.document.visit.working_view;
            let demand = canvas_view::PreviewDemand {
                proxy: ready.proxy.clone(),
                frame,
                capture,
                field: None,
                target: editor.document.target(),
            };
            let queue = services
                .map(|services| &services.queue)
                .or_else(|| working.queue());
            queue.and_then(|queue| {
                let rendered = self.preview.render(
                    demand,
                    working.kernels.get().cloned(),
                    queue,
                    canvas.ui.ctx(),
                    cx,
                )?;
                Some((rendered, preview_mapping(layout.image, draw)))
            })
        };
        let paint = visible.and_then(|(rendered, mapping)| {
            ImagePaint::rendered(
                presentation,
                rendered,
                mapping,
                editor.document.visit.working_view.queue()?.clone(),
                canvas.ui.ctx().clone(),
            )
            .map_err(|error| editor.status = error.to_string())
            .ok()
        });
        let Some(paint) = paint else {
            canvas.ui.label(if self.preview.pending() {
                "Loading preview…"
            } else {
                "Preview unavailable"
            });
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
