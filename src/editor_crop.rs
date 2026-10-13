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
        if self.busy || self.pick.is_some() || self.crop_edit.is_some() {
            return;
        }
        let Some(ready) = &self.ready else {
            return;
        };
        self.crop_edit = Some(CropEdit {
            draft: self.crop.unwrap_or(Crop::from_region(ready.area)),
            viewport: self.viewport,
            ratio: None,
            drag: None,
            layout: None,
            guides: true,
        });
        self.viewport.fit();
        self.last_layout = None;
        self.generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.signature = None;
        self.settled = None;
        self.status = "Crop draft; changes are not applied".into();
    }
    pub(super) fn finish_crop(&mut self, apply: bool) {
        let Some(edit) = self.crop_edit.take() else {
            return;
        };
        if apply {
            let active = self.ready.as_ref().expect("crop retains its source").area;
            edit.draft
                .region(active)
                .expect("crop draft is constrained");
            self.crop = (edit.draft != Crop::from_region(active)).then_some(edit.draft);
            self.viewport.fit();
            self.status = "Crop applied".into();
        } else {
            self.viewport = edit.viewport;
            self.status = "Crop cancelled; composition unchanged".into();
        }
        self.last_layout = None;
        self.signature = None;
        self.settled = None;
        self.generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    pub(super) fn composition_controls(&mut self, ui: &mut egui::Ui) {
        use icons::Icon;
        let cropping = self.crop_edit.is_some();
        let editable = !self.busy && self.ready.is_some() && self.pick.is_none() && !cropping;
        let mut begin = false;
        let mut reset_crop = false;
        let mut reset = false;
        ui.add_enabled_ui(editable || cropping, |ui| {
            (_, reset) = presentation::section(
                ui,
                "photo-composition",
                "Composition",
                presentation::SectionHeader {
                    summary: if cropping {
                        "Crop active"
                    } else if self.crop.is_some() {
                        "Cropped"
                    } else {
                        "Full frame"
                    },
                    changed: !self.photo_orientation.is_identity() || self.crop.is_some(),
                    reset: Some(
                        editable && (!self.photo_orientation.is_identity() || self.crop.is_some()),
                    ),
                    default_open: if cropping { None } else { Some(true) },
                },
                |ui| {
                    ui.add_enabled_ui(editable, |ui| {
                        ui.horizontal(|ui| {
                            if icons::button(
                                ui,
                                Icon::TurnLeft,
                                true,
                                "Turn left",
                                "Turn the displayed photo 90° left.",
                            )
                            .clicked()
                            {
                                self.photo_orientation.turn_counterclockwise();
                            }
                            if icons::button(
                                ui,
                                Icon::TurnRight,
                                true,
                                "Turn right",
                                "Turn the displayed photo 90° right.",
                            )
                            .clicked()
                            {
                                self.photo_orientation.turn_clockwise();
                            }
                            if icons::button(
                                ui,
                                Icon::FlipHorizontal,
                                true,
                                "Flip horizontal",
                                "Flip the displayed photo left/right.",
                            )
                            .clicked()
                            {
                                self.photo_orientation.flips.horizontal ^= true;
                            }
                            if icons::button(
                                ui,
                                Icon::FlipVertical,
                                true,
                                "Flip vertical",
                                "Flip the displayed photo top/bottom.",
                            )
                            .clicked()
                            {
                                self.photo_orientation.flips.vertical ^= true;
                            }
                            let crop = icons::button(
                                ui,
                                Icon::Crop,
                                true,
                                "Crop…",
                                "Edit this photo's crop and straighten.",
                            );
                            if cropping {
                                ui.painter().rect_stroke(
                                    crop.rect.shrink(1.),
                                    4.,
                                    egui::Stroke::new(1., ui.visuals().selection.stroke.color),
                                    egui::StrokeKind::Inside,
                                );
                            }
                            begin = crop.clicked();
                            if self.crop.is_some() {
                                reset_crop = icons::button(
                                    ui,
                                    Icon::Reset,
                                    true,
                                    "Reset crop",
                                    "Reset framing; keeps orientation and colour.",
                                )
                                .clicked();
                            }
                        });
                    });
                    if cropping {
                        ui.add_enabled_ui(!self.busy, |ui| {
                            controls::active_tool(
                                ui,
                                Icon::Crop,
                                "Crop & straighten",
                                Some(true),
                                |ui, reset| self.crop_tool_controls(ui, reset),
                            );
                        });
                    }
                },
            );
        });
        if reset {
            self.photo_orientation = PhotoOrientation::default();
            self.crop = None;
            self.viewport.fit();
        } else if reset_crop {
            self.crop = None;
            self.viewport.fit();
        } else if begin {
            self.begin_crop();
        }
    }
    pub(super) fn crop_tool_controls(&mut self, ui: &mut egui::Ui, reset: bool) {
        use icons::Icon;
        let edit = self.crop_edit.as_mut().unwrap();
        let area = self.ready.as_ref().expect("crop retains its source").area;
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
        egui::Grid::new("crop-parameters").min_col_width(0.).min_row_height(28.).spacing(egui::vec2(8., 4.)).show(ui, |ui| {
            ui.label("Aspect");
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
            if icons::button(
                ui,
                Icon::Swap,
                edit.ratio.is_some(),
                "Swap portrait / landscape",
                "Swap portrait / landscape",
            )
            .clicked()
            {
                edit.ratio = edit.ratio.map(|r| 1. / r);
            }
            ui.end_row();
            let angle_label = ui.label("Angle");
            let angle = ui.add_sized([96., 28.],
                egui::DragValue::new(&mut degrees)
                    .range(-180.0..=180.0)
                    .speed(0.1)
                    .fixed_decimals(2)
                    .update_while_editing(false)
                    .suffix("°"),
            ).labelled_by(angle_label.id);
            // egui 0.36 retains cancelled text for its second lost-focus pass.
            let cancel_id = ui.id().with("angle-cancelled");
            let cancelled = ui
                .ctx()
                .data_mut(|data| data.remove_temp::<bool>(cancel_id))
                .unwrap_or(false);
            let cancelling =
                angle.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Escape));
            if cancelled || cancelling || !ui.is_enabled() {
                degrees = previous_degrees;
            }
            if cancelling {
                ui.ctx().data_mut(|data| data.insert_temp(cancel_id, true));
            }
            reset_angle = icons::button(
                ui,
                Icon::Reset,
                degrees != 0.,
                "Reset straighten",
                "Reset straighten",
            )
            .clicked();
            ui.end_row();
            ui.label("Guides");
            icons::toggle(
                ui,
                Icon::Guides,
                &mut edit.guides,
                "Thirds guides",
                "Show thirds guides. Composition aid only; never affects measurements or exports.",
            );
            ui.end_row();
        });
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
        let Some(edit) = &mut self.crop_edit else {
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
        let Some(edit) = &self.crop_edit else {
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
        let crop = layout.image;
        let canvas = layout.canvas;
        let painter = ui.painter().with_clip_rect(canvas);
        let shade = egui::Color32::from_black_alpha(160);
        for rect in [
            egui::Rect::from_min_max(canvas.min, egui::pos2(canvas.max.x, crop.min.y)),
            egui::Rect::from_min_max(egui::pos2(canvas.min.x, crop.max.y), canvas.max),
            egui::Rect::from_min_max(
                egui::pos2(canvas.min.x, crop.min.y),
                egui::pos2(crop.min.x, crop.max.y),
            ),
            egui::Rect::from_min_max(
                egui::pos2(crop.max.x, crop.min.y),
                egui::pos2(canvas.max.x, crop.max.y),
            ),
        ] {
            if rect.is_positive() {
                painter.rect_filled(rect, 0., shade);
            }
        }
        painter.rect_stroke(
            crop,
            0.,
            egui::Stroke::new(2., egui::Color32::from_black_alpha(180)),
            egui::StrokeKind::Inside,
        );
        painter.rect_stroke(
            crop,
            0.,
            egui::Stroke::new(0.8, egui::Color32::WHITE),
            egui::StrokeKind::Inside,
        );
        let segment = |a: egui::Pos2, b: egui::Pos2, width: f32| {
            painter.line_segment(
                [a, b],
                egui::Stroke::new(width + 1.5, egui::Color32::from_black_alpha(200)),
            );
            painter.line_segment([a, b], egui::Stroke::new(width, egui::Color32::WHITE));
        };
        if self.crop_edit.as_ref().is_some_and(|edit| edit.guides) {
            for fraction in [1. / 3., 2. / 3.] {
                let x = egui::lerp(crop.x_range(), fraction);
                let y = egui::lerp(crop.y_range(), fraction);
                segment(egui::pos2(x, crop.top()), egui::pos2(x, crop.bottom()), 0.5);
                segment(egui::pos2(crop.left(), y), egui::pos2(crop.right(), y), 0.5);
            }
        }
        let corner_length = 18_f32.min(crop.width() * 0.3).min(crop.height() * 0.3);
        for (point, [x, y]) in handles(crop) {
            if x != 0 && y != 0 {
                segment(
                    point,
                    point - egui::vec2(f32::from(x) * corner_length, 0.),
                    2.5,
                );
                segment(
                    point,
                    point - egui::vec2(0., f32::from(y) * corner_length),
                    2.5,
                );
            } else {
                let half = if x == 0 {
                    egui::vec2(7_f32.min(crop.width() * 0.15), 0.)
                } else {
                    egui::vec2(0., 7_f32.min(crop.height() * 0.15))
                };
                segment(point - half, point + half, 2.5);
            }
        }
    }
}
