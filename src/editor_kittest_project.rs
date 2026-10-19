use super::workflow::{enter_numeric, wait_settled};
use super::*;

fn compare_pixel(editor: &Editor) -> (DVec3, Vec<glam::Vec3>, Vec<glam::Vec3>) {
    let document = &editor.document;
    let id = editor.current_source.unwrap();
    let source = document.project.source(id).unwrap();
    let scan = Scan::open(
        &document.source_path(id).unwrap(),
        Some(document.project.development.as_ref().unwrap().fixed_wb),
    )
    .unwrap();
    let pixel = scan
        .pixel(
            scan.area.x + scan.area.width / 2,
            scan.area.y + scan.area.height / 2,
        )
        .unwrap();
    let working = crate::processing::Pipeline::new(
        document.project.calibration().unwrap().unwrap(),
        document.project.roll,
        source.photo,
        scan.camera_to_ap1,
    )
    .unwrap()
    .apply(pixel)
    .unwrap()
    .rgb;
    (
        working,
        crate::rendering::CpuRenderer::new(OutputTarget::Srgb)
            .unwrap()
            .render(&[working])
            .unwrap(),
        crate::rendering::CpuRenderer::new(OutputTarget::DisplayP3)
            .unwrap()
            .render(&[working])
            .unwrap(),
    )
}

