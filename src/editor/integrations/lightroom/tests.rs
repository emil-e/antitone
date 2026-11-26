use super::*;
use serde_json::{Value, json};

fn wire(project: &Path, existing: bool, paths: &[PathBuf]) -> Value {
    json!({
        "version": PROTOCOL_VERSION,
        "mode": if existing { "open" } else { "new" },
        "project": existing.then_some(project),
        "photos": paths.iter().enumerate().map(|(index, path)| json!({
            "uuid": format!("photo-{index}"), "path": path,
        })).collect::<Vec<_>>(),
    })
}

fn request(project: &Path, receipt: &Path, existing: bool, paths: &[PathBuf]) -> Request {
    Request::parse(
        &wire(project, existing, paths).to_string(),
        receipt.to_owned(),
    )
    .unwrap()
}

#[test]
fn json_preserves_unicode_spaces_and_control_characters_in_locators() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("röll 'one'.antitone");
    let source = directory.path().join("frame\twith\nnewline.ARW");
    let parsed = request(
        &project,
        &directory.path().join("result.json"),
        true,
        std::slice::from_ref(&source),
    );
    assert_eq!(parsed.project.as_deref(), Some(project.as_path()));
    assert_eq!(parsed.photos[0].path, source);
    assert_eq!(parsed.photos[0].uuid, "photo-0");
    assert_eq!(parsed.mode, Mode::Open);

    // The Lua writer is independently compared with this fixture by its runner.
    let mut fixture: Value =
        serde_json::from_str(include_str!("../../../../tests/lightroom/request.json")).unwrap();
    if cfg!(windows) {
        for photo in fixture["photos"].as_array_mut().unwrap() {
            let relative = photo["path"].as_str().unwrap().strip_prefix('/').unwrap();
            photo["path"] = json!(directory.path().join(relative));
        }
    }
    let fixture =
        Request::parse(&fixture.to_string(), directory.path().join("result.json")).unwrap();
    assert_eq!(fixture.mode, Mode::New);
    assert_eq!(fixture.photos[0].uuid, "b");
    assert!(
        fixture.photos[0]
            .path
            .to_str()
            .unwrap()
            .contains("twö\tline\nframe")
    );
}

#[test]
fn json_rejects_ambiguous_identity_unknown_fields_and_relative_paths() {
    let directory = tempfile::tempdir().unwrap();
    let receipt = directory.path().join("result.json");
    let valid = wire(
        &directory.path().join("roll.antitone"),
        false,
        &[directory.path().join("raw.ARW")],
    );
    assert!(Request::parse(&valid.to_string(), receipt.clone()).is_ok());
    let changes = [
        ("photos", json!([])),
        ("photos", json!([valid["photos"][0], valid["photos"][0]])),
        (
            "photos",
            json!([{"uuid":"", "path":directory.path().join("raw.ARW")}]),
        ),
        ("photos", json!([{"uuid":"uuid", "path":"relative.ARW"}])),
        (
            "photos",
            json!([{"uuid":1, "path":directory.path().join("raw.ARW")}]),
        ),
        (
            "photos",
            json!([{"uuid":"uuid", "path":directory.path().join("raw.ARW"), "extra":true}]),
        ),
        ("photos", Value::Null),
        ("output", json!(directory.path())),
        ("receipt", json!(receipt)),
        (
            "photos",
            json!([
                {"uuid":"same", "path":directory.path().join("a.ARW")},
                {"uuid":"same", "path":directory.path().join("b.ARW")},
            ]),
        ),
        (
            "photos",
            json!([
                {"uuid":"a", "path":directory.path().join("same.ARW")},
                {"uuid":"b", "path":directory.path().join("same.ARW")},
            ]),
        ),
        ("mode", json!("unknown")),
        ("mode", json!("open")),
        ("version", json!(999)),
        ("version", json!("2")),
        ("relocated_from", json!("relative.antitone")),
        (
            "relocated_from",
            json!(directory.path().join("old.antitone")),
        ),
        ("project", json!("relative.antitone")),
        ("project", json!(directory.path().join("image.tif"))),
        ("project", json!(directory.path().join("roll.antitone"))),
    ];
    for (key, value) in changes {
        let mut invalid = valid.clone();
        invalid[key] = value;
        assert!(
            Request::parse(&invalid.to_string(), receipt.clone()).is_err(),
            "accepted {invalid}"
        );
    }
    for invalid in [
        valid
            .to_string()
            .replace("\"version\":5", "\"version\":5,\"version\":5"),
        valid.to_string() + " true",
        "{}".into(),
    ] {
        assert!(
            Request::parse(&invalid, receipt.clone()).is_err(),
            "accepted {invalid}"
        );
    }
    assert!(Request::parse(&valid.to_string(), PathBuf::from("result.json")).is_err());
}

