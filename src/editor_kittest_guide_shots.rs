use super::*;
use std::path::Path;

/// Documentation screenshots for `guide/` and the README.
///
/// The test always drives the documented states on the synthetic fixtures, so the
/// documentation's views must stay reachable. PNGs are written only when
/// `ANTITONE_UI_CAPTURE` names an output directory. The committed screenshots come
/// from real scans via `ANTITONE_DOC_SCANS` (a directory of one roll's ARW files),
/// because the fixtures render test patterns rather than photographs:
///
/// ```sh
/// ANTITONE_DOC_SCANS=/absolute/roll ANTITONE_UI_CAPTURE=assets/docs \
///   cargo test --locked --features ui-tests \
///   --lib editor::kittest_tests::guide_shots -- --test-threads=1
/// ```
///
/// `assets/docs/README.md` owns the regeneration procedure; the capture also writes
/// `SOURCES.txt` recording what produced the images, so a stale set is recognisable.
#[test]
fn documented_states_render() {
    let scans = doc_scans();
    assert!(
        scans.len() >= 2,
        "the screenshots need at least two scans; ANTITONE_DOC_SCANS holds {}",
        scans.len()
    );
    let roll_size = scans.len();
    let first_label = file_label(&scans[0]);
    let second_label = file_label(&scans[1]);
    let last_label = file_label(scans.last().unwrap());
    let real_scans = std::env::var_os("ANTITONE_DOC_SCANS").is_some();
    // 1× keeps the committed screenshots small enough that regenerating them is an
    // ordinary commit; the builder sets density because runtime changes break
    // kittest event mapping.
    editor_harness_with_gpu(true, |mut harness, _, _, _| {
        harness.set_size(egui::vec2(1440., 900.));
        // The macOS host installs native menus, so the window has no in-window
        // menu strip; captures show what the downloaded app shows.
        harness.update(host::App::native_menu_installed);
        harness.step();
        harness.enter(|editor, cx| editor.add_raws(scans, cx));
        wait_for_with_timeout(&mut harness, Duration::from_secs(600), |e| {
            !e.busy && e.document_pending.is_none()
        });
        harness.read(|editor| assert!(editor.document.visit.ready.is_some(), "{}", editor.status));

        // The scans read from the emulsion side arrive top/bottom mirrored; the
        // shared scan baseline corrects all photos at once, before analysis.
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "Flip vertical")
            .click();
        harness.step();
        wait_for(&mut harness, |e| !e.busy);

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
        capture(&mut harness, "analyze-review");

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
        harness.read(|editor| assert!(editor.dmin().is_some(), "{}", editor.status));

        // The Photo tab with the second photograph selected. The canvas shows
        // the developed, roll-graded photograph only once the visit is ready,
        // and filmstrip cards paint graded positives only once development
        // lands; capturing earlier would record preview fallbacks instead of
        // the documented grade.
        harness.get_by_label(&second_label).click();
        harness.step();
        wait_for(&mut harness, |e| !e.busy);
        tab(&mut harness, "Photo");
        wait_for_with_timeout(&mut harness, Duration::from_secs(600), |e| {
            !e.busy && e.document.visit.ready.is_some()
        });
        wait_for_with_timeout(&mut harness, Duration::from_secs(600), |e| {
            e.thumbnails.ready_count(&e.document, e.target) == roll_size
        });
        settle_view(&mut harness);
        capture(&mut harness, "getting-started-filmstrip");

        // The viewer bar with its help popover open.
        harness.get_by_label("Viewer help").click();
        frames(&mut harness);
        assert!(egui::Popup::is_any_open(&harness.ctx));
        settle_view(&mut harness);
        capture(&mut harness, "viewer-bar");
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
        settle_view(&mut harness);
        capture(&mut harness, "export-dialog");
        harness.update(|editor| editor.export_dialog = None);

        // The overview last, back on the first photograph: both photographs are
        // developed by now, so the filmstrip and the canvas show graded
        // positives even for full-resolution scans.
        harness.get_by_label(&first_label).click();
        harness.step();
        wait_for_with_timeout(&mut harness, Duration::from_secs(600), |e| {
            !e.busy && e.document.visit.ready.is_some()
        });
        tab(&mut harness, "Roll");
        wait_for_with_timeout(&mut harness, Duration::from_secs(600), |e| {
            e.thumbnails.ready_count(&e.document, e.target) == roll_size
        });
        settle_view(&mut harness);
        capture(&mut harness, "window-overview");
    });
    write_capture_sources(&CaptureSources {
        scans: roll_size,
        first: &first_label,
        last: &last_label,
        real_scans,
    });
}

/// Records what produced the captured images. Review compares it with the UI changes
/// in the same commit: unchanged sources behind a changed screen means the screenshots
/// were not regenerated.
struct CaptureSources<'a> {
    scans: usize,
    first: &'a str,
    last: &'a str,
    real_scans: bool,
}

fn write_capture_sources(sources: &CaptureSources<'_>) {
    let Some(directory) = std::env::var_os("ANTITONE_UI_CAPTURE") else {
        return;
    };
    let input = if sources.real_scans {
        format!(
            "{} real scans, {} to {}",
            sources.scans, sources.first, sources.last
        )
    } else {
        format!("{} synthetic fixtures (not for publication)", sources.scans)
    };
    // A recorded commit cannot survive rebasing, and a capture always precedes the
    // commit that adds the images; the date changes whenever they are regenerated.
    let record = format!(
        "Antitone {}, captured {}\nWindow 1440x900, harness density\nInput: {input}\n\
         Regenerate with the procedure in assets/docs/README.md.\n",
        env!("CARGO_PKG_VERSION"),
        utc_date()
    );
    std::fs::write(Path::new(&directory).join("SOURCES.txt"), record).unwrap();
}

/// The capture date as `YYYY-MM-DD` (UTC), without pulling in a date library.
fn utc_date() -> String {
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        / 86_400;
    // Civil-date conversion from days since the 1970 epoch.
    let days = days as i64 + 719_468;
    let era = days / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
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

/// Waits until the canvas's working view matches the current positive
/// processing — the state the workflow actually leaves the view in. Capturing
/// earlier records the provisional preview instead of the graded view.
fn settle_view(harness: &mut Harness<'_, Editor>) {
    wait_for_with_timeout(harness, Duration::from_secs(600), |e| {
        assert!(e.working.error.is_none(), "{:?}", e.working.error);
        if e.working.revision().is_none() {
            return false;
        }
        let expected = e
            .positive_processing(e.input(), e.roll, e.adjustment_layer_settings)
            .unwrap();
        !e.working.updating
            && e.working.displayed.as_ref().is_some_and(|view| {
                view.processing == expected.signature() && view.target == e.target
            })
    });
    frames(harness);
}

/// Renders one documented state. The harness paints a pointer triangle at the last
/// clicked control, so the pointer is dropped first: the images show the window as a
/// reader sees it.
fn capture(harness: &mut Harness<'_, Editor>, name: &str) {
    harness.event(egui::Event::PointerGone);
    harness.step();
    save_capture(&harness.render().unwrap(), name);
}

fn tab(harness: &mut Harness<'_, Editor>, name: &str) {
    harness
        .get_by_role_and_label(egui::accesskit::Role::Tab, name)
        .click();
    frames(harness);
}
