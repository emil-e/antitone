use super::*;
use crate::gpu::parameters::ProcessingParameters;

/// Display transforms follow captured source inputs, independently of document snapshots.
#[derive(Default, Debug)]
pub(super) struct SourcePreviews {
    entries: Vec<(crate::source::SourceKey, crate::raw::SourcePreview)>,
}

impl SourcePreviews {
    pub fn get(&self, key: &crate::source::SourceKey) -> crate::raw::SourcePreview {
        self.entries
            .iter()
            .find(|(captured, _)| captured == key)
            .map(|(_, preview)| *preview)
            .unwrap_or_default()
    }

    pub fn insert(&mut self, key: crate::source::SourceKey, preview: crate::raw::SourcePreview) {
        if let Some((_, current)) = self
            .entries
            .iter_mut()
            .find(|(captured, _)| captured == &key)
        {
            *current = preview;
        } else {
            self.entries.push((key, preview));
        }
    }

    pub fn extend(&mut self, other: Self) {
        for (key, preview) in other.entries {
            self.insert(key, preview);
        }
    }
}

pub(super) struct BarResponse {
    pub zoom: Option<f32>,
    pub fit: bool,
    pub fullscreen: bool,
    pub action: Option<tool::Action>,
}

impl Editor {
    pub(super) fn source_parameters(&self, exposure: f64) -> Result<ProcessingParameters, Error> {
        if let Some(ready) = &self.active_canvas().ready {
            return self.source_preview_parameters(
                ready
                    .developed
                    .metadata()
                    .source_preview(ready.developed.white_balance()),
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

    pub(super) fn source_preview_parameters(
        &self,
        preview: crate::raw::SourcePreview,
        exposure: f64,
    ) -> Result<ProcessingParameters, Error> {
        let transform = if self.input().is_negative() {
            preview.transform(self.source_white_balance)
        } else {
            glam::DMat3::IDENTITY
        };
        ProcessingParameters::source_preview_with_transform(
            // DevelopedSource already restores LibRaw's green-relative exposure scale.
            None, exposure, transform,
        )
    }

    pub(super) fn source_tool_warning(&self, id: SourceId) -> Option<String> {
        if self.document.visit.current_source == Some(id) && self.cropping() {
            return self.current_tool()?.presentation().warning;
        }
        None
    }

    fn viewer_warning(&self) -> Option<String> {
        self.document
            .visit
            .current_source
            .and_then(|id| self.source_tool_warning(id))
    }

    pub(super) fn viewer_bar(
        &mut self,
        ui: &mut egui::Ui,
        controls: impl FnOnce(&mut Editor, &mut egui::Ui, bool),
    ) -> BarResponse {
        use ui::icons::Icon;
        use ui::{TextAction, ToolGroup, ToolIdentity, VIEWER_ITEM_GAP};

        let mut zoom_action = None;
        let mut fit = false;
        let mut toggle_fullscreen = false;
        let fullscreen = self.fullscreen.is_some();
        let hosted = (!fullscreen)
            .then(|| self.current_tool().map(|tool| tool.presentation()))
            .flatten();
        let fit_label = hosted.as_ref().map_or("Fit", |tool| tool.fit_label);
        let masking = !fullscreen && self.masking();
        let tool = hosted
            .as_ref()
            .map(|tool| (tool.identity.icon, tool.identity.title));
        let has_repairs = self
            .document
            .visit
            .current_source
            .and_then(|id| self.dust_repairs(id))
            .is_some_and(|repairs| !repairs.repairs().is_empty());

        let mut cancel = false;
        let mut apply = false;
        let calibrated = self.input().ready();
        if !calibrated {
            self.document.source = true;
        }

        let navigating = self.canvas_frame.layout.is_some()
            && !self.interaction_blocked()
            && !self.mask_dragging();

        let tool_group = tool.map(|(icon, title)| ToolGroup {
            identity: ToolIdentity { icon, title },
            busy: self.interaction_blocked(),
            cancel_enabled: !self.interaction_blocked(),
            apply_enabled: hosted.as_ref().is_some_and(|tool| {
                !self.interaction_blocked() && tool.primary.is_some_and(|primary| primary.enabled)
            }),
        });

        let warning = hosted
            .as_ref()
            .and_then(|tool| tool.warning.clone())
            .or_else(|| self.viewer_warning());
        let look_error = self.look_rendering_error();
        let painted_source = self
            .canvas_frame
            .displayed
            .as_ref()
            .is_some_and(|painted| painted.view.capture.is_inspection());
        let bar = if fullscreen {
            ui::ViewerBar::Overlay
        } else {
            ui::ViewerBar::Docked
        };

        bar.show(ui, |ui| {
            if fullscreen && let Some(view) = self.activities.view() {
                view.compact(ui);
            }
            ui.spacing_mut().item_spacing.x = VIEWER_ITEM_GAP;
            ui::icons::row(ui, |ui| {
                let target = if self.document.target() == OutputTarget::DisplayP3 {
                    "Display P3"
                } else {
                    "sRGB"
                };
                let quality = if self.document.visit.pending_preview.is_some() {
                    " · Cached preview"
                } else if painted_source
                    || self.canvas_inspection()
                    || self.active_canvas().ready.is_none()
                {
                    ""
                } else if self.document.visit.working_view.enabled() {
                    if look_error.is_some() {
                        " · Look unavailable"
                    } else if self.document.visit.working_view.error.is_some() {
                        " · View unavailable"
                    } else if self.comparing_original() && self.active_canvas().original.failed() {
                        " · Reference unavailable"
                    } else if self.comparing_original()
                        && self.document.visit.working_view.reference_ready()
                    {
                        " · Original reference"
                    } else if self.comparing_original() {
                        " · Preparing reference"
                    } else if self.document.visit.working_view.displayed.is_none() {
                        " · Acquiring view"
                    } else {
                        ""
                    }
                } else if self.active_canvas().original.refining() {
                    " · Refining"
                } else if self.active_canvas().original.settled().is_none() {
                    " · Approximate"
                } else {
                    ""
                };
                let state = if has_repairs {
                    format!("{target}{quality} · {}", self.dust_render_state())
                } else {
                    format!("{target}{quality}")
                };
                let trailing_width = ui::CONTROL_HEIGHT
                    + VIEWER_ITEM_GAP
                    + if let Some((group, tool)) = tool_group.zip(hosted.as_ref()) {
                        group.width_actions(
                            ui,
                            tool.primary.map(|primary| primary.label),
                            tool.dismiss,
                        )
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
                let source_controls = 2;
                // Fit, zoom steps and fullscreen, plus the readout, separator and current view controls.
                let navigation_icons = 4 + source_controls;
                let navigation_width = 50.
                    + navigation_icons as f32 * ui::CONTROL_HEIGHT
                    + (navigation_icons + 3) as f32 * VIEWER_ITEM_GAP;
                let show_zoom_steps = tool.is_none() && !masking
                    || ui.available_width() >= trailing_width + navigation_width;
                if ui::icons::button(
                    ui,
                    Icon::Fit,
                    navigating,
                    fit_label,
                    &format!("{fit_label} (Z)."),
                )
                .clicked()
                {
                    fit = true;
                }
                let layout = self.canvas_frame.layout.map(|(layout, _)| layout);
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
                        if ui::icons::button(ui, icon, navigating, name, name).clicked() {
                            zoom_action = Some(-factor);
                        }
                    }
                }
                let (icon, name) = if fullscreen {
                    (Icon::ExitFullscreen, "Exit fullscreen")
                } else {
                    (Icon::Fullscreen, "Fullscreen viewing")
                };
                if ui::icons::button(
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
                    toggle_fullscreen = true;
                }
                ui.separator();
                let controls_width = (ui.available_width()
                    - trailing_width
                    - ui::CONTROL_HEIGHT
                    - 2. * VIEWER_ITEM_GAP)
                    .max(0.);
                ui.allocate_ui(egui::vec2(controls_width, ui::CONTROL_HEIGHT), |ui| {
                    controls(self, ui, navigating)
                });
                if let Some(error) = &look_error {
                    ui.colored_label(egui::Color32::LIGHT_RED, "Look rendering failed");
                    if TextAction::new("Details")
                        .toolbar(60.)
                        .name("Look rendering details")
                        .show(ui)
                        .clicked()
                    {
                        self.look_render_failed(error.to_string());
                    }
                } else if let Some(error) = &self.document.visit.working_view.error {
                    ui.colored_label(egui::Color32::LIGHT_RED, "View unavailable")
                        .on_hover_text(error);
                }
                let help = ui::icons::button(
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
                        ui.small(
                            hosted
                                .as_ref()
                                .and_then(|tool| tool.navigation_help)
                                .unwrap_or("Scroll pans · Cmd/Ctrl-scroll or pinch zooms"),
                        );
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
                        if let Some(help) = self
                            .mask_help()
                            .filter(|_| !fullscreen && self.tab == 1 && hosted.is_none())
                        {
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
                        }
                    });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some((group, tool)) = tool_group.zip(hosted.as_ref()) {
                        let actions = group
                            .show_actions(
                                ui,
                                tool.primary.map(|primary| primary.label),
                                tool.dismiss,
                                warning.as_deref(),
                            )
                            .inner;
                        cancel = actions.dismiss.clicked();
                        apply = actions.primary.is_some_and(|response| response.clicked());
                    } else {
                        ui.add(
                            egui::Label::new(egui::RichText::new(state).small().weak()).truncate(),
                        );
                    }
                });
            });
        });