#[test]
fn ordinary_save_links_a_new_lightroom_project_without_rendering_and_save_as_keeps_the_link() {
    use crate::application::test_support::TestApplication;
    use crate::editor::Editor;
    use std::{cell::RefCell, time::Duration};

    fn assert_header(editor: &Editor, expected: &str) {
        let context = egui::Context::default();
        context.enable_accesskit();
        let mut output = context.run_ui(egui::RawInput::default(), |ui| {
            editor.workspace_header(ui);
        });
        output.textures_delta.clear();
        let nodes = output.platform_output.accesskit_update.unwrap().nodes;
        assert!(nodes.iter().any(|(_, node)| node.value() == Some(expected)));
    }

    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("Lightroom roll.antitone");
    let copy = directory.path().join("project copy.antitone");
    let association = directory.path().join("saved.json");
    let returned = directory.path().join("result.json");
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng");
    let input = external_edit::Request::new(request(
        &project,
        &returned,
        false,
        std::slice::from_ref(&source),
    ));
    let owner = RefCell::new(Some(Editor::state()));
    let mut app = TestApplication::new(&owner, || {});
    app.read(|editor| assert_header(editor, "Untitled"));
    app.enter(|editor, cx| editor.open_external(input, cx));
    app.run_until(Duration::from_secs(10), |editor| {
        editor.document.external_edit.session.is_some() || editor.dialogs.is_open()
    });
    app.read(|editor| {
        assert!(
            editor.document.external_edit.session.is_some(),
            "{}",
            editor.status
        );
        assert!(editor.document.path.is_none());
        assert_header(editor, "Untitled • Modified");
    });

    assert!(!association.exists());
    app.enter(|editor, cx| {
        editor.save_project(project.clone(), crate::project::Save::New, None, cx);
    });
    app.run_until(Duration::from_secs(10), |editor| {
        !editor.document.files.pending()
    });
    let saved: Value = serde_json::from_slice(&std::fs::read(&association).unwrap()).unwrap();
    assert_eq!(saved["project"], json!(project));
    assert_eq!(saved["sources"], json!([{"id": "1", "path": source}]));
    assert!(saved.get("renders").is_none());
    app.read(|editor| assert_header(editor, "Lightroom roll.antitone"));

    app.enter(|editor, cx| {
        editor.save_project(copy.clone(), crate::project::Save::New, None, cx);
    });
    app.run_until(Duration::from_secs(10), |editor| {
        !editor.document.files.pending()
    });
    app.read(|editor| assert_header(editor, "project copy.antitone"));
    let association_after: Value =
        serde_json::from_slice(&std::fs::read(association).unwrap()).unwrap();
    assert_eq!(saved, association_after);
    assert!(!returned.exists());
    app.close_execution();
}

#[test]
fn opening_a_saved_project_rejects_requested_sources_outside_its_roll() {
    use external_edit::Input as _;
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("roll.antitone");
    let mut editor = crate::editor::tests::fixture();
    crate::editor::tests::with_registered_photo(&mut editor);
    let mut source = editor.document.project.sources[0].clone();
    let mut document = Document::default();
    for name in ["a.ARW", "b.ARW", "c.ARW"] {
        source.path = directory.path().join(name);
        document.register(source.clone()).unwrap();
    }
    document.save(&project, crate::project::Save::New).unwrap();
    let before = std::fs::read(&project).unwrap();
    let receipt = directory.path().join("result.json");
    let request = request(
        &project,
        &receipt,
        true,
        &[
            directory.path().join("a.ARW"),
            directory.path().join("d.ARW"),
        ],
    );
    let error = Box::new(request)
        .prepare(
            &crate::cancellation::Cancellation::default(),
            &mut |_, _| {},
        )
        .err()
        .unwrap();
    assert!(
        error
            .to_string()
            .contains("photo is not in this saved project"),
        "{error}"
    );
    assert_eq!(std::fs::read(&project).unwrap(), before);
    assert!(!receipt.exists());
}

