use super::*;
use crate::editor::tests::authored_fixture;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[test]
fn competing_destination_at_gpu_submission_is_preserved_for_skip_and_keep_both() {
    for collision in [Collision::Skip, Collision::KeepBoth] {
        let mut editor = authored_fixture(&["patches.dng"]);
        editor.dmin = Some(glam::DVec3::splat(0.8));
        editor.dmax = Some(glam::DVec3::splat(0.02));
        editor.crop = Some(crate::crop::Crop {
            center: [120., 100.],
            size: [16, 16],
            angle: 0.,
        });
        editor.sync_document();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("patches.exr");
        let mut settings = Draft::new(OutputTarget::Srgb, directory.path().to_owned());
        settings.kind = Kind::Exr;
        settings.collision = collision;
        let request = editor.batch_input(&settings).unwrap();
        let competing = path.clone();
        let armed = AtomicBool::new(true);
        // Source preparation is finished and authored_fixture disables thumbnails.
        // The next GPU submission is export, after its destination planning await.
        editor.submissions.observe_submissions(Arc::new(move || {
            if armed.swap(false, Ordering::Relaxed) {
                std::fs::write(&competing, b"competing writer").unwrap();
            }
        }));
        let services = editor.export_services.take().unwrap();
        drop(editor);
        let summary =
            pollster::block_on(services.batch(request, Cancellation::default(), |_, _| {}));
        assert_eq!(std::fs::read(path).unwrap(), b"competing writer");
        assert_eq!(summary.failed(), 0);
        if collision == Collision::KeepBoth {
            assert_eq!(summary.exported(), 1);
            assert_eq!(
                exr_size(&directory.path().join("patches (2).exr")),
                [16, 16]
            );
        } else {
            assert_eq!(summary.skipped(), 1);
            assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
        }
    }
}

#[test]
fn a_photo_specific_gpu_rejection_does_not_abort_later_exports() {
    let mut editor = fixture();
    let directory = tempfile::tempdir().unwrap();
    let mut request = editor.batch_input(&draft(directory.path())).unwrap();
    let mut mismatched_area = request.photos[0].recipe.composition.area();
    mismatched_area.width -= 1;
    request.photos[0].recipe.composition =
        crate::geometry::Composition::new(mismatched_area, Default::default(), Default::default())
            .unwrap();
    let services = editor.export_services.take().unwrap();
    drop(editor);
    let summary = pollster::block_on(services.batch(request, Cancellation::default(), |_, _| {}));
    assert_eq!(
        (summary.failed(), summary.exported(), summary.unattempted()),
        (1, 1, 0)
    );
    assert!(!summary.stopped);
    assert_eq!(exr_size(&directory.path().join("roll-1.exr")), [24, 12]);
}

#[test]
fn completed_photos_release_their_originals_before_the_batch_finishes() {
    let mut editor = fixture();
    let directory = tempfile::tempdir().unwrap();
    let request = editor.batch_input(&draft(directory.path())).unwrap();
    let Input::Ready(first) = &request.photos[0].input else {
        panic!("first photo must have a captured ready lease");
    };
    let first = Arc::downgrade(&first.source);
    let services = editor.export_services.take().unwrap();
    drop(editor);
    let mut checked = false;
    let summary =
        pollster::block_on(services.batch(request, Cancellation::default(), |done, _| {
            if done == 2 {
                assert!(
                    first.upgrade().is_none(),
                    "completed photos must not accumulate mandatory native leases"
                );
                checked = true;
            }
        }));
    assert_eq!(summary.exported(), 2);
    assert!(checked);
}

