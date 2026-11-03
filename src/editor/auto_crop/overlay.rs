use super::*;
use crate::{geometry::SourcePoint, viewport::CanvasLayout};

#[derive(Clone, Copy)]
pub(super) struct MarginDrag {
    source: SourceId,
    proposal: Crop,
    handle: [i8; 2],
    start: [f64; 2],
    percent: f64,
}

impl MarginDrag {
    fn margin(self, point: SourcePoint) -> f64 {
        let local = self.proposal.local_point(point);
        let delta = (0..2)
            .filter(|&i| self.handle[i] != 0)
            .map(|i| (self.start[i] - local[i]) * f64::from(self.handle[i]))
            .max_by(|a, b| a.abs().total_cmp(&b.abs()))
            .unwrap_or(0.);
        let percent = self.percent
            + delta * 100. / f64::from(self.proposal.size[0].min(self.proposal.size[1]));
        ((percent.clamp(0., framing::MAX_MARGIN_PERCENT) * 100.).round() / 100.)
            .clamp(0., framing::MAX_MARGIN_PERCENT)
    }
}

fn hit(q: [egui::Pos2; 4], point: egui::Pos2) -> Option<[i8; 2]> {
    if let Some((_, h)) = crop_editor::outline_handles(q)
        .into_iter()
        .find(|(p, _)| p.distance(point) <= 9.)
    {
        return Some(h);
    }
    [
        (0, 3, [-1, 0]),
        (1, 2, [1, 0]),
        (0, 1, [0, -1]),
        (3, 2, [0, 1]),
    ]
    .into_iter()
    .find_map(|(a, b, h)| {
        let d = q[b] - q[a];
        let t = ((point - q[a]).dot(d) / d.length_sq()).clamp(0., 1.);
        (point.distance(q[a] + d * t) <= 6.).then_some(h)
    })
}

fn cursor(q: [egui::Pos2; 4], handle: [i8; 2]) -> egui::CursorIcon {
    let d = (q[1] - q[0]).normalized() * f32::from(handle[0])
        + (q[3] - q[0]).normalized() * f32::from(handle[1]);
    match ((d.angle() / std::f32::consts::FRAC_PI_4).round() as i32).rem_euclid(4) {
        0 => egui::CursorIcon::ResizeHorizontal,
        1 => egui::CursorIcon::ResizeNwSe,
        2 => egui::CursorIcon::ResizeVertical,
        _ => egui::CursorIcon::ResizeNeSw,
    }
}

impl Draft {
    pub(super) fn context(&self, editor: &Editor) -> bool {
        editor.fullscreen.is_none() && self.belongs_to(editor)
    }

    fn margin_entry(&self, editor: &Editor) -> Option<&Entry> {
        let id = editor.document.visit.current_source?;
        self.entries
            .iter()
            .find(|entry| entry.source == id)
            .filter(|entry| {
                entry.current(editor) && editor.loaded_crop_source_current(id, &entry.file)
            })
    }

    pub(super) fn artwork_visible(&self, editor: &Editor, ui: &egui::Ui) -> bool {
        self.context(editor) && self.show_outline && !ui.input(|input| input.key_down(egui::Key::H))
    }

    pub(super) fn outline(&self, editor: &Editor, layout: CanvasLayout) -> Option<[egui::Pos2; 4]> {
        if !self.context(editor) {
            return None;
        }
        let entry = self.margin_entry(editor)?;
        let crop = entry
            .draft
            .unwrap_or(Crop::from_region(entry.development.active_area.into()));
        Some(
            crop.coverage()
                .ok()?
                .corners()
                .map(|point| layout.screen_frame_point(layout.composition.source_to_frame(point))),
        )
    }

    pub(super) fn margin_interaction(
        &mut self,
        editor: &mut Editor,
        ui: &egui::Ui,
        response: &egui::Response,
        layout: CanvasLayout,
    ) -> (bool, Option<egui::CursorIcon>) {
        if !ui.is_enabled()
            || !response.enabled()
            || !self.artwork_visible(editor, ui)
            || !self.editable(editor)
            || ui.input(|input| input.key_down(egui::Key::Space))
        {
            self.drag = None;
            return (false, None);
        }
        let Some(outline) = self.outline(editor, layout) else {
            self.drag = None;
            return (false, None);
        };
        let press = ui
            .input(|input| input.pointer.press_origin())
            .and_then(|point| hit(outline, point).map(|handle| (point, handle)));
        if response.drag_started_by(egui::PointerButton::Primary) {
            self.drag = None;
            if let Some((point, handle)) = press {
                let entry = self.margin_entry(editor).expect("outline retains source");
                self.drag = Some(MarginDrag {
                    source: entry.source,
                    proposal: entry.proposal,
                    handle,
                    start: entry.proposal.local_point(
                        layout
                            .composition
                            .frame_to_source(layout.frame_point(point)),
                    ),
                    percent: editor.auto_crop_margin,
                });
            }
        }
        let drag = self.drag.filter(|drag| {
            editor.document.visit.current_source == Some(drag.source)
                && self
                    .margin_entry(editor)
                    .is_some_and(|entry| entry.proposal == drag.proposal)
        });
        let claimed = drag.is_some() && response.dragged_by(egui::PointerButton::Primary)
            || press.is_some() && response.clicked_by(egui::PointerButton::Primary);
        if let Some(drag) = drag
            && response.dragged_by(egui::PointerButton::Primary)
            && let Some(point) = response.interact_pointer_pos()
        {
            let percent = drag.margin(
                layout
                    .composition
                    .frame_to_source(layout.frame_point(point)),
            );
            if percent != editor.auto_crop_margin {
                self.adjust_margin(editor, percent);
            }
        }
        let handle = if response.dragged_by(egui::PointerButton::Primary) {
            drag.map(|drag| drag.handle)
        } else {
            response.hover_pos().and_then(|point| hit(outline, point))
        };
        if response.drag_stopped_by(egui::PointerButton::Primary) {
            self.drag = None;
        }
        (claimed, handle.map(|handle| cursor(outline, handle)))
    }
}

impl Editor {
    #[cfg(test)]
    pub(in crate::editor) fn auto_crop_context(&self) -> bool {
        self.document.auto_crop.current().is_some_and(|state| {
            state
                .borrow()
                .as_ref()
                .is_some_and(|draft| draft.context(self))
        })
    }

    #[cfg(test)]
    pub(in crate::editor) fn auto_crop_outline(
        &self,
        layout: CanvasLayout,
    ) -> Option<[egui::Pos2; 4]> {
        let state = self.document.auto_crop.current()?;
        let state = state.borrow();
        state.as_ref()?.outline(self, layout)
    }
}

#[cfg(test)]
mod tests;
