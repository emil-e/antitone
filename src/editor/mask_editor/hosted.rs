use super::*;

impl MaskTool {
    pub(super) fn new(edit: Rc<RefCell<MaskEdit>>) -> Self {
        Self { edit }
    }

    fn identity(&self) -> ui::ToolIdentity<'static> {
        let edit = self.edit.borrow();
        ui::ToolIdentity {
            icon: match edit.kind {
                Kind::Linear => ui::icons::Icon::LinearGradient,
                Kind::Radial => ui::icons::Icon::RadialGradient,
            },
            title: match (edit.kind, edit.creating()) {
                (Kind::Linear, true) => "New linear gradient",
                (Kind::Radial, true) => "New radial gradient",
                (Kind::Linear, false) => "Linear gradient",
                (Kind::Radial, false) => "Radial gradient",
            },
        }
    }

    fn refresh(&mut self, editor: &mut Editor) -> bool {
        let edit = self.edit.borrow();
        let Some(id) = edit.target else {
            return true;
        };
        let current = editor.selected_mask().is_some_and(|layer| {
            editor.adjustment_layer_target() == adjustment_layers::Target::Layer(id)
                && matches!(
                    (edit.kind, layer.mask),
                    (Kind::Linear, AdjustmentLayerCoverage::Linear { .. })
                        | (Kind::Radial, AdjustmentLayerCoverage::Radial { .. })
                )
        });
        if !current {
            drop(edit);
            editor.end_tool();
        }
        current
    }

    pub(super) fn finish(&mut self, editor: &mut Editor) {
        let result = editor.accept_mask(&self.edit.borrow());
        match result {
            Ok(id) => {
                {
                    let mut edit = self.edit.borrow_mut();
                    edit.target = Some(id);
                    edit.cancel_gesture();
                }
                editor.select_adjustment_layer(adjustment_layers::Target::Layer(id));
                editor.status = format!("{} selected", self.edit.borrow().kind.title());
            }
            Err(error) => {
                editor.status = format!("Mask rejected: {error}");
                let mut edit = self.edit.borrow_mut();
                edit.cancel_gesture();
                edit.error = Some(error.to_string());
            }
        }
        self.refresh(editor);
    }
}

impl tool::Tool for MaskTool {
    fn presentation(&self, editor: &Editor) -> tool::Presentation {
        let edit = self.edit.borrow();
        tool::Presentation {
            identity: self.identity(),
            primary: None,
            dismiss: if edit.creating() { "Cancel" } else { "Done" },
            warning: edit.error.clone(),
            help: if editor.tab != 1 {
                "Return to Photo to edit the gradient. Grading remains available on each stage."
            } else if edit.creating() {
                "Draw a gradient; release creates the layer and continues editing.\nSpace-drag / middle-drag pans before drawing."
            } else {
                edit.kind.help()
            },
            navigation_help: None,
            fit_label: "Fit",
        }
    }

    fn drawer(&mut self, ui: &mut egui::Ui, editor: &mut Editor, _cx: &UiContext<'_, Editor>) {
        if !self.refresh(editor) {
            return;
        }
        let edit = self.edit.borrow();
        if edit.creating() {
            ui.small(if editor.tab != 1 {
                "Return to Photo to draw the gradient."
            } else {
                match edit.kind {
                    Kind::Linear => "Drag to draw a linear gradient.",
                    Kind::Radial => "Drag to draw a radial gradient.",
                }
            });
        }
        if let Some(error) = &edit.error {
            ui.colored_label(
                ui.visuals().error_fg_color,
                format!("Mask not accepted: {error}"),
            );
        }
    }

