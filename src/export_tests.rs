use super::*;
use tiff::decoder::{Decoder, DecodingResult};

#[test]
fn encoded_blocks_reopen_exactly_with_bounded_inputs_and_partial_edges() {
    let directory = tempfile::tempdir().unwrap();
    let size = ImageSize {
        width: 513,
        height: 257,
    };
    let codes = |x: u32, y: u32| [(x * 127 + y * 31) as u16, (y * 997) as u16, (x + y) as u16];
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        let path = directory.path().join(format!("{target:?}.tif"));
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
            decoder.get_tag_u8_vec(Tag::Unknown(34675)).unwrap(),
            profiles::icc(target)
        );
        let DecodingResult::U16(actual) = decoder.read_image().unwrap() else {
            panic!()
        };
        let expected: Vec<_> = (0..size.height)
            .flat_map(|y| (0..size.width).flat_map(move |x| codes(x, y)))
            .collect();
        assert_eq!(actual, expected);
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
