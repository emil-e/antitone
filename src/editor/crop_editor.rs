use super::*;
use crate::{geometry::FramePoint, viewport::CanvasLayout};
use std::{cell::RefCell, rc::Rc};

#[cfg(test)]
pub(super) mod tests;

#[derive(Default)]
pub(super) struct CropTool {
    edit: Option<Rc<RefCell<CropEdit>>>,
    viewport: Viewport,
    auto_crop_task: Option<async_executor::Task<()>>,
}

impl CropTool {
    fn request_auto_crop(
        &mut self,
        editor: &mut Editor,
        photo: SourceId,
        cx: &UiContext<'_, Editor>,
    ) {
        let Some(state) = self.edit.clone() else {
            return;
        };
        if let Some(task) = auto_crop::propose_photo_crop(editor, photo, state, cx) {
            self.auto_crop_task = Some(task);
        }
    }
}

pub(super) struct CropEdit {
    pub draft: Crop,
    base: Composition,
    ratio: Option<f64>,
    drag: Option<CropDrag>,
    pub(super) layout: Option<CanvasLayout>,
    pub(super) guides: bool,
    pub(super) auto_problem: Option<String>,
    refit: bool,
    reveal_controls: bool,
}

#[derive(Clone, Copy)]
enum CropDrag {
    Move {
        start: egui::Pos2,
        crop: Crop,
        layout: CanvasLayout,
    },
    Resize {
        handle: [i8; 2],
        layout: CanvasLayout,
    },
    Straighten {
        start_angle: f64,
        crop: Crop,
        layout: CanvasLayout,
    },
}

fn handles(image: egui::Rect) -> [(egui::Pos2, [i8; 2]); 8] {
    [
        (image.min, [-1, -1]),
        (image.right_top(), [1, -1]),
        (image.max, [1, 1]),
        (image.left_bottom(), [-1, 1]),
        (image.left_center(), [-1, 0]),
        (image.right_center(), [1, 0]),
        (image.center_top(), [0, -1]),
        (image.center_bottom(), [0, 1]),
    ]
}

fn handle_at(image: egui::Rect, point: egui::Pos2) -> Option<[i8; 2]> {
    handles(image)
        .into_iter()
        .find(|(position, _)| position.distance(point) <= 9.)
        .map(|(_, handle)| handle)
}

fn aspect_label(ratio: Option<f64>) -> String {
    let Some(ratio) = ratio else {
        return "Free".into();
    };
    let landscape = if ratio < 1. { 1. / ratio } else { ratio };
    for (value, [w, h]) in [
        (1., [1, 1]),
        (1.5, [3, 2]),
        (4. / 3., [4, 3]),
        (1.25, [5, 4]),
        (16. / 9., [16, 9]),
    ] {
        if (landscape - value).abs() < 1e-9 {
            return if ratio < 1. {
                format!("{h}:{w}")
            } else {
                format!("{w}:{h}")
            };
        }
    }
    format!("{ratio:.3}:1")
}

fn reframe_crop(
    viewport: &mut Viewport,
    previous: CanvasLayout,
    next: Composition,
    pixels_per_point: f32,
    keep_frame_center: bool,
) {
    let mut layout = previous;
    if keep_frame_center {
        let size = next.size().map(|v| v as f32);
        layout.image = egui::Rect::from_center_size(
            previous.image.center(),
            egui::vec2(size[0], size[1]) * previous.scale,
        );
        layout.composition = next;
        layout.area = next.region();
    }
    viewport.reframe(layout, next, pixels_per_point);
}

impl Editor {
    pub(super) fn begin_crop(&mut self) {
        if self.document.visit.canvas.ready.is_some()
            && self.activate_tool(tool::Placement::Photo, Box::<CropTool>::default())
        {
            self.status = "Crop draft; changes are not applied".into();
        }
    }
}

impl CropEdit {
    pub(super) fn install_proposal(&mut self, crop: Crop) {
        self.draft = crop;
        self.ratio = None;
        self.layout = None;
        self.refit = true;
    }
}

