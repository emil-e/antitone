use super::*;

impl Editor {
    pub(super) fn photo_tool_shelf(&mut self, ui: &mut egui::Ui, cx: &UiContext<'_, Self>) {
        use ui::icons::Icon;
        use ui::{ToolButton, ToolShelf};

        let cropping = self.cropping();
        let dusting = self.dusting();
        let tool_active = self.tool_interacting();
        let ready = !self.interaction_blocked() && self.selected_area().is_some();
        let commands_enabled = ready && !tool_active && !self.picking();
        let tool_ready = self.tool_start_available() && self.selected_area().is_some();
        let crop_enabled = cropping || tool_ready;
        let dust_enabled = dusting || tool_ready && self.document.project.input.ready();
        let repairs_changed = self
            .document
            .visit
            .current_source
            .and_then(|source| self.dust_repairs(source))
            .is_some_and(|set| !set.repairs().is_empty());
        let mut action = None;

        use editing::PhotoAction as Action;

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
                if cropping {
                    "Exit crop without applying the draft."
                } else {
                    "Edit this photo's crop and straighten."
                },
                cropping,
            )
            .enabled(crop_enabled)
            .changed(self.photo_crop().is_some())
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
            self.apply_photo_action(action, cx);
        }
    }
}
