use super::*;

pub(super) fn surface() -> egui::Color32 {
    egui::Color32::from_gray(16)
}

impl Editor {
    pub(super) fn source_parameters(&self, exposure: f64) -> Result<ProcessingParameters, Error> {
        if let Some(ready) = &self.active_canvas().ready {
            return self.source_preview_parameters(
                ready
                    .developed
                    .metadata()
                    .source_preview(ready.developed.fixed_wb()),
                exposure,
            );
        }
        let source = self
            .document
            .visit
            .current_source
            .and_then(|id| self.document.state.project.source(id).ok())
            .or_else(|| self.document.state.project.sources.first());
        match source {
            Some(source) => self.source_parameters_for(source, exposure),
            None => self.source_preview_parameters(Default::default(), exposure),
        }
    }

    pub(super) fn source_parameters_for(
        &self,
        source: &Source,
        exposure: f64,
    ) -> Result<ProcessingParameters, Error> {
        self.source_preview_parameters(self.document.source_preview(source), exposure)
    }

    fn source_preview_parameters(
        &self,
        preview: crate::raw::SourcePreview,
        exposure: f64,
    ) -> Result<ProcessingParameters, Error> {
        let transform = if self.document.input.is_negative() {
            preview.transform(self.source_white_balance)
        } else {
            glam::DMat3::IDENTITY
        };
        ProcessingParameters::source_preview_with_transform(
            // DevelopedSource already restores LibRaw's green-relative exposure scale.
            None, exposure, transform,
        )
    }

    fn viewer_tool(&self) -> Option<(icons::Icon, &'static str)> {
        if self.document.analysis_review().is_some() {
            return Some((icons::Icon::Analyze, "Analyze"));
        }
        if self.document.auto_crop_edit().is_some() {
            return Some((icons::Icon::Crop, "Auto-crop"));
        }
        self.legacy_pick().map(|display| {
            (
                if display.area {
                    icons::Icon::Crop
                } else {
                    icons::Icon::Pick
                },
                display.label,
            )
        })
    }

    pub(super) fn source_tool_warning(&self, id: SourceId) -> Option<String> {
        if self.document.auto_crop_edit().is_some() {
            return self.auto_crop_problem(id).map(str::to_owned);
        }
        if self.document.visit.current_source == Some(id) && self.cropping() {
            return self.current_tool()?.presentation().warning;
        }
        self.review_warning(id).map(str::to_owned)
    }

    fn viewer_warning(&self) -> Option<String> {
        let mut warning = self
            .document
            .visit
            .current_source
            .and_then(|id| self.source_tool_warning(id));
        if warning.is_none() {
            warning = self.auto_crop_batch_warning().map(str::to_owned);
        }
        if let Some(review) = &self.document.analysis_review() {
            let missing = review
                .approved
                .entries
                .iter()
                .filter(|e| !e.areas.ready())
                .count();
            if warning.is_none() && missing > 0 {
                warning = Some(format!(
                    "Sampling areas missing: {missing} photo{}",
                    if missing == 1 { "" } else { "s" }
                ));
            }
            if review.replaces_photos {
                let consequence =
                    "Analyze will replace Photo layers and remove any adjustment layers and masks.";
                if let Some(warning) = &mut warning {
                    warning.push_str(&format!("\n{consequence}"));
                } else {
                    warning = Some(consequence.into());
                }
            }
        }
        warning
    }

