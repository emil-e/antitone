use super::{Editor, Error, RollOrientation, UiContext, auto_crop, ui};

pub(super) fn tool_shelf(editor: &mut Editor, ui: &mut egui::Ui, cx: &UiContext<'_, Editor>) {
    use ui::icons::Icon;
    use ui::{ToolButton, ToolShelf};
    let reviewing = editor.document.analysis_review().is_some();
    let analyzing = editor.document.analysis_operation().is_some();
    let capabilities = editor.document.project.input.capabilities();
    let commands = !editor.interaction_blocked()
        && editor.selected_area().is_some()
        && !editor.tool_interacting()
        && !editor.dialogs.is_open();

    ToolShelf::show(ui, |ui| {
        for (icon, name, help, action) in [
            (
                Icon::TurnLeft,
                "Turn left",
                "Turn the shared scan baseline 90° left.",
                Orientation::Left,
            ),
            (
                Icon::TurnRight,
                "Turn right",
                "Turn the shared scan baseline 90° right.",
                Orientation::Right,
            ),
            (
                Icon::FlipHorizontal,
                "Flip horizontal",
                "Flip all scans left/right.",
                Orientation::Horizontal,
            ),
            (
                Icon::FlipVertical,
                "Flip vertical",
                "Flip all scans top/bottom.",
                Orientation::Vertical,
            ),
            (
                Icon::Reset,
                "Reset scan orientation",
                "Reset the shared scan orientation.",
                Orientation::Reset,
            ),
        ] {
            if ToolButton::command(icon, name, help)
                .enabled(
                    commands
                        && (!matches!(action, Orientation::Reset)
                            || !editor.document.project.orientation.is_identity()),
                )
                .show(ui)
                .clicked()
            {
                apply_orientation(editor, action);
            }
        }
        if capabilities.auto_crop || capabilities.roll_analysis || reviewing {
            ui.separator();
        }
        if capabilities.auto_crop {
            let cropping = editor
                .current_tool()
                .is_some_and(|tool| tool.is::<auto_crop::AutoCropTool>());
            if ToolButton::tool(
                Icon::BatchCrop,
                "Auto-crop roll",
                "Propose crop and straightening for the whole roll.",
                cropping,
            )
            .enabled(cropping || auto_crop::review_available(editor))
            .show(ui)
            .clicked()
            {
                if cropping {
                    editor.cancel_active_tool();
                } else {
                    auto_crop::start_review(
                        editor,
                        editor.document.project.order.clone(),
                        "Whole roll",
                        cx,
                    );
                }
            }
        }
        if capabilities.roll_analysis || reviewing {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let help = if analyzing {
                    "Exit analysis without applying the proposal."
                } else {
                    "Analyze the roll to estimate a starting grade from reviewed sampling areas."
                };
                if ToolButton::tool(Icon::Analyze, "Analyze", help, analyzing)
                    .enabled(editor.tool_start_available() && editor.selected_area().is_some())
                    .emphasized()
                    .show(ui)
                    .clicked()
                {
                    if analyzing {
                        editor.cancel_active_tool();
                    } else {
                        editor.request_analysis(cx);
                    }
                }
            });
        }
    });
}

enum Orientation {
    Left,
    Right,
    Horizontal,
    Vertical,
    Reset,
}

fn apply_orientation(editor: &mut Editor, action: Orientation) {
    let result = (|| -> Result<(), Error> {
        let mut draft = editor.edit_project("Roll orientation")?;
        draft.update(|project| match action {
            Orientation::Left => project.orientation.turn_counterclockwise(),
            Orientation::Right => project.orientation.turn_clockwise(),
            Orientation::Horizontal => project.orientation.horizontal ^= true,
            Orientation::Vertical => project.orientation.vertical ^= true,
            Orientation::Reset => project.orientation = RollOrientation::default(),
        });
        editor.commit_project(draft)
    })();
    if let Err(error) = result {
        editor.status = format!("Orientation rejected: {error}");
        return;
    }
    editor.fit_photo_view();
}
