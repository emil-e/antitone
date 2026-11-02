use super::*;

impl tool::Tool for DustTool {
    fn presentation(&self, editor: &Editor) -> tool::Presentation {
        tool::Presentation {
            show_viewer: !editor.canvas_inspection(),
            identity: presentation::ToolIdentity {
                icon: icons::Icon::Dust,
                title: "Dust cleanup",
            },
            primary: None,
            dismiss: "Done",
            warning: None,
            help: "Draw for a new heal · click an outline to select another heal.\nShift-drag adds to selected · Option-drag removes · Delete removes selected.\nDrag the source outline to move it · release commits changes · H hides outlines.",
            navigation_help: Some("Scroll changes brush size · Cmd/Ctrl-scroll or pinch zooms"),
            fit_label: "Fit",
        }
    }

    fn drawer(&mut self, ui: &mut egui::Ui, editor: &mut Editor, cx: &UiContext<'_, Editor>) {
        self.interaction.searches.borrow_mut().prune();
        if std::mem::take(&mut self.reveal_controls) {
            ui.scroll_to_cursor(Some(egui::Align::Min));
        }
        let Some(source) = editor.document.visit.current_source else {
            return;
        };
        let repair_count = editor
            .dust_repairs(source)
            .map_or(0, |set| set.repairs().len());
        let searching = !self.interaction.searches.borrow().is_empty();
        let reset_enabled = !editor.busy
            && editor.document.visit.canvas.ready.is_some()
            && self.donor().is_none()
            && (repair_count > 0 || searching);
        presentation::ToolIdentity {
            icon: icons::Icon::Dust,
            title: "Dust repair"
        }.inspector(
            ui,
            |ui| ui
                .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let reset = icons::button(
                        ui,
                        icons::Icon::Reset,
                        reset_enabled,
                        "Reset dust repairs",
                        "Reset dust repairs to defaults."
                    ).clicked();
                    ui.weak(format!("{repair_count} repairs"));
                    reset
                }).inner,
            |ui, reset| {
                ui.horizontal(|ui| {
                    ui.label("Brush size")
                        .on_hover_text("Diameter in source pixels.");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.weak(format!("{} px", editor.document.visit.dust.brush_size));
                    });
                });
                ui.scope(|ui| {
                    ui.spacing_mut().slider_width = ui.available_width();
                    ui.add(
                        egui::Slider::new(&mut editor.document.visit.dust.brush_size, 1..=128)
                            .show_value(false)
                            .step_by(1.),
                    );
                });
                if let Some(error) = &editor.document.visit.dust.render_error {
                    ui.small("A repair is outside the current grade's GPU range.")
                        .on_hover_text(error);
                }
                ui.checkbox(
                    &mut self.interaction.edit.borrow_mut().show_masks,
                    "Show repair outlines",
                )
                .on_hover_text("Hold H for a momentary unobstructed preview.");
                let rejected = self.mask().is_some_and(|draft| draft.rejected);
                let guidance = egui::RichText::new(if rejected {
                    "Stroke too large · release to discard it and use smaller repairs."
                } else if editor.selected_dust_repair().is_some() {
                    "Draw for a new heal · Shift adds to selected · Option removes · drag its source."
                } else {
                    "Draw for a new heal · click an outline to select it · Delete removes selected."
                });
                ui.small(if rejected {
                    guidance.color(ui.visuals().error_fg_color)
                } else {
                    guidance
                });
                if searching {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.weak("Finding donor…");
                    });
                } else if repair_count == 0 {
                    ui.small("Manual masks only · automatic proposals are not wired yet");
                }
                if reset {
                    editor.control_command(editing::Command::ResetRepairs(source), cx);
                    editor.status = "Dust repairs reset".into();
                    editor.invalidate_settled();
                }
            },
        );
    }

    fn canvas(
        &mut self,
        canvas: &mut canvas::Canvas<'_>,
        editor: &mut Editor,
        cx: &UiContext<'_, Editor>,
    ) -> crate::viewport::CanvasLayout {
        let rect = canvas.response.rect;
        let composition = canvas.composition;
        let ppp = canvas.ui.ctx().pixels_per_point();
        let relayout =
            |viewport: &mut Viewport| viewport.layout_composition(rect, composition, ppp);
        let layout = canvas.layout(&mut editor.active_canvas_mut().viewport, relayout);
        if canvas.ui.is_enabled() && editor.tab == 1 {
            self.adjust_dust_brush_from_scroll(editor, canvas.ui, layout);
        }
        let mut response = canvas.response.clone();
        if editor.tab == 1 {
            navigation::consume_primary(&mut response);
        }
        let (layout, panning) = navigation::navigate_view(
            &mut editor.active_canvas_mut().viewport,
            canvas.ui,
            &response,
            layout,
            relayout,
        );
        canvas.ui.add_enabled_ui(!panning, |ui| {
            self.dust_interaction(editor, ui, canvas.response, layout, cx);
        });
        let ready = canvas.paint_image(editor, layout, false, cx);
        self.paint_dust(editor, canvas.ui, layout);
        if canvas.ui.is_enabled() && ready && !panning && editor.tab == 1 && !editor.document.source
        {
            let (point, modifying, space) = canvas.ui.input(|input| {
                (
                    input.pointer.hover_pos(),
                    input.modifiers.alt || input.modifiers.shift,
                    input.key_down(egui::Key::Space),
                )
            });
            if !space && point.is_some_and(|point| layout.image.contains(point)) {
                canvas
                    .ui
                    .ctx()
                    .set_cursor_icon(self.dust_canvas_cursor(editor, layout, point, modifying));
            }
        }
        layout
    }

    fn shortcuts(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        if ui.is_enabled()
            && !editor.busy
            && ui.input_mut(|input| {
                let delete = input.consume_key(egui::Modifiers::NONE, egui::Key::Delete);
                let backspace = input.consume_key(egui::Modifiers::NONE, egui::Key::Backspace);
                delete || backspace
            })
        {
            self.delete_selected_dust_repair(editor);
        }
    }

    fn action(&mut self, action: tool::Action, editor: &mut Editor) {
        if matches!(action, tool::Action::Dismiss) && !editor.busy {
            if !self.interaction.searches.borrow().is_empty() {
                editor.status = "Unfinished Dust strokes discarded".into();
            }
            editor.end_tool();
        }
    }
}

