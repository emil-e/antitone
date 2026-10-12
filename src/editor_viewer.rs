use super::*;

pub(super) fn surface() -> egui::Color32 {
    egui::Color32::from_gray(16)
}

impl Editor {
    fn viewer_tool(&self) -> Option<(icons::Icon, &'static str)> {
        if self.crop_edit.is_some() {
            return Some((icons::Icon::Crop, "Crop"));
        }
        self.pick.map(|pick| match pick {
            Pick::Dmin => (icons::Icon::Pick, "Dmin"),
            Pick::Dmax => (icons::Icon::Pick, "Dmax"),
            Pick::Gamma => (icons::Icon::Pick, "Gamma neutral"),
            Pick::Wb => (icons::Icon::Pick, "WB neutral"),
            Pick::Flat => (icons::Icon::Crop, "Flat-field"),
        })
    }

    pub(super) fn viewer_bar(&mut self, ui: &mut egui::Ui) -> Option<f32> {
        use icons::Icon;
        let mut zoom_action = None;
        let cropping = self.crop_edit.is_some();
        let tool = self.viewer_tool();
        let mut cancel = false;
        let mut apply = false;
        let calibrated = self.calibration().is_some();
        if !calibrated {
            self.source = true;
        }
        let navigating = self.ready.is_some() && !self.busy;
        let inspecting = navigating && calibrated && tool.is_none();
        egui::Panel::bottom("viewer-bar")
            .exact_size(40.)
            .show_separator_line(false)
            .frame(egui::Frame::side_top_panel(ui.style()).fill(surface()))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 4.;
                icons::row(ui, |ui| {
                    let target = if self.target == OutputTarget::DisplayP3 { "Display P3" } else { "sRGB" };
                    let quality = if self.source || self.ready.is_none() { "" } else if self.refining.is_some() { " · Refining" } else if self.settled.is_none() { " · Approximate" } else { "" };
                    let state = format!("{target}{quality}");
                    let tool_font = egui::FontId::proportional(12.);
                    let tool_action_space = 8.;
                    let trailing_width = if let Some((_, title)) = tool {
                        let title_width = ui.painter().layout_no_wrap(title.into(), tool_font.clone(), ui.visuals().text_color()).size().x;
                        let actions_width = ["Cancel", "Apply"].into_iter().map(|label| ui.painter().layout_no_wrap(label.into(), egui::TextStyle::Button.resolve(ui.style()), ui.visuals().text_color()).size().x + 2. * ui.spacing().button_padding.x).sum::<f32>();
                        title_width + actions_width + 16. + 12. + 3. * ui.spacing().item_spacing.x + tool_action_space
                    } else {
                        ui.painter().layout_no_wrap(state.clone(), egui::TextStyle::Small.resolve(ui.style()), ui.visuals().text_color()).size().x
                    };
                    let show_zoom_steps = tool.is_none() || ui.available_width() >= trailing_width + if self.source { 224. } else { 192. };
                    if icons::button(ui, Icon::Fit, navigating, if cropping { "Fit source" } else { "Fit" }, "Fit the image (Z); crop mode fits the full source.").clicked() {
                        self.viewport.fit();
                    }
                    let layout = self.crop_edit.as_ref().and_then(|edit| edit.layout).or(self.last_layout.map(|(layout, _)| layout));
                    let zoom = layout.map_or("—".into(), |layout| format!("{:.0}%", 100. * layout.scale * ui.ctx().pixels_per_point()));
                    let zoom = ui.add_enabled(navigating, egui::Button::new(zoom).min_size(egui::vec2(50., 28.))).on_hover_text("Show 100%: one source pixel per physical display pixel.");
                    zoom.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, zoom.enabled(), "100%"));
                    if zoom.clicked() { zoom_action = Some(1.); }
                    if show_zoom_steps {
                        for (icon, name, factor) in [(Icon::ZoomOut, "Zoom out", 1. / 1.25), (Icon::ZoomIn, "Zoom in", 1.25)] {
                            if icons::button(ui, icon, navigating, name, "Zoom around the viewer centre.").clicked() { zoom_action = Some(-factor); }
                        }
                    }
                    ui.separator();
                    if tool.is_none() {
                        ui.add_enabled_ui(inspecting, |ui| {
                            icons::toggle(ui, Icon::Source, &mut self.source, "View source", if calibrated { "Switch between the positive grade and developed source." } else { "Measure Dmin and Dmax before viewing a positive grade." });
                        });
                    }
                    let inspection_id = ui.make_persistent_id("source-inspection-popup");
                    if self.source {
                        let exposure = format!("Source {:+.2} EV", self.source_exposure);
                        let inspection = icons::button(ui, Icon::Inspection, navigating, "Source inspection", &format!("{exposure} · Adjust source-preview brightness only; grade and export are unchanged."));
                        egui::Popup::from_toggle_button_response(&inspection)
                            .id(inspection_id)
                            .layout(egui::Layout::top_down(egui::Align::Min))
                            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                            .show(|ui| {
                                ui.set_width(240.);
                                ui.label(egui::RichText::new("Source inspection").strong());
                                ui.add_enabled_ui(navigating, |ui| {
                                    controls::parameter(ui, controls::Parameter::Exposure, &mut self.source_exposure);
                                });
                                ui.small("Preview brightness only · not saved");
                            });
                        let exposure_width = ui.painter().layout_no_wrap(exposure.clone(), egui::TextStyle::Small.resolve(ui.style()), ui.visuals().text_color()).size().x;
                        if ui.available_width() >= trailing_width + exposure_width + 40. {
                            ui.small(exposure);
                        }
                    } else {
                        egui::Popup::close_id(ui.ctx(), inspection_id);
                    }
                    let help = icons::button(ui, Icon::Help, true, "Viewer help", "Navigation and active-tool shortcuts.");
                    egui::Popup::from_toggle_button_response(&help)
                        .layout(egui::Layout::top_down(egui::Align::Min))
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                        .show(|ui| {
                            ui.set_width(290.);
                            ui.label(egui::RichText::new("Viewer navigation").strong());
                            ui.small("Scroll pans · Cmd/Ctrl-scroll or pinch zooms");
                            ui.small("Click: Fit / 100% · Z: Fit");
                            ui.small("Zoomed drag pans · Space-drag / middle-drag always pans");
                            if cropping {
                                ui.separator();
                                ui.small("Crop: drag inside to move the image, edges/corners to resize, outside to straighten.");
                                ui.small("Enter applies · Escape cancels");
                            } else if let Some(kind) = self.pick {
                                ui.separator();
                                ui.small("Click a patch or drag an area; drag its edges/corners to resize.");
                                if matches!(kind, Pick::Dmin | Pick::Dmax) {
                                    ui.small("To sample another image, select it in the filmstrip. Apply/Cancel returns to the working photo.");
                                }
                                ui.small("Enter applies · Escape cancels");
                            }
                        });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some((icon, title)) = tool {
                            egui::Frame::NONE.fill(ui.visuals().selection.bg_fill.linear_multiply(0.15)).inner_margin(egui::Margin::symmetric(6, 0)).show(ui, |ui| {
                                ui.allocate_ui_with_layout(egui::vec2(trailing_width - 12., 28.), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                    let (rect, _) = ui.allocate_exact_size(egui::vec2(16., 16.), egui::Sense::hover());
                                    if self.busy {
                                        ui.put(rect, egui::Spinner::new().size(16.));
                                    } else {
                                        icon.paint(ui, rect, ui.visuals().selection.stroke);
                                    }
                                    let label = ui.add(egui::Label::new(egui::RichText::new(title).font(tool_font).strong()));
                                    label.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, label.enabled(), format!("Active tool: {title}{}", if self.busy { " · Working" } else { "" })));
                                    ui.add_space(tool_action_space);
                                    cancel = ui.add_enabled(!self.busy, egui::Button::new("Cancel").min_size(egui::vec2(0., 28.))).on_hover_text("Cancel the active tool (Escape).").clicked();
                                    apply = ui.add_enabled(navigating && (cropping || self.region.is_some()), egui::Button::new("Apply").fill(ui.visuals().selection.bg_fill).min_size(egui::vec2(0., 28.))).on_hover_text("Apply the active tool (Enter).").clicked();
                                });
                            });
                        } else {
                            ui.add(egui::Label::new(egui::RichText::new(state).small().weak()).truncate()).on_hover_text("Shared preview / thumbnail / scope / TIFF target · 100 nit. EXR is scene-linear AP0.");
                        }
                    });
                });
            });
        if cancel {
            if cropping {
                self.finish_crop(false);
            } else {
                self.finish_pick();
            }
            ui.ctx().request_repaint();
        } else if apply {
            if cropping {
                self.finish_crop(true);
            } else {
                self.apply_pick();
            }
            ui.ctx().request_repaint();
        }
        zoom_action
    }

    pub(super) fn canvas_input_allowed(&self, ui: &egui::Ui, canvas: egui::Rect) -> bool {
        if egui::Popup::is_any_open(ui.ctx()) {
            return false;
        }
        let positions = ui.ctx().input(|input| {
            [
                input.pointer.hover_pos(),
                input.pointer.press_origin(),
                input.multi_touch().map(|touch| touch.center_pos),
            ]
        });
        positions.into_iter().flatten().all(|position| {
            !(canvas.contains(position)
                && ui
                    .ctx()
                    .layer_id_at(position)
                    .is_some_and(|layer| layer != ui.layer_id()))
        })
    }
}
