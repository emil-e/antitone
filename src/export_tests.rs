use super::*;
use tiff::{
    decoder::{Decoder, DecodingResult},
    tags::CompressionMethod,
};

#[test]
fn spatial_block_visits_preserve_complete_files_and_public_row_major_order() {
    use std::collections::BTreeSet;
    for size in [
        ImageSize {
            width: 1,
            height: 1,
        },
        ImageSize {
            width: 129,
            height: 257,
        },
        ImageSize {
            width: 5248,
            height: 8128,
        },
    ] {
        let row: Vec<_> = blocks(size).collect();
        let public: Vec<_> = ordered_blocks(size, BlockOrder::RowMajor).collect();
        assert_eq!(row, public);
        let spatial: Vec<_> = ordered_blocks(size, BlockOrder::Spatial2x2).collect();
        let mut sorted = spatial.clone();
        sorted.sort_by_key(|(origin, _)| (origin[1], origin[0]));
        assert_eq!(row, sorted);
        let origins: BTreeSet<_> = spatial.iter().map(|(origin, _)| *origin).collect();
        assert_eq!(origins.len(), row.len());
    }
    let directory = tempfile::tempdir().unwrap();
    let size = ImageSize {
        width: 513,
        height: 257,
    };
    let mut bytes = None;
    for (i, order) in [BlockOrder::RowMajor, BlockOrder::Spatial2x2]
        .into_iter()
        .enumerate()
    {
        let path = directory.path().join(format!("order-{i}.tif"));
        tiff_encoded_ordered(
            &path,
            size,
            OutputTarget::Srgb,
            Overwrite::Refuse,
            Response::film().into(),
            order,
            |origin, extent| {
                Ok((origin[1]..origin[1] + extent[1])
                    .flat_map(|y| {
                        (origin[0]..origin[0] + extent[0]).map(move |x| {
                            [(x * 127 + y * 31) as u16, (y * 997) as u16, (x + y) as u16]
                        })
                    })
                    .collect())
            },
        )
        .unwrap();
        let data = fs::read(path).unwrap();
        if let Some(expected) = &bytes {
            assert_eq!(&data, expected);
        } else {
            bytes = Some(data);
        }
    }
}

