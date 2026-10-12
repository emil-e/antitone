use antitone::{
    editor::{Editor, FlatFieldSmoke, SmokeEditor},
    platform::host,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    if arguments.first().is_some_and(|v| v == "--smoke-flat-field") {
        let result = std::sync::Arc::new(std::sync::Mutex::new(None));
        let report = result.clone();
        let presented = host::run("Antitone — flat-field smoke", move |device, queue| {
            let mut smoke = FlatFieldSmoke::new(device, queue, &arguments[1..])?;
            smoke.result = report;
            Ok(smoke)
        })?;
        match result.lock().unwrap().take() {
            Some(Ok(())) => {}
            Some(Err(error)) => return Err(error.into()),
            None => return Err("flat-field smoke closed before completing acceptance".into()),
        }
        if presented == 0 {
            return Err("editor smoke acquired no visible frame".into());
        }
        println!("flat-field editor submitted_frames={presented}");
    } else if arguments
        .first()
        .is_some_and(|v| v == "--smoke" || v == "--smoke-project")
    {
        let result = std::sync::Arc::new(std::sync::Mutex::new(None));
        let report = result.clone();
        let presented = host::run("Antitone — editor smoke", move |device, queue| {
            let mut smoke = SmokeEditor::new(device, queue, &arguments[1..])?;
            smoke.result = report;
            Ok(smoke)
        })?;
        if let Some(error) = result.lock().unwrap().take() {
            return Err(error.into());
        }
        if presented == 0 {
            return Err("editor smoke acquired no visible frame".into());
        }
        println!("editor submitted_frames={presented}");
    } else {
        let initial = arguments.first().map(std::path::PathBuf::from);
        host::run("Antitone", move |device, queue| {
            let mut editor = Editor::new(device, queue, initial);
            editor.restore_ui_preferences();
            Ok(editor)
        })?;
    }
    Ok(())
}
