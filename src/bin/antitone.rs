use antitone::{
    editor::{Editor, SmokeEditor},
    platform::host,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    if arguments.first().is_some_and(|v| v == "--smoke") {
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
        host::run(
            "Antitone — single-image prototype",
            move |device, queue| Ok(Editor::new(device, queue, initial)),
        )?;
    }
    Ok(())
}
