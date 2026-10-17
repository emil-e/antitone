use super::*;
use crate::viewport::CanvasLayout;

impl Editor {
    fn canvas_composition(&self, area: Rect, active_pick: Option<Pick>) -> Composition {
        let crop = if self.auto_crop_context()
            || matches!(
                active_pick,
                Some(Pick::Dmin | Pick::Dmax | Pick::Flat | Pick::Analysis)
            ) {
            None
        } else {
            self.crop_edit.as_ref().map(|edit| edit.draft).or(self.crop)
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
        if !fullscreen && (self.mask_edit.is_some() || self.mask_handles_available()) {
            self.mask_interaction(ui, response, geometry, panning, canvas_input);
        }
        if !cropping {
            self.last_layout = Some((geometry, pixels_per_point));
        }
        (geometry, panning)
    }

    fn canvas_parameters(&mut self, ui: &mut egui::Ui) -> Option<ProcessingInput> {
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
            let Some(cal) = self.calibration() else {
                ui.label("Positive preview requires valid calibration");
                return None;
            };
            match self.view_processing(cal, self.roll, self.photo) {
                Ok(p) => p,
                Err(error) => {
                    ui.label(format!("Positive preview unavailable: {error}"));
                    return None;
                }
            }
        };
        if self.pick.is_some()
            || self.flat_edit.is_some()
            || self.pick_return.is_some()
            || self.analysis_review.is_some()
        {
            parameters = parameters.with_look(None);
        }
        if !self.source && self.flat_edit.is_none() && self.pick_return.is_none() {
            if let Some(signature) = self.dust_signature() {
                self.schedule_dust_with_wake(signature, ui.ctx());
            }
            match self.dust_gpu_snapshot(parameters.grade().map(|grade| grade.snapshot())) {
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
        area: Rect,
        proxy: crate::viewer::GpuProxy,
        pending: bool,
        cropping: bool,
    ) -> Option<(egui::Rect, ImagePaint)> {
        let pixels_per_point = ui.ctx().pixels_per_point();
        let canvas = geometry.canvas;
        let active_pick = self.pick;
        let composition = geometry.composition;
        let rect = geometry.image.intersect(canvas);
        // An editing frame may be offscreen at explicit zoom; no refinement raster is requested.
        let physical_frame = if cropping {
            crate::frame_raster::FrameRaster::full(composition).expect("accepted composition")
        } else {
            geometry.physical_frame_raster(pixels_per_point)
        };
        let normal = !cropping
            && active_pick.is_none()
            && !pending
            && self.document_pending.is_none()
            && self.flat_edit.is_none()
            && self.pick_return.is_none();
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
        let parameters = self.canvas_parameters(ui)?;
        if working
            && self
                .working
                .observe(self.id, physical_frame, parameters.signature(), self.target)
        {
            self.original.discard();
        }
        let purpose = if normal && !self.source && self.working.compare {
            Some(OriginalPurpose::Reference)
        } else {
            (!cropping
                && !pending
                && (active_pick.is_some()
                    || self.flat_edit.is_some()
                    || self.pick_return.is_some()))
            .then_some(OriginalPurpose::Tool)
        };
        if let Some(purpose) = purpose {
            self.original.observe(
                purpose,
                (parameters.signature(), self.target, view, composition),
            );
        } else {
            self.original.leave();
        }
        let reference_allowed = if working {
            self.admit_view_reference(physical_frame)
        } else {
            self.thumbnails.reserve_view(0);
            true
        };
        let revision = self.original.revision();
        let rendered = match self.working_view(physical_frame, &parameters, ui.ctx()) {
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
            && let Some(error) =
                self.original
                    .schedule(self.id, &parameters, self.target, &self.commands, ui.ctx())
        {
            self.status = error.into();
        }
        let draw = if cropping {
            canvas
        } else if working {
            geometry.physical_draw(pixels_per_point)
        } else {
            rect
        };
        let paint = if let Some(rendered) = rendered {
            proxy.release_output();
            let mapping = rendered.mapping(physical_frame);
            ImagePaint::rendered(
                rendered,
                mapping,
                self.submissions.clone(),
                ui.ctx().clone(),
            )
        } else {
            let provisional = match proxy.image(self.target) {
                Ok(image) => image,
                Err(error) => {
                    ui.label(format!("View unavailable: {error}"));
                    return None;
                }
            };
            if let Err(error) = provisional.require_processing(&parameters) {
                ui.label(format!("Preview unavailable: {error}"));
                return None;
            }
            let (image, parameters, mapping) = match &self.original.settled {
                Some((settled_revision, settled_view, image))
                    if *settled_revision == revision && !cropping =>
                {
                    let mapping = if settled_view.frame.is_some() {
                        crate::geometry::TextureMapping::IDENTITY
                    } else {
                        geometry.uv(draw, settled_view.area)
                    };
                    (image.clone(), None, mapping)
                }
                _ => (provisional, Some(parameters), geometry.uv(draw, area)),
            };
            match processing::image_paint(image, parameters, mapping, ui.ctx()) {
                Ok(paint) => paint,
                Err(error) => {
                    ui.label(format!("Preview unavailable: {error}"));
                    return None;
                }
            }
        };
        Some((draw, paint))
    }

    fn canvas_pick(&mut self, ui: &egui::Ui, response: &egui::Response, geometry: CanvasLayout) {
        if response.clicked_by(egui::PointerButton::Primary)
            && let Some(point) = response.interact_pointer_pos()
            && (geometry.image.contains(point) || self.analysis_review.is_some())
        {
            if self.analysis_review.is_some() {
                self.select_review_area(geometry, point);
            } else {
                self.region = Some(geometry.pick_patch(point, 32));
            }
        }
        if response.drag_started_by(egui::PointerButton::Primary) {
            if self.analysis_review.is_some()
                && let Some(point) = ui.input(|i| i.pointer.press_origin())
            {
                self.select_review_area(geometry, point);
            }
            // Classify from the press, not the later point where egui recognizes the drag.
            self.pick_drag = ui
                .input(|input| input.pointer.press_origin())
                .filter(|point| geometry.image.contains(*point) || self.analysis_review.is_some())
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
        if self.analysis_review.is_some() {
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
        let composition = self.canvas_composition(area, active_pick);
        let cropping = !fullscreen && self.crop_edit.is_some();
        let geometry =
            self.canvas_layout(canvas, composition, pixels_per_point, zoom_action, cropping);
        let (geometry, panning) =
            self.canvas_navigation(ui, &response, geometry, canvas_input, cropping, fullscreen);
        let Some((draw, paint)) = self.canvas_image(ui, geometry, area, proxy, pending, cropping)
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
