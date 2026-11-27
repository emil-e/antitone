use super::*;
use serde_json::{Value, json};

mod lifecycle;

fn request(
    directory: &std::path::Path,
    project: &std::path::Path,
    existing: bool,
    photos: &[PathBuf],
) -> crate::editor::external_edit::Request {
    std::fs::create_dir_all(directory).unwrap();
    let wire = json!({
        "version": crate::editor::integrations::lightroom::PROTOCOL_VERSION,
        "mode": if existing { "open" } else { "new" },
        "project": existing.then_some(project),
        "photos": photos.iter().enumerate().map(|(index, path)| json!({
            "uuid": format!("catalog-photo-{index}"), "path": path,
        })).collect::<Vec<_>>(),
    });
    let file = directory.join("request.json");
    std::fs::write(&file, wire.to_string()).unwrap();
    crate::editor::integrations::lightroom::read(&file).unwrap()
}

fn originals(directory: &std::path::Path, names: &[&str]) -> Vec<PathBuf> {
    names
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let folder = directory.join(format!("original-{index}"));
            std::fs::create_dir_all(&folder).unwrap();
            let path = folder.join(name);
            std::fs::copy(fixture(name), &path).unwrap();
            path
        })
        .collect()
}

fn assert_return_header(harness: &Harness<'_, Editor>) {
    let rect = harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Send to Lightroom")
        .rect();
    let window = harness.ctx.content_rect();
    assert!(
        window.contains_rect(rect),
        "return action must remain visible"
    );
    assert!(
        window.right() - rect.right() < 24.,
        "return action must stay at the far right"
    );
    assert!(harness.query_by_label("From Lightroom").is_none());
}

fn button(harness: &mut Harness<'_, Editor>) {
    assert_return_header(harness);
    let button = harness.get_by_role_and_label(egui::accesskit::Role::Button, "Send to Lightroom");
    assert!(
        !button.accesskit_node().is_disabled(),
        "return must actually be available"
    );
    button.click();
    harness.step();
}

fn wait_sent(harness: &mut Harness<'_, Editor>) {
    wait_for(harness, |editor| !editor.document.external_edit.pending());
    harness.read(|editor| {
        assert!(!editor.dialogs.is_open(), "{}", editor.status);
        assert!(!editor.allow_close);
    });
}

fn tiff_dimensions(path: &std::path::Path) -> (u32, u32) {
    let mut decoder = tiff::decoder::Decoder::new(std::fs::File::open(path).unwrap()).unwrap();
    let dimensions = decoder.dimensions().unwrap();
    let tiff::decoder::DecodingResult::U16(samples) = decoder.read_image().unwrap() else {
        panic!("expected RGB16 return");
    };
    assert_eq!(
        samples.len(),
        dimensions.0 as usize * dimensions.1 as usize * 3
    );
    dimensions
}

fn returned_paths(receipt: &std::path::Path) -> Vec<PathBuf> {
    let wire: Value = serde_json::from_str(&std::fs::read_to_string(receipt).unwrap()).unwrap();
    wire["renders"]
        .as_array()
        .unwrap()
        .iter()
        .map(|render| PathBuf::from(render["path"].as_str().unwrap()))
        .collect()
}

