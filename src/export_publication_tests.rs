use super::*;
use std::cell::Cell;

#[test]
fn competing_publication_has_a_typed_collision_and_never_replaces_the_winner() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("competing.tif");
    let result = atomic_file(
        &path,
        Overwrite::Refuse,
        |file| file.write_all(b"our output").map_err(failure),
        || fs::write(&path, b"competing output").map_err(failure),
    );
    assert_eq!(result, Err(Error::DestinationExists));
    assert_eq!(fs::read(&path).unwrap(), b"competing output");
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn final_stop_after_all_pixels_preserves_existing_destination_and_retires_temporary_files() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("existing");
    fs::write(&path, b"original destination").unwrap();
    for exr in [false, true] {
        let completed = Cell::new(false);
        let size = ImageSize {
            width: 3,
            height: 2,
        };
        let metadata = Response::film().into();
        let check = || {
            assert!(
                completed.get(),
                "publication guard is after the final pixel producer"
            );
            Err(Error::Export("cancelled before publication".into()))
        };
        let result = if exr {
            exr_blocks_checked(
                (&path, Overwrite::Confirmed),
                size,
                ExrPrecision::Full,
                metadata,
                |_, extent| {
                    completed.set(true);
                    Ok(vec![DVec3::splat(0.18); (extent[0] * extent[1]) as usize])
                },
                check,
            )
        } else {
            tiff_encoded_checked(
                (&path, Overwrite::Confirmed),
                size,
                OutputTarget::Srgb,
                metadata,
                BlockOrder::RowMajor,
                |_, extent| {
                    completed.set(true);
                    Ok(vec![[12345; 3]; (extent[0] * extent[1]) as usize])
                },
                check,
            )
        };
        assert!(result.is_err());
        assert_eq!(fs::read(&path).unwrap(), b"original destination");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}

#[test]
fn cancelled_completion_cannot_publish_a_ready_pixel_receipt() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("never-created.tif");
    let (result, pixels) = crate::export_timing::pixels::capture(|| {
        tiff_encoded_checked(
            (&path, Overwrite::Refuse),
            ImageSize {
                width: 3,
                height: 2,
            },
            OutputTarget::Srgb,
            Response::film().into(),
            BlockOrder::RowMajor,
            |_, extent| Ok(vec![[12345; 3]; (extent[0] * extent[1]) as usize]),
            || Err(Error::Export("completion cancelled".into())),
        )
    });
    assert!(result.is_err());
    assert!(pixels.is_none());
    assert!(!path.exists());
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
}
