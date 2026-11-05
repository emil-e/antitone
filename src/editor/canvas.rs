use super::*;
use crate::{gpu::photograph::request::PhotographRequest, viewport::CanvasLayout};

#[cfg(test)]
mod immutable_source_tests;
#[cfg(test)]
mod tests;

#[derive(Default)]
pub(super) struct Frame {
    pub layout: Option<(CanvasLayout, f32)>,
    pub displayed: Option<canvas_view::Displayed>,
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
            presentation::viewer_surface(self.ui, fullscreen, |ui| {
                editor.viewer_canvas(ui, bar.zoom.or(self.zoom), fullscreen, cx)
            });
            return None;
        }
        let zoom = bar.zoom.or(self.zoom);
        let fullscreen = editor.fullscreen.is_some();
        editor.canvas_frame = Frame::default();
        presentation::viewer_surface(self.ui, fullscreen, |ui| {
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
                editor.paint_canvas_selection(ui, layout, fullscreen);
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
        let (layout, _) = editor.canvas_navigation(image.ui, image.response, layout, true, true);
        if editor.document.visit.picker.display().is_some() {
            image.paint_sample(editor, layout, editor.canvas_inspection(), cx);
        } else {
            image.paint_image(editor, layout, false, cx);
        }
        Some(layout)
    }
}

/// Photographic endpoint requested by a canvas consumer.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ImageMode {
    Photograph,
    Crop,
    PositiveSample,
    SourceSample,
}