impl DustTool {
    pub(super) fn new(show_masks: bool, reveal_controls: bool) -> Self {
        Self {
            interaction: Rc::new(Interaction {
                edit: Rc::new(RefCell::new(Edit {
                    show_masks,
                    ..Default::default()
                })),
                searches: RefCell::default(),
            }),
            reveal_controls,
        }
    }

    fn mask(&self) -> Option<Ref<'_, MaskDraft>> {
        Ref::filter_map(self.interaction.edit.borrow(), |edit| {
            edit.mask_draft.as_ref()
        })
        .ok()
    }
    fn mask_mut(&mut self) -> Option<RefMut<'_, MaskDraft>> {
        RefMut::filter_map(self.interaction.edit.borrow_mut(), |edit| {
            edit.mask_draft.as_mut()
        })
        .ok()
    }
    fn set_mask(&mut self, value: Option<MaskDraft>) {
        self.interaction.edit.borrow_mut().mask_draft = value;
    }
    fn donor(&self) -> Option<DonorDraft> {
        self.interaction.edit.borrow().donor_draft
    }
    fn donor_mut(&mut self) -> Option<RefMut<'_, DonorDraft>> {
        RefMut::filter_map(self.interaction.edit.borrow_mut(), |edit| {
            edit.donor_draft.as_mut()
        })
        .ok()
    }
    fn set_donor(&mut self, value: Option<DonorDraft>) {
        self.interaction.edit.borrow_mut().donor_draft = value;
    }
    fn take_donor(&mut self) -> Option<DonorDraft> {
        self.interaction.edit.borrow_mut().donor_draft.take()
    }

    pub(super) fn dust_interaction(
        &mut self,
        editor: &mut Editor,
        ui: &egui::Ui,
        response: &egui::Response,
        geometry: crate::viewport::CanvasLayout,
        cx: &UiContext<'_, Editor>,
    ) {
        if editor.tab != 1 || editor.fullscreen.is_some() || editor.document.source {
            return;
        }
        let modifies = ui.input(|input| {
            input
                .raw
                .events
                .iter()
                .find_map(|event| match event {
                    egui::Event::PointerButton {
                        button: egui::PointerButton::Primary,
                        modifiers,
                        ..
                    } => Some(*modifiers),
                    _ => None,
                })
                .unwrap_or(input.modifiers)
        });
        let allowed =
            !editor.busy && ui.is_enabled() && !ui.input(|i| i.key_down(egui::Key::Space));
        if allowed {
            let started = response.drag_started_by(egui::PointerButton::Primary);
            let clicked = response.clicked_by(egui::PointerButton::Primary);
            let start = started
                .then(|| ui.input(|i| i.pointer.press_origin()))
                .flatten()
                .or_else(|| response.interact_pointer_pos());
            if (started || clicked)
                && self.mask().is_none()
                && !self.dust_donor_dragging()
                && let Some(point) = start.filter(|p| geometry.image.contains(*p))
            {
                if modifies.alt || modifies.shift {
                    if let Some(id) = editor.selected_dust_repair() {
                        self.begin_dust_mask(editor, Some(id));
                    }
                } else if let Some(id) = self.dust_repair_at(editor, geometry, point) {
                    self.select_dust_repair(editor, id);
                } else if self.dust_donor_at(editor, geometry, point) {
                    if started {
                        self.begin_dust_donor_drag(editor, geometry, point);
                    }
                } else {
                    self.begin_dust_mask(editor, None);
                }
                if self.mask().is_some() {
                    self.begin_dust_stroke();
                    self.paint_dust_stroke(editor, geometry, point, modifies.alt);
                }
            }
            if response.dragged_by(egui::PointerButton::Primary)
                && let Some(point) = response.interact_pointer_pos()
            {
                if self.dust_donor_dragging() {
                    self.drag_dust_donor(editor, geometry, point);
                } else if self.mask().is_some() {
                    self.paint_dust_stroke(editor, geometry, point, modifies.alt);
                }
            }
            if clicked {
                self.commit_dust_mask(editor, cx);
            }
        }
        if response.drag_stopped_by(egui::PointerButton::Primary) {
            self.end_dust_stroke();
            self.end_dust_donor_drag(editor);
            self.commit_dust_mask(editor, cx);
        }
    }

    pub(super) fn begin_dust_mask(&mut self, editor: &mut Editor, target: Option<RepairId>) {
        if let (Some(source), Some(repair)) = (editor.document.visit.current_source, target)
            && editor.dust_target_pending(source, repair)
        {
            editor.status = "Wait for this repair's donor search before editing its mask".into();
            return;
        }
        self.interaction.edit.borrow_mut().show_masks = true;
        self.set_donor(None);
        let support = target.and_then(|id| {
            let source = editor.document.visit.current_source?;
            editor
                .dust_repairs(source)?
                .repairs()
                .iter()
                .find(|repair| repair.id == id)
                .map(|repair| repair.support.clone())
        });
        self.set_mask(Some(MaskDraft::new(target, support.as_ref())));
    }

    pub(super) fn cancel_dust_mask(&mut self) {
        self.set_mask(None);
        self.set_donor(None);
    }

    pub(super) fn commit_dust_mask(&mut self, editor: &mut Editor, cx: &UiContext<'_, Editor>) {
        let Some((target, empty)) = self
            .mask()
            .map(|draft| (draft.target, draft.pixels.is_empty()))
        else {
            return;
        };
        if empty {
            self.cancel_dust_mask();
            if target.is_some() {
                self.delete_selected_dust_repair(editor);
            }
        } else {
            self.apply_dust_mask(editor, cx);
        }
    }

    pub(super) fn adjust_dust_brush_from_scroll(
        &mut self,
        editor: &mut Editor,
        ui: &egui::Ui,
        geometry: crate::viewport::CanvasLayout,
    ) {
        if editor.document.source {
            return;
        }
        let delta = ui.input_mut(|input| {
            if input
                .pointer
                .hover_pos()
                .is_none_or(|point| !geometry.image.contains(point))
            {
                return 0.;
            }
            let delta = input
                .raw
                .events
                .iter()
                .filter_map(|event| match event {
                    egui::Event::MouseWheel {
                        delta, modifiers, ..
                    } if !modifiers.command && !modifiers.ctrl => Some(delta.x + delta.y),
                    _ => None,
                })
                .sum();
            // Egui smooths wheel input across later frames. Cleanup owns an ordinary
            // wheel gesture for brush sizing, so suppress both its first pan and tail.
            input.smooth_scroll_delta = egui::Vec2::ZERO;
            delta
        });
        if delta.abs() < f32::EPSILON {
            return;
        }
        let step = ((delta.abs() / 12.).round() as u32).max(1);
        editor.document.visit.dust.brush_size = if delta > 0. {
            editor
                .document
                .visit
                .dust
                .brush_size
                .saturating_add(step)
                .min(128)
        } else {
            editor
                .document
                .visit
                .dust
                .brush_size
                .saturating_sub(step)
                .max(1)
        };
    }

    fn apply_dust_mask(&mut self, editor: &mut Editor, cx: &UiContext<'_, Editor>) {
        let Some(source) = editor.document.visit.current_source else {
            self.cancel_dust_mask();
            return;
        };
        if self.mask().is_some_and(|draft| draft.rejected) {
            self.cancel_dust_mask();
            editor.status =
                "Dust stroke rejected: repair budget exceeded; no changes applied".into();
            return;
        }
        let Some((target, support)) = self.mask().map(|draft| (draft.target, draft.support()))
        else {
            return;
        };
        let support = match support {
            Ok(support) => support,
            Err(error) => {
                editor.status = format!("Dust mask rejected: {error}");
                self.cancel_dust_mask();
                return;
            }
        };
        let previous = target.and_then(|id| {
            editor
                .dust_repairs(source)?
                .repairs()
                .iter()
                .find(|r| r.id == id)
                .cloned()
        });
        if let Some(previous) = &previous
            && previous.support == support
        {
            self.cancel_dust_mask();
            return;
        }
        if target.is_some() && previous.is_none() {
            self.cancel_dust_mask();
            return;
        }
        self.cancel_dust_mask();
        editor.status = "Finding a donor…".into();
        self.find_dust_donor(editor, source, previous, support, cx);
        editor.invalidate_settled();
    }

    fn dust_source_position(
        &self,
        editor: &Editor,
        geometry: crate::viewport::CanvasLayout,
        point: egui::Pos2,
    ) -> Option<[f64; 2]> {
        if !geometry.image.contains(point) {
            return None;
        }
        let source = geometry
            .composition
            .frame_to_source(geometry.frame_point(point))
            .0;
        let active = editor.document.visit.canvas.ready.as_ref()?.area;
        (source[0] >= f64::from(active.x)
            && source[1] >= f64::from(active.y)
            && source[0] < f64::from(active.x + active.width)
            && source[1] < f64::from(active.y + active.height))
        .then_some(source)
    }

    pub(super) fn begin_dust_stroke(&mut self) {
        if let Some(mut draft) = self.mask_mut() {
            draft.clear_stroke();
        }
    }

    pub(super) fn paint_dust_stroke(
        &mut self,
        editor: &mut Editor,
        geometry: crate::viewport::CanvasLayout,
        point: egui::Pos2,
        temporary_erase: bool,
    ) {
        let Some(position) = self.dust_source_position(editor, geometry, point) else {
            self.end_dust_stroke();
            return;
        };
        let Some(source) = editor.document.visit.current_source else {
            return;
        };
        if self.mask().is_some_and(|draft| draft.rejected) {
            return;
        }
        let Some(target) = self.mask().map(|draft| draft.target) else {
            return;
        };
        let blocked = editor
            .dust_repairs(source)
            .into_iter()
            .flat_map(RepairSet::repairs)
            .filter(|repair| Some(repair.id) != target)
            .map(|repair| repair.support.clone())
            .chain(editor.dust_pending_supports(source))
            .collect::<Vec<_>>();
        let active = editor.document.visit.canvas.ready.as_ref().unwrap().area;
        let erase = temporary_erase;
        let brush_size = editor.document.visit.dust.brush_size;
        let result = brush_stroke(
            &mut self.mask_mut().unwrap(),
            position,
            brush_size,
            erase,
            active,
            &blocked,
        );
        match result {
            Ok((_, true)) => editor.status = "Dust masks cannot overlap another repair".into(),
            Ok(_) => {}
            Err(_) => {
                self.mask_mut().unwrap().rejected = true;
                editor.status = "Dust stroke too large; release to discard it".into();
            }
        }
    }

    pub(super) fn end_dust_stroke(&mut self) {
        if let Some(mut draft) = self.mask_mut() {
            draft.clear_stroke();
        }
    }

    pub(super) fn dust_canvas_cursor(
        &self,
        editor: &Editor,
        geometry: crate::viewport::CanvasLayout,
        point: Option<egui::Pos2>,
        modifying: bool,
    ) -> egui::CursorIcon {
        if self.dust_donor_dragging() {
            egui::CursorIcon::Grabbing
        } else if modifying || self.mask().is_some() {
            egui::CursorIcon::Crosshair
        } else if point.is_some_and(|p| self.dust_repair_at(editor, geometry, p).is_some()) {
            egui::CursorIcon::PointingHand
        } else if point.is_some_and(|p| self.dust_donor_at(editor, geometry, p)) {
            egui::CursorIcon::Grab
        } else {
            egui::CursorIcon::Crosshair
        }
    }

    pub(super) fn dust_repair_at(
        &self,
        editor: &Editor,
        geometry: crate::viewport::CanvasLayout,
        point: egui::Pos2,
    ) -> Option<RepairId> {
        if editor.busy
            || editor.tab != 1
            || editor.document.source
            || !self.interaction.edit.borrow().show_masks
            || editor.active_pick().is_some()
            || self.donor().is_some()
        {
            return None;
        }
        let source_point = self.dust_source_position(editor, geometry, point)?;
        let pixel = [
            source_point[0].floor() as u32,
            source_point[1].floor() as u32,
        ];
        let source = editor.document.visit.current_source?;
        let set = editor.dust_repairs(source)?;
        let selected = editor.selected_dust_repair();
        set.repairs()
            .iter()
            .filter(|repair| Some(repair.id) == selected)
            .chain(
                set.repairs()
                    .iter()
                    .rev()
                    .filter(|repair| Some(repair.id) != selected),
            )
            .find(|repair| {
                repair.support.contains(pixel)
                    || repair_outline_hit(geometry, &repair.support, point)
            })
            .map(|repair| repair.id)
    }

    pub(super) fn select_dust_repair(&mut self, editor: &mut Editor, id: RepairId) {
        if let Some(source) = editor.document.visit.current_source
            && editor
                .dust_repairs(source)
                .is_some_and(|set| set.repairs().iter().any(|repair| repair.id == id))
        {
            self.interaction
                .searches
                .borrow_mut()
                .manual_selection(source);
            editor.document.dust.selected.insert(source, id);
        }
    }

    pub(super) fn delete_selected_dust_repair(&mut self, editor: &mut Editor) {
        let Some(source) = editor.document.visit.current_source else {
            return;
        };
        let Some(id) = editor.selected_dust_repair() else {
            return;
        };
        self.cancel_dust_mask();
        let before = editor.history_snapshot();
        if editor.dust_repairs_mut(source).remove(id).is_ok() {
            editor.record_edit(before, "Delete Dust repair", Some(source));
            editor.document.dust.removed(source, id);
            self.interaction
                .searches
                .borrow_mut()
                .cancel_target(source, id);
            editor.document.visit.dust.removed(source, id);
            editor.status = "Dust repair deleted".into();
            editor.invalidate_settled();
        }
    }

    pub(super) fn dust_donor_regions(&self, editor: &Editor) -> Vec<DonorRegion> {
        self.dust_donor_regions_inner(editor).unwrap_or_default()
    }

    fn dust_donor_regions_inner(&self, editor: &Editor) -> Option<Vec<DonorRegion>> {
        let source = editor.document.visit.current_source?;
        let repair_id = self
            .donor()
            .map(|draft| draft.repair)
            .or_else(|| self.mask().and_then(|draft| draft.target))
            .or_else(|| {
                self.mask()
                    .is_none()
                    .then(|| editor.selected_dust_repair())
                    .flatten()
            })?;
        if editor.dust_target_pending(source, repair_id) {
            return None;
        }
        let effective = editor.effective_dust_repair(source, repair_id)?;
        let active = editor.document.visit.canvas.ready.as_ref()?.area;
        let target = donor_target(&effective.support, active).ok()?;
        let donor = SourceRegion {
            x: u32::try_from(i64::from(target.x) + i64::from(effective.offset[0])).ok()?,
            y: u32::try_from(i64::from(target.y) + i64::from(effective.offset[1])).ok()?,
            width: target.width,
            height: target.height,
        };
        Some(vec![DonorRegion {
            support: effective.support,
            target,
            donor,
        }])
    }

    pub(super) fn dust_donor_at(
        &self,
        editor: &Editor,
        geometry: crate::viewport::CanvasLayout,
        point: egui::Pos2,
    ) -> bool {
        self.dust_donor_hit(editor, geometry, point).is_some()
    }

    pub(super) fn dust_donor_hit(
        &self,
        editor: &Editor,
        geometry: crate::viewport::CanvasLayout,
        point: egui::Pos2,
    ) -> Option<DonorRegion> {
        if !self.interaction.edit.borrow().show_masks {
            return None;
        }
        let position = self.dust_source_position(editor, geometry, point)?;
        self.dust_donor_regions(editor).into_iter().find(|region| {
            let offset = [
                f64::from(region.donor.x) - f64::from(region.target.x),
                f64::from(region.donor.y) - f64::from(region.target.y),
            ];
            let target = [position[0] - offset[0], position[1] - offset[1]];
            let inside = target[0] >= 0.
                && target[1] >= 0.
                && region
                    .support
                    .contains([target[0].floor() as u32, target[1].floor() as u32]);
            let anchor = support_anchor(&region.support);
            inside
                || screen_source_point(geometry, [anchor[0] + offset[0], anchor[1] + offset[1]])
                    .distance(point)
                    <= 10.
        })
    }

    pub(super) fn begin_dust_donor_drag(
        &mut self,
        editor: &mut Editor,
        geometry: crate::viewport::CanvasLayout,
        point: egui::Pos2,
    ) {
        let Some(region) = self.dust_donor_hit(editor, geometry, point) else {
            return;
        };
        let Some(source_point) = self.dust_source_position(editor, geometry, point) else {
            return;
        };
        if self.donor().is_none() {
            let Some(repair) = editor.selected_dust_repair() else {
                return;
            };
            let (target, donor) = (region.target, region.donor);
            let (Ok(offset_x), Ok(offset_y)) = (
                i32::try_from(i64::from(donor.x) - i64::from(target.x)),
                i32::try_from(i64::from(donor.y) - i64::from(target.y)),
            ) else {
                return;
            };
            let offset = [offset_x, offset_y];
            self.set_donor(Some(DonorDraft {
                repair,
                offset,
                drag: None,
            }));
        }
        if let Some(mut draft) = self.donor_mut() {
            draft.drag = Some(DonorDrag {
                source: source_point,
                offset: draft.offset,
            });
        }
    }

    pub(super) fn drag_dust_donor(
        &mut self,
        editor: &mut Editor,
        geometry: crate::viewport::CanvasLayout,
        point: egui::Pos2,
    ) {
        let Some(position) = self.dust_source_position(editor, geometry, point) else {
            return;
        };
        let Some(active) = editor
            .document
            .visit
            .canvas
            .ready
            .as_ref()
            .map(|ready| ready.area)
        else {
            return;
        };
        let Some(support) = editor
            .document
            .visit
            .current_source
            .and_then(|source| {
                editor
                    .selected_dust_repair()
                    .and_then(|id| editor.effective_dust_repair(source, id))
            })
            .map(|r| r.support)
        else {
            return;
        };
        let Ok(target) = donor_target(&support, active) else {
            return;
        };
        let Some(mut draft) = self.donor_mut() else {
            return;
        };
        let Some(drag) = draft.drag else {
            return;
        };
        let requested = [
            i64::from(drag.offset[0]) + (position[0] - drag.source[0]).round() as i64,
            i64::from(drag.offset[1]) + (position[1] - drag.source[1]).round() as i64,
        ];
        let minimum = [
            i64::from(active.x) - i64::from(target.x),
            i64::from(active.y) - i64::from(target.y),
        ];
        let maximum = [
            i64::from(active.x + active.width) - i64::from(target.x + target.width),
            i64::from(active.y + active.height) - i64::from(target.y + target.height),
        ];
        let offset = [
            requested[0].clamp(minimum[0], maximum[0]) as i32,
            requested[1].clamp(minimum[1], maximum[1]) as i32,
        ];
        if draft.offset != offset {
            draft.offset = offset;
            editor.invalidate_settled();
        }
    }

    pub(super) fn dust_donor_dragging(&self) -> bool {
        self.donor().is_some_and(|draft| draft.drag.is_some())
    }

    pub(super) fn end_dust_donor_drag(&mut self, editor: &mut Editor) {
        let Some(draft) = self.take_donor() else {
            return;
        };
        if draft.drag.is_none() {
            return;
        }
        let Some(source) = editor.document.visit.current_source else {
            return;
        };
        let Some((support, accepted_offset)) = editor
            .dust_repairs(source)
            .and_then(|set| {
                set.repairs()
                    .iter()
                    .find(|repair| repair.id == draft.repair)
            })
            .map(|repair| (repair.support.clone(), repair.offset))
        else {
            return;
        };
        if draft.offset == accepted_offset {
            return;
        }
        let before = editor.history_snapshot();
        let result = editor
            .document
            .visit
            .canvas
            .ready
            .as_ref()
            .map(|ready| ready.area)
            .ok_or(Error::InvalidParameter("manual donor unavailable"))
            .and_then(|active| validate_dust_offset(draft.offset, &support, active))
            .and_then(|()| {
                editor
                    .dust_repairs_mut(source)
                    .set_offset(draft.repair, draft.offset)
            });
        match result {
            Ok(()) => {
                editor.dust_repair_changed(source, draft.repair);
                editor.record_edit(before, "Dust donor", Some(source));
                editor.status = "Dust source updated".into();
            }
            Err(error) => {
                editor.status = format!("Dust source rejected: {error}");
            }
        }
        editor.invalidate_settled();
    }

    pub(super) fn paint_dust(
        &self,
        editor: &Editor,
        ui: &egui::Ui,
        geometry: crate::viewport::CanvasLayout,
    ) {
        let Some(source) = editor.document.visit.current_source else {
            return;
        };
        if editor.tab != 1 || editor.fullscreen.is_some() || editor.canvas_inspection() {
            return;
        }
        let painter = ui.painter().with_clip_rect(geometry.canvas);
        let overlays_hidden = ui.input(|input| input.key_down(egui::Key::H));
        let show_masks = self.interaction.edit.borrow().show_masks;
        if show_masks && !overlays_hidden {
            self.paint_repair_outlines(editor, source, ui, geometry, &painter);
        }
        if !overlays_hidden {
            self.paint_mask_outline(ui, geometry, &painter);
        }
        self.paint_brush_cursor(editor, ui, geometry, &painter);
        if show_masks && !overlays_hidden {
            self.paint_donor_guides(editor, geometry, &painter);
        }
    }

    fn paint_repair_outlines(
        &self,
        editor: &Editor,
        source: SourceId,
        ui: &egui::Ui,
        geometry: crate::viewport::CanvasLayout,
        painter: &egui::Painter,
    ) {
        let set = editor.dust_repairs(source);
        let selected = editor.selected_dust_repair();
        let signature = editor.dust_signature();
        let mask_target = self.mask().and_then(|draft| draft.target);
        for repair in set.into_iter().flat_map(RepairSet::repairs) {
            if mask_target == Some(repair.id) || editor.dust_target_pending(source, repair.id) {
                continue;
            }
            let pending = editor
                .document
                .visit
                .dust
                .pending
                .as_ref()
                .is_some_and(|pending| {
                    pending.evaluation.loaded == editor.id
                        && pending.evaluation.source == source
                        && Some(pending.evaluation.signature) == signature
                        && pending.evaluation.repair == *repair
                });
            let failed = editor
                .document
                .dust
                .failed
                .get(&(source, repair.id))
                .is_some_and(|failed| {
                    failed.loaded == editor.id
                        && Some(failed.signature) == signature
                        && failed.repair == *repair
                });
            let active = selected == Some(repair.id);
            let color = if mask_target.is_some() {
                egui::Color32::from_white_alpha(45)
            } else if !repair.enabled {
                egui::Color32::from_white_alpha(65)
            } else if failed {
                ui.visuals().error_fg_color.linear_multiply(0.7)
            } else if pending {
                egui::Color32::from_rgb(235, 180, 80).linear_multiply(0.7)
            } else if active {
                egui::Color32::from_rgb(100, 220, 240)
            } else {
                egui::Color32::from_white_alpha(85)
            };
            paint_support_outline(
                painter,
                geometry,
                &repair.support,
                [0., 0.],
                color,
                if active { 1.5 } else { 1. },
            );
        }
        for support in editor.dust_pending_supports(source) {
            paint_support_outline(
                painter,
                geometry,
                &support,
                [0., 0.],
                ui.visuals().weak_text_color(),
                1.5,
            );
        }
    }

    fn paint_mask_outline(
        &self,
        ui: &egui::Ui,
        geometry: crate::viewport::CanvasLayout,
        painter: &egui::Painter,
    ) {
        if let Some(mask) = self.mask().and_then(|draft| draft.support().ok()) {
            paint_support_outline(
                painter,
                geometry,
                &mask,
                [0., 0.],
                if self.mask().is_some_and(|draft| draft.rejected) {
                    ui.visuals().error_fg_color
                } else {
                    egui::Color32::from_rgb(100, 220, 240)
                },
                1.,
            );
        }
    }

    fn paint_brush_cursor(
        &self,
        editor: &Editor,
        ui: &egui::Ui,
        geometry: crate::viewport::CanvasLayout,
        painter: &egui::Painter,
    ) {
        let (point, add, erase, hide_cursor) = ui.ctx().input(|input| {
            (
                input.pointer.hover_pos(),
                input.modifiers.shift,
                input.modifiers.alt,
                input.key_down(egui::Key::H) || input.key_down(egui::Key::Space),
            )
        });
        if !editor.document.source
            && !self.dust_donor_dragging()
            && !hide_cursor
            && let Some(point) = point
            && self.dust_canvas_cursor(editor, geometry, Some(point), add || erase)
                == egui::CursorIcon::Crosshair
            && let Some(source_point) = self.dust_source_position(editor, geometry, point)
        {
            let center = screen_source_point(geometry, source_point);
            let radius =
                (editor.document.visit.dust.brush_size as f32 * 0.5 * geometry.scale).max(2.);
            painter.circle_stroke(
                center,
                radius + 0.5,
                egui::Stroke::new(2., egui::Color32::from_black_alpha(170)),
            );
            painter.circle_stroke(
                center,
                radius,
                egui::Stroke::new(
                    1.5,
                    if self.mask().is_some_and(|draft| draft.rejected) {
                        ui.visuals().error_fg_color
                    } else if erase {
                        egui::Color32::from_rgb(240, 120, 100)
                    } else if add {
                        egui::Color32::from_rgb(100, 220, 240)
                    } else {
                        egui::Color32::WHITE
                    },
                ),
            );
        }
    }

    fn paint_donor_guides(
        &self,
        editor: &Editor,
        geometry: crate::viewport::CanvasLayout,
        painter: &egui::Painter,
    ) {
        for region in self.dust_donor_regions(editor) {
            let (target, donor, support) = (region.target, region.donor, region.support);
            let offset = [
                f64::from(donor.x) - f64::from(target.x),
                f64::from(donor.y) - f64::from(target.y),
            ];
            paint_support_outline(
                painter,
                geometry,
                &support,
                [0., 0.],
                egui::Color32::WHITE,
                2.,
            );
            paint_support_outline(
                painter,
                geometry,
                &support,
                offset,
                egui::Color32::from_rgb(100, 220, 240),
                2.,
            );
            let target_anchor = support_anchor(&support);
            let donor_anchor = [target_anchor[0] + offset[0], target_anchor[1] + offset[1]];
            painter.line_segment(
                [
                    screen_source_point(geometry, target_anchor),
                    screen_source_point(geometry, donor_anchor),
                ],
                egui::Stroke::new(1., egui::Color32::from_gray(220)),
            );
        }
    }
}