#[test]
#[cfg(unix)]
fn dangling_symlinks_are_existing_destinations_for_keep_both_and_skip() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("frame.tif");
    std::os::unix::fs::symlink(directory.path().join("missing"), &path).unwrap();
    assert_eq!(destination(&path, Collision::Skip, &[], &[]).unwrap(), None);
    assert_eq!(
        destination(&path, Collision::KeepBoth, &[], &[]).unwrap(),
        Some(directory.path().join("frame (2).tif"))
    );
    assert!(
        std::fs::symlink_metadata(path)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
#[cfg(unix)]
fn protected_destinations_are_never_replaced_or_followed_through_symlinks() {
    let directory = tempfile::tempdir().unwrap();
    let raw = directory.path().join("original.dng");
    let project = directory.path().join("grade.antitone");
    std::fs::write(&raw, b"raw").unwrap();
    std::fs::write(&project, b"project").unwrap();
    let link = directory.path().join("output.tif");
    std::os::unix::fs::symlink(&raw, &link).unwrap();
    for path in [&raw, &project, &link] {
        assert!(
            destination(
                path,
                Collision::Replace,
                &[raw.clone(), project.clone()],
                &[]
            )
            .is_err()
        );
    }
    assert_eq!(std::fs::read(raw).unwrap(), b"raw");
    assert_eq!(std::fs::read(project).unwrap(), b"project");
}

#[test]
fn filesystem_equivalent_unicode_names_never_replace_an_earlier_batch_output() {
    let mut editor = fixture();
    let directory = tempfile::tempdir().unwrap();
    let ids = editor.document.project.order.clone();
    for (index, stem) in ["Café", "Cafe\u{301}"].into_iter().enumerate() {
        let folder = directory.path().join(format!("source-{index}"));
        std::fs::create_dir(&folder).unwrap();
        let original = editor.document.source_path(ids[index]).unwrap();
        let copy = folder.join(format!("{stem}.dng"));
        std::fs::copy(original, &copy).unwrap();
        editor.document.project.source_mut(ids[index]).unwrap().path = copy;
    }
    let mut settings = draft(directory.path());
    settings.collision = Collision::Replace;
    let request = editor.batch_input(&settings).unwrap();
    let services = editor.export_services.take().unwrap();
    drop(editor);
    let summary = pollster::block_on(services.batch(request, Cancellation::default(), |_, _| {}));
    assert_eq!((summary.exported(), summary.failed()), (2, 0));
    let paths = summary
        .outcomes
        .iter()
        .filter_map(|outcome| match outcome {
            Outcome::Exported(path) => Some(path),
            _ => None,
        })
        .collect::<Vec<_>>();
    // Works on both normalization-sensitive and insensitive filesystems: each
    // reported file must still contain its own photograph after the whole batch.
    assert_eq!(exr_size(paths[0]), [16, 16]);
    assert_eq!(exr_size(paths[1]), [24, 12]);
}

#[test]
fn duplicate_basenames_are_disambiguated_even_when_replace_is_confirmed() {
    let mut editor = fixture();
    let directory = tempfile::tempdir().unwrap();
    let ids = editor.document.project.order.clone();
    // Distinct sources retain their interpretation/grades but have equal basenames.
    for (index, id) in ids.iter().enumerate() {
        let folder = directory.path().join(format!("input-{index}"));
        std::fs::create_dir(&folder).unwrap();
        let original = editor.document.source_path(*id).unwrap();
        let copy = folder.join("frame.dng");
        std::fs::copy(original, &copy).unwrap();
        editor.document.project.source_mut(*id).unwrap().path = copy;
    }
    let mut settings = draft(directory.path());
    settings.collision = Collision::Replace;
    let request = editor.batch_input(&settings).unwrap();
    assert_eq!(request.photos[0].path, directory.path().join("frame.exr"));
    assert_eq!(
        request.photos[1].path,
        directory.path().join("frame (2).exr")
    );
    let services = editor.export_services.take().unwrap();
    drop(editor);
    let summary = pollster::block_on(services.batch(request, Cancellation::default(), |_, _| {}));
    assert_eq!((summary.exported(), summary.failed()), (2, 0));
    let paths = summary
        .outcomes
        .iter()
        .filter_map(|outcome| match outcome {
            Outcome::Exported(path) => Some(path),
            _ => None,
        })
        .collect::<Vec<_>>();
    // Replace confirmation never collapses equal basenames: each reported file
    // must still contain its own photograph after the whole batch.
    assert_eq!(exr_size(paths[0]), [16, 16]);
    assert_eq!(exr_size(paths[1]), [24, 12]);
}