impl ImageMode {
    pub fn is_sample(self) -> bool {
        matches!(self, Self::PositiveSample | Self::SourceSample)
    }
    pub fn inspection(self, normal: bool) -> bool {
        match self {
            Self::Photograph | Self::Crop => normal,
            Self::PositiveSample => false,
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

    pub fn paint_sample(
        &mut self,
        editor: &mut Editor,
        layout: CanvasLayout,
        source: bool,
        cx: &UiContext<'_, Editor>,
    ) -> bool {
        let image = if source {
            ImageMode::SourceSample
        } else {
            ImageMode::PositiveSample
        };
        self.paint(editor, layout, image, cx)
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

impl Editor {
    pub(super) fn show_viewer(
        &mut self,
        ui: &mut egui::Ui,
        menu_zoom: Option<f32>,
        cx: &UiContext<'_, Self>,
    ) {
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
            if bar.fit {
                self.document.visit.canvas.viewport.fit();
            }
            if let Some(action) = bar.action {
                match action {
                    tool::Action::Primary => self.apply_pick(cx),
                    tool::Action::Dismiss if self.masking() => self.finish_mask(false),
                    tool::Action::Dismiss => self.finish_pick(cx),
                }
            }
            if bar.fullscreen {
                self.toggle_fullscreen(ui.ctx());
            }
            let fullscreen = self.fullscreen.is_some();
            presentation::viewer_surface(ui, fullscreen, |ui| {
                self.viewer_canvas(ui, bar.zoom.or(menu_zoom), fullscreen, cx)
            });
        }
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
            self.document.visit.photo_orientation
        } else {
            source.orientation
        };
        let crop = if self.document.visit.current_source == Some(source.id) {
            self.document.visit.crop
        } else {
            source.crop
        };
        Composition::with_crop(
            source.development.active_area.into(),
            self.document.roll_orientation,
            photo_orientation,
            crop,
        )
        .expect("accepted source composition")
    }

    pub(super) fn normal_canvas_composition(&self, area: Rect) -> Composition {
        Composition::with_crop(
            area,
            self.document.roll_orientation,
            self.document.visit.photo_orientation,
            self.document.visit.crop,
        )
        .expect("accepted crop belongs to the loaded source")
    }

    #[cfg(test)]
    pub(super) fn canvas_composition(&mut self, area: Rect) -> Composition {
        let normal = self.normal_canvas_composition(area);
        if self.fullscreen.is_some() || self.current_tool().is_none() {
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
        fullscreen: bool,
    ) -> (CanvasLayout, bool) {
        let pixels_per_point = ui.ctx().pixels_per_point();
        let (geometry, panning) = if !canvas_input {
            (geometry, false)
        } else {
            self.navigate(ui, response, geometry, pixels_per_point)
        };
        if !fullscreen
            && (self.document.visit.masks.gesture().is_some() || self.mask_handles_available())
        {
            self.mask_interaction(ui, response, geometry, panning, canvas_input);
        }
        self.active_canvas_mut().last_layout = Some((geometry, pixels_per_point));
        (geometry, panning)
    }

    fn canvas_calculation(
        &mut self,
        ui: &mut egui::Ui,
        image: ImageMode,
        cx: &UiContext<'_, Self>,
    ) -> Option<(
        crate::gpu::photograph::request::PhotographRequest,
        crate::gpu::repairs::Snapshot,
    )> {
        use crate::gpu::photograph::request::PhotographRequest;
        let inspection = image.inspection(self.canvas_inspection());
        let bypass_look = image.is_sample();
        let capture = if inspection {
            match self
                .source_parameters(self.source_exposure)
                .and_then(PhotographRequest::inspection)
            {
                Ok(capture) => capture,
                Err(error) => {
                    ui.label(format!("Source inspection unavailable: {error}"));
                    return None;
                }
            }
        } else {
            let captured = self
                .view_recipe(
                    self.input(),
                    self.document.roll,
                    self.document.visit.adjustment_layer_settings,
                )
                .map(|recipe| {
                    let recipe = Arc::new(recipe);
                    if bypass_look {
                        PhotographRequest::BeforeLooks(recipe)
                    } else {
                        PhotographRequest::Recipe(recipe)
                    }
                });
            let captured = captured.and_then(|capture| {
                let limits = self
                    .document
                    .visit
                    .working_view
                    .device()
                    .map(|device| device.limits())
                    .unwrap_or_default();
                capture.validate(&limits, &Default::default())?;
                Ok(capture)
            });
            match captured {
                Ok(capture) => capture,
                Err(error) => {
                    ui.label(format!("Positive preview unavailable: {error}"));
                    return None;
                }
            }
        };
        let mut repairs = Default::default();
        if let Some(recipe) = capture.recipe() {
            if let Some(signature) = self.dust_signature() {
                self.schedule_dust(signature, cx);
            }
            match self.dust_gpu_snapshot(Some(&recipe.grade)) {
                Ok(snapshot) => repairs = snapshot,
                Err(error) => self.status = format!("Dust view unavailable: {error}"),
            }
        }
        Some((capture, repairs))
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
        let normal = !source_outside_frame && !image.is_sample() && self.document_pending.is_none();
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
        let (capture, repairs) = self.canvas_calculation(ui, image, cx)?;
        if (working || !repairs.0.is_empty())
            && self.document.visit.working_view.observe(
                physical_frame,
                capture.clone(),
                self.document.target,
            )
        {
            self.discard_original();
        }
        let compare = self.comparing_original();
        let purpose = if normal && !inspection && compare {
            Some(OriginalPurpose::Reference)
        } else {
            (!source_outside_frame && !pending && image.is_sample())
                .then_some(OriginalPurpose::Tool)
        };
        self.observe_original(
            purpose,
            self.source_context().map(|source| original_view::Demand {
                capture: capture.clone(),
                repairs: repairs.clone(),
                target: self.document.target,
                view,
                composition,
                source,
            }),
        );
        let reference_allowed = if working {
            self.admit_view_reference(physical_frame)
        } else {
            self.document.thumbnails.reserve_view(0);
            true
        };
        let rendered =
            match self.working_view(physical_frame, (&capture, &repairs), image, ui.ctx(), cx) {
                Ok(view) => view,
                Err(error) => {
                    self.document.visit.working_view.error = Some(error.to_string());
                    self.document
                        .visit
                        .working_view
                        .hold(self.document.target, compare)
                }
            };
        if purpose.is_some()
            && !self.interaction_blocked()
            && reference_allowed
            && (!working || self.document.visit.working_view.support_ready())
        {
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
                    && rendered.spec.repairs == repairs)
        });
        let tool = if purpose == Some(OriginalPurpose::Tool) {
            match self.original_rendered(OriginalPurpose::Tool) {
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
            self.active_canvas_mut().preview_rendered = None;
            proxy.release_output();
            let mapping = if source_outside_frame {
                geometry.uv(draw, composition.area())
            } else {
                rendered.mapping(if working {
                    physical_frame
                } else {
                    geometry.frame_raster(pixels_per_point)
                })
            };
            ImagePaint::rendered(
                rendered,
                mapping,
                self.submissions.clone(),
                ui.ctx().clone(),
            )
        } else if let Some(paint) = self.preview_paint(
            &proxy,
            geometry,
            (&capture, &repairs),
            pending,
            source_outside_frame,
            ui.ctx(),
        ) {
            paint
        } else {
            ui.label(
                if !repairs.0.is_empty() && self.document.visit.working_view.error.is_none() {
                    "Updating repairs…"
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

    /// Grade the camera proxy through the physical-view renderer. Crop editing retains the
    /// whole source and maps it beneath the draft frame; other previews compose before grading.
    fn preview_paint(
        &mut self,
        proxy: &crate::viewer::GpuProxy,
        geometry: CanvasLayout,
        (capture, repairs): (
            &crate::gpu::photograph::request::PhotographRequest,
            &crate::gpu::repairs::Snapshot,
        ),
        placeholder: bool,
        source_outside_frame: bool,
        context: &egui::Context,
    ) -> Option<ImagePaint> {
        // Sparse repairs need original-sample footprints. The shared acquisition worker supplies
        // those views; a cached selected preview may stand in until the working view arrives.
        if !repairs.0.is_empty() && !placeholder {
            return None;
        }
        let repairs = if placeholder {
            Default::default()
        } else {
            repairs.clone()
        };
        let device = self.document.visit.working_view.device()?;
        let queue = self.document.visit.working_view.queue()?;
        let kernels = self.document.visit.working_view.kernels.get()?.clone();
        let frame = preview_frame(geometry.composition, proxy.size(), source_outside_frame).ok()?;
        let composition = frame.composition;
        let display_rect = if source_outside_frame {
            geometry.screen_region(composition.area())
        } else {
            geometry.image
        };
        let draw = display_rect.intersect(geometry.canvas);
        let mapping = if source_outside_frame {
            geometry.uv(draw, composition.area())
        } else if draw == display_rect {
            crate::geometry::TextureMapping::IDENTITY
        } else {
            let extent = display_rect.size();
            let origin = (draw.min - display_rect.min) / extent;
            let max = (draw.max - display_rect.min) / extent;
            crate::geometry::TextureMapping {
                origin: [origin.x, origin.y],
                x_axis: [max.x - origin.x, 0.],
                y_axis: [0., max.y - origin.y],
            }
        };
        let capture = capture.clone();
        if let Some((source, requested_frame, rendered)) = &self.active_canvas().preview_rendered
            && source.same_source(proxy)
            && rendered.capture == capture
            && rendered.has_correction(
                self.document
                    .correction
                    .as_ref()
                    .map(|correction| &correction.gpu),
            )
            && *requested_frame == frame
            && rendered.target == self.document.target
        {
            return Some(ImagePaint::rendered(
                rendered.clone(),
                mapping,
                self.submissions.clone(),
                context.clone(),
            ));
        }
        let spec = capture.source_request(frame, repairs).ok()?;
        let (source, processed) = proxy
            .preview(
                queue,
                &self.submissions,
                kernels,
                &capture,
                self.document
                    .correction
                    .as_ref()
                    .map(|correction| correction.gpu.clone()),
                spec,
            )
            .ok()?;
        let previous = self
            .active_canvas()
            .preview_rendered
            .as_ref()
            .map(|entry| &entry.2);
        let rendered = crate::gpu::view::RenderedView::from_processed(
            device,
            source,
            capture,
            self.document.target,
            processed,
            previous,
        )
        .ok()?;
        let source = proxy.clone();
        self.active_canvas_mut().preview_rendered = Some((source, frame, rendered.clone()));
        Some(ImagePaint::rendered(
            rendered.clone(),
            mapping,
            self.submissions.clone(),
            context.clone(),
        ))
    }

    fn paint_canvas_selection(&self, ui: &mut egui::Ui, geometry: CanvasLayout, _fullscreen: bool) {
        self.paint_mask(ui, geometry);
    }

    pub(super) fn viewer_canvas(
        &mut self,
        ui: &mut egui::Ui,
        zoom_action: Option<f32>,
        fullscreen: bool,
        cx: &UiContext<'_, Self>,
    ) {
        ui.push_id("viewer-canvas", |ui| {
            self.canvas_contents(ui, zoom_action, fullscreen, cx)
        });
    }

    fn canvas_contents(
        &mut self,
        ui: &mut egui::Ui,
        zoom_action: Option<f32>,
        fullscreen: bool,
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
        let composition = selected.as_ref().map(|(area, _, pending)| {
            if *pending {
                self.document
                    .visit
                    .pending_preview
                    .as_ref()
                    .map(|preview| preview.composition)
                    .unwrap_or_else(|| self.normal_canvas_composition(*area))
            } else {
                self.normal_canvas_composition(*area)
            }
        });
        let Some((_, proxy, pending)) = selected else {
            if let Some(view) = self.activities.view() {
                view.viewer(ui, canvas);
            } else {
                ui.painter().text(
                    canvas.center(),
                    egui::Align2::CENTER_CENTER,
                    if self.interaction_blocked() {
                        "Working…"
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
        let (geometry, _) =
            self.canvas_navigation(ui, &response, geometry, canvas_input, fullscreen);
        self.canvas_frame.layout = Some((geometry, pixels_per_point));
        let image_mode = if fullscreen && self.document.visit.picker.display().is_some() {
            if self.canvas_inspection() {
                ImageMode::SourceSample
            } else {
                ImageMode::PositiveSample
            }
        } else {
            ImageMode::Photograph
        };
        let image = self.canvas_image(ui, geometry, proxy, pending, image_mode, cx);
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
        } else if self.current_tool().is_none() {
            return;
        }
        self.paint_canvas_selection(ui, geometry, fullscreen);
    }
}

/// An independently prepared source image and its presentation resources.
#[derive(Default)]
pub(super) struct SourceView {
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
        if !editor.interaction_blocked()
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
