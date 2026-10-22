use super::*;
use std::path::Path;

/// Documentation screenshots for `guide/` and the README.
///
/// The test always drives the documented states on the synthetic fixtures, so the
/// documentation's views must stay reachable. PNGs are written only when
/// `ANTITONE_UI_CAPTURE` names an output directory, and are untracked artifacts —
/// publication screenshots come from real scans via `ANTITONE_DOC_SCANS` (a
/// directory of one roll's ARW files). Regenerate from the repository root with:
///
/// ```sh
/// ANTITONE_DOC_SCANS=/absolute/roll ANTITONE_UI_CAPTURE=assets/docs \
///   cargo test --locked --features ui-tests \
///   --lib editor::kittest_tests::guide_shots -- --test-threads=1
/// ```
///
/// `assets/docs/README.md` owns the workflow and the publishing boundary; the
/// output contains the supplied photographs and is never committed.
#[test]
fn documented_states_render() {
    let scans = doc_scans();
    let first_label = file_label(&scans[0]);
    let second_label = file_label(&scans[1]);
    // 2× density gives the captured screenshots retina resolution; the builder
    // keeps it for interaction as well.
    editor_harness_with_gpu_at(true, 2., |mut harness, _, _, _| {
        harness.set_size(egui::vec2(1440., 900.));
        harness.enter(|editor, cx| editor.add_raws(scans, cx));
        wait_for_with_timeout(&mut harness, Duration::from_secs(600), |e| {
            !e.busy && e.document_pending.is_none()
        });
        harness.read(|editor| assert!(editor.document.visit.ready.is_some(), "{}", editor.status));

        // Analyze's sampling-area review, entered immediately after loading like
        // the analysis tests do; interaction before it can change tool
        // availability.
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "Analyze")
            .scroll_to_me();
        frames(&mut harness);
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "Analyze")
            .click();
        harness.step();
        harness.step();
        wait_for_with_timeout(&mut harness, Duration::from_secs(600), |e| {
            e.document.analysis_review.is_some() || !e.busy
        });
        harness.read(|editor| {
            assert!(
                editor.document.analysis_review.is_some(),
                "{}",
                editor.status
            )
        });
        frames(&mut harness);
        save_capture(&harness.render().unwrap(), "analyze-review");

        // Confirm the review so every later shot shows the analyzed grade, the
        // state the workflow actually leaves the roll in.
        let mut buttons = harness
            .get_all_by_role_and_label(egui::accesskit::Role::Button, "Analyze roll")
            .filter(|node| !node.accesskit_node().is_disabled());
        buttons
            .next()
            .expect("enabled reviewed Analyze action")
            .click();
        drop(buttons);
        harness.step();
        wait_for_with_timeout(&mut harness, Duration::from_secs(600), |e| {
            !e.busy && e.document.analysis_review.is_none()
        });
        harness.read(|editor| assert!(editor.dmin.is_some(), "{}", editor.status));

        // The Photo tab with the second photograph selected.
        harness.get_by_label(&second_label).click();
        harness.step();
        wait_for(&mut harness, |e| !e.busy);
        tab(&mut harness, "Photo");
        settle(&mut harness);
        save_capture(&harness.render().unwrap(), "getting-started-filmstrip");

        // The viewer bar with its help popover open.
        harness.get_by_label("Viewer help").click();
        frames(&mut harness);
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
        frames(&mut harness);
        save_capture(&harness.render().unwrap(), "export-dialog");
        harness.update(|editor| editor.export_dialog = None);

        // The overview last: filmstrip thumbnails are sure to be developed and
        // painted by now, even for full-resolution scans.
        harness.get_by_label(&first_label).click();
        harness.step();
        wait_for(&mut harness, |e| !e.busy);
        tab(&mut harness, "Roll");
        settle(&mut harness);
        save_capture(&harness.render().unwrap(), "window-overview");
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

/// Paints a few frames without waiting for idle: full-resolution thumbnail work
/// keeps requesting repaints, so run-until-idle would exceed the harness step
/// bound while it streams.
fn frames(harness: &mut Harness<'_, Editor>) {
    for _ in 0..3 {
        harness.step();
    }
}

fn tab(harness: &mut Harness<'_, Editor>, name: &str) {
    harness
        .get_by_role_and_label(egui::accesskit::Role::Tab, name)
        .click();
    frames(harness);
}

/// Lets background thumbnail development and painting land before a capture.
/// Full-resolution development is served behind foreground work, so capture runs
/// wait generously; the assertions themselves do not depend on it.
fn settle(harness: &mut Harness<'_, Editor>) {
    let rounds = if std::env::var_os("ANTITONE_UI_CAPTURE").is_some() {
        60
    } else {
        4
    };
    for _ in 0..rounds {
        std::thread::sleep(Duration::from_millis(150));
        harness.step();
    }
}
