use super::*;
use crate::project::{AdjustmentLayerId, AdjustmentLayers};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(super) enum Target {
    #[default]
    None,
    Layer(AdjustmentLayerId),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct OrderKey {
    loaded: u64,
    source: SourceId,
    id: AdjustmentLayerId,
}

pub(super) struct OrderDrag {
    key: OrderKey,
    layers: AdjustmentLayers,
}

#[derive(Default)]
pub(super) struct AdjustmentLayerEditor {
    selected: Option<(SourceId, AdjustmentLayerId)>,
    owns_delete: bool,
    rename: Option<(SourceId, AdjustmentLayerId, String)>,

    pub(super) show_coverage: bool,
    panel_collapsed: bool,
    pub(super) adjusting_values: bool,
    pub(super) reveal_controls: bool,
    reveal: bool,
    rename_focus: bool,
    creation: Option<bool>,

    order_drag: Option<OrderDrag>,
}

impl AdjustmentLayerEditor {
    pub(super) fn selection_only(&self) -> Self {
        Self {
            selected: self.selected,
            ..Default::default()
        }
    }

    pub(super) fn dragging(&self) -> bool {
        self.order_drag.is_some()
    }
}

pub(super) enum Action {
    Select(Target),
    Overlay(bool),
    Enable(AdjustmentLayerId, bool),
    Reorder(OrderDrag, usize),
    RenameStart(AdjustmentLayerId),
    RenameCancel,
    Rename(AdjustmentLayerId, String),
    Delete(AdjustmentLayerId),
    CreateGlobal,
}

impl Editor {
    pub(super) fn refresh_adjustment_layer_selection(&mut self) {
        let selected = match self.adjustment_layer_target() {
            Target::Layer(id) => self
                .document
                .visit
                .current_source
                .map(|source| (source, id)),
            Target::None => None,
        };
        self.document.visit.adjustment_layers.selected = selected;
    }

    #[cfg(test)]
    pub(super) fn selected_adjustment_layer_settings(&self) -> Option<AdjustmentLayerSettings> {
        let Target::Layer(id) = self.adjustment_layer_target() else {
            return None;
        };
        self.document
            .state
            .project
            .source(self.document.visit.current_source?)
            .ok()?
            .adjustment_layers
            .as_ref()?
            .layers
            .iter()
            .find(|record| record.id == id)
            .map(|record| record.adjustment.photo)
    }

    pub(super) fn overlay_adjustment_layer_settings(
        &self,
        source: &mut Source,
        photo: AdjustmentLayerSettings,
    ) {
        if self.document.visit.current_source == Some(source.id)
            && let Target::Layer(id) = self.adjustment_layer_target()
            && let Some(record) = source
                .adjustment_layers
                .as_mut()
                .and_then(|layers| layers.layers.iter_mut().find(|record| record.id == id))
        {
            let project = self.document.project_snapshot();
            if self.editing()
                && let Ok(source) = project.source(source.id)
                && let Some(draft) = source
                    .adjustment_layers
                    .as_ref()
                    .and_then(|layers| layers.layers.iter().find(|record| record.id == id))
            {
                record.adjustment = draft.adjustment;
            }
            record.adjustment.photo = photo;
        }
    }

    pub(super) fn adjustment_layer_snapshot(
        &self,
        source: &Source,
        photo: AdjustmentLayerSettings,
    ) -> Result<AdjustmentLayerSnapshot, Error> {
        let mut candidate = source.clone();
        self.overlay_adjustment_layer_settings(&mut candidate, photo);
        candidate.grade_snapshot()
    }

    pub(super) fn adjustment_layer_target(&self) -> Target {
        self.document
            .visit
            .adjustment_layers
            .selected
            .filter(|(source, id)| {
                Some(*source) == self.document.visit.current_source
                    && self
                        .document
                        .project
                        .source(*source)
                        .ok()
                        .and_then(|source| source.adjustment_layers.as_ref())
                        .is_some_and(|layers| layers.layers.iter().any(|record| record.id == *id))
            })
            .map(|(_, id)| Target::Layer(id))
            .unwrap_or_else(|| {
                self.document
                    .visit
                    .current_source
                    .and_then(|id| self.document.state.project.source(id).ok())
                    .and_then(|source| source.adjustment_layers.as_ref())
                    .and_then(|layers| layers.layers.last())
                    .map_or(Target::None, |record| Target::Layer(record.id))
            })
    }

    pub(super) fn select_adjustment_layer(&mut self, target: Target) {
        if target != self.adjustment_layer_target() {
            self.document.visit.controls_edit = None;
            self.finish_mask(false);
        }
        if self
            .document
            .visit
            .picker
            .display()
            .is_some_and(|display| display.lock_layer_selection)
        {
            return;
        }

        self.document.visit.adjustment_layers.owns_delete = true;
        self.document.visit.adjustment_layers.selected = match target {
            Target::None => None,
            Target::Layer(id) => self
                .document
                .visit
                .current_source
                .filter(|source| {
                    self.document
                        .state
                        .project
                        .source(*source)
                        .is_ok_and(|source| {
                            source.adjustment_layers.as_ref().is_some_and(|layers| {
                                layers.layers.iter().any(|record| record.id == id)
                            })
                        })
                })
                .map(|source| (source, id)),
        };
        self.document.visit.adjustment_layers.rename = None;
    }

    pub(super) fn reveal_adjustment_layers(&mut self) {
        self.tab = 1;
        self.document.visit.adjustment_layers.reveal = true;
    }

    pub(super) fn finish_adjustment_layer_interaction(&mut self) {
        if self.creating_mask() {
            self.end_tool();
        } else {
            self.finish_mask(false);
        }
        self.document.visit.adjustment_layers.rename = None;
        self.document.visit.adjustment_layers.owns_delete = false;
        self.document.visit.adjustment_layers.order_drag = None;
        self.document.visit.adjustment_layers.creation = None;
        self.document.visit.adjustment_layers.reveal_controls = false;
    }

    fn adjustment_layer_actions_available(&self) -> bool {
        !self.interaction_blocked()
            && !self.controls_pending()
            && self.document.project.input.ready()
            && !self.tool_interacting()
            && !self.dialogs.is_open()
    }

    pub(super) fn edit_adjustment_layers(
        &mut self,
        edit: impl FnOnce(&mut AdjustmentLayers) -> Result<(), Error>,
    ) -> Result<(), Error> {
        self.edit_adjustment_layers_named("Adjustment layers", edit)
    }

    pub(super) fn edit_adjustment_layers_named(
        &mut self,
        label: &'static str,
        edit: impl FnOnce(&mut AdjustmentLayers) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let mut draft = self.edit_photo(label)?;
        let id = self
            .document
            .visit
            .current_source
            .ok_or(Error::InvalidParameter("no selected source"))?;
        let source = self.document.state.project.source(id)?;
        let before = source.grade_snapshot()?;
        let target = self.adjustment_layer_target();
        let mut candidate = source.clone();
        self.overlay_adjustment_layer_settings(&mut candidate, self.photo_settings());
        edit(
            candidate
                .adjustment_layers
                .get_or_insert_with(Default::default),
        )?;
        let after = candidate.grade_snapshot()?;
        self.validate_source_candidate(&candidate)?;
        draft.update(|project| {
            project
                .source_mut(id)
                .expect("current edit source")
                .adjustment_layers = candidate.adjustment_layers;
        });
        self.commit_project(draft)?;
        self.document.visit.adjustment_layers.selected = match target {
            Target::Layer(layer) => Some((id, layer)),
            Target::None => None,
        };
        self.refresh_adjustment_layer_selection();
        if before != after {
            self.invalidate_original();
        }
        Ok(())
    }

    pub(super) fn adjustment_layer_order_input(
        &mut self,
        start: Option<AdjustmentLayerId>,
        drop: Option<(OrderKey, usize)>,
        finish: bool,
        cx: &UiContext<'_, Self>,
    ) {
        if let Some(id) = start {
            match self.order_drag(id) {
                Ok(drag) => self.document.visit.adjustment_layers.order_drag = Some(drag),
                Err(error) => self.status = format!("Adjustment drag rejected: {error}"),
            }
        }
        if let Some((key, slot)) = drop
            && let Some(drag) = self
                .document
                .visit
                .adjustment_layers
                .order_drag
                .take()
                .filter(|drag| drag.key == key)
        {
            self.apply_adjustment_layer_action(Action::Reorder(drag, slot), cx);
        }
        if finish {
            self.document.visit.adjustment_layers.order_drag = None;
        }
    }

    fn order_drag(&self, id: AdjustmentLayerId) -> Result<OrderDrag, Error> {
        let source = self
            .document
            .visit
            .current_source
            .ok_or(Error::InvalidParameter("no selected source"))?;
        let layers = self
            .document
            .project
            .source(source)?
            .adjustment_layers
            .as_ref()
            .ok_or(Error::InvalidParameter("no adjustment layers"))?
            .clone();
        if !layers.layers.iter().any(|record| record.id == id) {
            return Err(Error::InvalidParameter("unknown adjustment-layer ID"));
        }
        Ok(OrderDrag {
            key: OrderKey {
                loaded: self.id,
                source,
                id,
            },
            layers,
        })
    }

    fn reorder_adjustment(&mut self, drag: OrderDrag, slot: usize) -> Result<(), Error> {
        if drag.key.loaded != self.id || Some(drag.key.source) != self.document.visit.current_source
        {
            return Err(Error::InvalidParameter(
                "adjustment source changed during drag",
            ));
        }
        self.edit_adjustment_layers(|layers| {
            if *layers != drag.layers {
                return Err(Error::InvalidParameter("layers changed during drag"));
            }
            if slot > layers.layers.len() {
                return Err(Error::InvalidParameter("invalid adjustment drop position"));
            }
            let index = layers
                .layers
                .iter()
                .position(|record| record.id == drag.key.id)
                .ok_or(Error::InvalidParameter("unknown adjustment-layer ID"))?;
            let record = layers.layers.remove(index);
            layers
                .layers
                .insert(slot - usize::from(index < slot), record);
            Ok(())
        })
    }

    pub(super) fn apply_adjustment_layer_action(
        &mut self,
        action: Action,
        cx: &UiContext<'_, Self>,
    ) {
        if !self.adjustment_layer_actions_available() || self.tab != 1 {
            return;
        }
        let Some(source) = self.document.visit.current_source else {
            return;
        };
        let result = match action {
            Action::CreateGlobal => {
                let above = match self.adjustment_layer_target() {
                    Target::Layer(id) => Some(id),
                    Target::None => None,
                };
                let mut created = None;
                self.edit_adjustment_layers(|layers| {
                    created = Some(layers.add_above(
                        above,
                        format!("Global {}", layers.next_id),
                        crate::adjustment_layers::AdjustmentLayer::global(
                            AdjustmentLayerSettings::default(),
                        ),
                    )?);
                    Ok(())
                })
                .map(|()| {
                    self.select_adjustment_layer(Target::Layer(created.unwrap()));
                })
            }
            Action::Select(target) => {
                self.select_adjustment_layer(target);
                Ok(())
            }
            Action::Overlay(show) => {
                self.document.visit.adjustment_layers.show_coverage = show;
                Ok(())
            }
            Action::RenameStart(id) => {
                self.select_adjustment_layer(Target::Layer(id));
                self.document.visit.adjustment_layers.rename_focus = true;
                self.document.visit.adjustment_layers.rename = self
                    .document
                    .project
                    .source(source)
                    .ok()
                    .and_then(|source| source.adjustment_layers.as_ref())
                    .and_then(|layers| layers.layers.iter().find(|record| record.id == id))
                    .map(|record| (source, id, record.name.clone()));
                Ok(())
            }
            Action::RenameCancel => {
                self.document.visit.adjustment_layers.rename = None;
                Ok(())
            }
            Action::Delete(id) => self.request_adjustment_layer_deletion(source, id, cx),
            Action::Enable(id, enabled) => self.edit_adjustment_layers(|layers| {
                record_mut(layers, id)?.adjustment.enabled = enabled;
                Ok(())
            }),
            Action::Reorder(drag, slot) => self.reorder_adjustment(drag, slot),
            Action::Rename(id, name) => {
                let result = self.edit_adjustment_layers(|layers| {
                    record_mut(layers, id)?.name = name;
                    Ok(())
                });
                if result.is_ok() {
                    self.document.visit.adjustment_layers.rename = None;
                }
                result
            }
        };
        if let Err(error) = result {
            self.status = format!("Adjustment edit rejected: {error}");
        }
    }

    pub(super) fn sync_adjustment_layer_drag(&mut self, ctx: &egui::Context) {
        if self
            .document
            .visit
            .adjustment_layers
            .order_drag
            .as_ref()
            .is_some_and(|drag| {
                drag.key.loaded != self.id
                    || Some(drag.key.source) != self.document.visit.current_source
                    || self.tab != 1
                    || self.fullscreen.is_some()
                    || !(self.tab == 1)
                    || self.interaction_blocked()
                    || !egui::DragAndDrop::has_payload_of_type::<OrderKey>(ctx)
            })
        {
            self.document.visit.adjustment_layers.order_drag = None;
            egui::DragAndDrop::take_payload::<OrderKey>(ctx);
        }
    }

    pub(super) fn adjustment_layer_owns_delete(&self) -> bool {
        self.tab == 1
            && !self.tool_interacting()
            && self.document.visit.adjustment_layers.owns_delete
            && matches!(self.adjustment_layer_target(), Target::Layer(_))
    }

    pub(super) fn delete_selected_adjustment_layer(&mut self, cx: &UiContext<'_, Self>) {
        if !self.document.visit.adjustment_layers.dragging()
            && let Target::Layer(id) = self.adjustment_layer_target()
        {
            self.apply_adjustment_layer_action(Action::Delete(id), cx);
        }
    }

    pub(super) fn adjustment_layer_overlays_visible(&self) -> bool {
        self.document.visit.masks.gesture().is_some()
            || (!self.document.visit.adjustment_layers.panel_collapsed
                && !self.document.visit.adjustment_layers.adjusting_values)
    }

    pub(super) fn photo_control(
        &mut self,
        response: ui::ControlResponse,
        write: impl FnOnce(&mut crate::adjustment_layers::AdjustmentLayer),
    ) {
        let Some(source) = self.document.visit.current_source else {
            return;
        };
        let adjustment_layers::Target::Layer(layer) = self.adjustment_layer_target() else {
            return;
        };
        let Some(mut edit) = self.begin_control(response, true) else {
            return;
        };
        if response.changed {
            let candidate = edit.values.update(|project| -> Result<Source, Error> {
                let source = project.source_mut(source)?;
                let record = source
                    .adjustment_layers
                    .as_mut()
                    .and_then(|layers| layers.layers.iter_mut().find(|record| record.id == layer))
                    .ok_or(Error::InvalidParameter("selected layer is unavailable"))?;
                write(&mut record.adjustment);
                Ok(source.clone())
            });
            if let Err(error) = candidate.and_then(|source| self.validate_source_candidate(&source))
            {
                self.status = format!("Edit rejected: {error}");
                return;
            }
        }
        self.retain_control(edit, response, true);
    }

    fn photographic_controls(
        &mut self,
        ui: &mut egui::Ui,
        mut photo: AdjustmentLayerSettings,
        editable: bool,
        can_pick: bool,
        reveal: bool,
    ) -> bool {
        ui.add_enabled_ui(editable, |ui| {
            if reveal {
                ui.scroll_to_cursor(Some(egui::Align::Min));
            }
            let (_, action) = ui::section(
                ui,
                "photo-exposure",
                "Exposure",
                ui::SectionHeader {
                    summary: &format!("{:+.2} EV", photo.exposure),
                    changed: photo.exposure != 0.,
                    reset: Some(photo.exposure != 0.),
                    tool: None,
                    default_open: Some(true),
                },
                |ui| {
                    let response = ui::ParameterRow::new(
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
                    .show(ui, &mut photo.exposure)
                    .edit;
                    self.photo_control(response, |adjustment| {
                        adjustment.photo.exposure = photo.exposure
                    });
                },
            );
            if action.reset {
                self.photo_control(
                    ui::ControlResponse::action(
                        ui.make_persistent_id("reset-photo-exposure"),
                        true,
                    ),
                    |adjustment| adjustment.photo.exposure = 0.,
                );
            }
        });
        let (responses, pick) = super::white_balance::controls(
            ui,
            "photo-white-balance",
            &mut photo.wb,
            editable,
            can_pick,
        );
        for response in responses {
            self.photo_control(response, |adjustment| adjustment.photo.wb = photo.wb);
        }
        ui.add_enabled_ui(editable, |ui| {
            let defaults = AdjustmentLayerSettings::default();
            let changed = photo.contrast != defaults.contrast
                || photo.pivot != defaults.pivot
                || photo.contrast_colour_mix != defaults.contrast_colour_mix;
            let (_, action) = ui::section(
                ui,
                "photo-contrast",
                "Contrast",
                ui::SectionHeader {
                    summary: &format!(
                        "{:.2} · Pivot {:.3} · Preserve {:.2}",
                        photo.contrast, photo.pivot, photo.contrast_colour_mix
                    ),
                    changed,
                    reset: Some(changed),
                    tool: None,
                    default_open: Some(true),
                },
                |ui| {
                    let response = ui::ParameterRow::new("contrast", "Contrast", 0.4..=1.6)
                        .default_value(defaults.contrast)
                        .help("Adjust contrast around Pivot; 1 leaves contrast unchanged.")
                        .show(ui, &mut photo.contrast)
                        .edit;
                    self.photo_control(response, |adjustment| {
                        adjustment.photo.contrast = photo.contrast
                    });
                    let response = ui::ParameterRow::new("pivot", "Pivot", 0.01..=1.0)
                        .default_value(defaults.pivot)
                        .step(0.001)
                        .decimals(3)
                        .logarithmic(true)
                        .help("Set the brightness that contrast pivots around.")
                        .show(ui, &mut photo.pivot)
                        .edit;
                    self.photo_control(response, |adjustment| adjustment.photo.pivot = photo.pivot);
                    let response =
                        ui::ParameterRow::new("contrast-colour-mix", "Preserve", 0.0..=1.0)
                            .default_value(defaults.contrast_colour_mix)
                            .help("Reduce colour shifts caused by contrast.")
                            .show(ui, &mut photo.contrast_colour_mix)
                            .edit;
                    self.photo_control(response, |adjustment| {
                        adjustment.photo.contrast_colour_mix = photo.contrast_colour_mix
                    });
                },
            );
            if action.reset {
                self.photo_control(
                    ui::ControlResponse::action(
                        ui.make_persistent_id("reset-photo-contrast"),
                        true,
                    ),
                    |adjustment| {
                        adjustment.photo.contrast = defaults.contrast;
                        adjustment.photo.pivot = defaults.pivot;
                        adjustment.photo.contrast_colour_mix = defaults.contrast_colour_mix;
                    },
                );
            }
            let changed = photo.saturation != defaults.saturation
                || photo.saturation_density != defaults.saturation_density;
            let (_, action) = ui::section(
                ui,
                "photo-saturation",
                "Saturation",
                ui::SectionHeader {
                    summary: &format!(
                        "{:.2} · Density {:.2}",
                        photo.saturation, photo.saturation_density
                    ),
                    changed,
                    reset: Some(changed),
                    tool: None,
                    default_open: Some(true),
                },
                |ui| {
                    let response = ui::ParameterRow::new("saturation", "Saturation", 0.0..=2.0)
                        .default_value(defaults.saturation)
                        .help("0 makes the image grey; 1 leaves saturation unchanged.")
                        .show(ui, &mut photo.saturation)
                        .edit;
                    self.photo_control(response, |adjustment| {
                        adjustment.photo.saturation = photo.saturation
                    });
                    let response = ui::ParameterRow::new("density", "Density", 0.0..=1.0)
                        .default_value(defaults.saturation_density)
                        .help("Higher values darken colours more as saturation increases.")
                        .show(ui, &mut photo.saturation_density)
                        .edit;
                    self.photo_control(response, |adjustment| {
                        adjustment.photo.saturation_density = photo.saturation_density
                    });
                },
            );
            if action.reset {
                self.photo_control(
                    ui::ControlResponse::action(
                        ui.make_persistent_id("reset-photo-saturation"),
                        true,
                    ),
                    |adjustment| {
                        adjustment.photo.saturation = defaults.saturation;
                        adjustment.photo.saturation_density = defaults.saturation_density;
                    },
                );
            }
        });
        pick
    }

    pub(super) fn adjustment_layer_controls(
        &mut self,
        ui: &mut egui::Ui,
        cx: &UiContext<'_, Self>,
        editable: bool,
        can_pick: bool,
    ) -> bool {
        let target = self.adjustment_layer_target();
        let records = self
            .document
            .visit
            .current_source
            .and_then(|id| {
                self.document
                    .project_snapshot()
                    .source(id)
                    .ok()
                    .and_then(|source| source.adjustment_layers.as_ref())
                    .map(|layers| layers.layers.clone())
            })
            .unwrap_or_default();
        let selected = match target {
            Target::None => None,
            Target::Layer(id) => records.iter().find(|record| record.id == id),
        };
        let adjustment = selected.map(|record| record.adjustment);
        let available = self.adjustment_layer_actions_available() && ui.is_enabled();
        let mut action = None;
        let mut rename_finish = None;
        let mut invert_action = None;

        let mut start_drag = None;
        let mut drop = None;
        // Keep layer management independent of photographic-control widget IDs.
        let panel_expanded = ui.push_id("adjustment-layers", |ui| {
            if std::mem::take(&mut self.document.visit.adjustment_layers.reveal) {
                ui.scroll_to_cursor(Some(egui::Align::Min));
            }
            let title = selected.map_or_else(|| "Layers".into(), |record| format!("Layers: {}", record.name));
            let (body, _) = ui::section(
                ui,
                "adjustment-layers",
                &title,
                ui::SectionHeader {
                    summary: "",
                    changed: false,
                    reset: None,
                    tool: None,
                    default_open: Some(true),
                },
                |ui| {
                    let (overlay, creation) = ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let mut overlay = None;
                            let mut creation = None;
                            if selected.is_some_and(|record| record.adjustment.mask != (crate::adjustment_layers::AdjustmentLayerCoverage::Global {})) {
                                let mut show = self.document.visit.adjustment_layers.show_coverage;
                                if ui
                                    .add_enabled_ui(available, |ui| {
                                        ui::icons::toggle(
                                            ui,
                                            ui::icons::Icon::Eye,
                                            &mut show,
                                            "Mask overlay",
                                            "Show mask coverage",
                                        )
                                    })
                                    .inner
                                    .clicked()
                                {
                                    overlay = Some(Action::Overlay(show));
                                }
                            }
                            if ui::icons::button(
                                ui,
                                ui::icons::Icon::AddGlobalAdjustmentLayer,
                                available && !self.document.source,
                                "Add global",
                                "Add a whole-image adjustment layer",
                            )
                            .clicked()
                            {
                                action = Some(Action::CreateGlobal);
                            }
                            for (icon, name, radial, help) in [
                                (
                                    ui::icons::Icon::AddRadialGradient,
                                    "Add radial",
                                    true,
                                    "Add radial gradient",
                                ),
                                (
                                    ui::icons::Icon::AddLinearGradient,
                                    "Add linear",
                                    false,
                                    "Add linear gradient",
                                ),
                            ] {
                                if ui::icons::button(ui, icon, self.tool_start_available() && self.document.project.input.ready(), name, help)
                                    .clicked()
                                {
                                    creation = Some(radial);
                                }
                            }
                            (overlay, creation)
                        }).inner
                    }).inner;
                    if overlay.is_some() {
                        action = overlay;
                    }
                    if creation.is_some() {
                        self.document.visit.adjustment_layers.creation = creation;
                    }
                    ui.add_enabled_ui(available, |ui| {
                        if records.is_empty() {
                            ui.weak("Add a layer to adjust this photo.");
                        }
                        for (index, record) in records.iter().enumerate().rev() {
                            ui.push_id(record.id, |ui| {
                                if let Some((source, id, text)) =
                                    &mut self.document.visit.adjustment_layers.rename
                                    && Some(*source) == self.document.visit.current_source
                                    && *id == record.id
                                {
                                    let rename_id = ui.make_persistent_id("rename-input");
                                    // Escape clears egui focus before widgets run; retain its previous owner.
                                    let cancel = ui.memory(|memory| {
                                        memory.has_focus(rename_id)
                                            || memory.had_focus_last_frame(rename_id)
                                    }) && !egui::Popup::is_any_open(ui.ctx())
                                        && ui.input_mut(|input| {
                                            input.consume_key(
                                                egui::Modifiers::NONE,
                                                egui::Key::Escape,
                                            )
                                        });
                                    let response = ui
                                        .add_sized(
                                            [ui.available_width(), ui::CONTROL_HEIGHT],
                                            egui::TextEdit::singleline(text)
                                                .id(rename_id)
                                                .hint_text("Adjustment name"),
                                        )
                                        .on_hover_text(
                                            "Enter or focus loss renames; Escape cancels.",
                                        );

                                    if std::mem::take(
                                        &mut self.document.visit.adjustment_layers.rename_focus,
                                    ) {
                                        response.request_focus();
                                    }
                                    if cancel {
                                        rename_finish = Some(Action::RenameCancel);
                                    } else if response.lost_focus() {
                                        rename_finish = Some(if text.trim().is_empty() {
                                            Action::RenameCancel
                                        } else {
                                            Action::Rename(*id, text.clone())
                                        });
                                    }
                                } else {
                                    let mut enabled = record.adjustment.enabled;
                                    let (row, toggle) = ui::draggable_icon_row(
                                        ui,
                                        mask_icon(record.adjustment.mask),
                                        &record.name,
                                        target == Target::Layer(record.id),
                                        Some(&mut enabled),
                                    );

                                    let key = OrderKey {
                                        loaded: self.id,
                                        source: self.document.visit.current_source.unwrap(),
                                        id: record.id,
                                    };
                                    row.dnd_set_drag_payload(key);
                                    if row.drag_started() {
                                        start_drag = Some(record.id);
                                    }
                                    let rect = toggle
                                        .as_ref()
                                        .map_or(row.rect, |toggle| row.rect.union(toggle.rect));
                                    let destination = ui.interact(
                                        rect,
                                        row.id.with("drop"),
                                        egui::Sense::hover(),
                                    );
                                    if available
                                        && let Some(payload) =
                                            destination.dnd_hover_payload::<OrderKey>()
                                        && payload.loaded == self.id
                                        && Some(payload.source) == self.document.visit.current_source
                                    {
                                        let below = ui.input(|input| {
                                            input
                                                .pointer
                                                .interact_pos()
                                                .is_some_and(|point| point.y >= rect.center().y)
                                        });
                                        let slot = index + usize::from(!below);
                                        let y = if below { rect.bottom() } else { rect.top() };
                                        ui.painter().line_segment(
                                            [
                                                egui::pos2(rect.left(), y),
                                                egui::pos2(rect.right(), y),
                                            ],
                                            ui.visuals().selection.stroke,
                                        );
                                        if let Some(payload) =
                                            destination.dnd_release_payload::<OrderKey>()
                                        {
                                            drop = Some((*payload, slot));
                                        }
                                    }
                                    if toggle.is_some_and(|response| response.changed()) {
                                        action = Some(Action::Enable(record.id, enabled));
                                    } else if row.double_clicked() {
                                        action = Some(Action::RenameStart(record.id));
                                    } else if row.clicked() {
                                        action = Some(Action::Select(Target::Layer(record.id)));
                                    }
                                }
                            });
                        }
                        if let Some(adjustment) = adjustment {
                            ui.add_space(6.);
                            ui.add_enabled_ui(adjustment.enabled, |ui| {
                                ui.push_id(("mask-input", self.document.visit.current_source, target), |ui| {
                                    ui.add_enabled_ui(adjustment.strength > 0., |ui| {
                                        let mut invert = adjustment.invert;
                                        if adjustment.mask
                                            != (crate::adjustment_layers::AdjustmentLayerCoverage::Global {})
                                            && ui.checkbox(&mut invert, "Invert mask").changed()
                                            && let Target::Layer(id) = target
                                        {
                                            invert_action = Some((id, invert));
                                        }
                                    });
                                    let mut strength = adjustment.strength;
                                    let response = ui::ParameterRow::new(
                                        "strength", "Strength", 0.0..=1.0,
                                    )
                                    .default_value(1.)
                                    .help("Scale the adjustment from no effect to full strength.")
                                    .show(ui, &mut strength)
                                    .edit;
                                    self.photo_control(response, |adjustment| adjustment.strength = strength);
                                    ui.add_enabled_ui(adjustment.strength > 0., |ui| {
                                        if let crate::adjustment_layers::AdjustmentLayerCoverage::Radial {
                                            mut feather,
                                            ..
                                        } = adjustment.mask
                                        {
                                            let response = ui::ParameterRow::new(
                                                "feather", "Feather", 0.0..=1.0,
                                            )
                                            .default_value(super::mask_editor::DEFAULT_RADIAL_FEATHER)
                                            .help(
                                                "Soften the oval's edge; 0 is hard, 1 fades across the full radius.",
                                            )
                                            .show(ui, &mut feather)
                                            .edit;
                                            self.photo_control(response, |adjustment| {
                                                if let crate::adjustment_layers::AdjustmentLayerCoverage::Radial { feather: value, .. } = &mut adjustment.mask { *value = feather; }
                                            });
                                        }
                                    });
                                });
                            });
                        }
                    });
                },
            );
            body.is_some()
        }).inner;
        self.document.visit.adjustment_layers.panel_collapsed = !panel_expanded;

        if action.is_some()
            || rename_finish.is_some()
            || self.document.visit.adjustment_layers.creation.is_some()
            || start_drag.is_some()
        {
            ui.ctx().memory_mut(|memory| {
                if let Some(id) = memory.focused() {
                    memory.surrender_focus(id);
                }
            });
        }
        if let Some(error) = self
            .document
            .visit
            .masks
            .gesture()
            .and_then(MaskEdit::error)
        {
            ui.weak(format!(
                "Mask not accepted: {error}. Draw again or Escape to cancel."
            ));
        }
        let editable = editable
            && selected
                .is_some_and(|record| record.adjustment.enabled && record.adjustment.strength > 0.)
            && self.adjustment_layer_actions_available();

        let reveal = std::mem::take(&mut self.document.visit.adjustment_layers.reveal_controls);
        let pick = ui
            .push_id(
                ("photo-input", self.document.visit.current_source, target),
                |ui| {
                    self.photographic_controls(
                        ui,
                        adjustment
                            .map(|adjustment| adjustment.photo)
                            .unwrap_or_default(),
                        editable,
                        can_pick
                            && selected.is_some_and(|record| {
                                record.adjustment.enabled && record.adjustment.strength > 0.
                            }),
                        reveal,
                    )
                },
            )
            .inner;
        if let Some((id, invert)) = invert_action {
            self.cancel_control_edit();
            if let Err(error) = self.edit_adjustment_layers(|layers| {
                record_mut(layers, id)?.adjustment.invert = invert;
                Ok(())
            }) {
                self.status = format!("Adjustment edit rejected: {error}");
            }
        }
        if let Some(rename) = rename_finish {
            self.apply_adjustment_layer_action(rename, cx);
        }
        if let Some(action) = action {
            self.apply_adjustment_layer_action(action, cx);
            ui.ctx().request_repaint();
        }

        let finish = ui.input(|input| input.pointer.any_released())
            || !egui::DragAndDrop::has_payload_of_type::<OrderKey>(ui.ctx());
        if start_drag.is_some()
            || drop.is_some()
            || finish && self.document.visit.adjustment_layers.dragging()
        {
            self.adjustment_layer_order_input(start_drag, drop, finish, cx);
            ui.ctx().request_repaint();
        }

        if let Some(radial) = self.document.visit.adjustment_layers.creation.take() {
            self.cancel_control_edit();
            if radial {
                self.begin_radial_mask(None);
            } else {
                self.begin_linear_mask(None);
            }
            ui.ctx().request_repaint();
        }
        pick
    }

    fn request_adjustment_layer_deletion(
        &mut self,
        source: SourceId,
        id: AdjustmentLayerId,
        cx: &UiContext<'_, Self>,
    ) -> Result<(), Error> {
        let source = self.document.project.source(source)?.clone();
        let record = source
            .adjustment_layers
            .as_ref()
            .and_then(|layers| layers.layers.iter().find(|record| record.id == id))
            .ok_or(Error::InvalidParameter("unknown adjustment-layer ID"))?;
        let title = format!("Delete {}?", record.name);
        let response = self.dialogs.show(move |context, dialog, escape| {
            ui::DecisionDialog::new(&title,
                "This removes the layer and its coverage. Other layers and the RAW file are unchanged.",
                "Delete layer").warning().show(context, dialog, escape)
        });
        self.document.visit.interaction_action = cx.spawn(move |cx| async move {
            if response.await != Ok(ui::DialogChoice::Confirm) {
                return;
            }
            cx.update(|editor| {
                if editor.document.visit.current_source != Some(source.id)
                    || editor.document.project.source(source.id).ok() != Some(&source)
                {
                    editor.status = "Adjustment changed; deletion cancelled".into();
                    return;
                }
                match editor.edit_adjustment_layers(|layers| layers.remove(id)) {
                    Ok(()) => {
                        if editor.adjustment_layer_target() == Target::None {
                            editor.document.visit.adjustment_layers.selected = None;
                            if editor.tab == 1 {
                                editor.reveal_adjustment_layers();
                            }
                        }
                        editor.document.visit.adjustment_layers.rename = None;
                    }
                    Err(error) => editor.status = format!("Adjustment deletion rejected: {error}"),
                }
            });
        });
        Ok(())
    }
}

fn mask_icon(mask: crate::adjustment_layers::AdjustmentLayerCoverage) -> ui::icons::Icon {
    match mask {
        crate::adjustment_layers::AdjustmentLayerCoverage::Global {} => {
            ui::icons::Icon::GlobalAdjustmentLayer
        }
        crate::adjustment_layers::AdjustmentLayerCoverage::Linear { .. } => {
            ui::icons::Icon::LinearGradient
        }
        crate::adjustment_layers::AdjustmentLayerCoverage::Radial { .. } => {
            ui::icons::Icon::RadialGradient
        }
    }
}

fn record_mut(
    layers: &mut AdjustmentLayers,
    id: AdjustmentLayerId,
) -> Result<&mut crate::project::AdjustmentLayerRecord, Error> {
    layers
        .layers
        .iter_mut()
        .find(|record| record.id == id)
        .ok_or(Error::InvalidParameter("unknown adjustment-layer ID"))
}
#[cfg(test)]
mod tests;
