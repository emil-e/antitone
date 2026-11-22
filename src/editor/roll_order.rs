use super::*;
use std::{borrow::Cow, cmp::Ordering, collections::HashSet};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Sort {
    #[default]
    Custom,
    NameAscending,
    NameDescending,
    CaptureOldest,
    CaptureNewest,
}

impl Sort {
    pub(super) fn ids(self, project: &crate::project::Project) -> Cow<'_, [SourceId]> {
        // Traverse Custom first so all ties (including unknown dates) retain Custom order.
        if self == Self::Custom {
            return Cow::Borrowed(&project.order);
        }
        let sources: std::collections::HashMap<_, _> = project
            .sources
            .iter()
            .map(|source| (source.id, source))
            .collect();
        let ordered = project.order.iter().map(|id| sources[id]);
        let ids = match self {
            Self::NameAscending | Self::NameDescending => {
                let mut named: Vec<_> = ordered
                    .map(|source| {
                        let name = source.path.file_name().unwrap_or_default();
                        (source.id, name.to_string_lossy().to_lowercase())
                    })
                    .collect();
                named.sort_by(|(_, left), (_, right)| {
                    let order = natural_cmp(left.as_bytes(), right.as_bytes());
                    if self == Self::NameAscending {
                        order
                    } else {
                        order.reverse()
                    }
                });
                named.into_iter().map(|(id, _)| id).collect()
            }
            Self::CaptureOldest | Self::CaptureNewest => {
                let mut dated: Vec<_> = ordered.collect();
                dated.sort_by(
                    |left, right| match (left.capture_time, right.capture_time) {
                        (Some(left), Some(right)) if self == Self::CaptureOldest => {
                            left.cmp(&right)
                        }
                        (Some(left), Some(right)) => right.cmp(&left),
                        (Some(_), None) => Ordering::Less,
                        (None, Some(_)) => Ordering::Greater,
                        (None, None) => Ordering::Equal,
                    },
                );
                dated.into_iter().map(|source| source.id).collect()
            }
            Self::Custom => unreachable!(),
        };
        Cow::Owned(ids)
    }

    fn description(self) -> &'static str {
        match self {
            Self::Custom => "Custom",
            Self::NameAscending => "File name, A to Z",
            Self::NameDescending => "File name, Z to A",
            Self::CaptureOldest => "Capture time, oldest first",
            Self::CaptureNewest => "Capture time, newest first",
        }
    }
}