        let action = if cancel {
            Some(tool::Action::Dismiss)
        } else if apply {
            Some(tool::Action::Primary)
        } else {
            None
        };
        BarResponse {
            zoom: zoom_action,
            fit,
            fullscreen: toggle_fullscreen,
            action,
        }
    }

    pub(super) fn photograph_controls(&mut self, ui: &mut egui::Ui, navigating: bool) {
        let inspecting = navigating
            && self.input().ready()
            && (!self.has_hosted_tool() || self.fullscreen.is_some())
            && !self.masking();
        let correction_ready =
            !self.document.correction.pending() && self.document.correction.error().is_none();
        let can_toggle = correction_ready || !self.document.source;
        ui.add_enabled_ui(inspecting && can_toggle, |ui| {
            ui::icons::toggle(
                ui,
                ui::icons::Icon::Source,
                &mut self.document.source,
                "View source",
                if can_toggle {
                    "Switch between the positive grade and developed source."
                } else {
                    "Disable or replace flat-field correction to view the positive."
                },
            );
        });
        if !self.canvas_inspection() && self.document.visit.working_view.enabled() {
            ui.add_enabled_ui(inspecting, |ui| {
                ui::icons::toggle(
                    ui,
                    ui::icons::Icon::Inspection,
                    &mut self.document.visit.working_view.compare,
                    "Original-resolution reference",
                    "Compare with original-resolution processing; export is unchanged.",
                );
            });
        }
        if self.canvas_inspection() {
            self.source_controls(ui, navigating);
        }
    }

    pub(super) fn source_controls(&mut self, ui: &mut egui::Ui, navigating: bool) {
        let inspection_id = ui.make_persistent_id("source-inspection-popup");
        let exposure = format!("Source {:+.2} EV", self.source_exposure);
        let inspection = ui::icons::button(
            ui,
            ui::icons::Icon::Inspection,
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
                    if self.input().is_negative() {
                        use crate::raw::SourceWhiteBalance;
                        ui::labelled_row(ui, "Viewing WB", None, |ui, _| {
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
                    ui::ParameterRow::new(
                        "exposure",
                        "Exposure",
                        -crate::processing::MAX_EXPOSURE..=crate::processing::MAX_EXPOSURE,
                    )
                    .default_value(0.)
                    .slider_range(-3.0..=3.0)
                    .step(0.05)
                    .speed(0.01)
                    .unit("EV")
                    .slider_help(
                        "Slider: ±3 EV; type a value for up to ±16 EV. Shift-drag for finer steps.",
                    )
                    .show(ui, &mut self.source_exposure);
                });
                ui.small("Canvas and source thumbnails · not saved");
            });
        let readout =
            ui::instrument_readout(&exposure, egui::TextStyle::Small.resolve(ui.style()).size);
        let width = ui
            .painter()
            .layout_no_wrap(
                exposure,
                egui::TextStyle::Small.resolve(ui.style()),
                ui.visuals().text_color(),
            )
            .size()
            .x;
        if ui.available_width() >= width {
            ui.label(readout);
        }
    }

    pub(super) fn canvas_input_allowed(&self, ui: &egui::Ui, canvas: egui::Rect) -> bool {
        if self.workspace_input.is_blocked() || egui::Popup::is_any_open(ui.ctx()) {
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