impl CropTool {
    fn controls(&mut self, ui: &mut egui::Ui, reset: bool, status: &mut String) {
        use icons::Icon;
        let Some(state) = &self.edit else {
            return;
        };
        let mut edit = state.borrow_mut();
        let viewport = &mut self.viewport;
        let area = edit.base.area();
        let full = edit.base.cropped(Crop::from_region(area)).unwrap();
        let composition = edit.base.cropped(edit.draft).unwrap();
        let previous = edit.draft;
        let size = composition.size();
        let previous_ratio = edit.ratio;
        let mut degrees = -edit.draft.angle.to_degrees() * composition.handedness();
        let previous_degrees = degrees;
        let mut reset_angle = false;
        icons::row_at(ui, presentation::INSPECTOR_ROW_HEIGHT, |ui| {
            let label = presentation::name_label(ui, "Aspect");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icons::button_at(
                    ui,
                    Icon::Swap,
                    edit.ratio.is_some(),
                    "Swap portrait / landscape",
                    "Swap portrait / landscape",
                    presentation::INSPECTOR_ROW_HEIGHT,
                )
                .clicked()
                {
                    edit.ratio = edit.ratio.map(|r| 1. / r);
                }
                egui::ComboBox::from_id_salt("crop-aspect")
                    .width(96.)
                    .selected_text(aspect_label(edit.ratio))
                    .show_ui(ui, |ui| {
                        for (label, ratio) in [
                            ("Free", None),
                            ("Current", Some(f64::from(size[0]) / f64::from(size[1]))),
                            (
                                "Original",
                                Some(f64::from(full.size()[0]) / f64::from(full.size()[1])),
                            ),
                            ("1:1", Some(1.)),
                            ("3:2", Some(1.5)),
                            ("4:3", Some(4. / 3.)),
                            ("5:4", Some(1.25)),
                            ("16:9", Some(16. / 9.)),
                        ] {
                            ui.selectable_value(&mut edit.ratio, ratio, label);
                        }
                    });
            });
            label
        });
        ui.add_space(presentation::PARAMETER_GAP);
        icons::row_at(ui, presentation::INSPECTOR_ROW_HEIGHT, |ui| {
            let angle_label = presentation::name_label(ui, "Angle");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                reset_angle = icons::button_at(
                    ui,
                    Icon::Reset,
                    degrees != 0.,
                    "Reset straighten",
                    "Reset straighten",
                    presentation::INSPECTOR_ROW_HEIGHT,
                )
                .clicked();
                let angle = ui
                    .add_sized(
                        [96., presentation::INSPECTOR_ROW_HEIGHT - 2.],
                        egui::DragValue::new(&mut degrees)
                            .range(-180.0..=180.0)
                            .speed(0.1)
                            .fixed_decimals(2)
                            .update_while_editing(false)
                            .suffix("°"),
                    )
                    .labelled_by(angle_label.id);
                if controls::numeric_cancelled(ui, &angle) || !ui.is_enabled() {
                    degrees = previous_degrees;
                }
            });
        });
        ui.add_space(presentation::PARAMETER_GAP);
        icons::row_at(ui, presentation::INSPECTOR_ROW_HEIGHT, |ui| {
            presentation::name_label(ui, "Guides");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let size = presentation::INSPECTOR_ROW_HEIGHT;
                icons::toggle_at(
                    ui,
                    Icon::Guides,
                    &mut edit.guides,
                    "Thirds guides",
                    "Show thirds guides. Composition aid only; never affects measurements or exports.",
                    size,
                );
            });
        });
        ui.add_space(presentation::PARAMETER_GAP);
        if edit.ratio != previous_ratio
            && let Some(ratio) = edit.ratio
        {
            let extent = [f64::from(size[0]), f64::from(size[0]) / ratio];
            let min = [
                (f64::from(size[0]) - extent[0]) * 0.5,
                (f64::from(size[1]) - extent[1]) * 0.5,
            ];
            edit.draft = composition.crop_from_frame(
                FramePoint(min),
                FramePoint([min[0] + extent[0], min[1] + extent[1]]),
            );
        }
        if degrees != previous_degrees || reset_angle {
            let angle = if reset_angle {
                0.
            } else {
                -degrees.to_radians() * composition.handedness()
            };
            match edit.draft.straightened(area, angle) {
                Ok(crop) => edit.draft = crop,
                Err(error) => *status = format!("Straighten rejected: {error}"),
            }
        }
        if reset {
            edit.draft = Crop::from_region(area);
            edit.ratio = None;
            edit.drag = None;
            viewport.fit();
        } else if edit.draft != previous {
            edit.drag = None;
            if let Some(layout) = edit.layout {
                let next = edit.base.cropped(edit.draft).unwrap();
                reframe_crop(
                    viewport,
                    layout,
                    next,
                    ui.ctx().pixels_per_point(),
                    edit.draft.angle != previous.angle,
                );
            }
        }
        if edit.draft != previous {
            ui.ctx().request_repaint();
        }
    }

    fn interact(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        mut layout: CanvasLayout,
        status: &mut String,
    ) -> CanvasLayout {
        let Some(state) = &self.edit else {
            return layout;
        };
        let mut edit = state.borrow_mut();
        let viewport = &mut self.viewport;
        // Disabled widgets may not receive drag_stopped; physical release still
        // ends the owned gesture before gating new canvas interaction.
        if response.drag_stopped_by(egui::PointerButton::Primary)
            || !ui.input(|input| input.pointer.primary_down())
        {
            edit.drag = None;
        }
        if !ui.is_enabled() {
            return layout;
        }
        let hover = response.hover_pos();
        let handle = match edit.drag {
            Some(CropDrag::Resize { handle, .. }) => Some(handle),
            Some(CropDrag::Move { .. } | CropDrag::Straighten { .. }) => None,
            None => {
                let point = if response.drag_started_by(egui::PointerButton::Primary) {
                    ui.input(|input| input.pointer.press_origin())
                } else {
                    hover
                };
                point.and_then(|point| handle_at(layout.image, point))
            }
        };
        if response.drag_started_by(egui::PointerButton::Primary)
            && let Some(start) = ui.input(|input| input.pointer.press_origin())
        {
            edit.drag = match handle_at(layout.image, start) {
                Some(handle) => Some(CropDrag::Resize { handle, layout }),
                None if layout.image.contains(start) => Some(CropDrag::Move {
                    start,
                    crop: edit.draft,
                    layout,
                }),
                None => {
                    let delta = start - layout.image.center();
                    Some(CropDrag::Straighten {
                        start_angle: f64::from(delta.y.atan2(delta.x)),
                        crop: edit.draft,
                        layout,
                    })
                }
            };
        }
        if response.dragged_by(egui::PointerButton::Primary)
            && let (Some(drag), Some(point)) = (edit.drag, response.interact_pointer_pos())
        {
            match drag {
                CropDrag::Move {
                    start,
                    crop,
                    layout: original,
                } => {
                    let a = original
                        .composition
                        .frame_to_source(original.frame_point(start));
                    let b = original
                        .composition
                        .frame_to_source(original.frame_point(point));
                    edit.draft = Crop {
                        center: [
                            crop.center[0] - (b.0[0] - a.0[0]),
                            crop.center[1] - (b.0[1] - a.0[1]),
                        ],
                        ..crop
                    }
                    .constrained(original.composition.area())
                    .expect("moving a valid frame retains its coverage basis");
                }
                CropDrag::Resize {
                    handle,
                    layout: original,
                } => {
                    let dimensions = original.composition.size().map(f64::from);
                    let p = original.frame_point(point).0;
                    let mut min = [0., 0.];
                    let mut max = dimensions;
                    for axis in 0..2 {
                        if handle[axis] < 0 {
                            min[axis] = p[axis].min(max[axis] - 1.);
                        }
                        if handle[axis] > 0 {
                            max[axis] = p[axis].max(min[axis] + 1.);
                        }
                    }
                    if let Some(ratio) = edit.ratio {
                        let mut extent = [max[0] - min[0], max[1] - min[1]];
                        if handle[0] == 0
                            || (handle[1] != 0
                                && (extent[1] - dimensions[1]).abs() * ratio
                                    > (extent[0] - dimensions[0]).abs())
                        {
                            extent[0] = extent[1] * ratio;
                        } else {
                            extent[1] = extent[0] / ratio;
                        }
                        for axis in 0..2 {
                            if handle[axis] < 0 {
                                min[axis] = max[axis] - extent[axis];
                            } else if handle[axis] > 0 {
                                max[axis] = min[axis] + extent[axis];
                            } else {
                                min[axis] = (dimensions[axis] - extent[axis]) * 0.5;
                                max[axis] = min[axis] + extent[axis];
                            }
                        }
                    }
                    edit.draft = original
                        .composition
                        .crop_from_frame(FramePoint(min), FramePoint(max));
                    layout.image = original.screen_coverage(edit.draft.coverage().unwrap());
                    layout.scale = original.scale;
                }
                CropDrag::Straighten {
                    start_angle,
                    crop,
                    layout: original,
                } => {
                    let delta = point - original.image.center();
                    let turn = f64::from(delta.y.atan2(delta.x)) - start_angle;
                    let turn = turn.sin().atan2(turn.cos());
                    let angle = (crop.angle - turn * original.composition.handedness())
                        .clamp(-std::f64::consts::PI, std::f64::consts::PI);
                    match crop.straightened(original.composition.area(), angle) {
                        Ok(crop) => {
                            edit.draft = crop;
                            let size = original.composition.size().map(|v| v as f32);
                            let next = edit.base.cropped(crop).unwrap().size().map(|v| v as f32);
                            // Coverage may shrink, but the straighten pivot stays at the frame centre.
                            layout.image = egui::Rect::from_center_size(
                                original.image.center(),
                                original.image.size()
                                    * egui::vec2(next[0] / size[0], next[1] / size[1]),
                            );
                            layout.scale = original.scale;
                        }
                        Err(error) => *status = format!("Straighten rejected: {error}"),
                    }
                }
            }
            layout.composition = edit.base.cropped(edit.draft).unwrap();
            ui.ctx().request_repaint();
            layout.area = layout.composition.region();
        }
        if response.hovered() || response.dragged() {
            // On release, describe the next gesture at the updated frame.
            let handle = if response.drag_stopped_by(egui::PointerButton::Primary) {
                hover.and_then(|point| handle_at(layout.image, point))
            } else {
                handle
            };
            let cursor = match handle {
                Some([x, y]) if x != 0 && y != 0 => {
                    if x * y > 0 {
                        egui::CursorIcon::ResizeNwSe
                    } else {
                        egui::CursorIcon::ResizeNeSw
                    }
                }
                Some([0, _]) => egui::CursorIcon::ResizeVertical,
                Some(_) => egui::CursorIcon::ResizeHorizontal,
                None if matches!(edit.drag, Some(CropDrag::Straighten { .. }))
                    && response.dragged_by(egui::PointerButton::Primary) =>
                {
                    egui::CursorIcon::Crosshair
                }
                None if response.dragged() => egui::CursorIcon::Grabbing,
                None if hover.is_some_and(|point| layout.image.contains(point)) => {
                    egui::CursorIcon::Grab
                }
                None => egui::CursorIcon::Crosshair,
            };
            ui.ctx().set_cursor_icon(cursor);
        }
        // The painted layout is the editing viewport: retain it, not a new Fit of the crop.
        viewport.reframe(layout, layout.composition, ui.ctx().pixels_per_point());
        edit.layout = Some(layout);
        layout
    }

    fn rotation_cursor(&self, ui: &egui::Ui, response: &egui::Response, layout: CanvasLayout) {
        if !ui.is_enabled() || !response.enabled() {
            return;
        }
        let Some(state) = &self.edit else {
            return;
        };
        let edit = state.borrow();
        let position = match edit.drag {
            Some(CropDrag::Straighten {
                layout: original, ..
            }) if response.dragged_by(egui::PointerButton::Primary) => response
                .interact_pointer_pos()
                .map(|point| (point, original.image.center())),
            None if response.hovered() && !response.dragged() => response
                .hover_pos()
                .filter(|point| {
                    !layout.image.contains(*point) && handle_at(layout.image, *point).is_none()
                })
                .map(|point| (point, layout.image.center())),
            _ => None,
        };
        let Some((point, pivot)) = position.filter(|(point, _)| layout.canvas.contains(*point))
        else {
            return;
        };
        let angle = (point - pivot).angle() + std::f32::consts::FRAC_PI_2;
        let image = icons::rotation_cursor(ui.ctx(), response.id.with("rotation-cursor"), angle);
        ui.ctx().set_cursor_image(Some(image));
    }

    fn paint(&self, ui: &egui::Ui, layout: CanvasLayout) {
        let Some(state) = &self.edit else {
            return;
        };
        let corners = [
            layout.image.min,
            layout.image.right_top(),
            layout.image.max,
            layout.image.left_bottom(),
        ];
        paint_outline(ui, layout.canvas, corners, state.borrow().guides, 160);
    }
}