#[test]
fn encoded_blocks_reopen_exactly_with_bounded_inputs_and_partial_edges() {
    type Codes = fn(u32, u32) -> [u16; 3];

    let directory = tempfile::tempdir().unwrap();
    let size = ImageSize {
        width: 513,
        height: 257,
    };
    let fixtures: [(&str, Codes); 3] = [
        ("flat", |_, _| [0, 32768, 65535]),
        ("ramp", |x, y| {
            [(x * 127 + y * 31) as u16, (y * 997) as u16, (x + y) as u16]
        }),
        ("noise", |x, y| {
            // Coordinate-keyed mixing keeps noisy inputs independent of block visitation.
            let mut state = (u64::from(y) << 32) | u64::from(x);
            std::array::from_fn(|_| {
                state = state.wrapping_add(0x9e3779b97f4a7c15);
                let mut word = state;
                word = (word ^ (word >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
                word = (word ^ (word >> 27)).wrapping_mul(0x94d049bb133111eb);
                (word ^ (word >> 31)) as u16
            })
        }),
    ];

    for (name, codes) in fixtures {
        for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
            let path = directory.path().join(format!("{name}-{target:?}.tif"));
            tiff_encoded(
                &path,
                size,
                target,
                Overwrite::Refuse,
                Response::film().into(),
                |origin, extent| {
                    assert!(extent[0] <= BLOCK[0] && extent[1] <= BLOCK[1]);
                    Ok((origin[1]..origin[1] + extent[1])
                        .flat_map(|y| (origin[0]..origin[0] + extent[0]).map(move |x| codes(x, y)))
                        .collect())
                },
            )
            .unwrap();
            let mut decoder = Decoder::new(fs::File::open(path).unwrap()).unwrap();
            assert_eq!(decoder.dimensions().unwrap(), (size.width, size.height));
            assert_eq!(
                decoder.get_tag_u32(Tag::Compression).unwrap(),
                u32::from(CompressionMethod::Deflate.to_u16())
            );
            assert_eq!(
                decoder.get_tag_u8_vec(Tag::Unknown(34675)).unwrap(),
                profiles::icc(target)
            );
            let DecodingResult::U16(actual) = decoder.read_image().unwrap() else {
                panic!("expected RGB16 for {name}, {target:?}")
            };
            let expected: Vec<_> = (0..size.height)
                .flat_map(|y| (0..size.width).flat_map(move |x| codes(x, y)))
                .collect();
            assert_eq!(actual, expected, "{name}, {target:?}");
        }
    }

    let pixel = |x: u32, y: u32| DVec3::new(x as f64 / 10. - 4., y as f64 / 7., -0.1);
    for precision in [ExrPrecision::Half, ExrPrecision::Full] {
        let path = directory.path().join(format!("{precision:?}.exr"));
        exr_blocks(
            &path,
            size,
            precision,
            Overwrite::Refuse,
            Response::film().into(),
            |origin, extent| {
                assert!(extent[0] <= BLOCK[0] && extent[1] <= BLOCK[1]);
                Ok((origin[1]..origin[1] + extent[1])
                    .flat_map(|y| (origin[0]..origin[0] + extent[0]).map(move |x| pixel(x, y)))
                    .collect())
            },
        )
        .unwrap();
        let image = read_first_flat_layer_from_file(path).unwrap();
        for channel in &image.layer_data.channel_data.list {
            let c = match channel.name.to_string().as_str() {
                "R" => 0,
                "G" => 1,
                "B" => 2,
                _ => panic!(),
            };
            for i in 0..size.count().unwrap() {
                let expected =
                    (ap1_to_ap0() * pixel(i as u32 % size.width, i as u32 / size.width))[c];
                match &channel.sample_data {
                    FlatSamples::F16(v) => assert_eq!(v[i], half::f16::from_f64(expected)),
                    FlatSamples::F32(v) => assert_eq!(v[i], expected as f32),
                    _ => panic!(),
                }
            }
        }
    }
}

#[test]
fn failed_blocks_and_competing_destinations_preserve_existing_files_and_clean_temporaries() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("output.tif");
    let size = ImageSize {
        width: 1,
        height: 1,
    };

    fn write(
        path: &Path,
        overwrite: Overwrite,
        block: impl FnMut([u32; 2], [u32; 2]) -> Result<Vec<[u16; 3]>>,
    ) -> Result<()> {
        tiff_encoded(
            path,
            ImageSize {
                width: 1,
                height: 1,
            },
            OutputTarget::Srgb,
            overwrite,
            Response::film().into(),
            block,
        )
    }
    assert!(
        write(&path, Overwrite::Refuse, |_, _| {
            fs::write(&path, b"competitor").unwrap();
            Ok(vec![[0; 3]])
        })
        .is_err()
    );
    assert_eq!(fs::read(&path).unwrap(), b"competitor");
    assert!(
        write(&path, Overwrite::Confirmed, |_, _| Err(Error::Gpu(
            "deliberate GPU failure".into()
        )))
        .is_err()
    );
    assert_eq!(fs::read(&path).unwrap(), b"competitor");
    assert!(write(&path, Overwrite::Confirmed, |_, _| Ok(Vec::new())).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"competitor");
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    write(&path, Overwrite::Confirmed, |_, _| Ok(vec![[1, 2, 3]])).unwrap();
    let mut decoder = Decoder::new(fs::File::open(&path).unwrap()).unwrap();
    let DecodingResult::U16(codes) = decoder.read_image().unwrap() else {
        panic!()
    };
    assert_eq!(codes, [1, 2, 3]);
    for pixel in [
        DVec3::splat(f64::NAN),
        DVec3::splat(f64::INFINITY),
        DVec3::splat(1e40),
    ] {
        assert!(
            exr(
                &path,
                size,
                ExrPrecision::Full,
                Overwrite::Confirmed,
                Response::film(),
                |_, _| Ok(pixel)
            )
            .is_err()
        );
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }
    assert!(
        exr(
            &path,
            size,
            ExrPrecision::Half,
            Overwrite::Confirmed,
            Response::film(),
            |_, _| Ok(DVec3::splat(70000.))
        )
        .is_err()
    );
    assert!(
        exr_blocks(
            &path,
            size,
            ExrPrecision::Full,
            Overwrite::Confirmed,
            Response::film().into(),
            |_, _| Ok(Vec::new())
        )
        .is_err()
    );
}