#[test]
fn project_widgets_save_precision_order_wb_and_reject_open_atomically() {
    let directory = tempfile::tempdir().unwrap();
    let paths: Vec<_> = ["calibration.dng", "working.dng", "unvisited.dng"]
        .map(|name| {
            let path = directory.path().join(name);
            std::fs::copy(fixture("patches.dng"), &path).unwrap();
            path
        })
        .into();
    let (mut harness, _) = editor_harness_with_thumbnails(true);
    let custom = [2., 1., 1.5, 1.];
    let raw = crate::raw::RawFile::open_with_wb(&paths[0], Some(custom)).unwrap();
    harness.state_mut().document.project.development = Some(raw.metadata.development(custom));
    harness.state_mut().add_raws(paths.clone());
    wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
    assert!(
        harness.state().ready.is_some(),
        "{}",
        harness.state().status
    );
    let mut scan = Scan::open(&paths[0], Some(custom)).unwrap();
    {
        let e = harness.state_mut();
        e.dmin = Some(
            scan.reference(Rect {
                x: 16,
                y: 16,
                width: 32,
                height: 32,
            })
            .unwrap(),
        );
        e.dmax = Some(
            scan.reference(Rect {
                x: 80,
                y: 16,
                width: 32,
                height: 32,
            })
            .unwrap(),
        );
        e.source = false;
        e.sync_document();
    }
    let ids = harness.state().document.project.order.clone();
    harness.step();
    harness.get_by_label("working.dng").click();
    harness.step();
    wait_for(&mut harness, |e| !e.busy);
    assert_eq!(harness.state().current_source, Some(ids[1]));
    stage(&mut harness, "Photo");
    enter_numeric(&mut harness, "Exposure", "7.123456789012345");
    assert_eq!(harness.state().photo.exposure, 7.123456789012345);
    stage(&mut harness, "Roll");
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Toe & shoulder")
        .scroll_to_me();
    harness.try_run_realtime().unwrap();
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Toe & shoulder")
        .click();
    harness.step();
    enter_numeric(&mut harness, "Toe", "2.789123456789");
    enter_numeric(&mut harness, "Shoulder", "0.003123456789");
    assert_eq!(
        harness.state().roll.response,
        crate::processing::Response::Film {
            toe: 2.789123456789,
            shoulder: 0.003123456789
        }
    );
    // Supply a reordered roll to exercise source-owned Save/Open behavior.
    {
        let e = harness.state_mut();
        e.document.project.order.swap(0, 1);
        e.selected = 0;
        e.sync_document();
        let path = directory.path().join("film.antitone");
        e.document = e.document.save(&path, crate::project::Save::New).unwrap();
    }
    assert_eq!(
        harness
            .state()
            .document
            .project
            .source(ids[2])
            .unwrap()
            .photo,
        PhotoSettings::default()
    );
    stage(&mut harness, "Photo");
    enter_numeric(&mut harness, "Warmth", "0.1");
    assert!(harness.state().document.dirty());
    let expected = harness.state().document.project.clone();
    let pixel = compare_pixel(harness.state());
    save_existing_project(&mut harness);
    file_menu(&mut harness, "New");
    wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
    assert!(harness.state().document.project.development.is_none());
    assert!(harness.state().ready.is_none());
    harness.state_mut().add_raws(vec![paths[0].clone()]);
    wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
    let as_shot = crate::raw::RawFile::open(&paths[0])
        .unwrap()
        .metadata
        .as_shot_wb();
    assert_eq!(
        harness
            .state()
            .document
            .project
            .development
            .as_ref()
            .unwrap()
            .fixed_wb,
        as_shot
    );
    assert_ne!(as_shot, custom);
    let before_open = harness.state().document.project.clone();
    open_project(&mut harness, directory.path().join("film.antitone"));
    assert!(harness.state().transition.is_some());
    harness.get_by_label("Cancel").click();
    harness.step();
    assert_eq!(harness.state().document.project, before_open);
    open_project(&mut harness, directory.path().join("film.antitone"));
    harness.get_by_label("Discard changes").click();
    harness.step();
    wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
    assert_eq!(harness.state().document.project, expected);
    assert!(!harness.state().document.dirty());
    assert_eq!(harness.state().current_source, Some(ids[1]));
    assert_eq!(
        harness.state().ready.as_ref().unwrap().development.fixed_wb,
        custom
    );
    wait_settled(&mut harness);
    assert_eq!(compare_pixel(harness.state()), pixel);
    stage(&mut harness, "Rendering");
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        select_target(&mut harness, target);
        wait_settled(&mut harness);
        let timing = Arc::new(SelectionTiming::new(harness.state().id, Instant::now()));
        harness.state_mut().selection_timing = Some(timing.clone());
        harness.step();
        let png = harness.render().unwrap();
        assert!(timing.ready_was_painted());
        save_capture(
            &png,
            if target == OutputTarget::Srgb {
                "project-film-srgb-offscreen"
            } else {
                "project-film-p3-offscreen"
            },
        );
    }
    save_existing_project(&mut harness);
    let mut classic = harness.state().document.project.clone();
    classic.roll.response = crate::processing::Response::Original;
    let classic_path = directory.path().join("classic.antitone.json");
    classic
        .save(&classic_path, crate::project::Save::New)
        .unwrap();
    open_project(&mut harness, classic_path);
    wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
    assert_eq!(harness.state().document.project, classic);
    let classic_pixel = compare_pixel(harness.state());
    stage(&mut harness, "Photo");
    enter_numeric(&mut harness, "Tint", "-0.05");
    classic = harness.state().document.project.clone();
    let classic_pixel_edited = compare_pixel(harness.state());
    assert_ne!(classic_pixel_edited, classic_pixel);
    save_existing_project(&mut harness);
    open_project(&mut harness, directory.path().join("classic.antitone.json"));
    wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
    assert!(harness.state().roll.response.is_original());
    assert_eq!(harness.state().document.project, classic);
    assert!(!harness.state().document.dirty());
    wait_settled(&mut harness);
    assert_eq!(compare_pixel(harness.state()), classic_pixel_edited);
    let selected = harness.state().current_source;
    let ready = harness.state().ready.as_ref().unwrap().proxy.clone();
    for (name, id, missing) in [
        ("changed", ids[1], false),
        ("missing", ids[1], true),
        ("incompatible", ids[2], false),
    ] {
        let mut rejected = classic.clone();
        if missing {
            rejected.source_mut(id).unwrap().path = directory.path().join("missing.dng");
        } else {
            rejected.source_mut(id).unwrap().development.dimensions[0] += 1;
        }
        let path = directory.path().join(format!("{name}.antitone.json"));
        rejected.save(&path, crate::project::Save::New).unwrap();
        open_project(&mut harness, path);
        wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
        assert!(
            harness.state().status.contains(if missing {
                "missing.dng"
            } else {
                "development identity differs"
            }),
            "{}",
            harness.state().status
        );
        assert_eq!(harness.state().document.project, classic);
        assert_eq!(harness.state().current_source, selected);
        assert!(
            harness
                .state()
                .ready
                .as_ref()
                .unwrap()
                .proxy
                .same_source(&ready)
        );
        assert!(!harness.state().document.dirty());
    }
    wait_for(&mut harness, |e| {
        e.thumbnails.ready_count(&e.document, e.target) >= e.document.project.order.len().min(16)
    });
    assert_eq!(harness.state().document.project, classic);
    stage(&mut harness, "Photo");
    enter_numeric(&mut harness, "Exposure", "0.5");
    let dirty = harness.state().document.project.clone();
    let saved_path = harness.state().document.path.clone();
    let failed = directory.path().join("unwritable.antitone.json");
    std::fs::create_dir(&failed).unwrap();
    harness.state_mut().document.path = Some(failed);
    file_menu(&mut harness, "New");
    harness.step();
    assert!(harness.state().transition.is_some());
    let request = harness.state().next_request;
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Save changes")
        .click();
    harness.step();
    assert!(
        harness.state().next_request > request,
        "transition Save did not dispatch"
    );
    wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
    assert_eq!(harness.state().document.project, dirty);
    assert!(harness.state().document.dirty());
    assert_eq!(harness.state().transition, Some(files::Transition::New));
    assert!(harness.state().ready.is_some());
    assert_eq!(
        harness.state().failure.as_ref().unwrap().kind,
        FailureKind::Save
    );
    harness.get_by_label("OK").click();
    harness.step();
    harness.step();
    assert!(harness.state().failure.is_none());
    assert_eq!(harness.state().document.project, dirty);
    harness.get_by_label("Cancel").click();
    harness.step();
    harness.state_mut().document.path = saved_path;
    assert!(harness.state().transition.is_none());
    assert!(harness.state().preference_store.is_none());
}