fn natural_cmp(mut left: &[u8], mut right: &[u8]) -> Ordering {
    fn digit_run(text: &[u8]) -> (&[u8], &[u8]) {
        let end = text
            .iter()
            .position(|byte| !byte.is_ascii_digit())
            .unwrap_or(text.len());
        let first = text[..end]
            .iter()
            .position(|byte| *byte != b'0')
            .unwrap_or(end);
        (&text[first..end], &text[end..])
    }

    while let ([l, l_tail @ ..], [r, r_tail @ ..]) = (left, right) {
        let order = if l.is_ascii_digit() && r.is_ascii_digit() {
            let (l, l_tail) = digit_run(left);
            let (r, r_tail) = digit_run(right);
            (left, right) = (l_tail, r_tail);
            l.len().cmp(&r.len()).then_with(|| l.cmp(r))
        } else {
            (left, right) = (l_tail, r_tail);
            l.cmp(r)
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    left.len().cmp(&right.len())
}

#[derive(Default)]
pub(super) struct View {
    pub sort: Sort,
    pub drag: Option<Drag>,
}

impl View {
    pub fn control(&mut self, ui: &mut egui::Ui, has_capture_time: bool) {
        let button = ui::icons::button(
            ui,
            ui::icons::Icon::Sort,
            self.drag.is_none(),
            "Sort photos",
            &format!("Sort photos · {}", self.sort.description()),
        );
        if self.sort != Sort::Custom {
            ui.painter().rect_stroke(
                button.rect.shrink(1.),
                4.,
                egui::Stroke::new(1., ui.visuals().selection.stroke.color),
                egui::StrokeKind::Inside,
            );
        }
        egui::Popup::menu(&button)
            .align(egui::RectAlign::TOP_START)
            .show(|ui| {
                ui.selectable_value(&mut self.sort, Sort::Custom, "Custom");
                ui.separator();
                ui.label("File name");
                ui.selectable_value(&mut self.sort, Sort::NameAscending, "A to Z");
                ui.selectable_value(&mut self.sort, Sort::NameDescending, "Z to A");
                ui.separator();
                ui.label("Capture time");
                ui.add_enabled_ui(has_capture_time, |ui| {
                    ui.selectable_value(&mut self.sort, Sort::CaptureOldest, "Oldest first");
                    ui.selectable_value(&mut self.sort, Sort::CaptureNewest, "Newest first");
                });
                if !has_capture_time {
                    ui.weak("No capture times recorded");
                }
            });
    }
}

pub(super) struct Drag {
    order: Vec<SourceId>,
    moved: HashSet<SourceId>,
    name: String,
    preview: Option<egui::TextureHandle>,
}

impl Drag {
    pub fn new(
        order: Vec<SourceId>,
        lead: SourceId,
        selection: &RollSelection,
        name: String,
        preview: Option<egui::TextureHandle>,
    ) -> Self {
        let moved = if selection.contains(lead) {
            order
                .iter()
                .copied()
                .filter(|id| selection.contains(*id))
                .collect()
        } else {
            HashSet::from([lead])
        };
        Self {
            order,
            moved,
            name,
            preview,
        }
    }

    pub fn contains(&self, id: SourceId) -> bool {
        self.moved.contains(&id)
    }

    pub fn current(&self, displayed: &[SourceId]) -> bool {
        self.order == displayed
    }

    /// `slot` lies between cards in the displayed order, before removing any moved photo.
    fn reordered(&self, slot: usize) -> Option<Vec<SourceId>> {
        let (before, after) = self.order.split_at(slot);
        let remain = |id: &&SourceId| !self.moved.contains(id);
        let moved = self.order.iter().filter(|id| self.moved.contains(id));
        let next: Vec<_> = before
            .iter()
            .filter(remain)
            .chain(moved)
            .chain(after.iter().filter(remain))
            .copied()
            .collect();
        (next != self.order).then_some(next)
    }

    pub fn position(&self, ui: &egui::Ui, content: egui::Rect, pitch: f32) -> Option<usize> {
        let clip = ui.clip_rect().intersect(content);
        let pointer = ui.input(|input| input.pointer.interact_pos())?;
        if !clip.contains(pointer) {
            return None;
        }
        let slot = (((pointer.x - content.left()) / pitch).round() as usize).min(self.order.len());
        if ui.input(|input| input.pointer.primary_down()) {
            // Scroll toward the pointer at the edges, including a partially shown card.
            let edge = 30.;
            let right = (1. - (clip.right() - pointer.x) / edge).clamp(0., 1.);
            let left = (1. - (pointer.x - clip.left()) / edge).clamp(0., 1.);
            let step = ui.input(|input| input.stable_dt).min(1. / 30.) * 480.;
            if right != left {
                ui.scroll_with_delta(egui::vec2((left - right) * step, 0.));
                ui.ctx().request_repaint();
            }
        }
        Some(slot)
    }

    pub fn paint(&self, ui: &egui::Ui, content: egui::Rect, pitch: f32, slot: usize, sort: Sort) {
        let ink = ui.visuals().selection.stroke.color;
        let x = (content.left() + slot as f32 * pitch - 6.).max(content.left() + 1.);
        let painter = ui.painter();
        painter.vline(x, content.y_range(), egui::Stroke::new(2., ink));
        for y in [content.top() + 1., content.bottom() - 1.] {
            painter.hline((x - 4.)..=(x + 4.), y, egui::Stroke::new(2., ink));
        }

        let Some(pointer) = ui.input(|input| input.pointer.interact_pos()) else {
            return;
        };
        let foreground = ui.ctx().layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            ui.id().with("photo-drag"),
        ));
        let label = if self.moved.len() > 1 {
            format!("{} photos", self.moved.len())
        } else {
            self.name.clone()
        };
        let label = if sort == Sort::Custom {
            label
        } else {
            format!("{label}\nDrop to set Custom order")
        };
        let galley = foreground.layout_no_wrap(
            label,
            egui::FontId::proportional(12.),
            ui.visuals().text_color(),
        );
        let preview_size = self.preview.as_ref().map_or(egui::Vec2::ZERO, |texture| {
            let size = texture.size_vec2();
            size * (116. / size.x).min(72. / size.y)
        });
        let size = egui::vec2(
            galley.size().x.max(preview_size.x) + 16.,
            galley.size().y + preview_size.y + 12.,
        );
        let x = (pointer.x - size.x / 2.).clamp(
            ui.ctx().content_rect().left(),
            (ui.ctx().content_rect().right() - size.x).max(ui.ctx().content_rect().left()),
        );
        let rect = egui::Rect::from_min_size(egui::pos2(x, content.top() - size.y - 6.), size);
        foreground.rect(
            rect,
            4.,
            ui.visuals().window_fill(),
            egui::Stroke::new(1., ink),
            egui::StrokeKind::Inside,
        );
        if let Some(texture) = &self.preview {
            let image = egui::Rect::from_center_size(
                egui::pos2(rect.center().x, rect.top() + 4. + preview_size.y / 2.),
                preview_size,
            );
            foreground.image(
                texture.id(),
                image,
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
                egui::Color32::WHITE,
            );
        }
        foreground.galley(
            egui::pos2(rect.left() + 8., rect.top() + 6. + preview_size.y),
            galley,
            ui.visuals().text_color(),
        );
    }
}

impl Editor {
    pub(super) fn reorder_available(&self) -> bool {
        !self.editing() && !self.workspace_input.is_blocked()
    }

    pub(super) fn drop_photo_order(&mut self, slot: usize) {
        let Some(drag) = self.document.roll_order.drag.take() else {
            return;
        };
        if !self.reorder_available() || !drag.current(&self.document.displayed_order()) {
            return;
        }
        let Some(order) = drag.reordered(slot) else {
            return;
        };
        let result = (|| {
            let mut edit = self.edit_project("Reorder photos")?;
            edit.update(|project| project.reorder_photos(order))?;
            self.commit_project(edit)
        })();
        match result {
            Ok(()) => self.document.roll_order.sort = Sort::Custom,
            Err(error) => self.status = error.to_string(),
        }
    }
}

#[cfg(test)]
mod tests;
