use super::*;

#[test]
fn rgb16_late_status_errors_preserve_destination_and_remove_temporary() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("existing");
    let composition = Composition::new(
        [0, 0, 129, 1].into(),
        Default::default(),
        Default::default(),
    )
    .unwrap();
    for status in [1u32, 2] {
        std::fs::write(&path, b"keep existing bytes").unwrap();
        let mut blocks = 0;
        let mut published = false;
        let result = encode_with_checked(
            composition,
            Format::Tiff(OutputTarget::Srgb),
            (&path, Overwrite::Confirmed),
            |origin, extent| {
                blocks += 1;
                let mut words = vec![[0x55552222, 0x1111]; (extent[0] * extent[1]) as usize];
                if origin[0] != 0 {
                    words[0][1] |= status << 16;
                }
                Ok(OutputBlock::Rgb16 {
                    target: OutputTarget::Srgb,
                    words,
                })
            },
            || {
                published = true;
                Ok(())
            },
        );
        if status == 1 {
            assert!(matches!(result, Err(Error::Gpu(_))));
        } else {
            assert!(matches!(result, Err(Error::InvalidParameter(_))));
        }
        assert_eq!(blocks, 2, "failure follows a successful destination block");
        assert!(!published);
        assert_eq!(std::fs::read(&path).unwrap(), b"keep existing bytes");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }
    let result = encode_with_checked(
        composition,
        Format::Tiff(OutputTarget::Srgb),
        (&path, Overwrite::Confirmed),
        |_, extent| {
            Ok(OutputBlock::Rgb16 {
                target: OutputTarget::DisplayP3,
                words: vec![[0, 0]; (extent[0] * extent[1]) as usize],
            })
        },
        || panic!("mismatched output cannot reach publication"),
    );
    assert!(matches!(result, Err(Error::Gpu(_))));
    assert_eq!(std::fs::read(&path).unwrap(), b"keep existing bytes");
}
