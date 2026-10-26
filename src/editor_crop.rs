use super::*;
use crate::{geometry::FramePoint, viewport::CanvasLayout};

#[cfg(test)]
#[path = "editor_crop_tests.rs"]
mod tests;

pub(super) struct CropEdit {
    pub draft: Crop,
    viewport: Viewport,
    ratio: Option<f64>,
    drag: Option<CropDrag>,
    pub(super) layout: Option<CanvasLayout>,
    pub(super) guides: bool,
    pub(super) auto_problem: Option<String>,
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

pub(super) enum ToolAction {
    TurnLeft,
    TurnRight,
    FlipHorizontal,
    FlipVertical,
    Crop,
    Dust,
}

impl Editor {
    pub(super) fn begin_crop(&mut self) {
        self.finish_dust_cleanup();
        if self.busy
            || self.pick.is_some()
            || self.document.visit.crop_edit.is_some()
            || self.document.auto_crop_edit.is_some()
            || self.document.visit.mask_edit.is_some()
        {
            return;
        }
        let Some(area) = self.document.visit.ready.as_ref().map(|ready| ready.area) else {
            return;
        };
        self.finish_adjustment_layer_interaction();
        self.document.visit.crop_edit = Some(CropEdit {
            draft: self.crop.unwrap_or(Crop::from_region(area)),
            viewport: self.viewport,
            ratio: None,
            drag: None,
            layout: None,
            guides: true,
            auto_problem: None,
            reveal_controls: true,
        });
        self.viewport.fit();
        self.last_layout = None;
        self.invalidate_original();
        self.status = "Crop draft; changes are not applied".into();
    }

    pub(super) fn initialize_crop_frame(&mut self, crop: Crop) {
        let edit = self
            .document
            .visit
            .crop_edit
            .as_mut()
            .expect("crop initialization retains its tool");
        edit.draft = crop;
        edit.ratio = None;
        edit.layout = None;
        self.viewport.fit();
        self.last_layout = None;
        self.invalidate_original();
    }

    pub(super) fn finish_crop(&mut self, apply: bool) {
        let Some(edit) = self.document.visit.crop_edit.take() else {
            return;
        };
        if apply {
            let before = self.history_snapshot();
            let active = self
                .document
                .visit
                .ready
                .as_ref()
                .expect("crop retains its source")
                .area;
            edit.draft
                .region(active)
                .expect("crop draft is constrained");
            self.crop = (edit.draft != Crop::from_region(active)).then_some(edit.draft);
            self.viewport.fit();
            self.status = "Crop applied".into();
            self.record_edit(before, "Crop", self.current_source);
        } else {
            self.viewport = edit.viewport;
            self.status = "Crop cancelled; composition unchanged".into();
        }
        self.last_layout = None;
        self.invalidate_original();
    }

    pub(super) fn photo_tool_shelf(&mut self, ui: &mut egui::Ui, cx: &UiContext<'_, Self>) {
        use icons::Icon;
        use presentation::{ToolButton, ToolShelf};

        let cropping = self.document.visit.crop_edit.is_some();
        let dusting = self.document.dust.cleanup;
        let auto_cropping = self.document.auto_crop_edit.is_some();
        let tool_active = cropping
            || dusting
            || auto_cropping
            || self.flat_edit.is_some()
            || self.document.visit.mask_edit.is_some();
        let ready = !self.busy && self.document.visit.ready.is_some();
        let commands_enabled = ready && !tool_active && self.pick.is_none();
        let crop_enabled =
            cropping || auto_cropping || ready && self.pick.is_none() && !tool_active;
        let dust_enabled = dusting
            || ready && self.input.ready() && !self.source && self.pick.is_none() && !tool_active;
        let repairs_changed = self
            .current_source
            .and_then(|source| self.dust_repairs(source))
            .is_some_and(|set| !set.repairs().is_empty());
        let mut action = None;

        use ToolAction as Action;

        ToolShelf::show(ui, |ui| {
            for (candidate, button) in [
                (
                    Action::TurnLeft,
                    ToolButton::command(
                        Icon::TurnLeft,
                        "Turn left",
                        "Turn the displayed photo 90° left.",
                    ),
                ),
                (
                    Action::TurnRight,
                    ToolButton::command(
                        Icon::TurnRight,
                        "Turn right",
                        "Turn the displayed photo 90° right.",
                    ),
                ),
                (
                    Action::FlipHorizontal,
                    ToolButton::command(
                        Icon::FlipHorizontal,
                        "Flip horizontal",
                        "Flip the displayed photo left/right.",
                    ),
                ),
                (
                    Action::FlipVertical,
                    ToolButton::command(
                        Icon::FlipVertical,
                        "Flip vertical",
                        "Flip the displayed photo top/bottom.",
                    ),
                ),
            ] {
                if button.enabled(commands_enabled).show(ui).clicked() {
                    action = Some(candidate);
                }
            }
            ui.separator();
            if ToolButton::tool(
                Icon::Crop,
                "Crop & straighten",
                if cropping || auto_cropping {
                    "Exit crop without applying the draft."
                } else {
                    "Edit this photo's crop and straighten."
                },
                cropping || auto_cropping,
            )
            .enabled(crop_enabled)
            .changed(self.crop.is_some())
            .show(ui)
            .clicked()
            {
                action = Some(Action::Crop);
            }
            if ToolButton::tool(
                Icon::Dust,
                "Dust repair",
                if dusting {
                    "Finish dust cleanup."
                } else {
                    "Open continuing dust cleanup for this photo."
                },
                dusting,
            )
            .enabled(dust_enabled)
            .changed(repairs_changed)
            .show(ui)
            .clicked()
            {
                action = Some(Action::Dust);
            }
        });

        if let Some(action) = action {
            self.control_command(editing::Command::Photo(action), cx);
        }
    }