    pub(super) fn viewer_bar(
        &mut self,
        ui: &mut egui::Ui,
        cx: &UiContext<'_, Self>,
    ) -> Option<f32> {
        use icons::Icon;
        use presentation::{TextAction, ToolGroup, ToolIdentity, VIEWER_ITEM_GAP};

        let mut zoom_action = None;
        let fullscreen = self.fullscreen.is_some();
        let hosted = (!fullscreen)
            .then(|| {
                self.current_tool()
                    .map(|tool| tool.presentation())
                    .filter(|tool| tool.show_viewer)
            })
            .flatten();
        let fit_label = hosted.as_ref().map_or("Fit", |tool| tool.fit_label);
        let masking = !fullscreen && self.masking();
        let tool = hosted
            .as_ref()
            .map(|tool| (tool.identity.icon, tool.identity.title))
            .or_else(|| (!fullscreen).then(|| self.viewer_tool()).flatten());
        let has_repairs = self
            .document
            .visit
            .current_source
            .and_then(|id| self.dust_repairs(id))
            .is_some_and(|repairs| !repairs.repairs().is_empty());

        let mut cancel = false;
        let mut apply = false;
        let calibrated = self.document.input.ready();
        if !calibrated && self.document.flat_edit().is_none() {
            self.document.source = true;
        }

        let navigating =
            self.active_canvas().ready.is_some() && !self.busy && !self.mask_dragging();
        let reviewing = self.document.analysis_review().is_some();
        let apply_label = if reviewing { "Analyze roll" } else { "Apply" };
        let review_ready = self.document.analysis_review().is_none_or(|review| {
            review
                .approved
                .entries
                .iter()
                .all(|entry| entry.areas.ready())
        });
        let tool_group = tool.map(|(icon, title)| ToolGroup {
            identity: ToolIdentity { icon, title },
            busy: self.busy,
            cancel_enabled: !self.busy || reviewing && !self.analysis_fitting(),
            apply_enabled: if self.document.auto_crop_edit().is_some() {
                self.auto_crop_editable()
            } else {
                navigating
                    && review_ready
                    && hosted.as_ref().map_or_else(
                        || self.legacy_pick_region().is_some(),
                        |tool| tool.primary.is_none_or(|primary| primary.enabled),
                    )
            },
        });

        let warning = hosted
            .as_ref()
            .and_then(|tool| tool.warning.clone())
            .or_else(|| self.viewer_warning());
        let look_error = self.look_rendering_error();
        let normal_controls =
            tool.is_none() || hosted.is_some() && self.document.visit.picker.display().is_none();
        let inspecting = navigating && calibrated && tool.is_none() && !masking;
        let bar = if fullscreen {
            presentation::ViewerBar::Overlay
        } else {
            presentation::ViewerBar::Docked
        };

        bar.show(ui, |ui| {
            if fullscreen
                && let Some(view) = self.tasks.view()
                && let Some(request) = view.compact(ui)
            {
                self.cancel_task(request);
            }
            ui.spacing_mut().item_spacing.x = VIEWER_ITEM_GAP;
            icons::row(ui, |ui| {
                let target = if self.document.target == OutputTarget::DisplayP3 {
                    "Display P3"
                } else {
                    "sRGB"
                };
                let quality = if self.document.visit.pending_preview.is_some() {
                    " · Cached preview"
                } else if self.canvas_inspection() || self.active_canvas().ready.is_none() {
                    ""
                } else if self.document.visit.working_view.enabled() && normal_controls {
                    if look_error.is_some() {
                        " · Look unavailable"
                    } else if self.document.visit.working_view.error.is_some() {
                        " · View unavailable"
                    } else if self.document.visit.working_view.compare && self.active_canvas().grade.original.failed() {
                        " · Reference unavailable"
                    } else if self.document.visit.working_view.compare && self.document.visit.working_view.reference_ready() {
                        " · Original reference"
                    } else if self.document.visit.working_view.compare {
                        " · Preparing reference"
                    } else if self.document.visit.working_view.displayed.is_none() {
                        " · Acquiring view"
                    } else {
                        ""
                    }
                } else if self.active_canvas().grade.original.refining() {
                    " · Refining"
                } else if self.active_canvas().grade.original.settled().is_none() {
                    " · Approximate"
                } else {
                    ""
                };
                let state = if has_repairs {
                    format!("{target}{quality} · {}", self.dust_render_state())
                } else {
                    format!("{target}{quality}")
                };
                let trailing_width = presentation::CONTROL_HEIGHT
                    + VIEWER_ITEM_GAP
                    + if let Some(group) = tool_group {
                        if let Some(tool) = &hosted { group.width_actions(ui, tool.primary.map(|primary| primary.label), tool.dismiss) }
                        else if reviewing {
                            group.width_named(ui, apply_label)
                        } else {
                            group.width(ui)
                        }
                    } else {
                        ui.painter()
                            .layout_no_wrap(
                                state.clone(),
                                egui::TextStyle::Small.resolve(ui.style()),
                                ui.visuals().text_color(),
                            )
                            .size()
                            .x
                    };
                let source_controls = usize::from(normal_controls)
                    + usize::from(self.canvas_inspection()
                        || normal_controls && self.document.visit.working_view.enabled());
                // Fit, zoom steps and fullscreen, plus the readout, separator and current view controls.
                let navigation_icons = 4 + source_controls;
                let navigation_width = 50.
                    + navigation_icons as f32 * presentation::CONTROL_HEIGHT
                    + (navigation_icons + 3) as f32 * VIEWER_ITEM_GAP;
                let show_zoom_steps = tool.is_none() && !masking
                    || ui.available_width() >= trailing_width + navigation_width;
                if icons::button(
                    ui,
                    Icon::Fit,
                    navigating,
                    fit_label,
                    &format!("{fit_label} (Z)."),
                )
                .clicked()
                {
                    self.with_tool_viewport(Viewport::fit);
                }
                let layout = self.active_canvas().last_layout.map(|(layout, _)| layout);
                let zoom = layout.map_or("—".into(), |layout| {
                    format!("{:.0}%", 100. * layout.scale * ui.ctx().pixels_per_point())
                });
                let zoom = TextAction::new(&zoom)
                    .readout()
                    .enabled(navigating)
                    .toolbar(50.)
                    .name("100%")
                    .help("Zoom to 100% (one image pixel per display pixel).")
                    .show(ui);
                if zoom.clicked() {
                    zoom_action = Some(1.);
                }
                if show_zoom_steps {
                    for (icon, name, factor) in [
                        (Icon::ZoomOut, "Zoom out", 1. / 1.25),
                        (Icon::ZoomIn, "Zoom in", 1.25),
                    ] {
                        if icons::button(ui, icon, navigating, name, name).clicked() {
                            zoom_action = Some(-factor);
                        }
                    }
                }
                let (icon, name) = if fullscreen {
                    (Icon::ExitFullscreen, "Exit fullscreen")
                } else {
                    (Icon::Fullscreen, "Fullscreen viewing")
                };
                if icons::button(
                    ui,
                    icon,
                    self.fullscreen_available(),
                    name,
                    if fullscreen {
                        "Exit fullscreen (F or Escape)."
                    } else {
                        "View photo fullscreen (F)."
                    },
                )
                .clicked()
                {
                    self.toggle_fullscreen(ui.ctx());
                }
                ui.separator();
                if normal_controls {
                    ui.add_enabled_ui(inspecting, |ui| {
                        icons::toggle(
                            ui,
                            Icon::Source,
                            &mut self.document.source,
                            "View source",
                            if calibrated {
                                "Switch between the positive grade and developed source."
                            } else {
                                "Measure Dmin and Dmax before viewing a positive grade."
                            },
                        );
                    });
                }
                if !self.canvas_inspection() && normal_controls && self.document.visit.working_view.enabled() {
                    ui.add_enabled_ui(inspecting, |ui| {
                        icons::toggle(
                            ui,
                            Icon::Inspection,
                            &mut self.document.visit.working_view.compare,
                            "Original-resolution reference",
                            "Compare with original-resolution processing; export is unchanged.",
                        );
                    });
                }
                let inspection_id = ui.make_persistent_id("source-inspection-popup");
                if self.canvas_inspection() {
                    let exposure = format!("Source {:+.2} EV", self.source_exposure);
                    let inspection = icons::button(
                        ui,
                        Icon::Inspection,
                        navigating,
                        "Source inspection",
                        "Adjust source-preview white balance and brightness.",
                    );
                    egui::Popup::from_toggle_button_response(&inspection)
                        .id(inspection_id)
                        .layout(egui::Layout::top_down(egui::Align::Min))
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                        .show(|ui| {
                            ui.set_width(240.);
                            ui.label(egui::RichText::new("Source inspection").strong());
                            ui.add_enabled_ui(navigating, |ui| {
                                if self.document.input.is_negative() {
                                    use crate::raw::SourceWhiteBalance;
                                    presentation::labelled_row(ui, "Viewing WB", None, |ui, _| {
                                        ui.selectable_value(
                                            &mut self.source_white_balance,
                                            SourceWhiteBalance::AsShot,
                                            "As shot",
                                        );
                                        ui.selectable_value(
                                            &mut self.source_white_balance,
                                            SourceWhiteBalance::Daylight,
                                            "Daylight",
                                        );
                                    });
                                }
                                controls::parameter(
                                    ui,
                                    controls::Parameter::Exposure,
                                    &mut self.source_exposure,
                                );
                            });
                            ui.small("Canvas and source thumbnails · not saved");
                        });
                    let readout = presentation::instrument_readout(
                        &exposure,
                        egui::TextStyle::Small.resolve(ui.style()).size,
                    );
                    let exposure_width = egui::WidgetText::from(readout.clone())
                        .into_galley(
                            ui,
                            Some(egui::TextWrapMode::Extend),
                            f32::INFINITY,
                            egui::TextStyle::Small,
                        )
                        .size()
                        .x;
                    if ui.available_width() >= trailing_width + exposure_width + 40. {
                        ui.label(readout);
                    }
                } else {
                    egui::Popup::close_id(ui.ctx(), inspection_id);
                }
                if let Some(error) = &look_error {
                    ui.colored_label(egui::Color32::LIGHT_RED, "Look rendering failed");
                    if TextAction::new("Details").toolbar(60.).name("Look rendering details").show(ui).clicked() {
                        self.report_failure(FailureKind::LookRendering,error.to_string());
                    }
                } else if let Some(error) = &self.document.visit.working_view.error {
                    ui.colored_label(egui::Color32::LIGHT_RED, "View unavailable")
                        .on_hover_text(error);
                }
                let help = icons::button(
                    ui,
                    Icon::Help,
                    true,
                    "Viewer help",
                    "Navigation and active-tool shortcuts.",
                );
                egui::Popup::from_toggle_button_response(&help)
                    .layout(egui::Layout::top_down(egui::Align::Min))
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .show(|ui| {
                        ui.set_width(290.);
                        ui.label(egui::RichText::new("Viewer navigation").strong());
                        ui.small(hosted.as_ref().and_then(|tool| tool.navigation_help)
                            .unwrap_or("Scroll pans · Cmd/Ctrl-scroll or pinch zooms"));
                        ui.small("Click: Fit / 100% outside canvas tools · Z: Fit · F: Fullscreen");
                        ui.small("Zoomed drag pans · Space-drag / middle-drag always pans");
                        if !fullscreen && tool.is_none() {
                            ui.separator();
                            ui.label(egui::RichText::new("Filmstrip selection").strong());
                            ui.small("Cmd-click toggles selection · Shift-click selects a range");
                            ui.small("Shift–Cmd–A: select active photo only");
                        }
                        if fullscreen {
                            ui.small("F or Escape: return to the workspace");
                        }
                        if let Some(help) = self.mask_help().filter(|_| {
                            !fullscreen
                                && self.tab == 1
                                && hosted.is_none()
                                && self.legacy_pick().is_none()
                        }) {
                            ui.separator();
                            ui.small(help);
                        } else if let Some(tool) = &hosted {
                            ui.separator();
                            ui.small(tool.help);
                            ui.small(if tool.primary.is_some() {
                                "Enter: primary action · Escape: dismiss"
                            } else {
                                "Escape: dismiss"
                            });
                        } else if !fullscreen &&self.document.auto_crop_edit().is_some() {
                            ui.separator();
                            ui.small(
                                "Auto-crop: drag an edge or corner for the shared inward margin; Space-drag / middle-drag pans.",
                            );
                            ui.small(
                                "Browsing is optional and never retargets the proposals · H hides artwork.",
                            );
                            ui.small("Enter applies the fixed set · Escape cancels");

                        } else if !fullscreen &&self.document.analysis_review().is_some() {
                            ui.separator();
                            ui.small(
                                "Drag either area's corners to resize, or its interior to move. Click a label when outlines coincide.",
                            );
                            ui.small("Enter analyzes · Escape cancels");
                        } else if !fullscreen && let Some(kind) = self.legacy_pick() {
                            ui.separator();
                            ui.small(
                                "Click a patch or drag an area; drag its edges/corners to resize.",
                            );
                            if let Some(help) = kind.help { ui.small(help); }
                            ui.small("Enter applies · Escape cancels");
                        }
                    });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(group) = tool_group {
                        if let Some(tool) = &hosted {
                            let actions = group.show_actions(ui, tool.primary.map(|primary| primary.label), tool.dismiss, warning.as_deref()).inner;
                            cancel = actions.dismiss.clicked();
                            apply = actions.primary.is_some_and(|response| response.clicked());
                            return;
                        }
                        let actions = if warning.is_some() {
                            group.show_warned(ui, apply_label, warning.as_deref())
                        } else if reviewing {
                            group.show_named(ui, apply_label)
                        } else {
                            group.show(ui)
                        }
                        .inner;
                        cancel = actions.cancel.clicked();
                        apply = actions.apply.clicked();

                    } else {
                        ui.add(
                            egui::Label::new(egui::RichText::new(state).small().weak()).truncate(),
                        );
                    }
                });
            });
        });

        if cancel {
            if hosted.is_some() {
                self.hosted_action(tool::Action::Dismiss);
            } else if self.document.auto_crop_edit().is_some() {
                self.finish_auto_crop(false);
            } else if masking {
                self.finish_mask(false);
            } else {
                self.finish_pick(cx);
            }
            ui.ctx().request_repaint();
        } else if apply {
            if hosted.is_some() {
                self.hosted_action(tool::Action::Primary);
            } else if self.document.auto_crop_edit().is_some() {
                self.finish_auto_crop(true);
            } else {
                self.apply_pick(cx);
            }
            ui.ctx().request_repaint();
        }
        zoom_action
    }

    pub(super) fn canvas_input_allowed(&self, ui: &egui::Ui, canvas: egui::Rect) -> bool {
        if self.menus.panel.is_some()
            || self.document.visit.adjustment_layers.confirming()
            || egui::Popup::is_any_open(ui.ctx())
        {
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
