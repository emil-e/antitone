use super::*;

fn noise(x: u32, y: u32) -> [u16; 3] {
    let mut word = (u64::from(y) << 32) | u64::from(x);
    std::array::from_fn(|_| {
        word = word.wrapping_add(0x9e3779b97f4a7c15);
        let mut value = word;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
        (value ^ (value >> 31)) as u16
    })
}

#[test]
fn codecs_emit_compressed_data_before_requesting_later_bands_and_preserve_producer_errors() {
    let size = ImageSize {
        width: 513,
        height: 513,
    };
    // Coordinate-keyed noise makes one completed band's payload larger than metadata and
    // buffering. A full-frame packer cannot emit that payload before the producer fails.
    for order in [BlockOrder::RowMajor, BlockOrder::Spatial2x2] {
        let mut file = tempfile::tempfile().unwrap();
        let observer = file.try_clone().unwrap();
        let stop = match order {
            BlockOrder::RowMajor => 128,
            BlockOrder::Spatial2x2 => 256,
        };
        let mut visited = 0;
        let error = stream::tiff(
            &mut file,
            size,
            OutputTarget::Srgb,
            order,
            &mut |origin, extent| {
                visited += 1;
                if origin[1] >= stop {
                    assert!(
                        observer.metadata().unwrap().len() > 32768,
                        "completed strips must reach the file before acquiring another band"
                    );
                    return Err(Error::Cancelled);
                }
                Ok((origin[1]..origin[1] + extent[1])
                    .flat_map(|y| (origin[0]..origin[0] + extent[0]).map(move |x| noise(x, y)))
                    .collect())
            },
        )
        .unwrap_err();
        assert!(matches!(error, Error::Cancelled));
        assert!(visited > 1);
    }
    for precision in [ExrPrecision::Half, ExrPrecision::Full] {
        let mut file = tempfile::tempfile().unwrap();
        let observer = file.try_clone().unwrap();
        let error = stream::exr(&mut file, size, precision, &mut |origin, extent| {
            if origin[1] >= 128 {
                assert!(
                    observer.metadata().unwrap().len() > 32768,
                    "completed ZIP16 chunks must precede the next input band"
                );
                return Err(Error::Cancelled);
            }
            Ok((origin[1]..origin[1] + extent[1])
                .flat_map(|y| {
                    (origin[0]..origin[0] + extent[0]).map(move |x| {
                        DVec3::from_array(noise(x, y).map(|v| f64::from(v) / 4096. - 4.))
                    })
                })
                .collect())
        })
        .unwrap_err();
        assert!(
            matches!(error, Error::Cancelled),
            "producer errors must survive the EXR callback boundary"
        );
    }
}

#[test]
fn large_output_workspace_is_independent_of_frame_height_and_admits_gfx_sized_full_float() {
    for (rows, bytes) in [(128, 6), (256, 6), (128, 12)] {
        let gfx = stream::workspace(
            ImageSize {
                width: 11648,
                height: 8736,
            },
            rows,
            bytes,
            10000,
        )
        .unwrap();
        let taller = stream::workspace(
            ImageSize {
                width: 11648,
                height: 17472,
            },
            rows,
            bytes,
            20000,
        )
        .unwrap();
        assert_eq!(gfx, taller, "only one band's samples are retained");
        assert!((gfx as u64) * bytes < 40 << 20);
    }
    assert!(
        stream::workspace(
            ImageSize {
                width: i32::MAX as u32,
                height: 128
            },
            128,
            12,
            0
        )
        .is_err()
    );
}
