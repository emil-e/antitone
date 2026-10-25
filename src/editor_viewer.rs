use super::*;

pub(super) fn surface() -> egui::Color32 {
    egui::Color32::from_gray(16)
}

impl Editor {
    pub(super) fn source_parameters(&self, exposure: f64) -> Result<ProcessingParameters, Error> {
        let decode = self
            .document
            .visit
            .ready
            .as_ref()
            .map(|ready| &ready.record.development.decode)
            .or_else(|| {
                self.document
                    .project
                    .sources
                    .first()
                    .map(|source| &source.development.decode)
            });
        ProcessingParameters::source_preview(
            // SDK camera-to-PCS already defines the balanced white scale; only
            // LibRaw's max-gain normalization needs this preview compensation.
            decode
                .filter(|d| d.contract != crate::project::DNG_DEVELOPMENT_CONTRACT)
                .map(|d| d.fixed_wb),
            exposure,
        )
    }

    fn viewer_tool(&self) -> Option<(icons::Icon, &'static str)> {
        if self.document.analysis_review.is_some() {
            return Some((icons::Icon::Analyze, "Analyze"));
        }
        if self.document.visit.crop_edit.is_some() {
            return Some((icons::Icon::Crop, "Crop"));
        }
        if self.document.auto_crop_edit.is_some() {
            return Some((icons::Icon::Crop, "Auto-crop"));
        }
        self.pick.map(|pick| match pick {
            Pick::Dmin => (icons::Icon::Pick, "Dmin"),
            Pick::Dmax => (icons::Icon::Pick, "Dmax"),
            Pick::Gamma => (icons::Icon::Pick, "Gamma neutral"),
            Pick::RollWb => (icons::Icon::Pick, "Roll WB neutral"),
            Pick::Wb => (icons::Icon::Pick, "WB neutral"),
            Pick::Flat => (icons::Icon::Crop, "Flat-field"),
            Pick::Analysis => (icons::Icon::Crop, "Picture area"),
        })
    }

    pub(super) fn source_tool_warning(&self, id: SourceId) -> Option<&str> {
        if self.document.auto_crop_edit.is_some() {
            return self.auto_crop_problem(id);
        }
        if self.current_source == Some(id)
            && let Some(edit) = &self.document.visit.crop_edit
        {
            return edit.auto_problem.as_deref();
        }
        self.review_warning(id)
    }

