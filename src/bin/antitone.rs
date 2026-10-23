use antitone::{editor::Editor, platform::host};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let argument = std::env::args().nth(1);
    if argument.as_deref() == Some("--version") {
        println!("Antitone {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let initial = argument.map(std::path::PathBuf::from);
    host::run("Antitone", move |device, queue, cx| {
        antitone::platform::install_icon()?;
        let mut editor = Editor::new(device, queue);
        if let Some(path) = initial {
            editor.open_initial(path, &cx);
        }
        editor.restore_ui_preferences();
        Ok(editor)
    })?;
    Ok(())
}