#[test]
fn locating_a_moved_project_preserves_relative_originals_and_saved_grading() {
    use external_edit::Input as _;
    let directory = tempfile::tempdir().unwrap();
    let old_folder = directory.path().join("old");
    let new_folder = directory.path().join("moved");
    std::fs::create_dir(&old_folder).unwrap();
    std::fs::create_dir(&new_folder).unwrap();
    let old = old_folder.join("roll.antitone");
    let moved = new_folder.join("renamed.antitone");
    let originals = [old_folder.join("a.ARW"), old_folder.join("b.ARW")];
    let mut editor = crate::editor::tests::fixture();
    crate::editor::tests::with_registered_photo(&mut editor);
    let mut source = editor.document.project.sources[0].clone();
    let mut document = Document::default();
    document.path = Some(old.clone());
    for (index, path) in originals.iter().enumerate() {
        std::fs::write(path, "original").unwrap();
        source.path = PathBuf::from(if index == 0 { "a.ARW" } else { "b.ARW" });
        document.register(source.clone()).unwrap();
    }
    document.project.roll.exposure = 0.75;
    document.save(&old, crate::project::Save::New).unwrap();
    assert!(
        crate::project::Project::load(&old).unwrap().sources[0]
            .path
            .is_relative()
    );
    std::fs::rename(&old, &moved).unwrap();
    let receipt = directory.path().join("result.json");
    let plain = request(&moved, &receipt, true, &originals[..1]);
    assert!(
        Box::new(plain)
            .prepare(&Default::default(), &mut |_, _| {})
            .is_err()
    );
    let mut value = wire(&moved, true, &originals[..1]);
    value["relocated_from"] = json!(old);
    let input = Request::parse(&value.to_string(), receipt).unwrap();
    let opened = Box::new(input)
        .prepare(&Default::default(), &mut |_, _| {})
        .unwrap();
    assert_eq!(opened.document.project.roll.exposure, 0.75);
    assert_eq!(opened.document.project.order.len(), 2);
    for (id, path) in opened.document.project.order.iter().zip(&originals) {
        assert_eq!(
            opened
                .document
                .source_path(*id)
                .unwrap()
                .canonicalize()
                .unwrap(),
            path.canonicalize().unwrap()
        );
    }
    assert!(opened.document.dirty(), "rebased locators need a real Save");
    assert!(
        !old.exists(),
        "recovery never recreates the missing project silently"
    );
    let saved = opened
        .document
        .save(&moved, crate::project::Save::Replace)
        .unwrap();
    let reopened = Document::open(&moved).unwrap();
    assert_eq!(reopened.project.roll, saved.project.roll);
    assert!(
        reopened.find_source(&originals[1]).is_some(),
        "unrequested roll members also retain their locators"
    );
}

fn save_for_session(
    destination: &Destination,
    document: &Document,
    path: &Path,
    policy: crate::project::Save,
) -> Result<Document, Error> {
    let (saved, notification) = crate::editor::save::Write {
        document: document.clone(),
        path: path.to_owned(),
        policy,
    }
    .execute(Some(destination))?;
    notification?;
    Ok(saved)
}

#[test]
fn save_creates_and_updates_the_canonical_project_without_retargeting_a_save_as_copy() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("roll.antitone");
    let copy = directory.path().join("copy.antitone");
    let destination = Destination::new(request(
        &project,
        &directory.path().join("result.json"),
        false,
        &[directory.path().join("source.ARW")],
    ));
    let document = save_for_session(
        &destination,
        &Document::default(),
        &project,
        crate::project::Save::New,
    )
    .unwrap();
    assert_eq!(document.path.as_ref(), Some(&project));
    assert!(!document.dirty());
    assert!(project.is_file());
    let mut document = document;
    document.project.roll.exposure = 0.5;
    let mut document = document.save(&copy, crate::project::Save::New).unwrap();
    document.project.roll.exposure = 0.75;
    let canonical = destination.project.get().unwrap();
    assert_eq!(canonical, &project);
    let saved = save_for_session(
        &destination,
        &document,
        canonical,
        crate::project::Save::Replace,
    )
    .unwrap();
    assert_eq!(saved.path.as_ref(), Some(&project));
    assert!(!saved.dirty());
    assert_eq!(
        crate::project::Project::load(&project)
            .unwrap()
            .roll
            .exposure,
        0.75
    );
    assert_eq!(
        crate::project::Project::load(&copy).unwrap().roll.exposure,
        0.5
    );
}

