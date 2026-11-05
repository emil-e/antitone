use super::*;

pub(in crate::editor) struct CreationTool {
    edit: Rc<RefCell<MaskEdit>>,
    reveal: bool,
}

impl CreationTool {
    pub(super) fn new(edit: Rc<RefCell<MaskEdit>>) -> Self {
        Self { edit, reveal: true }
    }

    fn identity(&self) -> presentation::ToolIdentity<'static> {
        match self.edit.borrow().kind {
            Kind::Linear => presentation::ToolIdentity {
                icon: icons::Icon::LinearGradient,
                title: "New linear gradient",
            },
            Kind::Radial => presentation::ToolIdentity {
                icon: icons::Icon::RadialGradient,
                title: "New radial gradient",
            },
        }
    }

    fn guidance(&self) -> &'static str {
        match self.edit.borrow().kind {
            Kind::Linear => {
                "Draw from full effect toward no effect. Release creates and selects the layer."
            }
            Kind::Radial => "Draw outward from the centre. Release creates and selects the layer.",
        }
    }

    fn finish(&mut self, editor: &mut Editor) {
        let (current, candidate) = {
            let edit = self.edit.borrow();
            (editor.mask_is_current(&edit), edit.candidate())
        };
        if !current {
            editor.status = "Mask gesture discarded: source or settings changed".into();
            editor.end_tool();
            return;
        }
        let result = candidate.and_then(|candidate| editor.accept_mask(candidate));
        match result {
            Ok(id) => {
                editor.select_adjustment_layer(adjustment_layers::Target::Layer(id));
                editor.document.visit.adjustment_layers.reveal_controls = true;
                editor.status = format!("{} selected", self.edit.borrow().kind.title());
                editor.end_tool();
            }
            Err(error) => {
                editor.status = format!("Mask rejected: {error}");
                let mut edit = self.edit.borrow_mut();
                edit.mask = None;
                edit.drag = None;
                edit.error = Some(error.to_string());
            }
        }
    }
}

impl tool::Tool for CreationTool {
    fn presentation(&self, editor: &Editor) -> tool::Presentation {
        tool::Presentation {
            identity: self.identity(),
            primary: None,
            dismiss: "Cancel",
            warning: self.edit.borrow().error.clone(),
            help: if editor.tab == 1 {
                "Draw a gradient to create a layer. Release accepts; Escape cancels.\nSpace-drag / middle-drag pans before drawing."
            } else {
                "Return to Photo to draw the gradient; Escape cancels creation."
            },
            navigation_help: None,
            fit_label: "Fit",
        }
    }

    fn drawer(&mut self, ui: &mut egui::Ui, _editor: &mut Editor, _cx: &UiContext<'_, Editor>) {
        if std::mem::take(&mut self.reveal) {
            ui.scroll_to_cursor(Some(egui::Align::Min));
        }
        self.identity().inspector(
            ui,
            |_| (),
            |ui, ()| {
                ui.small(self.guidance());
                if let Some(error) = self.edit.borrow().error.as_deref() {
                    ui.colored_label(
                        ui.visuals().error_fg_color,
                        format!("{error}. Draw again or cancel."),
                    );
                }
            },
        );
    }

    fn canvas(
        &mut self,
        viewer: &mut canvas::Viewer<'_>,
        editor: &mut Editor,
        cx: &UiContext<'_, Editor>,
    ) -> Option<CanvasLayout> {
        viewer.show(
            self,
            editor,
            cx,
            Editor::photograph_controls,
            |tool, surface, editor, cx| {
                if editor.fullscreen.is_some() {
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
                let rect = canvas.response.rect;
                let composition = canvas.composition;
                let ppp = canvas.ui.ctx().pixels_per_point();
                let relayout =
                    |viewport: &mut Viewport| viewport.layout_composition(rect, composition, ppp);
                let layout = canvas.layout(&mut editor.active_canvas_mut().viewport, relayout);
                let (layout, panning) = if captured {
                    (layout, false)
                } else {
                    let mut response = canvas.response.clone();
                    if active {
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
                if captured && ((!allowed && released) || (!down && !released)) {
                    editor.status = "Mask gesture cancelled".into();
                    editor.end_tool();
                } else if allowed && !panning {
                    if pressed
                        && !space
                        && canvas.response.is_pointer_button_down_on()
                        && let Some(origin) = origin.filter(|point| layout.canvas.contains(*point))
                    {
                        let mut edit = tool.edit.borrow_mut();
                        edit.begin_drag(layout, origin);
                        edit.error = None;
                    }
                    {
                        let mut edit = tool.edit.borrow_mut();
                        if let (Some(drag), Some(point)) = (&edit.drag, point) {
                            edit.mask = Some(drag.update(point));
                            canvas.ui.ctx().request_repaint();
                        }
                    }
                    if released && tool.edit.borrow().dragging() {
                        tool.finish(editor);
                    }
                }
                canvas.paint_image(editor, layout, false, cx);
                if active {
                    let edit = tool.edit.borrow();
                    if let Some(mask) = edit.mask {
                        paint_geometry(
                            canvas.ui,
                            layout,
                            mask,
                            false,
                            editor.document.visit.adjustment_layers.show_coverage,
                            edit.dragging(),
                        );
                    }
                    if allowed && !panning && !space && canvas.response.hovered() {
                        canvas.ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
                    }
                }
                Some(layout)
            },
        )
    }

    fn action(&mut self, action: tool::Action, editor: &mut Editor, _cx: &UiContext<'_, Editor>) {
        if matches!(action, tool::Action::Dismiss) {
            editor.status = "Mask creation cancelled".into();
            editor.end_tool();
        }
    }
}