    fn viewer_warning(&self) -> Option<String> {
        let mut warning = self
            .current_source
            .and_then(|id| self.source_tool_warning(id))
            .map(str::to_owned);
        if warning.is_none() {
            warning = self.auto_crop_batch_warning().map(str::to_owned);
        }
        if let Some(review) = &self.document.analysis_review {
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
                let consequence = "Analyze will replace Photo adjustments and remove any local adjustments and masks.";
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
        use presentation::{
            SessionToolGroup, TextAction, ToolGroup, ToolIdentity, VIEWER_ITEM_GAP,
        };

        let mut zoom_action = None;
        let fullscreen = self.fullscreen.is_some();
        let cropping = !fullscreen && self.document.visit.crop_edit.is_some();
        let masking = !fullscreen && self.document.visit.mask_edit.is_some();
        let creating = !fullscreen
            && self
                .document
                .visit
                .mask_edit
                .as_ref()
                .is_some_and(MaskEdit::creating);
        let dust_session = !fullscreen && self.document.dust.cleanup && !self.source;
        let tool = (!fullscreen && !dust_session)
            .then(|| self.viewer_tool())
            .flatten();
        let session_group = dust_session.then_some(SessionToolGroup {
            identity: ToolIdentity {
                icon: Icon::Dust,
                title: "Dust cleanup",
            },
            busy: self.busy,
            done_enabled: !self.busy,
        });
        let has_repairs = self
            .current_source
            .and_then(|id| self.dust_repairs(id))
            .is_some_and(|repairs| !repairs.repairs().is_empty());
        let mut finish_dust = false;

        let mut cancel = false;
        let mut apply = false;
        let calibrated = self.input.ready();
        if !calibrated {
            self.source = true;
        }

        let navigating = self.document.visit.ready.is_some()
            && !self.busy
            && !self
                .document
                .visit
                .mask_edit
                .as_ref()
                .is_some_and(MaskEdit::dragging);
        let reviewing = self.document.analysis_review.is_some();
        let apply_label = if reviewing { "Analyze roll" } else { "Apply" };
        let review_ready = self.document.analysis_review.as_ref().is_none_or(|review| {
            review
                .approved
                .entries
                .iter()
                .all(|entry| entry.areas.ready())
        });
        let tool_group = tool.map(|(icon, title)| ToolGroup {
            identity: ToolIdentity { icon, title },
            busy: self.busy,
            cancel_enabled: !self.busy || reviewing && self.document.analysis_pending.is_none(),
            apply_enabled: if self.document.auto_crop_edit.is_some() {
                self.auto_crop_editable()
            } else {
                navigating && review_ready && (cropping || self.region.is_some())
            },
        });

        let warning = self.viewer_warning();
        let look_error = self.look_render_failure();
        let inspecting = navigating && calibrated && tool.is_none() && !dust_session && !masking;
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
                let target = if self.target == OutputTarget::DisplayP3 {
                    "Display P3"
                } else {
                    "sRGB"
                };
                let quality = if self.pending_preview.is_some() {
                    " · Cached preview"
                } else if self.source || self.document.visit.ready.is_none() {
                    ""
                } else if self.working.enabled() && tool.is_none() {
                    if look_error.is_some() {
                        " · Look unavailable"
                    } else if self.working.error.is_some() {
                        " · View unavailable"
                    } else if self.working.compare && self.original.refinement_error == Some(self.original.revision()) {
                        " · Reference unavailable"
                    } else if self.working.compare && self.working.reference_ready() {
                        " · Original reference"
                    } else if self.working.compare {
                        " · Preparing reference"
                    } else if self.working.displayed.is_none() {
                        " · Acquiring view"
                    } else {
                        ""
                    }
                } else if self.original.refining.is_some() {
                    " · Refining"
                } else if self.original.settled.is_none() {
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
                        if reviewing {
                            group.width_named(ui, apply_label)
                        } else {
                            group.width(ui)
                        }
                    } else if let Some(group) = session_group {
                        group.width(ui)
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
                let show_zoom_steps = tool.is_none() && !dust_session && !masking
                    || ui.available_width()
                        >= trailing_width
                            + presentation::CONTROL_HEIGHT
                            + VIEWER_ITEM_GAP
                            + if self.source { 224. } else { 192. };
                if icons::button(
                    ui,
                    Icon::Fit,
                    navigating,
                    if cropping { "Fit source" } else { "Fit" },
                    if cropping {
                        "Fit the full source (Z)."
                    } else {
                        "Fit the image (Z)."
                    },
                )
                .clicked()
                {
                    self.viewport.fit();
                }
                let layout = if cropping {
                    self.document.visit.crop_edit.as_ref().and_then(|edit| edit.layout)
                } else {
                    None
                }
                .or(self.last_layout.map(|(layout, _)| layout));
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
                if tool.is_none() {
                    ui.add_enabled_ui(inspecting, |ui| {
                        icons::toggle(
                            ui,
                            Icon::Source,
                            &mut self.source,
                            "View source",
                            if calibrated {
                                "Switch between the positive grade and developed source."
                            } else {
                                "Measure Dmin and Dmax before viewing a positive grade."
                            },
                        );
                    });
                }
                if !self.source && tool.is_none() && self.working.enabled() {
                    ui.add_enabled_ui(inspecting, |ui| {
                        icons::toggle(
                            ui,
                            Icon::Inspection,
                            &mut self.working.compare,
                            "Original-resolution reference",
                            "Compare with original-resolution processing; export is unchanged.",
                        );
                    });
                }
                let inspection_id = ui.make_persistent_id("source-inspection-popup");
                if self.source {
                    let exposure = format!("Source {:+.2} EV", self.source_exposure);
                    let inspection = icons::button(
                        ui,
                        Icon::Inspection,
                        navigating,
                        "Source inspection",
                        "Adjust source-preview brightness.",
                    );
                    egui::Popup::from_toggle_button_response(&inspection)
                        .id(inspection_id)
                        .layout(egui::Layout::top_down(egui::Align::Min))
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                        .show(|ui| {
                            ui.set_width(240.);
                            ui.label(egui::RichText::new("Source inspection").strong());
                            ui.add_enabled_ui(navigating, |ui| {
                                controls::parameter(
                                    ui,
                                    controls::Parameter::Exposure,
                                    &mut self.source_exposure,
                                );
                            });
                            ui.small("Relative to default source preview · not saved");
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
                } else if let Some(error) = &self.working.error {
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
                        ui.small(if dust_session {
                            "Scroll changes brush size · Cmd/Ctrl-scroll or pinch zooms"
                        } else {
                            "Scroll pans · Cmd/Ctrl-scroll or pinch zooms"
                        });
                        ui.small("Click: Fit / 100% outside canvas tools · Z: Fit · F: Fullscreen");
                        ui.small("Zoomed drag pans · Space-drag / middle-drag always pans");
                        if !fullscreen && tool.is_none() && !dust_session {
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
                                && !dust_session
                                && !cropping
                                && self.pick.is_none()
                        }) {
                            ui.separator();
                            ui.small(help);
                        } else if cropping {
                            ui.separator();
                            ui.small(
                                "Crop: drag inside to move the image, edges/corners to resize, outside to straighten.",
                            );
                            ui.small("Enter applies · Escape cancels");
                        } else if !fullscreen && self.document.auto_crop_edit.is_some() {
                            ui.separator();
                            ui.small(
                                "Auto-crop: drag an edge or corner for the shared inward margin; Space-drag / middle-drag pans.",
                            );
                            ui.small(
                                "Browsing is optional and never retargets the proposals · H hides artwork.",
                            );
                            ui.small("Enter applies the fixed set · Escape cancels");
                        } else if dust_session {
                            ui.separator();
                            ui.small(
                                "Draw for a new heal · click an outline to select another heal.",
                            );
                            ui.small(
                                "Shift-drag adds to selected · Option-drag removes · Delete removes selected.",
                            );
                            ui.small(
                                "Drag the source outline to move it · release commits changes · H hides outlines.",
                            );
                        } else if !fullscreen && self.document.analysis_review.is_some() {
                            ui.separator();
                            ui.small(
                                "Drag either area's corners to resize, or its interior to move. Click a label when outlines coincide.",
                            );
                            ui.small("Enter analyzes · Escape cancels");
                        } else if !fullscreen && let Some(kind) = self.pick {
                            ui.separator();
                            ui.small(
                                "Click a patch or drag an area; drag its edges/corners to resize.",
                            );
                            if kind == Pick::Analysis {
                                ui.small(
                                    "Picture statistics only · source-aligned · not an output crop",
                                );
                            }
                            if matches!(kind, Pick::Dmin | Pick::Dmax) {
                                ui.small(
                                    "To sample another image, select it in the filmstrip. Apply/Cancel returns to the working photo.",
                                );
                            }
                            ui.small("Enter applies · Escape cancels");
                        }
                    });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(group) = tool_group {
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
                    } else if let Some(group) = session_group {
                        finish_dust = group.show(ui).inner.clicked();
                    } else if creating {
                        cancel = TextAction::new("Cancel drawing").show(ui).clicked();
                        ui.add(
                            egui::Label::new(self.document.visit.mask_edit.as_ref().unwrap().creation_label())
                                .truncate(),
                        );
                    } else {
                        ui.add(
                            egui::Label::new(egui::RichText::new(state).small().weak()).truncate(),
                        );
                    }
                });
            });
        });

        if finish_dust {
            self.finish_dust_cleanup();
            ui.ctx().request_repaint();
        } else if cancel {
            if self.document.auto_crop_edit.is_some() {
                self.finish_auto_crop(false);
            } else if masking {
                self.finish_mask(false);
            } else if cropping {
                self.finish_crop(false);
            } else {
                self.finish_pick(cx);
            }
            ui.ctx().request_repaint();
        } else if apply {
            if self.document.auto_crop_edit.is_some() {
                self.finish_auto_crop(true);
            } else if cropping {
                self.finish_crop(true);
            } else {
                self.apply_pick(cx);
            }
            ui.ctx().request_repaint();
        }
        zoom_action
    }

    pub(super) fn canvas_input_allowed(&self, ui: &egui::Ui, canvas: egui::Rect) -> bool {
        if self.menus.panel.is_some()
            || self.document.visit.adjustments.confirming()
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
