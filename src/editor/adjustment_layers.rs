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
    deletion: Option<(Source, AdjustmentLayerId)>,
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

    pub(super) fn confirming(&self) -> bool {
        self.deletion.is_some()
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
        self.document.visit.adjustment_layer_settings = self
            .selected_adjustment_layer_settings()
            .unwrap_or_default();
    }

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
        if self
            .active_pick()
            .is_some_and(|display| display.lock_layer_selection)
        {
            return;
        }
        self.sync_document();
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
        self.document.visit.adjustment_layer_settings = self
            .selected_adjustment_layer_settings()
            .unwrap_or_default();
        self.document.visit.adjustment_layers.rename = None;
    }

    pub(super) fn reveal_adjustment_layers(&mut self) {
        self.tab = 1;
        self.document.visit.adjustment_layers.reveal = true;
    }

    pub(super) fn finish_adjustment_layer_interaction(&mut self) {
        self.finish_mask(false);
        self.document.visit.adjustment_layers.rename = None;
        self.document.visit.adjustment_layers.owns_delete = false;
        self.document.visit.adjustment_layers.order_drag = None;
        self.document.visit.adjustment_layers.creation = None;
        self.document.visit.adjustment_layers.deletion = None;
        self.document.visit.adjustment_layers.reveal_controls = false;
    }

    fn adjustment_layer_actions_available(&self) -> bool {
        !self.busy
            && self.document.input.ready()
            && self.active_pick().is_none()
            && self.document.visit.crop_edit.is_none()
            && self.document.auto_crop_edit.is_none()
            && self.document.flat_edit.is_none()
            && self.document.visit.mask_edit.is_none()
            && self.document.analysis_review.is_none()
            && !self.document.visit.adjustment_layers.confirming()
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
        let project_before = self.history_snapshot();
        let id = self
            .document
            .visit
            .current_source
            .ok_or(Error::InvalidParameter("no selected source"))?;
        let source = self.document.state.project.source(id)?;
        let before = source.grade_snapshot()?;
        let target = self.adjustment_layer_target();
        let mut candidate = source.clone();
        self.overlay_adjustment_layer_settings(
            &mut candidate,
            self.document.visit.adjustment_layer_settings,
        );
        edit(
            candidate
                .adjustment_layers
                .get_or_insert_with(Default::default),
        )?;
        let after = candidate.grade_snapshot()?;
        self.validate_source_candidate(&candidate)?;
        self.document
            .state
            .project
            .source_mut(id)?
            .adjustment_layers = candidate.adjustment_layers;
        self.document.visit.adjustment_layers.selected = match target {
            Target::Layer(layer) => Some((id, layer)),
            Target::None => None,
        };
        self.refresh_adjustment_layer_selection();
        if before != after {
            self.invalidate_original();
        }
        self.record_edit(project_before, label, Some(id));
        Ok(())
    }

    pub(super) fn adjustment_layer_order_input(
        &mut self,
        start: Option<AdjustmentLayerId>,
        drop: Option<(OrderKey, usize)>,
        finish: bool,
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
            self.apply_adjustment_layer_action(Action::Reorder(drag, slot));
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

    pub(super) fn apply_adjustment_layer_action(&mut self, action: Action) {
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
            Action::Delete(id) => {
                self.document.visit.adjustment_layers.deletion = self
                    .document
                    .project
                    .source(source)
                    .ok()
                    .map(|source| (source.clone(), id));
                Ok(())
            }
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
                    || self.busy
                    || !egui::DragAndDrop::has_payload_of_type::<OrderKey>(ctx)
            })
        {
            self.document.visit.adjustment_layers.order_drag = None;
            egui::DragAndDrop::take_payload::<OrderKey>(ctx);
        }
    }

    pub(super) fn adjustment_layer_owns_delete(&self) -> bool {
        self.tab == 1
            && !self.document.visit.dust.cleanup
            && self.active_pick().is_none()
            && self.document.visit.crop_edit.is_none()
            && self.document.auto_crop_edit.is_none()
            && self.document.flat_edit.is_none()
            && self.document.visit.mask_edit.is_none()
            && self.document.analysis_review.is_none()
            && self.document.visit.adjustment_layers.owns_delete
            && matches!(self.adjustment_layer_target(), Target::Layer(_))
    }

    pub(super) fn delete_selected_adjustment_layer(&mut self) {
        if !self.document.visit.adjustment_layers.dragging()
            && let Target::Layer(id) = self.adjustment_layer_target()
        {
            self.apply_adjustment_layer_action(Action::Delete(id));
        }
    }

    pub(super) fn adjustment_layer_overlays_visible(&self) -> bool {
        self.document.visit.mask_edit.is_some()
            || (!self.document.visit.adjustment_layers.panel_collapsed
                && !self.document.visit.adjustment_layers.adjusting_values)
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
            .and_then(|id| self.document.state.project.source(id).ok())
            .and_then(|source| source.adjustment_layers.as_ref())
            .map(|layers| layers.layers.clone())
            .unwrap_or_default();
        let selected = match target {
            Target::None => None,
            Target::Layer(id) => records.iter().find(|record| record.id == id),
        };
        let mut adjustment = selected.map(|record| record.adjustment);
        let mut photo = selected.map_or(self.document.visit.adjustment_layer_settings, |record| {
            record.adjustment.photo
        });
        let before = (photo, adjustment);
        let available = self.adjustment_layer_actions_available()
            && !self.document.visit.dust.cleanup
            && ui.is_enabled();
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
            let (body, _) = presentation::section(
                ui,
                "adjustment-layers",
                &title,
                presentation::SectionHeader {
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
                                        icons::toggle(
                                            ui,
                                            icons::Icon::Eye,
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
                            if icons::button(
                                ui,
                                icons::Icon::AddGlobalAdjustmentLayer,
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
                                    icons::Icon::AddRadialGradient,
                                    "Add radial",
                                    true,
                                    "Add radial gradient",
                                ),
                                (
                                    icons::Icon::AddLinearGradient,
                                    "Add linear",
                                    false,
                                    "Add linear gradient",
                                ),
                            ] {
                                if icons::button(ui, icon, available && !self.document.source, name, help)
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
                                            [ui.available_width(), presentation::CONTROL_HEIGHT],
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
                                    let (row, toggle) = presentation::draggable_icon_row(
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
                        if let Some(adjustment) = &mut adjustment {
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
                                            invert_action = Some(
                                                editing::Command::InvertAdjustmentLayer(id, invert),
                                            );
                                        }
                                    });
                                    controls::parameter(
                                        ui,
                                        controls::Parameter::Strength,
                                        &mut adjustment.strength,
                                    );
                                    ui.add_enabled_ui(adjustment.strength > 0., |ui| {
                                        if let crate::adjustment_layers::AdjustmentLayerCoverage::Radial {
                                            feather,
                                            ..
                                        } = &mut adjustment.mask
                                        {
                                            controls::parameter(ui, controls::Parameter::Feather, feather);
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
            .mask_edit
            .as_ref()
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
                    ui.add_enabled_ui(editable, |ui| {
                        if reveal {
                            ui.scroll_to_cursor(Some(egui::Align::Min));
                        }
                        controls::photographic_exposure(ui, &mut photo);
                        controls::photographic_colour(
                            ui,
                            &mut photo,
                            can_pick
                                && selected.is_some_and(|record| {
                                    record.adjustment.enabled && record.adjustment.strength > 0.
                                }),
                        )
                    })
                    .inner
                },
            )
            .inner;
        if (photo, adjustment) != before {
            match target {
                Target::None => {}
                Target::Layer(id) => {
                    let mut adjustment = adjustment.expect("selected adjustment layer");
                    adjustment.photo = photo;
                    if let Err(error) =
                        self.edit_adjustment_layers_named("Photo settings", |layers| {
                            record_mut(layers, id)?.adjustment = adjustment;
                            Ok(())
                        })
                    {
                        self.status = format!("Adjustment edit rejected: {error}");
                    }
                }
            }
        }
        if let Some(command) = invert_action {
            self.control_command(command, cx);
        }
        if let Some(rename) = rename_finish {
            self.control_command(editing::Command::Adjustment(rename), cx);
        }
        if let Some(action) = action {
            self.control_command(editing::Command::Adjustment(action), cx);
            ui.ctx().request_repaint();
        }

        let finish = ui.input(|input| input.pointer.any_released())
            || !egui::DragAndDrop::has_payload_of_type::<OrderKey>(ui.ctx());
        if start_drag.is_some()
            || drop.is_some()
            || finish && self.document.visit.adjustment_layers.dragging()
        {
            self.control_command(
                editing::Command::OrderInput {
                    start: start_drag,
                    drop,
                    finish,
                },
                cx,
            );
            ui.ctx().request_repaint();
        }

        if let Some(radial) = self.document.visit.adjustment_layers.creation.take() {
            self.control_command(editing::Command::CreateMask(radial), cx);
            ui.ctx().request_repaint();
        }
        pick
    }

    pub(super) fn adjustment_layer_dialog(
        &mut self,
        context: &egui::Context,
        escape_available: bool,
    ) {
        let Some((source, id)) = self.document.visit.adjustment_layers.deletion.clone() else {
            return;
        };
        let Some(record) = source
            .adjustment_layers
            .as_ref()
            .and_then(|layers| layers.layers.iter().find(|record| record.id == id))
        else {
            self.document.visit.adjustment_layers.deletion = None;
            return;
        };
        let title = format!("Delete {}?", record.name);
        let choice = presentation::DecisionDialog::new(
            &title,
            "This removes the layer and its coverage. Other layers and the RAW file are unchanged.",
            "Delete layer",
        )
        .warning()
        .show(
            context,
            egui::Id::new("delete-adjustment-layer"),
            escape_available,
        );
        if choice == Some(presentation::DialogChoice::Cancel) {
            self.document.visit.adjustment_layers.deletion = None;
        } else if choice == Some(presentation::DialogChoice::Confirm) {
            self.document.visit.adjustment_layers.deletion = None;
            if self.document.visit.current_source != Some(source.id)
                || self.document.state.project.source(source.id).ok() != Some(&source)
            {
                self.status = "Adjustment changed; deletion cancelled".into();
                return;
            }
            match self.edit_adjustment_layers(|layers| layers.remove(id)) {
                Ok(()) => {
                    if self.adjustment_layer_target() == Target::None {
                        self.document.visit.adjustment_layers.selected = None;
                        if self.tab == 1 {
                            self.reveal_adjustment_layers();
                        }
                    }
                    self.document.visit.adjustment_layers.rename = None;
                }
                Err(error) => self.status = format!("Adjustment deletion rejected: {error}"),
            }
        }
    }
}

fn mask_icon(mask: crate::adjustment_layers::AdjustmentLayerCoverage) -> icons::Icon {
    match mask {
        crate::adjustment_layers::AdjustmentLayerCoverage::Global {} => {
            icons::Icon::GlobalAdjustmentLayer
        }
        crate::adjustment_layers::AdjustmentLayerCoverage::Linear { .. } => {
            icons::Icon::LinearGradient
        }
        crate::adjustment_layers::AdjustmentLayerCoverage::Radial { .. } => {
            icons::Icon::RadialGradient
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
