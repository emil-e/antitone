use super::*;
use crate::viewport::CanvasLayout;

#[cfg(test)]
mod tests;
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
    /// The accepted source framing before any active crop draft, picker or review override.
    pub(super) fn source_composition(&self, source: &Source) -> Composition {
        let photo_orientation = if self.current_source == Some(source.id) {
            self.photo_orientation
        } else {
            source.orientation
        };
        let crop = if self.current_source == Some(source.id) {
            self.crop
        } else {
            source.crop
        };
        Composition::with_crop(
            source.development.active_area.into(),
            self.roll_orientation,
            photo_orientation,
            crop,
        )
        .expect("accepted source composition")
    }

    pub(super) fn canvas_composition(&self, area: Rect, active_pick: Option<Pick>) -> Composition {
        let crop = if self.auto_crop_context()
            || matches!(
                active_pick,
                Some(Pick::Dmin | Pick::Dmax | Pick::Flat | Pick::Analysis)
            ) {
            None
        } else {
            self.document
                .visit
                .crop_edit
                .as_ref()
                .map(|edit| edit.draft)
                .or(self.crop)
        };
        if matches!(active_pick, Some(Pick::Flat | Pick::Analysis)) {
            Composition::new(
                area,
                RollOrientation::default(),
                PhotoOrientation::default(),
            )
        } else {
            Composition::with_crop(area, self.roll_orientation, self.photo_orientation, crop)
        }
        .expect("accepted crop belongs to the loaded source")
    }

    fn canvas_layout(
        &mut self,
        canvas: egui::Rect,
        composition: Composition,
        pixels_per_point: f32,
        zoom_action: Option<f32>,
        cropping: bool,
    ) -> CanvasLayout {
        let inset = 20_f32.min((canvas.width().min(canvas.height()) - 1.).max(0.) * 0.5);
        let layout_canvas = if cropping {
            canvas.shrink(inset)
        } else {
            canvas
        };
        let mut geometry = if cropping {
            self.viewport
                .layout_crop(layout_canvas, composition, pixels_per_point)
        } else {
            self.viewport
                .layout_composition(layout_canvas, composition, pixels_per_point)
        };
        if let Some(zoom) = zoom_action {
            self.viewport.zoom_at(
                geometry,
                pixels_per_point,
                if zoom > 0. {
                    zoom
                } else {
                    geometry.scale * pixels_per_point * -zoom
                },
                canvas.center(),
            );
        }
        geometry = if cropping {
            self.viewport
                .layout_crop(layout_canvas, composition, pixels_per_point)
        } else {
            self.viewport
                .layout_composition(layout_canvas, composition, pixels_per_point)
        };
        geometry.canvas = canvas;
        geometry
    }

    fn canvas_navigation(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        geometry: CanvasLayout,
        canvas_input: bool,
        cropping: bool,
        fullscreen: bool,
    ) -> (CanvasLayout, bool) {
        let pixels_per_point = ui.ctx().pixels_per_point();
        if canvas_input && self.tab == 1 && !fullscreen {
            self.adjust_dust_brush_from_scroll(ui, geometry);
        }
        if response.drag_started_by(egui::PointerButton::Primary) {
            self.pick_drag = None;
        }
        let (geometry, panning) = if !canvas_input {
            (geometry, false)
        } else if cropping {
            (self.crop_interaction(ui, response, geometry), false)
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
        self.dust_interaction(ui, response, geometry, canvas_input, panning);
        if cropping && canvas_input {
            self.crop_rotation_cursor(ui, response, geometry);
        }
        if !fullscreen && (self.document.visit.mask_edit.is_some() || self.mask_handles_available())
        {
            self.mask_interaction(ui, response, geometry, panning, canvas_input);
        }
        if !cropping {
            self.last_layout = Some((geometry, pixels_per_point));
        }
        (geometry, panning)
    }

    fn canvas_parameters(
        &mut self,
        ui: &mut egui::Ui,
        cx: &UiContext<'_, Self>,
    ) -> Option<ProcessingInput> {
        let mut parameters = if self.source {
            let source_parameters = self.source_parameters(self.source_exposure);
            match source_parameters {
                Ok(parameters) => ProcessingInput::from(parameters),
                Err(error) => {
                    ui.label(format!("Source inspection unavailable: {error}"));
                    return None;
                }
            }
        } else {
            match self.view_processing(self.input(), self.roll, self.adjustment_layer_settings) {
                Ok(p) => p,
                Err(error) => {
                    ui.label(format!("Positive preview unavailable: {error}"));
                    return None;
                }
            }
        };
        if self.pick.is_some()
            || self.flat_edit.is_some()
            || self.document.pick_return.is_some()
            || self.document.analysis_review.is_some()
        {
            parameters = parameters.with_look(None);
        }
        if !self.source && self.flat_edit.is_none() && self.document.pick_return.is_none() {
            if let Some(signature) = self.dust_signature() {
                self.schedule_dust(signature, cx);
            }
            match self.dust_gpu_snapshot(parameters.pipeline().map(|grade| grade.snapshot())) {
                Ok(repairs) => parameters.repairs = repairs,
                Err(error) => self.status = format!("Dust view unavailable: {error}"),
            }
        }
        Some(parameters)
    }

    fn canvas_image(
        &mut self,
        ui: &mut egui::Ui,
        geometry: CanvasLayout,
        proxy: crate::viewer::GpuProxy,
        pending: bool,
        cropping: bool,
        cx: &UiContext<'_, Self>,
    ) -> Option<(egui::Rect, ImagePaint)> {
        let pixels_per_point = ui.ctx().pixels_per_point();
        let canvas = geometry.canvas;
        let active_pick = self.pick;
        let composition = geometry.composition;
        // The camera proxy covers the full source, not the crop's bounding region.
        let rect = geometry.image.intersect(canvas);
        // An editing frame may be offscreen at explicit zoom; no refinement raster is requested.
        let physical_frame = if cropping {
            preview_frame(composition, proxy.size(), true).ok()?
        } else {
            geometry.physical_frame_raster(pixels_per_point)
        };
        let normal = !cropping
            && active_pick.is_none()
            && self.document_pending.is_none()
            && self.flat_edit.is_none()
            && self.document.pick_return.is_none();
        let working = normal && self.working.enabled();
        let view = if working {
            Some(ViewRequest {
                area: physical_frame.bounds().expect("validated physical view"),
                output: physical_frame.output,
                frame: Some(physical_frame),
            })
        } else {
            (!cropping).then(|| ViewRequest {
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
        self.observe_grade(&parameters);
        if (working || !parameters.repairs.0.is_empty())
            && self
                .working
                .observe(self.id, physical_frame, parameters.signature(), self.target)
        {
            self.discard_original();
        }
        let purpose = if normal && !self.source && self.working.compare {
            Some(OriginalPurpose::Reference)
        } else {
            (!cropping
                && !pending
                && (active_pick.is_some()
                    || self.flat_edit.is_some()
                    || self.document.pick_return.is_some()))
            .then_some(OriginalPurpose::Tool)
        };
        self.observe_original(
            purpose,
            (parameters.signature(), self.target, view, composition),
        );
        let reference_allowed = if working {
            self.admit_view_reference(physical_frame)
        } else {
            self.thumbnails.reserve_view(0);
            true
        };
        let rendered = match self.working_view(physical_frame, &parameters, ui.ctx(), cx) {
            Ok(view) => view,
            Err(error) => {
                self.working.error = Some(error.to_string());
                self.working.hold(self.id, self.target)
            }
        };
        if purpose.is_some()
            && !self.busy
            && reference_allowed
            && (!working || self.working.support_ready())
            && let Some(source) = self.source_context()
        {
            self.request_original(&parameters, self.target, ui.ctx(), source, cx);
        }
        let full_rect = geometry.screen_region(composition.area());
        let display_rect = if cropping { full_rect } else { geometry.image };
        let draw = if cropping {
            display_rect.intersect(canvas)
        } else if working {
            geometry.physical_draw(pixels_per_point)
        } else {
            rect
        };
        let rendered = rendered.filter(|rendered| {
            !cropping
                || (rendered.spec.frame.composition == physical_frame.composition
                    && rendered.spec.frame.min == physical_frame.min
                    && rendered.spec.frame.max == physical_frame.max
                    && rendered.spec.repairs == parameters.repairs)
        });
        let tool = if purpose == Some(OriginalPurpose::Tool) {
            match self.original_rendered(OriginalPurpose::Tool, &parameters, proxy.matrix()) {
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
            self.preview_rendered = None;
            proxy.release_output();
            let mapping = if cropping {
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
        } else if let Some(paint) =
            self.preview_paint(&proxy, geometry, &parameters, pending, ui.ctx())
        {
            paint
        } else {
            ui.label(
                if !parameters.repairs.0.is_empty() && self.working.error.is_none() {
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
        let device = self.working.device()?;
        let queue = self.working.queue()?;
        let kernels = self.working.kernels.get()?.clone();
        let cropping = self.document.visit.crop_edit.is_some();
        let frame = preview_frame(geometry.composition, proxy.size(), cropping).ok()?;
        let composition = frame.composition;
        let field = self
            .correction
            .as_ref()
            .map(|correction| correction.settings.map.clone());
        let inspection = parameters.pipeline().is_none();
        let settings = match self.input.calibration().filter(|_| !inspection) {
            Some(base) => {
                let source = self.document.project.source(self.current_source?).ok()?;
                self.input.with_calibration(
                    source
                        .development
                        .processing_calibration(
                            base,
                            self.correction
                                .as_ref()
                                .map(|correction| correction.settings.map.as_ref()),
                        )
                        .ok()?,
                )
            }
            None => self.input,
        };
        let display_rect = if cropping {
            geometry.screen_region(composition.area())
        } else {
            geometry.image
        };
        let draw = display_rect.intersect(geometry.canvas);
        let mapping = if cropping {
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
        let signature = parameters.signature();
        if let Some((id, cached_signature, cached_frame, cached_target, rendered)) =
            &self.preview_rendered
            && *id == self.id
            && *cached_signature == signature
            && *cached_frame == frame
            && *cached_target == self.target
        {
            return Some(ImagePaint::rendered(
                rendered.clone(),
                mapping,
                self.submissions.clone(),
                context.clone(),
            ));
        }
        let spec = if inspection {
            crate::gpu::view::ViewSourceSpec::inspection(self.id, frame, proxy.matrix(), field)
        } else {
            let source = self.document.project.source(self.current_source?).ok()?;
            let matrix = self.document.positive_matrix(source, settings).ok()?;
            crate::gpu::view::ViewSourceSpec::new(self.id, frame, settings, matrix, field)
        }
        .ok()?
        .with_repairs(parameters.repairs.clone())
        .with_support(parameters.pixel_pipeline().ok()?.support([1., 1.]).ok()?);
        let source = proxy.view_source(queue, kernels, spec).ok()?;
        let previous = self.preview_rendered.as_ref().map(|entry| &entry.4);
        let rendered = if inspection {
            crate::gpu::view::RenderedView::inspection(
                device,
                source,
                parameters,
                self.target,
                previous,
            )
        } else {
            crate::gpu::view::RenderedView::new(device, source, parameters, self.target, previous)
        }
        .ok()?;
        self.preview_rendered = Some((self.id, signature, frame, self.target, rendered.clone()));
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
            && (geometry.image.contains(point) || self.document.analysis_review.is_some())
        {
            if self.document.analysis_review.is_some() {
                self.select_review_area(geometry, point);
            } else {
                self.region = Some(geometry.pick_patch(point, 32));
            }
        }
        if response.drag_started_by(egui::PointerButton::Primary) {
            if self.document.analysis_review.is_some()
                && let Some(point) = ui.input(|i| i.pointer.press_origin())
            {
                self.select_review_area(geometry, point);
            }
            // Classify from the press, not the later point where egui recognizes the drag.
            self.pick_drag = ui
                .input(|input| input.pointer.press_origin())
                .filter(|point| {
                    geometry.image.contains(*point) || self.document.analysis_review.is_some()
                })
                .map(|point| PickDrag::begin(geometry, point, self.region));
        }
        if response.dragged_by(egui::PointerButton::Primary)
            && let (Some(drag), Some(end)) = (self.pick_drag, response.interact_pointer_pos())
        {
            self.region = Some(drag.update(geometry, end));
        }
    }

    fn paint_canvas_selection(&self, ui: &mut egui::Ui, geometry: CanvasLayout, fullscreen: bool) {
        let canvas = geometry.canvas;
        self.paint_mask(ui, geometry);
        if self.document.analysis_review.is_some() {
            self.paint_review_areas(ui, geometry, canvas);
        } else if !fullscreen && let Some(region) = self.region {
            ui.painter().with_clip_rect(canvas).rect_stroke(
                geometry.screen_coverage(region),
                0.,
                egui::Stroke::new(1., egui::Color32::WHITE),
                egui::StrokeKind::Inside,
            );
            let screen = geometry.screen_coverage(region);
            for point in [
                screen.min,
                screen.max,
                screen.left_bottom(),
                screen.right_top(),
            ] {
                ui.painter().with_clip_rect(canvas).rect_filled(
                    egui::Rect::from_center_size(point, egui::vec2(6., 6.)),
                    0.,
                    egui::Color32::WHITE,
                );
            }
        }
    }

    pub(super) fn viewer_canvas(
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
        let active_pick = self.pick;
        let composition = if pending {
            self.pending_preview
                .as_ref()
                .map(|preview| preview.composition)
                .unwrap_or_else(|| self.canvas_composition(area, active_pick))
        } else {
            self.canvas_composition(area, active_pick)
        };
        let cropping = !fullscreen && self.document.visit.crop_edit.is_some();
        let geometry =
            self.canvas_layout(canvas, composition, pixels_per_point, zoom_action, cropping);
        let (geometry, panning) =
            self.canvas_navigation(ui, &response, geometry, canvas_input, cropping, fullscreen);
        self.start_dust_searches(cx);
        let Some((draw, paint)) = self.canvas_image(ui, geometry, proxy, pending, cropping, cx)
        else {
            return;
        };
        ui.painter()
            .with_clip_rect(canvas)
            .add(egui_wgpu::Callback::new_paint_callback(
                draw,
                SelectionPaint::new(paint, self.selection_timing.clone(), pending),
            ));
        if cropping {
            self.paint_crop(ui, geometry);
        } else {
            if self.auto_crop_artwork_visible(ui)
                && let Some(q) = self.auto_crop_outline(geometry)
            {
                crop_editor::paint_outline(ui, canvas, q, false, 64);
            }
            self.paint_dust(ui, geometry);
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
            self.pick_drag = None;
        }
    }
}