    fn canvas(
        &mut self,
        viewer: &mut canvas::Viewer<'_>,
        editor: &mut Editor,
        cx: &UiContext<'_, Editor>,
    ) -> Option<CanvasLayout> {
        let current = self.refresh(editor);
        viewer.show(
            self,
            editor,
            cx,
            Editor::photograph_controls,
            |tool, surface, editor, cx| {
                if !current {
                    tool.edit.borrow_mut().cancel_gesture();
                    return surface.photograph(editor, cx);
                }
                let mut canvas = surface.selected()?;
                let (pressed, released, down, space, origin, point) = canvas.ui.input(|input| {
                    (
                        input.pointer.button_pressed(egui::PointerButton::Primary),
                        input.pointer.button_released(egui::PointerButton::Primary),
                        input.pointer.button_down(egui::PointerButton::Primary),
                        input.key_down(egui::Key::Space),
                        input.pointer.press_origin(),
                        input.pointer.interact_pos(),
                    )
                });
                let active = editor.tab == 1 && !editor.document.source;
                let allowed = active && canvas.ui.is_enabled() && !editor.interaction_blocked();
                let captured = tool.edit.borrow().dragging();
                if captured && (!allowed || !down && !released) {
                    tool.edit.borrow_mut().cancel_gesture();
                    editor.status = "Mask gesture cancelled".into();
                }
                let captured = tool.edit.borrow().dragging();
                let rect = canvas.response.rect;
                let composition = canvas.composition;
                let ppp = canvas.ui.ctx().pixels_per_point();
                let relayout =
                    |viewport: &mut Viewport| viewport.layout_composition(rect, composition, ppp);
                let layout = canvas.layout(&mut editor.active_canvas_mut().viewport, relayout);
                let interested = allowed && {
                    let edit = tool.edit.borrow();
                    captured
                        || edit.creating()
                        || origin.filter(|_| down).or(point).is_some_and(|point| {
                            editor
                                .selected_mask()
                                .is_some_and(|layer| hits(layer.mask, layout, point))
                        })
                };
                let (layout, panning) = {
                    let mut response = canvas.response.clone();
                    if interested {
                        navigation::consume_primary(&mut response);
                    }
                    navigation::navigate_view(
                        &mut editor.active_canvas_mut().viewport,
                        canvas.ui,
                        &response,
                        layout,
                        relayout,
                    )
                };
                if allowed && !panning {
                    if pressed
                        && !space
                        && interested
                        && canvas.response.is_pointer_button_down_on()
                        && let Some(origin) = origin.filter(|point| layout.canvas.contains(*point))
                    {
                        let mut edit = tool.edit.borrow_mut();
                        edit.begin_drag(
                            layout,
                            origin,
                            editor.selected_mask().map(|layer| layer.mask),
                        );
                        edit.error = None;
                    }
                    if let Some(point) = point {
                        let mut edit = tool.edit.borrow_mut();
                        if edit.dragging() {
                            edit.update(layout, point);
                            canvas.ui.ctx().request_repaint();
                        }
                    }
                    if released && tool.edit.borrow().dragging() {
                        tool.finish(editor);
                    }
                }
                canvas.paint_image(editor, layout, false, cx);
                if active {
                    tool.edit.borrow().paint(
                        canvas.ui,
                        layout,
                        editor.selected_mask(),
                        editor.document.visit.adjustment_layers.show_coverage,
                    );
                    if interested && !panning && !space && canvas.response.hovered() {
                        canvas.ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
                    }
                }
                Some(layout)
            },
        )
    }

    fn shortcuts(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        if self.edit.borrow().dragging()
            && !self.edit.borrow().creating()
            && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.edit.borrow_mut().cancel_gesture();
            editor.status = "Mask gesture cancelled".into();
            ui.ctx().request_repaint();
        }
    }

    fn action(&mut self, action: tool::Action, editor: &mut Editor, _cx: &UiContext<'_, Editor>) {
        if matches!(action, tool::Action::Dismiss) {
            editor.status = if self.edit.borrow().creating() {
                "Mask creation cancelled".into()
            } else {
                "Mask editing finished".into()
            };
            editor.end_tool();
        }
    }
}
