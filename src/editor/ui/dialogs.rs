use super::{
    TextAction,
    feedback::{Severity, notice_heading},
};
use egui::{Context, Id, Ui};
use std::{fmt::Debug, hash::Hash};

#[cfg(test)]
mod tests;

const BODY_WIDTH: f32 = 400.;
const WIDE_BODY_WIDTH: f32 = 500.;
const FRAME_MARGIN: i8 = 20;
const VIEWPORT_GUTTER: f32 = 16.;
const ITEM_GAP: f32 = 8.;
const FOOTER_GAP: f32 = 16.;
const ACTION_HEIGHT: f32 = 32.;
const ACTION_MIN_WIDTH: f32 = 64.;
const ACTION_PADDING: f32 = 11.;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::editor) enum DialogRole {
    Cancel,
    Secondary,
    Destructive,
    Primary,
}

pub(in crate::editor) struct DialogAction<'a, T> {
    value: T,
    role: DialogRole,
    button: TextAction<'a>,
}

impl<'a, T> DialogAction<'a, T> {
    pub(in crate::editor) fn new(value: T, label: &'a str, role: DialogRole) -> Self {
        let button = TextAction::new(label)
            .minimum_size(ACTION_MIN_WIDTH, ACTION_HEIGHT)
            .horizontal_padding(ACTION_PADDING);
        let button = match role {
            DialogRole::Primary => button.primary(),
            DialogRole::Destructive => button.destructive(),
            DialogRole::Cancel | DialogRole::Secondary => button,
        };
        Self {
            value,
            role,
            button,
        }
    }

    pub(in crate::editor) fn enabled(mut self, enabled: bool) -> Self {
        self.button = self.button.enabled(enabled);
        self
    }
}

/// Shared frame, scrollable body and persistent action footer. Roles don't assign keyboard focus.
pub(in crate::editor) struct Dialog<'a, T> {
    actions: &'a [DialogAction<'a, T>],
    width: f32,
    initial_focus: Option<T>,
    escape: Option<T>,
}

impl<'a, T: Copy + Eq + Hash + Debug> Dialog<'a, T> {
    pub(in crate::editor) fn new(actions: &'a [DialogAction<'a, T>]) -> Self {
        Self {
            actions,
            width: BODY_WIDTH,
            initial_focus: None,
            escape: None,
        }
    }

    pub(in crate::editor) fn wide(mut self) -> Self {
        self.width = WIDE_BODY_WIDTH;
        self
    }

    pub(in crate::editor) fn initial_focus(mut self, action: T) -> Self {
        self.initial_focus = Some(action);
        self
    }

    /// Use the pre-paint Escape availability, before an underlying popup can close.
    pub(in crate::editor) fn escape(mut self, available: bool, action: T) -> Self {
        self.escape = available.then_some(action);
        self
    }

    pub(in crate::editor) fn show<R>(
        self,
        context: &Context,
        id: Id,
        body: impl FnOnce(&mut Ui) -> R,
    ) -> (R, Option<T>) {
        let frame = egui::Frame::popup(&context.global_style()).inner_margin(FRAME_MARGIN);
        let response = egui::Modal::new(id).frame(frame).show(context, |ui| {
            let viewport = context.content_rect().size();
            let available =
                viewport - egui::Vec2::splat(2. * f32::from(FRAME_MARGIN) + VIEWPORT_GUTTER);
            let width = available.x.clamp(1., self.width);
            ui.set_width(width);
            ui.spacing_mut().item_spacing = egui::Vec2::splat(ITEM_GAP);

            let footer = Footer::new(ui, self.actions, width);
            let body_height = (available.y - footer.height()).max(1.);
            let content = egui::ScrollArea::vertical()
                .max_height(body_height)
                .min_scrolled_height(1.)
                .auto_shrink([false, true])
                .show(ui, body)
                .inner;
            (content, footer.show(ui, id, self.initial_focus))
        });

        // Centered areas use the previous measured size to position the next pass.
        let size = response.response.rect.size();
        let changed = context.data_mut(|data| {
            let key = id.with("measured-size");
            let previous = data.get_temp::<egui::Vec2>(key);
            data.insert_temp(key, size);
            previous != Some(size)
        });
        if changed {
            context.request_repaint();
        }

        let (content, clicked) = response.inner;
        // Unlike Modal::should_close, Escape never treats a backdrop click as dismissal.
        if let Some(escape) = self.escape
            && response.is_top_modal
            && !response.any_popup_open
            && context
                .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            return (content, Some(escape));
        }
        (content, clicked)
    }
}

struct Footer<'a, T> {
    actions: Vec<&'a DialogAction<'a, T>>,
    row_width: f32,
    stacked: bool,
}

