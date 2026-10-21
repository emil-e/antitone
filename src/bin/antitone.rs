use antitone::{editor::Editor, platform::host};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let initial = std::env::args().nth(1).map(std::path::PathBuf::from);
    host::run("Antitone", move |device, queue, cx| {
        antitone::platform::macos::set_application_icon()?;
        let mut editor = Editor::new(device, queue);
        if let Some(path) = initial {
            editor.open_initial(path, &cx);
        }
        editor.restore_ui_preferences();
        Ok(editor)
    })?;
    Ok(())
}
