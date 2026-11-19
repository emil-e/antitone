use super::*;

const STALE: &str = "Preview not updated — showing previous image: not enough graphics memory";
const UNAVAILABLE: &str = "Preview unavailable: not enough graphics memory";

fn snapshot(h: &mut Harness<'_, Editor>, name: &str) {
    h.event(egui::Event::PointerGone);
    h.try_run_realtime().unwrap();
    save_capture(&h.render().unwrap(), name);
}

fn edit_exposure(h: &mut Harness<'_, Editor>, value: &str) {
    let role = egui::accesskit::Role::SpinButton;
    h.get_by_role_and_label(role, "Exposure").scroll_to_me();
    h.try_run_realtime().unwrap();
    h.get_by_role_and_label(role, "Exposure").click();
    h.step();
    h.event(egui::Event::Key {
        key: egui::Key::A,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    });
    h.event(egui::Event::Text(value.into()));
    h.step();
    for event in ui::test_support::key(egui::Key::Enter) {
        h.event(event);
    }
    h.step();
}

fn assert_passive_status(h: &Harness<'_, Editor>, label: &str) {
    assert!(h.query_by_label(label).is_some());
    assert!(h.query_by_label("Retry").is_none());
    assert!(h.query_by_label("Details").is_none());
    let status = h.get_by_label(label).rect();
    h.read(|e| {
        let canvas = e.canvas_frame.layout.unwrap().0.canvas;
        assert!(
            status.top() >= canvas.bottom(),
            "failure belongs in the existing status bar"
        );
        assert!(
            e.document.visit.working_view.task.is_none(),
            "a failed demand must not spin retries"
        );
        assert!(!e.dialogs.is_open() && !e.interaction_blocked());
    });
}

fn exercise(mut h: Harness<'_, Editor>, size: egui::Vec2, name: &str) {
    h.set_size(size);
    h.step();
    stage(&mut h, "Photo");
    wait_for(&mut h, |e| {
        e.document.visit.working_view.support_ready()
            && e.document.visit.working_view.displayed.is_some()
    });
    h.render().unwrap();
    wait_for(&mut h, |e| e.document.visit.working_view.idle());
    let previous = h.read(|e| e.document.visit.working_view.displayed.clone().unwrap());
    h.update(|e| {
        e.document.visit.working_view.fail_next = Some(Error::GpuOutOfMemory(
            "Injected allocation failure; no real memory exhaustion.".into(),
        ));
        e.photo_settings_mut().exposure += 0.125;
    });
    h.run_steps(3);
    let accepted = h.read(|e| {
        assert!(matches!(
            e.document.visit.working_view.error,
            Some(Error::GpuOutOfMemory(_))
        ));
        assert!(
            e.document
                .visit
                .working_view
                .displayed
                .as_ref()
                .unwrap()
                .capture
                == previous.capture
        );
        e.document.project_snapshot()
    });
    assert_passive_status(&h, STALE);
    snapshot(&mut h, &format!("preview-status-{name}"));

    // Tabs and repaints leave the calculation unchanged and must not cause retries.
    stage(&mut h, "Roll");
    stage(&mut h, "Photo");
    h.run_steps(5);
    assert_passive_status(&h, STALE);
    h.read(|e| assert_eq!(e.document.project_snapshot(), accepted));

    edit_exposure(&mut h, "0.75");
    wait_for(&mut h, |e| {
        e.document.visit.working_view.support_ready()
            && e.document.visit.working_view.error.is_none()
            && !e.document.visit.working_view.updating
    });
    h.render().unwrap();
    assert!(h.query_by_label(STALE).is_none());
    h.read(|e| {
        assert_eq!(e.photo_settings().exposure, 0.75);
        assert!(
            e.document
                .visit
                .working_view
                .displayed
                .as_ref()
                .unwrap()
                .capture
                != previous.capture
        );
        assert!(!e.interaction_blocked());
    });

    // Failure without usable pixels also stays passive; an ordinary edit starts new work.
    h.update(|e| {
        e.document.visit.working_view.clear();
        e.active_canvas_mut().preview_rendered = None;
        e.document.visit.working_view.fail_next = Some(Error::GpuOutOfMemory(
            "Injected first-preview allocation failure".into(),
        ));
    });
    h.run_steps(3);
    assert_passive_status(&h, UNAVAILABLE);
    snapshot(&mut h, &format!("preview-status-unavailable-{name}"));
    edit_exposure(&mut h, "0.875");
    wait_for(&mut h, |e| {
        e.document.visit.working_view.support_ready()
            && e.document.visit.working_view.error.is_none()
            && !e.document.visit.working_view.updating
    });
    h.render().unwrap();
    assert!(h.query_by_label(UNAVAILABLE).is_none());
    h.read(|e| assert_eq!(e.photo_settings().exposure, 0.875));

    h.update(|e| {
        e.document.visit.working_view.fail_next = Some(Error::GpuOutOfMemory(
            "Injected failure before navigation".into(),
        ));
        e.photo_settings_mut().exposure += 0.125;
    });
    h.run_steps(3);
    assert_passive_status(&h, STALE);
    let (navigation, center) = h.read(|e| {
        (
            e.active_canvas().viewport,
            e.canvas_frame.layout.unwrap().0.canvas.center(),
        )
    });
    h.hover_at(center);
    h.event(egui::Event::Zoom(1.25));
    h.step();
    wait_for(&mut h, |e| {
        e.document.visit.working_view.support_ready()
            && e.document.visit.working_view.error.is_none()
            && !e.document.visit.working_view.updating
    });
    h.render().unwrap();
    assert!(h.query_by_label(STALE).is_none());
    h.read(|e| {
        assert_ne!(e.active_canvas().viewport, navigation);
        assert_eq!(e.photo_settings().exposure, 1.);
    });
}

