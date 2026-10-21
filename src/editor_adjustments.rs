use super::*;
use crate::project::{AdjustmentId, LocalAdjustments};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(super) enum Target {
    #[default]
    Global,
    Local(AdjustmentId),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct OrderKey {
    loaded: u64,
    source: SourceId,
    id: AdjustmentId,
}

pub(super) struct OrderDrag {
    key: OrderKey,
    locals: LocalAdjustments,
}

#[derive(Default)]
pub(super) struct Adjustments {
    selected: Option<(SourceId, AdjustmentId)>,
    rename: Option<(SourceId, AdjustmentId, String)>,
    deletion: Option<(Source, AdjustmentId)>,
    pub(super) show_coverage: bool,
    pub(super) reveal_controls: bool,
    open: bool,
    toggle: bool,
    reveal_drawer: bool,
    rename_focus: bool,
    creation: Option<bool>,
    target_visible: bool,
    order_drag: Option<OrderDrag>,
}

impl Adjustments {
    pub(super) fn confirming(&self) -> bool {
        self.deletion.is_some()
    }

    pub(super) fn drawer_open(&self) -> bool {
        self.open
    }

    pub(super) fn dragging(&self) -> bool {
        self.order_drag.is_some()
    }

    pub(super) fn close_drawer(&mut self) {
        self.open = false;
        self.target_visible = false;
        self.rename = None;
    }
}

pub(super) enum Action {
    Select(Target),
    Overlay(bool),
    Enable(AdjustmentId, bool),
    Reorder(OrderDrag, usize),
    RenameStart(AdjustmentId),
    RenameCancel,
    Rename(AdjustmentId, String),
    Delete(AdjustmentId),
}

impl Editor {
    pub(super) fn adjustment_target(&self) -> Target {
        if !self.document.visit.adjustments.open {
            return Target::Global;
        }
        self.document
            .visit
            .adjustments
            .selected
            .filter(|(source, id)| {
                Some(*source) == self.current_source
                    && self
                        .document
                        .project
                        .source(*source)
                        .ok()
                        .and_then(|source| source.local_adjustments.as_ref())
                        .is_some_and(|locals| {
                            locals.adjustments.iter().any(|record| record.id == *id)
                        })
            })
            .map_or(Target::Global, |(_, id)| Target::Local(id))
    }

    pub(super) fn select_adjustment(&mut self, target: Target) {
        self.document.visit.adjustments.selected = match target {
            Target::Global => None,
            Target::Local(id) => self
                .current_source
                .filter(|source| {
                    self.document.project.source(*source).is_ok_and(|source| {
                        source.local_adjustments.as_ref().is_some_and(|locals| {
                            locals.adjustments.iter().any(|record| record.id == id)
                        })
                    })
                })
                .map(|source| (source, id)),
        };
        if self.document.visit.adjustments.selected.is_some() {
            self.document.visit.adjustments.open = true;
        } else {
            self.close_adjustments();
        }
        self.document.visit.adjustments.rename = None;
    }

    pub(super) fn open_adjustments(&mut self) {
        self.document.visit.adjustments.open = true;
        if self.adjustment_target() == Target::Global {
            self.document.visit.adjustments.selected = self.current_source.and_then(|source| {
                self.document
                    .project
                    .source(source)
                    .ok()?
                    .local_adjustments
                    .as_ref()?
                    .adjustments
                    .first()
                    .map(|record| (source, record.id))
            });
        }
        self.document.visit.adjustments.reveal_drawer = true;
        self.document.visit.adjustments.target_visible = false;
    }

    pub(super) fn close_adjustments(&mut self) {
        self.finish_mask(false);
        self.document.visit.adjustments.close_drawer();
        self.document.visit.adjustments.order_drag = None;
        self.document.visit.adjustments.creation = None;
        self.document.visit.adjustments.deletion = None;
        self.document.visit.adjustments.reveal_controls = false;
        if let Some(context) = &mut self.document.pick_return
            && let Some((_, target)) = &mut context.adjustment
        {
            *target = Target::Global;
        }
    }

    fn adjustment_actions_available(&self) -> bool {
        !self.busy
            && self.document.visit.ready.is_some()
            && self.calibration().is_some()
            && self.pick.is_none()
            && self.document.visit.crop_edit.is_none()
            && self.document.auto_crop_edit.is_none()
            && self.flat_edit.is_none()
            && self.document.visit.mask_edit.is_none()
            && self.document.analysis_review.is_none()
            && !self.document.visit.adjustments.confirming()
    }

    pub(super) fn edit_adjustments(
        &mut self,
        edit: impl FnOnce(&mut LocalAdjustments) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let project_before = self.history_snapshot();
        let id = self
            .current_source
            .ok_or(Error::InvalidParameter("no selected source"))?;
        let source = self.document.project.source(id)?;
        let before = source.grade_snapshot()?;
        let mut candidate = source.clone();
        candidate.photo = self.photo;
        edit(
            candidate
                .local_adjustments
                .get_or_insert_with(Default::default),
        )?;
        let after = candidate.grade_snapshot()?;
        self.validate_source_candidate(&candidate)?;
        self.document.project.source_mut(id)?.local_adjustments = candidate.local_adjustments;
        if before != after {
            self.invalidate_original();
        }
        self.record_edit(project_before, "Local adjustment", Some(id));
        Ok(())
    }

    pub(super) fn adjustment_order_input(
        &mut self,
        start: Option<AdjustmentId>,
        drop: Option<(OrderKey, usize)>,
        finish: bool,
    ) {
        if let Some(id) = start {
            match self.order_drag(id) {
                Ok(drag) => self.document.visit.adjustments.order_drag = Some(drag),
                Err(error) => self.status = format!("Adjustment drag rejected: {error}"),
            }
        }
        if let Some((key, slot)) = drop
            && let Some(drag) = self
                .document
                .visit
                .adjustments
                .order_drag
                .take()
                .filter(|drag| drag.key == key)
        {
            self.apply_adjustment_action(Action::Reorder(drag, slot));
        }
        if finish {
            self.document.visit.adjustments.order_drag = None;
        }
    }

    fn order_drag(&self, id: AdjustmentId) -> Result<OrderDrag, Error> {
        let source = self
            .current_source
            .ok_or(Error::InvalidParameter("no selected source"))?;
        let locals = self
            .document
            .project
            .source(source)?
            .local_adjustments
            .as_ref()
            .ok_or(Error::InvalidParameter("no local adjustments"))?
            .clone();
        if !locals.adjustments.iter().any(|record| record.id == id) {
            return Err(Error::InvalidParameter("unknown local-adjustment ID"));
        }
        Ok(OrderDrag {
            key: OrderKey {
                loaded: self.id,
                source,
                id,
            },
            locals,
        })
    }

    fn reorder_adjustment(&mut self, drag: OrderDrag, slot: usize) -> Result<(), Error> {
        if drag.key.loaded != self.id || Some(drag.key.source) != self.current_source {
            return Err(Error::InvalidParameter(
                "adjustment source changed during drag",
            ));
        }
        self.edit_adjustments(|locals| {
            if *locals != drag.locals {
                return Err(Error::InvalidParameter("adjustments changed during drag"));
            }
            if slot > locals.adjustments.len() {
                return Err(Error::InvalidParameter("invalid adjustment drop position"));
            }
            let index = locals
                .adjustments
                .iter()
                .position(|record| record.id == drag.key.id)
                .ok_or(Error::InvalidParameter("unknown local-adjustment ID"))?;
            let record = locals.adjustments.remove(index);
            locals
                .adjustments
                .insert(slot - usize::from(index < slot), record);
            Ok(())
        })
    }

    pub(super) fn apply_adjustment_action(&mut self, action: Action) {
        if !self.adjustment_actions_available() || !self.document.visit.adjustments.open {
            return;
        }
        let Some(source) = self.current_source else {
            return;
        };
        let result = match action {
            Action::Select(target) => {
                self.select_adjustment(target);
                Ok(())
            }
            Action::Overlay(show) => {
                self.document.visit.adjustments.show_coverage = show;
                Ok(())
            }
            Action::RenameStart(id) => {
                self.select_adjustment(Target::Local(id));
                self.document.visit.adjustments.rename_focus = true;
                self.document.visit.adjustments.rename = self
                    .document
                    .project
                    .source(source)
                    .ok()
                    .and_then(|source| source.local_adjustments.as_ref())
                    .and_then(|locals| locals.adjustments.iter().find(|record| record.id == id))
                    .map(|record| (source, id, record.name.clone()));
                Ok(())
            }
            Action::RenameCancel => {
                self.document.visit.adjustments.rename = None;
                Ok(())
            }
            Action::Delete(id) => {
                self.document.visit.adjustments.deletion = self
                    .document
                    .project
                    .source(source)
                    .ok()
                    .map(|source| (source.clone(), id));
                Ok(())
            }
            Action::Enable(id, enabled) => self.edit_adjustments(|locals| {
                record_mut(locals, id)?.adjustment.enabled = enabled;
                Ok(())
            }),
            Action::Reorder(drag, slot) => self.reorder_adjustment(drag, slot),
            Action::Rename(id, name) => {
                let result = self.edit_adjustments(|locals| {
                    record_mut(locals, id)?.name = name;
                    Ok(())
                });
                if result.is_ok() {
                    self.document.visit.adjustments.rename = None;
                }
                result
            }
        };
        if let Err(error) = result {
            self.status = format!("Adjustment edit rejected: {error}");
        }
    }

    pub(super) fn sync_adjustment_drag(&mut self, ctx: &egui::Context) {
        if self
            .document
            .visit
            .adjustments
            .order_drag
            .as_ref()
            .is_some_and(|drag| {
                drag.key.loaded != self.id
                    || Some(drag.key.source) != self.current_source
                    || self.tab != 1
                    || self.fullscreen.is_some()
                    || !self.document.visit.adjustments.open
                    || self.busy
                    || !egui::DragAndDrop::has_payload_of_type::<OrderKey>(ctx)
            })
        {
            self.document.visit.adjustments.order_drag = None;
            egui::DragAndDrop::take_payload::<OrderKey>(ctx);
        }
    }

    pub(super) fn adjustment_owns_delete(&self) -> bool {
        self.tab == 1
            && !self.document.dust.cleanup
            && self.pick.is_none()
            && self.document.visit.crop_edit.is_none()
            && self.document.auto_crop_edit.is_none()
            && self.flat_edit.is_none()
            && self.document.visit.mask_edit.is_none()
            && self.document.analysis_review.is_none()
            && self.document.visit.adjustments.open
    }

    pub(super) fn delete_selected_adjustment(&mut self) {
        if !self.document.visit.adjustments.dragging()
            && let Target::Local(id) = self.adjustment_target()
        {
            self.apply_adjustment_action(Action::Delete(id));
        }
    }

    pub(super) fn adjustment_target_label(&self, ui: &mut egui::Ui) {
        if self.document.visit.mask_edit.is_some() || self.document.visit.adjustments.target_visible
        {
            return;
        }
        let record = match self.adjustment_target() {
            Target::Global => None,
            Target::Local(id) => self
                .current_source
                .and_then(|source| self.document.project.source(source).ok())
                .and_then(|source| source.local_adjustments.as_ref())
                .and_then(|locals| locals.adjustments.iter().find(|record| record.id == id)),
        };
        let Some(record) = record else {
            return;
        };
        let (name, icon) = (record.name.as_str(), mask_icon(record.adjustment.mask));
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(16., 16.), egui::Sense::hover());
            icon.paint(ui, rect, ui.visuals().widgets.noninteractive.fg_stroke);
            let text = egui::RichText::new(format!("Editing: {name}"));
            ui.add(egui::Label::new(text.strong()).truncate())
                .on_hover_text(name);
        });
    }

    pub(super) fn adjustment_launcher(&mut self, ui: &mut egui::Ui, enabled: bool) {
        let changed = self
            .current_source
            .and_then(|id| self.document.project.source(id).ok())
            .and_then(|source| source.local_adjustments.as_ref())
            .is_some_and(|locals| !locals.adjustments.is_empty());
        if presentation::ToolButton::tool(
            icons::Icon::LinearGradient,
            "Adjustments",
            "Edit local adjustments",
            self.document.visit.adjustments.open,
        )
        .enabled(
            enabled
                || self.document.visit.adjustments.open
                    && !self.busy
                    && !self.document.visit.adjustments.dragging()
                    && !self
                        .document
                        .visit
                        .mask_edit
                        .as_ref()
                        .is_some_and(MaskEdit::dragging),
        )
        .changed(changed)
        .show(ui)
        .clicked()
        {
            self.document.visit.adjustments.toggle = true;
        }
    }

    pub(super) fn adjustment_controls(
        &mut self,
        ui: &mut egui::Ui,
        cx: &UiContext<'_, Self>,
        editable: bool,
        can_pick: bool,
    ) -> bool {
        let target = self.adjustment_target();
        let records = self
            .current_source
            .and_then(|id| self.document.project.source(id).ok())
            .and_then(|source| source.local_adjustments.as_ref())
            .map(|locals| locals.adjustments.clone())
            .unwrap_or_default();
        let selected = match target {
            Target::Global => None,
            Target::Local(id) => records.iter().find(|record| record.id == id),
        };
        let mut adjustment = selected.map(|record| record.adjustment);
        let mut photo = selected.map_or(self.photo, |record| record.adjustment.photo);
        let before = (photo, adjustment);
        let available =
            self.adjustment_actions_available() && !self.document.dust.cleanup && ui.is_enabled();
        let mut action = None;
        let mut rename_finish = None;
        let mut invert_action = None;
        let mut target_visible = false;
        let mut start_drag = None;
        let mut drop = None;
        // Keep one stable child even when closed: hiding the drawer must not reset grading inputs.
        ui.push_id("adjustment-drawer", |ui| {
            if !self.document.visit.adjustments.open {
                return;
            }
            if std::mem::take(&mut self.document.visit.adjustments.reveal_drawer) {
                ui.scroll_to_cursor(Some(egui::Align::Min));
            }
            let identity = presentation::ToolIdentity {
                icon: icons::Icon::LinearGradient,
                title: "Adjustments",
            };
            identity.inspector(
                ui,
                |ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let mut overlay = None;
                        let mut creation = None;
                        if selected.is_some() {
                            let mut show = self.document.visit.adjustments.show_coverage;
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
                            if icons::button(ui, icon, available && !self.source, name, help)
                                .clicked()
                            {
                                creation = Some(radial);
                            }
                        }
                        (overlay, creation)
                    })
                    .inner
                },
                |ui, (overlay, creation)| {
                    if overlay.is_some() {
                        action = overlay;
                    }
                    if creation.is_some() {
                        self.document.visit.adjustments.creation = creation;
                    }
                    ui.add_enabled_ui(available, |ui| {
                        if records.is_empty() {
                            ui.weak("Add a gradient to start a local adjustment.");
                        }
                        for (index, record) in records.iter().enumerate() {
                            ui.push_id(record.id, |ui| {
                                if let Some((source, id, text)) =
                                    &mut self.document.visit.adjustments.rename
                                    && Some(*source) == self.current_source
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
                                    if target == Target::Local(record.id) {
                                        target_visible =
                                            ui.clip_rect().contains_rect(response.rect);
                                    }
                                    if std::mem::take(
                                        &mut self.document.visit.adjustments.rename_focus,
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
                                        target == Target::Local(record.id),
                                        Some(&mut enabled),
                                    );
                                    if target == Target::Local(record.id) {
                                        target_visible = ui.clip_rect().contains_rect(row.rect);
                                    }
                                    let key = OrderKey {
                                        loaded: self.id,
                                        source: self.current_source.unwrap(),
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
                                        && Some(payload.source) == self.current_source
                                    {
                                        let below = ui.input(|input| {
                                            input
                                                .pointer
                                                .interact_pos()
                                                .is_some_and(|point| point.y >= rect.center().y)
                                        });
                                        let slot = index + usize::from(below);
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
                                        action = Some(Action::Select(Target::Local(record.id)));
                                    }
                                }
                            });
                        }
                        if let Some(adjustment) = &mut adjustment {
                            ui.add_space(6.);
                            ui.push_id(("mask-input", self.current_source, target), |ui| {
                                let mut invert = adjustment.invert;
                                if ui.checkbox(&mut invert, "Invert mask").changed()
                                    && let Target::Local(id) = target
                                {
                                    invert_action = Some(editing::Command::InvertLocal(id, invert));
                                }
                                controls::parameter(
                                    ui,
                                    controls::Parameter::Strength,
                                    &mut adjustment.strength,
                                );
                                if let crate::local_adjustments::GradientMask::Radial {
                                    feather,
                                    ..
                                } = &mut adjustment.mask
                                {
                                    controls::parameter(ui, controls::Parameter::Feather, feather);
                                }
                            });
                        }
                    });
                },
            );
        });
        if self.document.visit.adjustments.target_visible != target_visible {
            ui.ctx().request_repaint();
        }
        self.document.visit.adjustments.target_visible = target_visible;
        if action.is_some()
            || rename_finish.is_some()
            || self.document.visit.adjustments.creation.is_some()
            || self.document.visit.adjustments.toggle
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
        if selected.is_some_and(|record| !record.adjustment.enabled) {
            ui.weak("Disabled · changes have no effect");
        }
        let editable = editable
            && (!self.document.visit.adjustments.open || selected.is_some())
            && (target == Target::Global || self.adjustment_actions_available());
        let reveal = std::mem::take(&mut self.document.visit.adjustments.reveal_controls);
        let pick = ui
            .push_id(("photo-input", self.current_source, target), |ui| {
                ui.add_enabled_ui(editable, |ui| {
                    if reveal {
                        ui.scroll_to_cursor(Some(egui::Align::Min));
                    }
                    controls::photographic_exposure(ui, &mut photo);
                    controls::photographic_colour(
                        ui,
                        &mut photo,
                        can_pick && target == Target::Global,
                    )
                })
                .inner
            })
            .inner;
        if (photo, adjustment) != before {
            match target {
                Target::Global => self.photo = photo,
                Target::Local(id) => {
                    let mut adjustment = adjustment.expect("selected local adjustment");
                    adjustment.photo = photo;
                    if let Err(error) = self.edit_adjustments(|locals| {
                        record_mut(locals, id)?.adjustment = adjustment;
                        Ok(())
                    }) {
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
        let drop = drop.filter(|_| !self.document.visit.adjustments.toggle);
        let finish = ui.input(|input| input.pointer.any_released())
            || !egui::DragAndDrop::has_payload_of_type::<OrderKey>(ui.ctx());
        if start_drag.is_some()
            || drop.is_some()
            || finish && self.document.visit.adjustments.dragging()
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
        if std::mem::take(&mut self.document.visit.adjustments.toggle) {
            self.control_command(editing::Command::ToggleAdjustments, cx);
            ui.ctx().request_repaint();
        }
        if let Some(radial) = self.document.visit.adjustments.creation.take() {
            self.control_command(editing::Command::CreateMask(radial), cx);
            ui.ctx().request_repaint();
        }
        pick
    }

    pub(super) fn adjustment_dialog(&mut self, context: &egui::Context, escape_available: bool) {
        let Some((source, id)) = self.document.visit.adjustments.deletion.clone() else {
            return;
        };
        let Some(record) = source
            .local_adjustments
            .as_ref()
            .and_then(|locals| locals.adjustments.iter().find(|record| record.id == id))
        else {
            self.document.visit.adjustments.deletion = None;
            return;
        };
        let title = format!("Delete {}?", record.name);
        let choice = presentation::DecisionDialog::new(
            &title,
            "This removes the adjustment and its mask. Other adjustments and the RAW file are unchanged.",
            "Delete adjustment",
        )
        .warning()
        .show(
            context,
            egui::Id::new("delete-local-adjustment"),
            escape_available,
        );
        if choice == Some(presentation::DialogChoice::Cancel) {
            self.document.visit.adjustments.deletion = None;
        } else if choice == Some(presentation::DialogChoice::Confirm) {
            self.document.visit.adjustments.deletion = None;
            if self.current_source != Some(source.id)
                || self.document.project.source(source.id).ok() != Some(&source)
            {
                self.status = "Adjustment changed; deletion cancelled".into();
                return;
            }
            match self.edit_adjustments(|locals| locals.remove(id)) {
                Ok(()) => {
                    if self.adjustment_target() == Target::Global {
                        self.document.visit.adjustments.selected = None;
                        if self.document.visit.adjustments.open {
                            self.open_adjustments();
                        }
                    }
                    self.document.visit.adjustments.rename = None;
                }
                Err(error) => self.status = format!("Adjustment deletion rejected: {error}"),
            }
        }
    }
}

fn mask_icon(mask: crate::local_adjustments::GradientMask) -> icons::Icon {
    match mask {
        crate::local_adjustments::GradientMask::Linear { .. } => icons::Icon::LinearGradient,
        crate::local_adjustments::GradientMask::Radial { .. } => icons::Icon::RadialGradient,
    }
}

fn record_mut(
    locals: &mut LocalAdjustments,
    id: AdjustmentId,
) -> Result<&mut crate::project::AdjustmentRecord, Error> {
    locals
        .adjustments
        .iter_mut()
        .find(|record| record.id == id)
        .ok_or(Error::InvalidParameter("unknown local-adjustment ID"))
}
#[cfg(test)]
#[path = "editor_adjustments_tests.rs"]
mod tests;