impl tool::Tool for CropTool {
    fn presentation(&self, _editor: &Editor) -> tool::Presentation {
        tool::Presentation {
            show_viewer: true,
            identity: presentation::ToolIdentity {
                icon: icons::Icon::Crop,
                title: "Crop",
            },
            primary: Some("Apply"),
            dismiss: "Cancel",
            warning: self
                .edit
                .as_ref()
                .and_then(|edit| edit.borrow().auto_problem.clone()),
            help: "Crop: drag inside to move the image, edges/corners to resize, outside to straighten.",
            navigation_help: None,
            fit_label: "Fit source",
        }
    }
    fn activity(&self, editor: &Editor) -> Option<TaskRequest> {
        self.auto_crop_task
            .as_ref()
            .filter(|task| !task.is_finished())?;
        editor
            .document
            .spatial_preparing
            .as_ref()
            .map(|work| TaskRequest::Regions(work.id))
    }
    fn composition(&mut self, base: Composition) -> Composition {
        let state = self.edit.get_or_insert_with(|| {
            Rc::new(RefCell::new(CropEdit {
                draft: base.crop(),
                base,
                ratio: None,
                drag: None,
                layout: None,
                guides: true,
                auto_problem: None,
                refit: false,
                reveal_controls: true,
            }))
        });
        let mut edit = state.borrow_mut();
        edit.base = base;
        edit.base.cropped(edit.draft).expect("valid Crop draft")
    }

