use egui::{Response, Ui};

/// egui 0.36 retains cancelled text for a second lost-focus pass; reject both writes.
pub(in crate::editor) fn numeric_cancelled(ui: &Ui, response: &Response) -> bool {
    let id = response.id.with("cancelled-numeric-edit");
    let cancelled = ui
        .ctx()
        .data_mut(|data| data.remove_temp::<bool>(id))
        .unwrap_or(false);
    let cancelling = ui.input(|input| input.key_pressed(egui::Key::Escape))
        && (response.lost_focus() || ui.memory(|memory| memory.had_focus_last_frame(response.id)));
    if cancelling {
        ui.ctx().data_mut(|data| data.insert_temp(id, true));
    }
    cancelled || cancelling
}

/// Reports input activity in this paint; features own edit lifetime and completion.
pub(in crate::editor) fn values_are_being_adjusted(context: &egui::Context) -> bool {
    context.data(|data| data.get_temp::<u64>(egui::Id::new("value-adjustment-frame")))
        == Some(context.cumulative_frame_nr())
}

pub(super) fn control_active(ui: &Ui, response: &Response) -> bool {
    let keyboard = response.has_focus()
        && (ui.ctx().text_edit_focused()
            || ui.input(|input| {
                [
                    egui::Key::ArrowLeft,
                    egui::Key::ArrowRight,
                    egui::Key::ArrowUp,
                    egui::Key::ArrowDown,
                ]
                .into_iter()
                .any(|key| input.key_down(key))
            }));
    response.enabled()
        && ((response.is_pointer_button_down_on()
            && ui.input(|input| input.pointer.primary_down()))
            || response.dragged_by(egui::PointerButton::Primary)
            || keyboard)
}

pub(super) fn record_control_interaction(ui: &Ui, response: &Response) {
    if control_active(ui, response) {
        let frame = ui.ctx().cumulative_frame_nr();
        ui.ctx()
            .data_mut(|data| data.insert_temp(egui::Id::new("value-adjustment-frame"), frame));
    }
}

/// Interaction reported by a value widget; feature code owns its edit and value.
#[derive(Clone, Copy)]
pub(in crate::editor) struct ControlResponse {
    pub id: egui::Id,
    pub active: bool,
    pub cancelled: bool,
    pub changed: bool,
    pub enabled: bool,
}

impl ControlResponse {
    /// A checkbox or other instantaneous widget has no unfinished gesture to retain.
    pub(in crate::editor) fn discrete(response: &Response) -> Self {
        Self {
            id: response.id,
            active: false,
            cancelled: false,
            changed: response.changed(),
            enabled: response.enabled(),
        }
    }

    pub(in crate::editor) fn action(id: egui::Id, changed: bool) -> Self {
        Self {
            id,
            active: false,
            cancelled: false,
            changed,
            enabled: true,
        }
    }

    pub(in crate::editor) fn idle(id: egui::Id) -> Self {
        Self {
            enabled: false,
            ..Self::action(id, false)
        }
    }
}

#[cfg(test)]
mod tests;