    pub(super) fn apply_photo_tool_action(&mut self, action: ToolAction) {
        let before = self.history_snapshot();
        let orientation = self.photo_orientation;
        match action {
            ToolAction::TurnLeft => self.photo_orientation.turn_counterclockwise(),
            ToolAction::TurnRight => self.photo_orientation.turn_clockwise(),
            ToolAction::FlipHorizontal => self.photo_orientation.flips.horizontal ^= true,
            ToolAction::FlipVertical => self.photo_orientation.flips.vertical ^= true,
            ToolAction::Crop => {
                if self.document.auto_crop_edit.is_some() {
                    self.finish_auto_crop(false);
                } else if self.document.visit.crop_edit.is_some() {
                    self.finish_crop(false);
                } else {
                    self.begin_crop();
                }
            }
            ToolAction::Dust => {
                if self.document.dust.cleanup {
                    self.finish_dust_cleanup();
                } else {
                    self.begin_dust_cleanup();
                }
            }
        }
        if orientation != self.photo_orientation {
            self.viewport.fit();
        }
        self.record_edit(before, "Photo orientation", self.current_source);
    }

    pub(super) fn crop_controls(&mut self, ui: &mut egui::Ui) {
        if let Some(edit) = &mut self.document.visit.crop_edit {
            if std::mem::take(&mut edit.reveal_controls) {
                ui.scroll_to_cursor(Some(egui::Align::Min));
            }
            ui.add_enabled_ui(!self.busy, |ui| {
                controls::active_tool(
                    ui,
                    icons::Icon::Crop,
                    "Crop & straighten",
                    Some(true),
                    |ui, reset| {
                        self.auto_crop_action(ui, auto_crop::Target::Photo, "Auto-crop");
                        self.crop_tool_controls(ui, reset);
                    },
                );
            });
        }
    }

    pub(super) fn crop_tool_controls(&mut self, ui: &mut egui::Ui, reset: bool) {
        use icons::Icon;
        let edit = self.document.visit.crop_edit.as_mut().unwrap();
        let area = self
            .document
            .visit
            .ready
            .as_ref()
            .expect("crop retains its source")
            .area;
        let full = Composition::new(area, self.roll_orientation, self.photo_orientation).unwrap();
        let composition = Composition::with_crop(
            area,
            self.roll_orientation,
            self.photo_orientation,
            Some(edit.draft),
        )
        .unwrap();
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
                Err(error) => self.status = format!("Straighten rejected: {error}"),
            }
        }
        if reset {
            edit.draft = Crop::from_region(area);
            edit.ratio = None;
            edit.drag = None;
            self.viewport.fit();
        } else if edit.draft != previous {
            edit.drag = None;
            if let Some(layout) = edit.layout {
                let next = Composition::with_crop(
                    area,
                    self.roll_orientation,
                    self.photo_orientation,
                    Some(edit.draft),
                )
                .unwrap();
                reframe_crop(
                    &mut self.viewport,
                    layout,
                    next,
                    ui.ctx().pixels_per_point(),
                    edit.draft.angle != previous.angle,
                );
            }
        }
    }

    pub(super) fn crop_interaction(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        mut layout: CanvasLayout,
    ) -> CanvasLayout {
        let Some(edit) = &mut self.document.visit.crop_edit else {
            return layout;
        };
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
                            let next = Composition::with_crop(
                                original.composition.area(),
                                self.roll_orientation,
                                self.photo_orientation,
                                Some(crop),
                            )
                            .unwrap()
                            .size()
                            .map(|v| v as f32);
                            // Coverage may shrink, but the straighten pivot stays at the frame centre.
                            layout.image = egui::Rect::from_center_size(
                                original.image.center(),
                                original.image.size()
                                    * egui::vec2(next[0] / size[0], next[1] / size[1]),
                            );
                            layout.scale = original.scale;
                        }
                        Err(error) => self.status = format!("Straighten rejected: {error}"),
                    }
                }
            }
            layout.composition = Composition::with_crop(
                layout.composition.area(),
                self.roll_orientation,
                self.photo_orientation,
                Some(edit.draft),
            )
            .unwrap();
            layout.area = layout.composition.region();
        }
        if response.drag_stopped_by(egui::PointerButton::Primary) {
            edit.drag = None;
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
        self.viewport
            .reframe(layout, layout.composition, ui.ctx().pixels_per_point());
        edit.layout = Some(layout);
        layout
    }

    pub(super) fn crop_rotation_cursor(
        &self,
        ui: &egui::Ui,
        response: &egui::Response,
        layout: CanvasLayout,
    ) {
        if !ui.is_enabled() || !response.enabled() {
            return;
        }
        let Some(edit) = &self.document.visit.crop_edit else {
            return;
        };
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

    pub(super) fn paint_crop(&self, ui: &egui::Ui, layout: CanvasLayout) {
        paint_outline(
            ui,
            layout.canvas,
            [
                layout.image.min,
                layout.image.right_top(),
                layout.image.max,
                layout.image.left_bottom(),
            ],
            self.document
                .visit
                .crop_edit
                .as_ref()
                .is_some_and(|e| e.guides),
            160,
        );
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