#[test]
fn preview_failure_stays_in_status_bar_and_recovers_on_an_ordinary_edit() {
    for (size, name) in [
        (egui::vec2(1440., 900.), "wide"),
        (egui::vec2(800., 600.), "narrow"),
    ] {
        tools::calibrated_harness(|h| exercise(h, size, name));
    }
}

#[test]
#[ignore = "optional read-only photograph for preview-error screenshots"]
fn preview_failure_screenshots() {
    let path = PathBuf::from(
        std::env::var_os("ANTITONE_PREVIEW_SAMPLE").expect("ANTITONE_PREVIEW_SAMPLE"),
    );
    editor_harness_with_thumbnails(true, |mut h, _| {
        h.set_size(egui::vec2(1440., 900.));
        h.step();
        h.update(|e| e.document.project.input = crate::processing::InputSettings::Digital);
        h.enter(|e, cx| e.add_raws(vec![path], cx));
        wait_opened_photo(&mut h);
        wait_for(&mut h, |e| {
            e.document.visit.working_view.support_ready()
                && e.document.visit.working_view.displayed.is_some()
        });
        h.render().unwrap();
        wait_for(&mut h, |e| e.document.visit.working_view.idle());
        for (size, name) in [
            (egui::vec2(1440., 900.), "photo-wide"),
            (egui::vec2(800., 600.), "photo-narrow"),
        ] {
            h.update(|e| {
                e.document.visit.working_view.fail_next = Some(Error::GpuOutOfMemory(
                    "Injected allocation failure; no real memory exhaustion.".into(),
                ));
                e.photo_settings_mut().exposure += 0.125;
            });
            h.set_size(size);
            h.run_steps(3);
            assert_passive_status(&h, STALE);
            snapshot(&mut h, &format!("preview-status-{name}"));
        }
        h.update(|e| {
            e.document.visit.working_view.clear();
            e.active_canvas_mut().preview_rendered = None;
            e.document.visit.working_view.fail_next = Some(Error::GpuOutOfMemory(
                "Injected first-preview allocation failure".into(),
            ));
        });
        h.run_steps(3);
        assert_passive_status(&h, UNAVAILABLE);
        snapshot(&mut h, "preview-status-unavailable-photo-narrow");
    });
}
