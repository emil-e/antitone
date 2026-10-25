use super::*;
#[path = "editor_export_collision_tests.rs"]
mod collisions;
#[path = "editor_export_content_tests.rs"]
mod content;
use crate::editor::{Thumbnails, tests::complete_document_operation};
use crate::{application::test_support::with_application, cancellation::Cancellation};
use std::time::Duration;

fn fixture() -> Editor {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let mut editor = Editor::new(&device, &queue);
        editor.thumbnails = Thumbnails::default();
        editor = complete_document_operation(editor, Duration::from_secs(30), |editor, cx| {
            editor.add_raws(
                ["roll-0.dng", "roll-1.dng"]
                    .map(|name| {
                        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                            .join("tests/fixtures/raw")
                            .join(name)
                    })
                    .to_vec(),
                cx,
            );
        });
        assert_eq!(editor.document.project.order.len(), 2, "{}", editor.status);
        editor.set_dmin(Some(glam::DVec3::splat(0.8)));
        editor.set_dmax(Some(glam::DVec3::splat(0.02)));
        editor.sync_document();
        for (index, id) in editor
            .document
            .project
            .order
            .clone()
            .into_iter()
            .enumerate()
        {
            let source = editor.document.project.source_mut(id).unwrap();
            source.photo.exposure = index as f64 * 0.75;
            source.crop = Some(crate::crop::Crop {
                center: [200., 200.],
                size: if index == 0 { [16, 16] } else { [24, 12] },
                angle: 0.,
            });
        }
        // Keep the active control mirror consistent with the accepted first source.
        let active = editor
            .document
            .project
            .source(editor.current_source.unwrap())
            .unwrap();
        editor.photo = active.photo;
        editor.crop = active.crop;
        editor
    })
}

fn draft(folder: &Path) -> Draft {
    let mut draft = Draft::new(OutputTarget::DisplayP3, folder.to_owned());
    draft.scope = Scope::All;
    draft.kind = Kind::Exr;
    draft
}

fn exr_size(path: &Path) -> [usize; 2] {
    let image = exr::prelude::read_first_flat_layer_from_file(path).unwrap();
    [image.layer_data.size.0, image.layer_data.size.1]
}

#[test]
fn filenames_preserve_dotted_stems_and_do_not_accept_path_injection() {
    assert_eq!(
        filename(Path::new("frame.v2.DNG"), "_positive", Kind::Tiff).unwrap(),
        PathBuf::from("frame.v2_positive.tif")
    );
    assert_eq!(
        numbered(Path::new("frame.v2.exr"), 2),
        PathBuf::from("frame.v2 (2).exr")
    );
    for suffix in ["/escape", "\\escape", ":escape", "\n"] {
        assert!(filename(Path::new("frame.dng"), suffix, Kind::Exr).is_err());
    }
}

#[test]
fn captured_multisource_batch_finishes_without_paint_after_document_retirement() {
    let mut editor = fixture();
    let directory = tempfile::tempdir().unwrap();
    let request = editor.batch_input(&draft(directory.path())).unwrap();
    assert!(
        request
            .photos
            .iter()
            .any(|photo| matches!(photo.input, Input::Acquire(_))),
        "fixture must exercise offscreen acquisition"
    );
    let first = directory.path().join("roll-0.exr");
    let second = directory.path().join("roll-1.exr");
    with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.start_export(request, cx);
            editor.document = crate::editor::document::Document::default().into();
        });
        app.run_until(Duration::from_secs(30), |editor| {
            editor.export_operation.pending.is_none()
        });
        app.read(|editor| {
            assert!(
                !editor.busy && editor.failure.is_none(),
                "{}",
                editor.status
            );
            let summary = editor.export_summary.as_ref().unwrap();
            assert_eq!(
                (summary.exported(), summary.failed(), summary.unattempted()),
                (2, 0, 0)
            );
            assert!(editor.document.project.order.is_empty());
        });
    });
    assert_eq!(exr_size(&first), [16, 16]);
    assert_eq!(exr_size(&second), [24, 12]);
}

#[test]
fn batch_selection_uses_filmstrip_order_and_never_exports_nonordered_sources() {
    let editor = fixture();
    let ids = editor.document.project.order.clone();
    with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.select_roll_photo(1, egui::Modifiers::NONE, cx);
            assert_eq!(editor.export_ids(Scope::Selected), vec![ids[1]]);
            assert_eq!(editor.export_ids(Scope::All), ids);
            editor.document.project.order.pop();
            assert_eq!(editor.export_ids(Scope::All), vec![ids[0]]);
            assert_eq!(
                editor.document.project.sources.len(),
                2,
                "retained source must exist outside filmstrip"
            );
        });
    });
}

#[test]
fn per_photo_acquisition_failure_does_not_discard_later_success() {
    let mut editor = fixture();
    let directory = tempfile::tempdir().unwrap();
    let mut request = editor.batch_input(&draft(directory.path())).unwrap();
    let mut unavailable = editor.source_input(directory.path().join("missing.dng"));
    unavailable.cache = None;
    request.photos[0].input = Input::Acquire(Box::new(unavailable));
    let services = editor.export_services.take().unwrap();
    drop(editor);
    let summary = pollster::block_on(services.batch(request, Cancellation::default(), |_, _| {}));
    assert_eq!(
        (summary.exported(), summary.failed(), summary.unattempted()),
        (1, 1, 0)
    );
    assert!(!directory.path().join("roll-0.exr").exists());
    assert_eq!(exr_size(&directory.path().join("roll-1.exr")), [24, 12]);
}

#[test]
fn cancellation_between_photos_preserves_published_output_and_reports_unattempted_work() {
    let mut editor = fixture();
    let directory = tempfile::tempdir().unwrap();
    let request = editor.batch_input(&draft(directory.path())).unwrap();
    let services = editor.export_services.take().unwrap();
    let cancellation = Cancellation::default();
    let stop = cancellation.clone();
    drop(editor);
    let summary = pollster::block_on(services.batch(request, cancellation, move |done, _| {
        if done == 1 {
            stop.cancel();
        }
    }));
    assert!(summary.stopped);
    assert_eq!(
        (summary.exported(), summary.failed(), summary.unattempted()),
        (1, 0, 1)
    );
    assert_eq!(exr_size(&directory.path().join("roll-0.exr")), [16, 16]);
    assert_eq!(
        std::fs::read_dir(directory.path()).unwrap().count(),
        1,
        "no later output or temporary file"
    );
}

#[test]
fn keep_both_and_skip_preserve_existing_destinations() {
    let mut editor = fixture();
    let directory = tempfile::tempdir().unwrap();
    for collision in [Collision::KeepBoth, Collision::Skip] {
        let folder = directory.path().join(format!("{collision:?}"));
        std::fs::create_dir(&folder).unwrap();
        let existing = folder.join("roll-0.exr");
        std::fs::write(&existing, b"existing destination").unwrap();
        let mut settings = draft(&folder);
        settings.collision = collision;
        let request = editor.batch_input(&settings).unwrap();
        let summary = pollster::block_on(editor.export_services.clone().unwrap().batch(
            request,
            Cancellation::default(),
            |_, _| {},
        ));
        assert_eq!(std::fs::read(&existing).unwrap(), b"existing destination");
        if collision == Collision::KeepBoth {
            assert_eq!((summary.exported(), summary.skipped()), (2, 0));
            assert_eq!(exr_size(&folder.join("roll-0 (2).exr")), [16, 16]);
        } else {
            assert_eq!((summary.exported(), summary.skipped()), (1, 1));
        }
    }
}