    fn viewport<'a>(&'a mut self, _editor: &'a mut Editor) -> &'a mut Viewport {
        &mut self.viewport
    }

    fn drawer(&mut self, ui: &mut egui::Ui, editor: &mut Editor, cx: &UiContext<'_, Editor>) {
        let Some(state) = &self.edit else {
            return;
        };
        if std::mem::take(&mut state.borrow_mut().reveal_controls) {
            ui.scroll_to_cursor(Some(egui::Align::Min));
        }
        presentation::ToolIdentity {
            icon: icons::Icon::Crop,
            title: "Crop & straighten",
        }
        .inspector(
            ui,
            |ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    icons::button(
                        ui,
                        icons::Icon::Reset,
                        true,
                        "Reset draft",
                        "Reset crop and angle.",
                    )
                    .clicked()
                })
                .inner
            },
            |ui, reset| {
                let propose = presentation::TextAction::new("Auto-crop")
                    .show(ui)
                    .clicked();
                self.controls(ui, reset, &mut editor.status);
                if propose && let Some(photo) = editor.document.visit.current_source {
                    self.request_auto_crop(editor, photo, cx);
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
        let composition = tool::Tool::composition(self, canvas.composition);
        let (dragging, refit) = {
            let mut edit = self.edit.as_ref().expect("Crop initialized").borrow_mut();
            (edit.drag.is_some(), std::mem::take(&mut edit.refit))
        };
        if refit {
            self.viewport.fit();
        }
        let rect = canvas.response.rect;
        let ppp = canvas.ui.ctx().pixels_per_point();
        let margin = 20_f32.min((rect.width().min(rect.height()) - 1.).max(0.) * 0.5);
        let relayout = |viewport: &mut Viewport| {
            let mut layout = viewport.layout_crop(rect.shrink(margin), composition, ppp);
            layout.canvas = rect;
            layout
        };
        let mut layout = canvas.layout(&mut self.viewport, relayout);
        let mut response = canvas.response.clone();
        navigation::consume_primary(&mut response);
        let panning = if dragging {
            false
        } else {
            let result = navigation::navigate_view(
                &mut self.viewport,
                canvas.ui,
                &response,
                layout,
                relayout,
            );
            layout = result.0;
            result.1
        };
        let ready = canvas.paint_image(editor, layout, true, cx);
        let interactive = ready
            && !panning
            && (dragging || !canvas.ui.input(|input| input.key_down(egui::Key::Space)));
        canvas.ui.add_enabled_ui(interactive, |ui| {
            let layout = self.interact(ui, canvas.response, layout, &mut editor.status);
            self.rotation_cursor(ui, canvas.response, layout);
        });
        // Geometry edits stage the next pass; this contour matches the painted image.
        self.paint(canvas.ui, layout);
        layout
    }
    fn action(&mut self, action: tool::Action, editor: &mut Editor) {
        match action {
            tool::Action::Primary => {
                let Some(edit) = &self.edit else {
                    return;
                };
                let draft = edit.borrow().draft;
                match editor.accept_crop(draft) {
                    Ok(()) => {
                        editor.status = "Crop applied".into();
                        editor.end_tool();
                    }
                    Err(error) => editor.status = format!("Crop rejected: {error}"),
                }
            }
            tool::Action::Dismiss => {
                editor.status = "Crop cancelled; composition unchanged".into();
                editor.end_tool();
            }
        }
    }
}