#[test]
fn a_new_canonical_destination_is_never_replaced_before_this_invocation_saves_it() {
    use external_edit::Destination as _;
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("roll.antitone");
    let destination = Destination::new(request(
        &project,
        &directory.path().join("result.json"),
        false,
        &[directory.path().join("source.ARW")],
    ));
    std::fs::write(&project, "unrelated file created while editing").unwrap();
    use crate::project::Save;
    assert!(save_for_session(&destination, &Document::default(), &project, Save::New).is_err());
    assert!(destination.project.get().is_none());
    assert!(!directory.path().join("saved.json").exists());
    assert_eq!(
        std::fs::read_to_string(&project).unwrap(),
        "unrelated file created while editing"
    );
    std::fs::remove_file(&project).unwrap();
    let saved = Document::default()
        .save(&project, crate::project::Save::New)
        .unwrap();
    destination.saved(&saved).unwrap();
    let mut copy = saved
        .save(
            &directory.path().join("copy.antitone"),
            crate::project::Save::New,
        )
        .unwrap();
    copy.project.roll.exposure = 0.5;
    assert!(
        save_for_session(
            &destination,
            &copy,
            destination.project.get().unwrap(),
            Save::Replace
        )
        .is_ok(),
        "ordinary canonical Save grants replacement even after Save As"
    );
}

#[test]
fn send_receipts_are_independent_and_project_receipt_preserves_lossless_source_ids() {
    use external_edit::Destination as _;
    let directory = tempfile::tempdir().unwrap();
    let mut editor = crate::editor::tests::fixture();
    crate::editor::tests::with_registered_photo(&mut editor);
    let mut source = editor.document.project.sources[0].clone();
    let mut document = Document::default();
    source.path = directory.path().join("a.ARW");
    let a = document.register(source.clone()).unwrap();
    source.path = directory.path().join("b.ARW");
    let b = document.register(source).unwrap();
    let large = crate::project::SourceId(u64::MAX);
    document.project.source_mut(b).unwrap().id = large;
    *document
        .project
        .order
        .iter_mut()
        .find(|id| **id == b)
        .unwrap() = large;
    let b = large;
    document.path = Some(directory.path().join("roll.antitone"));
    let destination = Destination::new(request(
        document.path.as_ref().unwrap(),
        &directory.path().join("result.json"),
        true,
        &[directory.path().join("b.ARW")],
    ));
    let mut unsaved = document.clone();
    unsaved.path = None;
    let rendered = external_edit::Rendered {
        source: b,
        path: directory.path().join("positive.tif"),
    };
    destination
        .publish(&unsaved, std::slice::from_ref(&rendered))
        .unwrap();
    let first = &destination.request.receipt;
    let text = std::fs::read_to_string(first).unwrap();
    let receipt: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        receipt,
        json!({"version": PROTOCOL_VERSION, "renders": [{
            "uuid": "photo-0", "original": directory.path().join("b.ARW"), "path": rendered.path
        }]})
    );
    let saved = directory.path().join("saved.json");
    assert!(!saved.exists());
    let next = directory.path().join("result-2.json");
    assert!(destination.publish(&unsaved, &[]).is_err());
    assert!(!next.exists());
    let second = external_edit::Rendered {
        source: b,
        path: directory.path().join("different.tif"),
    };
    destination.publish(&unsaved, &[second]).unwrap();
    assert!(next.exists());
    assert_eq!(std::fs::read_to_string(first).unwrap(), text);

    destination.saved(&document).unwrap();
    let saved: Value = serde_json::from_slice(&std::fs::read(saved).unwrap()).unwrap();
    assert_eq!(saved["sources"].as_array().unwrap().len(), 2);
    assert_eq!(saved["sources"][0]["id"], a.0.to_string());
    assert_eq!(saved["sources"][1]["id"], b.0.to_string());
    assert!(saved.get("renders").is_none());
}
