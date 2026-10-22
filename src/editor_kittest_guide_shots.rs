use super::*;
use std::path::Path;

/// Documentation screenshots for `guide/` and the README.
///
/// The test always drives the documented states, so the documentation's views must
/// stay reachable. The PNGs are written only when `ANTITONE_UI_CAPTURE` names an
/// output directory. Regenerate from the repository root with:
///
/// ```sh
/// ANTITONE_UI_CAPTURE=assets/docs cargo test --locked --features ui-tests \
///   --lib editor::kittest_tests::guide_shots -- --test-threads=1
/// ```
///
/// Committed screenshots use the synthetic fixtures in `tests/fixtures/raw/`, which
/// contain no photograph or camera sample. `ANTITONE_DOC_SCANS` (a directory of ARW
/// files) substitutes real scans: only do that for screenshots intended for
/// publication, because the output then contains those photographs and cannot be
/// regenerated from the checkout.
#[test]
fn documented_states_render() {
    let scans = doc_scans();
    let second_label = file_label(&scans[1]);
    // 2× density gives the committed screenshots retina resolution; the harness
    // keeps it for interaction as well.
    editor_harness_with_gpu_at(true, 2., |mut harness, _, _, _| {
        harness.set_size(egui::vec2(1440., 900.));
        harness.enter(|editor, cx| editor.add_raws(scans, cx));
        wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
        harness.read(|editor| assert!(editor.document.visit.ready.is_some(), "{}", editor.status));
        // Pump frames so filmstrip thumbnails develop and paint before capture.
        for _ in 0..8 {
            std::thread::sleep(Duration::from_millis(50));
            harness.run();
        }
        save_capture(&harness.render().unwrap(), "window-overview");

        // Analyze's sampling-area review, entered immediately after loading like the
        // analysis tests do; interaction before it can change tool availability.
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "Analyze")
            .scroll_to_me();
        harness.try_run_realtime().unwrap();
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "Analyze")
            .click();
        harness.step();
        harness.step();
        wait_for(&mut harness, |e| {
            e.document.analysis_review.is_some() || !e.busy
        });
        harness.read(|editor| {
            assert!(
                editor.document.analysis_review.is_some(),
                "{}",
                editor.status
            )
        });
        harness.run();
        save_capture(&harness.render().unwrap(), "analyze-review");
        harness.event(escape());
        harness.step();
        harness.read(|editor| assert!(editor.document.analysis_review.is_none()));

        // The Photo tab with the second photograph selected.
        harness.get_by_label(&second_label).click();
        harness.step();
        wait_for(&mut harness, |e| !e.busy);
        stage(&mut harness, "Photo");
        harness.run();
        save_capture(&harness.render().unwrap(), "getting-started-filmstrip");

        // The viewer bar with its help popover open.
        harness.get_by_label("Viewer help").click();
        harness.run();
        assert!(egui::Popup::is_any_open(&harness.ctx));
        save_capture(&harness.render().unwrap(), "viewer-bar");
        harness.event(escape());
        harness.step();
        assert!(!egui::Popup::is_any_open(&harness.ctx));

        // The export dialog.
        harness.update(|editor| {
            editor.export_dialog = Some(export_batch::Draft::new(
                OutputTarget::Srgb,
                PathBuf::from("/exports"),
            ));
        });
        harness.run();
        save_capture(&harness.render().unwrap(), "export-dialog");
    });
}

fn doc_scans() -> Vec<PathBuf> {
    match std::env::var_os("ANTITONE_DOC_SCANS") {
        Some(directory) => {
            let mut scans: Vec<_> = std::fs::read_dir(directory)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .filter(|path| {
                    path.extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("arw"))
                })
                .collect();
            scans.sort();
            assert!(
                !scans.is_empty(),
                "ANTITONE_DOC_SCANS contains no ARW files"
            );
            scans
        }
        None => vec![fixture("roll-0.dng"), fixture("roll-1.dng")],
    }
}

fn file_label(path: &Path) -> String {
    path.file_name().unwrap().to_string_lossy().into_owned()
}

fn escape() -> egui::Event {
    super::super::presentation::tests::key(egui::Key::Escape).remove(0)
}
