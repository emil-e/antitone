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

impl Editor {
    pub(in crate::editor) fn auto_crop_context(&self) -> bool {
        self.fullscreen.is_none()
            && self.document.visit.crop_edit.is_none()
            && self.active_pick().is_none()
            && self.document.flat_edit.is_none()
            && self
                .document
                .auto_crop_edit
                .as_ref()
                .is_some_and(|d| d.belongs_to(self))
    }

    pub(super) fn auto_crop_margin_entry(&self) -> Option<&Entry> {
        let id = self.document.visit.current_source?;
        self.document
            .auto_crop_edit
            .as_ref()?
            .entries
            .iter()
            .find(|e| e.source == id)
            .filter(|e| e.current(self) && self.loaded_crop_source_current(id, &e.file))
    }

    pub(in crate::editor) fn auto_crop_artwork_visible(&self, ui: &egui::Ui) -> bool {
        self.auto_crop_context()
            && self
                .document
                .auto_crop_edit
                .as_ref()
                .is_some_and(|d| d.show_outline)
            && !ui.input(|i| i.key_down(egui::Key::H))
    }

    pub(in crate::editor) fn auto_crop_outline(
        &self,
        layout: CanvasLayout,
    ) -> Option<[egui::Pos2; 4]> {
        if !self.auto_crop_context() {
            return None;
        }
        let entry = self.auto_crop_margin_entry()?;
        let crop = entry
            .draft
            .unwrap_or(Crop::from_region(entry.development.active_area.into()));
        Some(
            crop.coverage()
                .ok()?
                .corners()
                .map(|p| layout.screen_frame_point(layout.composition.source_to_frame(p))),
        )
    }

    pub(in crate::editor) fn auto_crop_margin_interaction(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        layout: CanvasLayout,
    ) -> (bool, Option<egui::CursorIcon>) {
        if !ui.is_enabled()
            || !response.enabled()
            || !self.auto_crop_artwork_visible(ui)
            || !self.auto_crop_editable()
            || ui.input(|i| i.key_down(egui::Key::Space))
        {
            if let Some(d) = &mut self.document.auto_crop_edit {
                d.drag = None;
            }
            return (false, None);
        }
        let Some(q) = self.auto_crop_outline(layout) else {
            if let Some(d) = &mut self.document.auto_crop_edit {
                d.drag = None;
            }
            return (false, None);
        };
        let press = ui
            .input(|i| i.pointer.press_origin())
            .and_then(|p| hit(q, p).map(|h| (p, h)));
        if response.drag_started_by(egui::PointerButton::Primary)
            && let Some(d) = &mut self.document.auto_crop_edit
        {
            d.drag = None;
        }
        if response.drag_started_by(egui::PointerButton::Primary)
            && let Some((point, handle)) = press
        {
            let entry = self
                .auto_crop_margin_entry()
                .expect("outline retains source");
            let drag = MarginDrag {
                source: entry.source,
                proposal: entry.proposal,
                handle,
                start: entry.proposal.local_point(
                    layout
                        .composition
                        .frame_to_source(layout.frame_point(point)),
                ),
                percent: self.auto_crop_margin,
            };
            self.document.auto_crop_edit.as_mut().unwrap().drag = Some(drag);
        }
        let drag = self
            .document
            .auto_crop_edit
            .as_ref()
            .and_then(|d| d.drag)
            .filter(|d| {
                self.document.visit.current_source == Some(d.source)
                    && self
                        .auto_crop_margin_entry()
                        .is_some_and(|e| e.proposal == d.proposal)
            });
        let claimed = drag.is_some() && response.dragged_by(egui::PointerButton::Primary)
            || press.is_some() && response.clicked_by(egui::PointerButton::Primary);
        if let Some(d) = drag
            && response.dragged_by(egui::PointerButton::Primary)
            && let Some(point) = response.interact_pointer_pos()
        {
            let percent = d.margin(
                layout
                    .composition
                    .frame_to_source(layout.frame_point(point)),
            );
            if percent != self.auto_crop_margin {
                self.adjust_auto_crop_margin(percent);
            }
        }
        let handle = if response.dragged_by(egui::PointerButton::Primary) {
            drag.map(|d| d.handle)
        } else {
            response.hover_pos().and_then(|p| hit(q, p))
        };
        if response.drag_stopped_by(egui::PointerButton::Primary)
            && let Some(d) = &mut self.document.auto_crop_edit
        {
            d.drag = None;
        }
        (claimed, handle.map(|h| cursor(q, h)))
    }
}

#[cfg(test)]
mod tests;
