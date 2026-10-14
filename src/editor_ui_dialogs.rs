use super::{
    TextAction,
    feedback::{Severity, notice_heading},
};
use egui::{Context, Id, Ui};

#[cfg(test)]
#[path = "editor_ui_dialog_tests.rs"]
mod tests;

const BODY_WIDTH: f32 = 400.;
const FRAME_MARGIN: i8 = 20;
const ITEM_GAP: f32 = 8.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    warning: bool,
    confirm: TextAction<'a>,
    alternative: Option<TextAction<'a>>,
}

impl<'a> DecisionDialog<'a> {
    pub fn new(title: &'a str, explanation: &'a str, confirm: &'a str) -> Self {
        Self {
            title,
            explanation,
            detail: None,
            warning: false,
            confirm: TextAction::new(confirm).primary(),
            alternative: None,
        }
    }

    pub fn detail(mut self, detail: &'a str) -> Self {
        self.detail = Some(detail);
        self
    }

    pub fn warning(mut self) -> Self {
        self.warning = true;
        self
    }

    pub fn alternative(mut self, label: &'a str) -> Self {
        self.alternative = Some(TextAction::new(label));
        self
    }

    fn actions(&self, ui: &mut Ui) -> Option<DialogChoice> {
        let cancel = TextAction::new("Cancel");
        let width = cancel.width(ui)
            + self.confirm.width(ui)
            + self.alternative.map_or(0., |action| action.width(ui))
            + ITEM_GAP * if self.alternative.is_some() { 2. } else { 1. };
        let stacked = width > ui.available_width();
        let draw = |ui: &mut Ui, action: TextAction<'_>| {
            if stacked {
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        action.show(ui)
                    })
                    .inner
                })
                .inner
            } else {
                action.show(ui)
            }
        };
        let show = |ui: &mut Ui| {
            if !stacked {
                ui.add_space((ui.available_width() - width).max(0.));
            }
            let cancel = draw(ui, cancel);
            let alternative = self.alternative.map(|action| draw(ui, action));
            let confirm = draw(ui, self.confirm);
            focus_default(ui, &cancel);
            if cancel.clicked() {
                Some(DialogChoice::Cancel)
            } else if alternative.is_some_and(|response| response.clicked()) {
                Some(DialogChoice::Alternative)
            } else if confirm.clicked() {
                Some(DialogChoice::Confirm)
            } else {
                None
            }
        };
        if stacked {
            ui.vertical(show).inner
        } else {
            ui.horizontal(show).inner
        }
    }

    /// Capture Escape availability before drawing underlying popovers, not after they may close.
    pub fn show(self, context: &Context, id: Id, escape_available: bool) -> Option<DialogChoice> {
        let response = modal(context, id, |ui| {
            if self.warning {
                notice_heading(ui, Severity::Warning, self.title);
            } else {
                ui.heading(egui::RichText::new(self.title).strong());
            }
            ui.label(self.explanation);
            if let Some(detail) = self.detail {
                ui.add(egui::Label::new(detail).wrap());
            }
            ui.add_space(8.);
            self.actions(ui)
        });
        let choice = if escape_requested(&response, escape_available) {
            Some(DialogChoice::Cancel)
        } else {
            response.inner
        };
        // Stable dialog IDs must not carry action focus into a later decision.
        if choice.is_some()
            && let Some(focused) = context.memory(|memory| memory.focused())
            && context
                .read_response(focused)
                .is_some_and(|focused_response| {
                    focused_response.layer_id == response.response.layer_id
                })
        {
            context.memory_mut(|memory| memory.surrender_focus(focused));
        }
        choice
    }
}

pub(super) fn modal<R>(
    context: &Context,
    id: Id,
    contents: impl FnOnce(&mut Ui) -> R,
) -> egui::ModalResponse<R> {
    egui::Modal::new(id)
        .frame(frame(context))
        .show(context, |ui| {
            body(ui);
            contents(ui)
        })
}

pub(super) fn frame(context: &Context) -> egui::Frame {
    egui::Frame::popup(&context.global_style()).inner_margin(FRAME_MARGIN)
}

pub(super) fn body(ui: &mut Ui) {
    let width = (ui.ctx().content_rect().width() - 2. * f32::from(FRAME_MARGIN) - 16.)
        .clamp(1., BODY_WIDTH);
    ui.set_width(width);
    ui.spacing_mut().item_spacing = egui::vec2(ITEM_GAP, ITEM_GAP);
}

pub(super) fn focus_default(ui: &Ui, action: &egui::Response) {
    let focused = ui.memory(|memory| memory.focused());
    let focused_here = focused
        .and_then(|id| ui.ctx().read_response(id))
        .is_some_and(|response| response.layer_id == ui.layer_id());
    if !focused_here && !egui::Popup::is_any_open(ui.ctx()) {
        action.request_focus();
    }
}

pub(super) fn escape_requested<R>(response: &egui::ModalResponse<R>, available: bool) -> bool {
    // Modal::should_close also accepts backdrop clicks; these messages need explicit dismissal.
    available
        && response.is_top_modal
        && !response.any_popup_open
        && response
            .response
            .ctx
            .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
}