pub(super) fn outline_handles(q: [egui::Pos2; 4]) -> [(egui::Pos2, [i8; 2]); 8] {
    [
        (q[0], [-1, -1]),
        (q[1], [1, -1]),
        (q[2], [1, 1]),
        (q[3], [-1, 1]),
        (q[0] + (q[3] - q[0]) * 0.5, [-1, 0]),
        (q[1] + (q[2] - q[1]) * 0.5, [1, 0]),
        (q[0] + (q[1] - q[0]) * 0.5, [0, -1]),
        (q[3] + (q[2] - q[3]) * 0.5, [0, 1]),
    ]
}

/// Shared crop artwork in canvas coordinates, including source-view rotated/reflected frames.
pub(super) fn paint_outline(
    ui: &egui::Ui,
    canvas: egui::Rect,
    q: [egui::Pos2; 4],
    guides: bool,
    shade: u8,
) {
    let painter = ui.painter().with_clip_rect(canvas);
    let u = q[1] - q[0];
    let v = q[3] - q[0];
    let width = u.length();
    let height = v.length();
    if width <= 0. || height <= 0. {
        return;
    }
    let center = q[0] + (q[2] - q[0]) * 0.5;
    let radius =
        center.distance(canvas.min).max(center.distance(canvas.max)) + canvas.size().length();
    let scale = (radius / (width.min(height) * 0.5) + 2.).max(2.);
    let outer = q.map(|p| center + (p - center) * scale);
    // A clipped ring shades only outside the frame, including rotated/offscreen frames.
    let mut mesh = egui::Mesh::default();
    for p in q.into_iter().chain(outer) {
        mesh.colored_vertex(p, egui::Color32::from_black_alpha(shade));
    }
    for i in 0..4 {
        let j = (i + 1) % 4;
        mesh.add_triangle(i, j, 4 + j);
        mesh.add_triangle(i, 4 + j, 4 + i);
    }
    painter.add(egui::Shape::mesh(mesh));
    let bounds = egui::Rect::from_points(&q);
    let aligned = q
        == [
            bounds.min,
            bounds.right_top(),
            bounds.max,
            bounds.left_bottom(),
        ];
    for stroke in [
        egui::Stroke::new(2., egui::Color32::from_black_alpha(180)),
        egui::Stroke::new(0.8, egui::Color32::WHITE),
    ] {
        if aligned {
            painter.rect_stroke(bounds, 0., stroke, egui::StrokeKind::Inside);
        } else {
            painter.add(egui::Shape::closed_line(q.to_vec(), stroke));
        }
    }
    let segment = |a, b, width| {
        painter.line_segment(
            [a, b],
            egui::Stroke::new(width + 1.5, egui::Color32::from_black_alpha(200)),
        );
        painter.line_segment([a, b], egui::Stroke::new(width, egui::Color32::WHITE));
    };
    if guides {
        for fraction in [1. / 3., 2. / 3.] {
            if aligned {
                let x = egui::lerp(bounds.x_range(), fraction);
                let y = egui::lerp(bounds.y_range(), fraction);
                segment(
                    egui::pos2(x, bounds.top()),
                    egui::pos2(x, bounds.bottom()),
                    0.5,
                );
                segment(
                    egui::pos2(bounds.left(), y),
                    egui::pos2(bounds.right(), y),
                    0.5,
                );
            } else {
                segment(q[0] + u * fraction, q[3] + u * fraction, 0.5);
                segment(q[0] + v * fraction, q[1] + v * fraction, 0.5);
            }
        }
    }
    let u = u / width;
    let v = v / height;
    let length = 18_f32.min(width * 0.3).min(height * 0.3);
    for (point, [x, y]) in outline_handles(q) {
        if x != 0 && y != 0 {
            segment(point, point - u * f32::from(x) * length, 2.5);
            segment(point, point - v * f32::from(y) * length, 2.5);
        } else {
            let half = if x == 0 {
                u * 7_f32.min(width * 0.15)
            } else {
                v * 7_f32.min(height * 0.15)
            };
            segment(point - half, point + half, 2.5);
        }
    }
}