#[test]
fn lightroom_new_roll_returns_rendered_photos_and_single_member_reopens_the_whole_roll() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("saved roll.antitone");
    let paths = originals(directory.path(), &["patches.dng", "roll-0.dng"]);
    let original_bytes = paths
        .iter()
        .map(|p| std::fs::read(p).unwrap())
        .collect::<Vec<_>>();
    let first = directory.path().join("first");
    let input = request(&first, &project, false, &paths);
    editor_harness(|mut h, _| {
        assert!(
            h.query_by_label("Send to Lightroom").is_none(),
            "ordinary documents have no return action"
        );
        h.enter(|editor, cx| editor.open_external(input, cx));
        wait_for(&mut h, |editor| !editor.interaction_blocked());
        h.run_steps(2);
        let height = h.ctx.content_rect().height();
        for width in [640., 1100.] {
            h.set_size(egui::vec2(width, height));
            h.run_steps(2);
            assert_return_header(&h);
        }
        assert!(
            h.get_by_role_and_label(egui::accesskit::Role::Button, "Send to Lightroom")
                .accesskit_node()
                .is_disabled(),
            "uncalibrated negatives cannot return a positive"
        );
        h.update(|editor| {
            editor.set_dmin(Some(DVec3::splat(0.8)));
            editor.set_dmax(Some(DVec3::splat(0.02)));
            editor.document.project.roll.exposure = 0.25;
            let ids = editor.document.project.order.clone();
            for (id, size) in ids.into_iter().zip([[16, 12], [20, 16]]) {
                editor.document.project.source_mut(id).unwrap().crop = Some(Crop {
                    center: [120., 100.],
                    size,
                    angle: 0.,
                });
            }
        });
        h.step();
        save_capture(&h.render().unwrap(), "lightroom-return-offscreen");
        button(&mut h);
        wait_sent(&mut h);
        h.read(|editor| assert!(editor.document.dirty()));
        assert!(!project.exists());
        assert!(!first.join("saved.json").exists());
        let first_render = returned_paths(&first.join("result.json"))[0].clone();
        let first_bytes = std::fs::read(&first_render).unwrap();
        let hdr = OutputTarget::Rec2020_2000;
        h.update(|editor| {
            editor.document.set_target(hdr);
            assert_eq!(editor.preview_target(), OutputTarget::DisplayP3);
            let id = editor.document.project.order[0];
            let crop = editor
                .document
                .project
                .source_mut(id)
                .unwrap()
                .crop
                .as_mut()
                .unwrap();
            crop.size = [24, 18];
        });
        h.step();
        button(&mut h);
        wait_sent(&mut h);
        let next = returned_paths(&first.join("result-2.json"));
        assert_eq!(next.len(), 2);
        assert_ne!(next[0], first_render);
        assert_eq!(tiff_dimensions(&next[0]), (24, 18));
        for (path, target) in [(&first_render, OutputTarget::DisplayP3), (&next[0], hdr)] {
            let mut tiff = tiff::decoder::Decoder::new(std::fs::File::open(path).unwrap()).unwrap();
            assert_eq!(
                tiff.get_tag_u8_vec(tiff::tags::Tag::Unknown(34675))
                    .unwrap(),
                crate::profiles::icc(target),
                "Send must use the project target independently of preview capability"
            );
        }
        assert_eq!(std::fs::read(first_render).unwrap(), first_bytes);
        h.enter(|editor, cx| {
            editor.save_project(project.clone(), crate::project::Save::New, None, cx)
        });
        wait_for(&mut h, |editor| !editor.document.files.pending());
        h.read(|editor| assert!(!editor.document.dirty()));
    });
    let returned = returned_paths(&first.join("result.json"));
    assert_eq!(returned.len(), 2);
    for (rendered, original) in returned.iter().zip(&paths) {
        assert_eq!(rendered.parent(), original.parent());
        assert_ne!(rendered.parent(), project.parent());
    }
    let first_b = returned[1].clone();
    let first_b_bytes = std::fs::read(&first_b).unwrap();
    assert_eq!(tiff_dimensions(&returned[0]), (16, 12));
    assert_eq!(tiff_dimensions(&returned[1]), (20, 16));
    let saved = crate::project::Project::load(&project).unwrap();
    assert_eq!(saved.order.len(), 2);
    assert_eq!(saved.roll.exposure, 0.25);

    let second = directory.path().join("second");
    let input = request(&second, &project, true, &paths[1..]);
    editor_harness(|mut h, _| {
        h.enter(|editor, cx| editor.open_external(input, cx));
        wait_for(&mut h, |editor| !editor.interaction_blocked());
        h.read(|editor| {
            assert_eq!(editor.document.project.order.len(), 2);
            assert_eq!(editor.document.active_index(), Some(1));
            assert_eq!(editor.document.project.roll.exposure, 0.25);
            assert_eq!(editor.document.project, saved);
        });
        // Browsing changes neither the requested source nor the return scope.
        h.enter(|editor, cx| editor.select_roll_photo(0, egui::Modifiers::NONE, cx));
        wait_for(&mut h, |editor| !editor.interaction_blocked());
        h.step();
        button(&mut h);
        wait_sent(&mut h);
    });
    let returned = returned_paths(&second.join("result.json"));
    assert_eq!(returned[0].parent(), paths[1].parent());
    assert_ne!(returned[0], first_b, "return creates a new TIFF version");
    assert_eq!(std::fs::read(first_b).unwrap(), first_b_bytes);
    assert_eq!(
        returned.len(),
        1,
        "opening the whole roll must not return the whole roll"
    );
    assert_eq!(
        tiff_dimensions(&returned[0]),
        (20, 16),
        "return the requested photo, not the photo browsed to"
    );
    let rendered_bytes = std::fs::read(&returned[0]).unwrap();
    editor_harness(|mut h, _| {
        h.enter(|editor, cx| editor.open_project(project.clone(), cx));
        wait_for(&mut h, |editor| !editor.document.files.pending());
        h.step();
        h.read(|editor| {
            assert!(editor.document.external_edit.session.is_none());
            assert_eq!(editor.document.project, saved);
        });
        assert!(h.query_by_label("Send to Lightroom").is_none());
        h.update(|editor| editor.document.project.roll.exposure = 0.75);
        h.enter(|editor, cx| {
            editor.save_project(project.clone(), crate::project::Save::Replace, None, cx)
        });
        wait_for(&mut h, |editor| !editor.document.write.pending());
        h.read(|editor| {
            assert!(!editor.dialogs.is_open(), "{}", editor.status);
            assert!(!editor.document.dirty());
            assert!(editor.document.external_edit.session.is_none());
        });
    });
    assert_eq!(
        crate::project::Project::load(&project)
            .unwrap()
            .roll
            .exposure,
        0.75
    );
    assert_eq!(
        std::fs::read(&returned[0]).unwrap(),
        rendered_bytes,
        "standalone Save does not update the returned TIFF"
    );
    for (path, original) in paths.iter().zip(original_bytes) {
        assert_eq!(std::fs::read(path).unwrap(), original, "RAW source changed");
    }
}

