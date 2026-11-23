use super::*;
use crate::export::{BLOCK, FileFormat, Overwrite, Plan, RenderTarget};
use image::{ImageDecoder, codecs::jpeg::JpegDecoder};

fn plan(target: OutputTarget, quality: JpegQuality) -> Plan {
    Plan::new(RenderTarget::Display(target), FileFormat::Jpeg(quality)).unwrap()
}

#[test]
fn reopens_with_the_selected_icc_and_distinct_bands_through_partial_edges() {
    let directory = tempfile::tempdir().unwrap();
    let size = ImageSize {
        width: 139,
        height: 143,
    };
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        let path = directory.path().join(format!("{target:?}.jpg"));
        let mut visits = Vec::new();
        plan(target, JpegQuality::new(100).unwrap())
            .write(
                (&path, Overwrite::Refuse),
                size,
                (BlockOrder::RowMajor, BLOCK),
                |origin, extent| {
                    visits.push((origin, extent));
                    let colour = match origin {
                        [0, 0] => [192, 48, 32],
                        [128, 0] => [24, 112, 44],
                        [0, 128] => [64, 96, 216],
                        [128, 128] => [224, 180, 82],
                        origin => panic!("unexpected block at {origin:?}"),
                    };
                    Ok(Samples::Rgb8(vec![
                        colour;
                        (extent[0] * extent[1]) as usize
                    ]))
                },
                || Ok(()),
            )
            .unwrap();
        assert_eq!(
            visits,
            [
                ([0, 0], [128, 128]),
                ([128, 0], [11, 128]),
                ([0, 128], [128, 15]),
                ([128, 128], [11, 15])
            ]
        );
        let mut decoder =
            JpegDecoder::new(std::io::BufReader::new(File::open(&path).unwrap())).unwrap();
        assert_eq!(decoder.dimensions(), (size.width, size.height));
        assert_eq!(
            decoder.icc_profile().unwrap().as_deref(),
            Some(profiles::icc(target))
        );
        let mut pixels = vec![0; decoder.total_bytes() as usize];
        decoder.read_image(&mut pixels).unwrap();
        for (x, y, expected) in [
            (0, 0, [192, 48, 32]),
            (138, 0, [24, 112, 44]),
            (0, 142, [64, 96, 216]),
            (138, 142, [224, 180, 82]),
        ] {
            let index = 3 * (y * size.width + x) as usize;
            let actual = &pixels[index..index + 3];
            assert!(
                actual
                    .iter()
                    .zip(expected)
                    .all(|(v, expected)| v.abs_diff(expected) <= 3),
                "pixel ({x}, {y}): expected {expected:?}, got {actual:?}",
            );
        }
    }
}

#[test]
fn later_band_failure_preserves_typed_error_and_existing_file() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("existing.jpg");
    let size = ImageSize {
        width: 513,
        height: 263,
    };
    let quality = JpegQuality::default();
    std::fs::write(&path, b"existing destination").unwrap();
    let mut visits = 0;
    let result = plan(OutputTarget::Srgb, quality).write(
        (&path, Overwrite::Confirmed),
        size,
        (BlockOrder::RowMajor, BLOCK),
        |[x, y], extent| {
            visits += 1;
            if y != 0 {
                // The first band's compressed bytes must already have reached the temp file.
                let temp_bytes = std::fs::read_dir(directory.path())
                    .unwrap()
                    .map(|entry| entry.unwrap())
                    .find(|entry| entry.path() != path)
                    .unwrap()
                    .metadata()
                    .unwrap()
                    .len();
                assert!(temp_bytes > 8192);
                return Err(Error::Cancelled);
            }
            let samples = (0..extent[1])
                .flat_map(|row| {
                    (x..x + extent[0]).map(move |x| {
                        let value = x.wrapping_mul(131).wrapping_add(row.wrapping_mul(109));
                        [
                            value as u8,
                            (value >> 1) as u8,
                            (value ^ (value >> 6)) as u8,
                        ]
                    })
                })
                .collect();
            Ok(Samples::Rgb8(samples))
        },
        || panic!("failed producer must not reach publication"),
    );
    assert!(matches!(result, Err(Error::Cancelled)));
    assert_eq!(visits, 6, "no retry or request after the producer error");
    assert_eq!(std::fs::read(&path).unwrap(), b"existing destination");
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn quality_is_bounded_and_controls_the_written_quantization_tables() {
    assert!(JpegQuality::new(0).is_err());
    assert!(JpegQuality::new(101).is_err());
    let directory = tempfile::tempdir().unwrap();
    let first_quantizer = |quality| {
        let path = directory.path().join(format!("quality-{quality}.jpg"));
        plan(OutputTarget::Srgb, JpegQuality::new(quality).unwrap())
            .write(
                (&path, Overwrite::Refuse),
                ImageSize {
                    width: 1,
                    height: 1,
                },
                (BlockOrder::RowMajor, BLOCK),
                |_, _| Ok(Samples::Rgb8(vec![[32, 64, 96]])),
                || Ok(()),
            )
            .unwrap();
        let bytes = std::fs::read(path).unwrap();
        let mut offset = 2;
        loop {
            assert_eq!(bytes[offset], 0xff);
            let marker = bytes[offset + 1];
            assert_ne!(marker, 0xda, "missing quantization table");
            if marker == 0xdb {
                return bytes[offset + 5];
            }
            let len = u16::from_be_bytes([bytes[offset + 2], bytes[offset + 3]]);
            offset += 2 + usize::from(len);
        }
    };
    assert!(first_quantizer(1) > first_quantizer(100));
}

#[test]
fn rejects_oversized_jpeg_before_requesting_pixels() {
    let mut file = tempfile::tempfile().unwrap();
    for size in [
        ImageSize {
            width: 65536,
            height: 1,
        },
        ImageSize {
            width: 1,
            height: 65536,
        },
    ] {
        assert!(
            encode(
                &mut file,
                size,
                OutputTarget::Srgb,
                JpegQuality::default(),
                BLOCK,
                &mut |_, _| panic!("invalid dimensions must not request pixels"),
            )
            .is_err()
        );
        assert_eq!(file.metadata().unwrap().len(), 0);
    }
}
