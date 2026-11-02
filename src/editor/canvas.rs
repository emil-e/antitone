use super::*;
use crate::viewport::CanvasLayout;

#[cfg(test)]
mod tests;

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
        if let Some((draw, paint)) = editor.canvas_image(
            self.ui,
            layout,
            self.proxy.clone(),
            self.pending,
            source_outside_frame,
            cx,
        ) {
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
    pub(super) fn hosted_canvas(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        composition: Composition,
        zoom_action: Option<f32>,
        enabled: bool,
        cx: &UiContext<'_, Self>,
    ) -> Option<CanvasLayout> {
        let (_, proxy, pending) = self.selected_view()?;
        ui.add_enabled_ui(enabled, |ui| {
            self.with_current_tool(|tool, editor| {
                tool.canvas(
                    &mut Canvas {
                        ui,
                        response,
                        composition,
                        zoom_action,
                        proxy,
                        pending,
                    },
                    editor,
                    cx,
                )
            })
        })
        .inner
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

    pub(super) fn normal_canvas_composition(
        &self,
        area: Rect,
        active_pick: Option<picking::Display>,
    ) -> Composition {
        let crop = if self.auto_crop_context()
            || active_pick.is_some_and(|display| display.view != picking::View::Working)
        {
            None
        } else {
            self.document.visit.crop
        };
        if active_pick.is_some_and(|display| display.view == picking::View::Native) {
            Composition::new(
                area,
                RollOrientation::default(),
                PhotoOrientation::default(),
            )
        } else {
            Composition::with_crop(
                area,
                self.document.roll_orientation,
                self.document.visit.photo_orientation,
                crop,
            )
        }
        .expect("accepted crop belongs to the loaded source")
    }

    pub(super) fn canvas_composition(
        &mut self,
        area: Rect,
        active_pick: Option<picking::Display>,
    ) -> Composition {
        let normal = self.normal_canvas_composition(area, active_pick);
        if self.fullscreen.is_some() || self.current_tool().is_none() {
            normal
        } else {
            self.with_current_tool(|tool, _| tool.composition(normal))
                .expect("composition access cannot recursively invoke the executing tool")
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
        if response.drag_started_by(egui::PointerButton::Primary) {
            self.active_canvas_mut().pick_drag = None;
        }
        let (geometry, panning) = if !canvas_input {
            (geometry, false)
        } else {
            let (claimed, cursor) = self.auto_crop_margin_interaction(ui, response, geometry);
            let result = if claimed {
                (geometry, false)
            } else {
                self.navigate(ui, response, geometry, pixels_per_point)
            };
            if let Some(cursor) = cursor {
                ui.ctx().set_cursor_icon(cursor);
            }
            result
        };
        if !fullscreen && (self.document.mask_edit().is_some() || self.mask_handles_available()) {
            self.mask_interaction(ui, response, geometry, panning, canvas_input);
        }
        self.active_canvas_mut().last_layout = Some((geometry, pixels_per_point));
        (geometry, panning)
    }

    fn canvas_parameters(
        &mut self,
        ui: &mut egui::Ui,
        cx: &UiContext<'_, Self>,
    ) -> Option<ProcessingInput> {
        let bypass_look = self.active_pick().is_some()
            || self.document.flat_edit().is_some()
            || self.document.analysis_review().is_some();
        let mut recipe = None;
        let mut parameters = if self.canvas_inspection() {
            let source_parameters = self.source_parameters(self.source_exposure);
            match source_parameters {
                Ok(parameters) => ProcessingInput::from(parameters),
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
                .and_then(|mut captured| {
                    if bypass_look {
                        captured.look = None;
                    }
                    let limits = self
                        .document
                        .visit
                        .working_view
                        .device()
                        .map(|device| device.limits())
                        .unwrap_or_default();
                    let processing = ProcessingInput::from_recipe(&limits, &captured)?;
                    recipe = Some(Arc::new(captured));
                    Ok(processing)
                });
            match captured {
                Ok(p) => p,
                Err(error) => {
                    ui.label(format!("Positive preview unavailable: {error}"));
                    return None;
                }
            }
        };
        if !self.canvas_inspection() && self.document.flat_edit().is_none() {
            if let Some(signature) = self.dust_signature() {
                self.schedule_dust(signature, cx);
            }
            match self.dust_gpu_snapshot(parameters.grade().map(|grade| grade.snapshot())) {
                Ok(repairs) => parameters.repairs = repairs,
                Err(error) => self.status = format!("Dust view unavailable: {error}"),
            }
        }
        self.observe_grade(recipe);
        Some(parameters)
    }

    fn canvas_image(
        &mut self,
        ui: &mut egui::Ui,
        geometry: CanvasLayout,
        proxy: crate::viewer::GpuProxy,
        pending: bool,
        source_outside_frame: bool,
        cx: &UiContext<'_, Self>,
    ) -> Option<(egui::Rect, ImagePaint)> {
        let pixels_per_point = ui.ctx().pixels_per_point();
        let canvas = geometry.canvas;
        let active_pick = self.active_pick();
        let composition = geometry.composition;
        // The camera proxy covers the full source, not the crop's bounding region.
        let rect = geometry.image.intersect(canvas);
        // An editing frame may be offscreen at explicit zoom; no refinement raster is requested.
        let physical_frame = if source_outside_frame {
            preview_frame(composition, proxy.size(), true).ok()?
        } else {
            geometry.physical_frame_raster(pixels_per_point)
        };
        let normal = !source_outside_frame
            && active_pick.is_none()
            && self.document_pending.is_none()
            && self.document.flat_edit().is_none();
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
        let parameters = self.canvas_parameters(ui, cx)?;
        let capture = self
            .active_canvas()
            .grade
            .recipe()
            .map(working_view::Capture::Photograph)
            .unwrap_or_else(|| working_view::Capture::Primitive(parameters.clone()));
        if (working || !parameters.repairs.0.is_empty())
            && self.document.visit.working_view.observe(
                physical_frame,
                capture.clone(),
                self.document.target,
            )
        {
            self.discard_original();
        }
        let purpose =
            if normal && !self.canvas_inspection() && self.document.visit.working_view.compare {
                Some(OriginalPurpose::Reference)
            } else {
                (!source_outside_frame
                    && !pending
                    && (active_pick.is_some() || self.document.flat_edit().is_some()))
                .then_some(OriginalPurpose::Tool)
            };
        self.observe_original(
            purpose,
            self.canvas_source_context()
                .map(|source| original_view::Demand {
                    capture,
                    repairs: parameters.repairs.clone(),
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
        let rendered = match self.working_view(physical_frame, &parameters, ui.ctx(), cx) {
            Ok(view) => view,
            Err(error) => {
                self.document.visit.working_view.error = Some(error.to_string());
                self.document.visit.working_view.hold(self.document.target)
            }
        };
        if purpose.is_some()
            && !self.busy
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
                    && rendered.spec.repairs == parameters.repairs)
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
            &parameters,
            pending,
            source_outside_frame,
            ui.ctx(),
        ) {
            paint
        } else {
            ui.label(
                if !parameters.repairs.0.is_empty()
                    && self.document.visit.working_view.error.is_none()
                {
                    "Updating repairs…"
                } else {
                    "Preview unavailable"
                },
            );
            return None;
        };
        Some((draw, paint))
    }

    /// Grade the camera proxy through the physical-view renderer. Crop editing retains the
    /// whole source and maps it beneath the draft frame; other previews compose before grading.
    fn preview_paint(
        &mut self,
        proxy: &crate::viewer::GpuProxy,
        geometry: CanvasLayout,
        parameters: &ProcessingInput,
        placeholder: bool,
        source_outside_frame: bool,
        context: &egui::Context,
    ) -> Option<ImagePaint> {
        // Sparse repairs need original-sample footprints. The shared acquisition worker supplies
        // those views; a cached selected preview may stand in until the working view arrives.
        if !parameters.repairs.0.is_empty() && !placeholder {
            return None;
        }
        let mut preview = parameters.clone();
        if placeholder {
            preview.repairs = Default::default();
        }
        let parameters = &preview;
        let device = self.document.visit.working_view.device()?;
        let queue = self.document.visit.working_view.queue()?;
        let kernels = self.document.visit.working_view.kernels.get()?.clone();
        let frame = preview_frame(geometry.composition, proxy.size(), source_outside_frame).ok()?;
        let composition = frame.composition;
        let inspection = parameters.grade().is_none();
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
        let capture = self
            .active_canvas()
            .grade
            .recipe()
            .map(crate::gpu::view::Capture::Photograph)
            .unwrap_or_else(|| crate::gpu::view::Capture::Primitive(parameters.clone()));
        if let Some((source, requested_frame, rendered)) = &self.active_canvas().preview_rendered
            && source.same_source(proxy)
            && rendered.capture == capture
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
        let spec = if inspection {
            crate::gpu::view::SourceRequest::inspection(frame)
        } else {
            crate::gpu::view::SourceRequest::for_processing(frame, parameters)
        }
        .ok()?
        .with_repairs(parameters.repairs.clone())
        .with_pipeline(parameters.pixel_pipeline())
        .ok()?;
        let (source, processed) = proxy
            .preview(queue, &self.submissions, kernels, parameters, spec)
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

    fn canvas_pick(&mut self, ui: &egui::Ui, response: &egui::Response, geometry: CanvasLayout) {
        if response.clicked_by(egui::PointerButton::Primary)
            && let Some(point) = response.interact_pointer_pos()
            && (geometry.image.contains(point) || self.document.analysis_review().is_some())
        {
            if self.document.analysis_review().is_some() {
                self.select_review_area(geometry, point);
            } else {
                self.set_pick_region(Some(geometry.pick_patch(point, 32)));
            }
        }
        if response.drag_started_by(egui::PointerButton::Primary) {
            if self.document.analysis_review().is_some()
                && let Some(point) = ui.input(|i| i.pointer.press_origin())
            {
                self.select_review_area(geometry, point);
            }
            // Classify from the press, not the later point where egui recognizes the drag.
            self.active_canvas_mut().pick_drag = ui
                .input(|input| input.pointer.press_origin())
                .filter(|point| {
                    geometry.image.contains(*point) || self.document.analysis_review().is_some()
                })
                .map(|point| PickDrag::begin(geometry, point, self.pick_region()));
        }
        if response.dragged_by(egui::PointerButton::Primary)
            && let (Some(drag), Some(end)) = (
                self.active_canvas().pick_drag,
                response.interact_pointer_pos(),
            )
        {
            self.set_pick_region(Some(drag.update(geometry, end)));
        }
    }

    fn paint_canvas_selection(&self, ui: &mut egui::Ui, geometry: CanvasLayout, fullscreen: bool) {
        self.paint_mask(ui, geometry);
        if self.hosted_picker() {
            return;
        }
        if self.document.analysis_review().is_some() {
            self.paint_review_areas(ui, geometry, geometry.canvas);
        } else if !fullscreen && let Some(region) = self.pick_region() {
            picking::paint_selection(ui, geometry, region);
        }
    }

    pub(super) fn viewer_canvas(
        &mut self,
        ui: &mut egui::Ui,
        zoom_action: Option<f32>,
        fullscreen: bool,
        cx: &UiContext<'_, Self>,
    ) {
        let identity = if self.document.flat_edit().is_some() {
            "flat-reference"
        } else {
            "working-photo"
        };
        ui.push_id(identity, |ui| {
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
        let available = ui
            .available_rect_before_wrap()
            .intersect(ui.clip_rect())
            .size()
            .max(egui::vec2(1., 1.));
        let (canvas, response) = ui.allocate_exact_size(available, egui::Sense::click_and_drag());
        let Some((area, proxy, pending)) = self.selected_view() else {
            if let Some(view) = self.tasks.view() {
                view.viewer(ui, canvas);
            } else {
                ui.painter().text(
                    canvas.center(),
                    egui::Align2::CENTER_CENTER,
                    if self.busy {
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
        let pixels_per_point = ui.ctx().pixels_per_point();
        let canvas_input = self.canvas_input_allowed(ui, canvas);
        let active_pick = self.active_pick();
        let composition = if pending {
            self.document
                .visit
                .pending_preview
                .as_ref()
                .map(|preview| preview.composition)
                .unwrap_or_else(|| self.normal_canvas_composition(area, active_pick))
        } else {
            self.normal_canvas_composition(area, active_pick)
        };
        if !fullscreen && self.current_tool().is_some() {
            let geometry = self
                .hosted_canvas(ui, &response, composition, zoom_action, canvas_input, cx)
                .expect("canvas callback owns the selected image and tool");
            self.active_canvas_mut().last_layout = Some((geometry, pixels_per_point));
            self.paint_canvas_selection(ui, geometry, fullscreen);
            return;
        }
        let geometry = self.canvas_layout(canvas, composition, pixels_per_point, zoom_action);
        let (geometry, panning) =
            self.canvas_navigation(ui, &response, geometry, canvas_input, fullscreen);
        let image = self.canvas_image(ui, geometry, proxy, pending, false, cx);
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
        if self.auto_crop_artwork_visible(ui)
            && let Some(q) = self.auto_crop_outline(geometry)
        {
            crop_editor::paint_outline(ui, canvas, q, false, 64);
        }
        if !fullscreen
            && active_pick.is_some()
            && !self.busy
            && canvas_input
            && !panning
            && !ui.input(|input| input.key_down(egui::Key::Space))
        {
            self.canvas_pick(ui, &response, geometry);
        }
        self.paint_canvas_selection(ui, geometry, fullscreen);
        if response.drag_stopped_by(egui::PointerButton::Primary) {
            self.active_canvas_mut().pick_drag = None;
        }
    }
}