impl<'a, T: Copy + Eq + Hash + Debug> Footer<'a, T> {
    fn new(ui: &Ui, actions: &'a [DialogAction<'a, T>], width: f32) -> Self {
        let row_width = actions
            .iter()
            .map(|action| action.button.width(ui))
            .sum::<f32>()
            + ITEM_GAP * actions.len().saturating_sub(1) as f32;
        let mut actions: Vec<_> = actions.iter().collect();
        let stacked = row_width > width && actions.len() > 1;
        if stacked {
            actions.sort_by_key(|action| std::cmp::Reverse(action.role));
        } else {
            actions.sort_by_key(|action| action.role);
        }
        Self {
            actions,
            row_width,
            stacked,
        }
    }

    fn height(&self) -> f32 {
        if self.actions.is_empty() {
            return 0.;
        }
        let rows = if self.stacked { self.actions.len() } else { 1 };
        FOOTER_GAP + ACTION_HEIGHT * rows as f32 + ITEM_GAP * (rows - 1) as f32
    }

    fn show(&self, ui: &mut Ui, id: Id, initial_focus: Option<T>) -> Option<T> {
        if self.actions.is_empty() {
            return None;
        }
        ui.add_space(FOOTER_GAP - ITEM_GAP);
        let layout = if self.stacked {
            egui::Layout::top_down(egui::Align::Max)
        } else {
            egui::Layout::left_to_right(egui::Align::Center)
        };
        let size = egui::vec2(ui.available_width(), self.height() - FOOTER_GAP);
        ui.allocate_ui_with_layout(size, layout, |ui| {
            if !self.stacked {
                ui.add_space((ui.available_width() - self.row_width).max(0.));
            }
            let mut focus = None;
            let mut clicked = None;
            for action in &self.actions {
                // Preserve action identity when the row changes its visual and tab order.
                let response = ui
                    .scope_builder(
                        egui::UiBuilder::new().id(id.with(("action", action.value))),
                        |ui| action.button.show(ui),
                    )
                    .inner;
                if response.clicked() {
                    clicked = Some(action.value);
                }
                if response.enabled() && initial_focus == Some(action.value) {
                    focus = Some(response);
                }
            }
            if let Some(response) = focus
                && dialog_focus(ui.ctx(), id).is_none()
                && !egui::Popup::is_any_open(ui.ctx())
            {
                response.request_focus();
            }
            clicked
        })
        .inner
    }
}

fn dialog_focus(context: &Context, id: Id) -> Option<Id> {
    let focused = context.memory(|memory| memory.focused())?;
    context
        .read_response(focused)
        .filter(|response| response.layer_id.id == id)
        .map(|_| focused)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DialogChoice {
    Cancel,
    Alternative,
    Confirm,
}

/// A blocking decision's presentation; its payload and dispatch remain caller-owned.
pub struct DecisionDialog<'a> {
    title: &'a str,
    explanation: &'a str,
    detail: Option<&'a str>,
    confirm: &'a str,
    confirm_role: DialogRole,
    discard: Option<&'a str>,
}

impl<'a> DecisionDialog<'a> {
    pub fn new(title: &'a str, explanation: &'a str, confirm: &'a str) -> Self {
        Self {
            title,
            explanation,
            detail: None,
            confirm,
            confirm_role: DialogRole::Primary,
            discard: None,
        }
    }

    pub fn detail(mut self, detail: &'a str) -> Self {
        self.detail = Some(detail);
        self
    }

    pub fn destructive(mut self) -> Self {
        self.confirm_role = DialogRole::Destructive;
        self
    }

    pub fn destructive_alternative(mut self, label: &'a str) -> Self {
        self.discard = Some(label);
        self
    }

    pub fn show(self, context: &Context, id: Id, escape_available: bool) -> Option<DialogChoice> {
        use DialogChoice::{Alternative, Cancel, Confirm};

        let mut actions = vec![
            DialogAction::new(Cancel, "Cancel", DialogRole::Cancel),
            DialogAction::new(Confirm, self.confirm, self.confirm_role),
        ];
        if let Some(discard) = self.discard {
            actions.push(DialogAction::new(
                Alternative,
                discard,
                DialogRole::Destructive,
            ));
        }
        let (_, choice) = Dialog::new(&actions)
            .initial_focus(Cancel)
            .escape(escape_available, Cancel)
            .show(context, id, |ui| {
                if self.confirm_role == DialogRole::Destructive {
                    notice_heading(ui, Severity::Warning, self.title);
                } else {
                    ui.heading(egui::RichText::new(self.title).strong());
                }
                ui.label(self.explanation);
                if let Some(detail) = self.detail {
                    ui.add(egui::Label::new(detail).wrap());
                }
            });
        // Stable decision IDs must not carry action focus into a later decision.
        if choice.is_some()
            && let Some(focused) = dialog_focus(context, id)
        {
            context.memory_mut(|memory| memory.surrender_focus(focused));
        }
        choice
    }
}