#[test]
fn lightroom_send_failure_leaves_the_project_unsaved_and_publishes_no_receipt() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("roll.antitone");
    let session = directory.path().join("session");
    // The source name fits the filesystem, but adding the return suffix does not.
    let source = directory.path().join(format!("{}.dng", "a".repeat(250)));
    std::fs::copy(fixture("patches.dng"), &source).unwrap();
    // Windows needs the verbatim absolute path for the native DNG reader.
    let source = source.canonicalize().unwrap();
    let input = request(&session, &project, false, std::slice::from_ref(&source));
    editor_harness(|mut h, _| {
        h.enter(|editor, cx| editor.open_external(input, cx));
        wait_for(&mut h, |editor| !editor.interaction_blocked());
        h.update(|editor| {
            let current = editor.document.visit.current_source.unwrap_or_else(|| {
                panic!(
                    "long-name RAW must open before testing return failure: {}",
                    editor.status
                )
            });
            editor.set_dmin(Some(DVec3::splat(0.8)));
            editor.set_dmax(Some(DVec3::splat(0.02)));
            editor.document.project.source_mut(current).unwrap().crop = Some(Crop {
                center: [120., 100.],
                size: [16, 16],
                angle: 0.,
            });
        });
        let oversized = directory
            .path()
            .join(format!("{}-Antitone.tif", "a".repeat(250)));
        assert!(
            std::fs::File::create(oversized).is_err(),
            "fixture must actually provoke a filesystem failure"
        );
        h.step();
        workflow::wait_settled(&mut h);
        button(&mut h);
        wait_for(&mut h, |editor| editor.dialogs.is_open());
        h.read(|editor| {
            assert!(!editor.allow_close);
            assert!(
                editor.document.dirty(),
                "failed send must not save the project"
            );
        });
        assert!(!project.exists());
        assert!(!session.join("saved.json").exists());
        assert!(!session.join("result.json").exists());
    });
}
