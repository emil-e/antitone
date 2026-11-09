use super::*;

#[test]
fn runtime_precision_accepts_a_correction_map_extending_beyond_the_active_raster() {
    let area = Rect::from([13, 29, 64, 48]);
    let input = AnalysisInput::authored(
        SourceRaster::new(area, [2, 2]).unwrap(),
        vec![[1000.; 3]; 4],
        crate::processing::CAMERA_INPUT_FLOOR,
    );
    let field = FlatField::from_nodes([45, 29, 64, 48], [2, 2], vec![DVec3::splat(2.); 4]).unwrap();
    assert_eq!(
        input.effective_floor(Some(&field)).unwrap(),
        2. * crate::processing::CAMERA_INPUT_FLOOR
    );
    assert_eq!(input.area(), area);
}

#[test]
fn capture_owns_one_original_rgb_raster_without_a_native_lease() {
    let file = crate::raw::fixture::bayer_dng_with_rgb([6000, 12000, 9000]);
    let scan = Scan::open(file.path(), None).unwrap();
    let native = Arc::downgrade(scan.source());
    let input = AnalysisInput::capture(&scan, || false).unwrap().unwrap();
    let area = scan.area;
    assert_eq!(input.size(), [area.width, area.height]);
    assert_eq!(
        input.retained_bytes(),
        (size_of::<AnalysisInput>() + area.width as usize * area.height as usize * 12) as u64
    );
    let image = scan.source().camera_image().unwrap();
    for (index, &pixel) in input.pixels.iter().enumerate() {
        let x = area.x + index as u32 % area.width;
        let y = area.y + index as u32 / area.width;
        let expected = image.pixel_samples(x, y).unwrap();
        assert_eq!(pixel, [expected[0], expected[1], expected[2]]);
        assert_eq!(input.position(index).unwrap(), [x, y]);
    }
    drop(scan);
    assert!(
        native.upgrade().is_none(),
        "camera evidence must not pin the original"
    );
    let selected = input
        .select(area, 16, |_| true, &mut || false)
        .unwrap()
        .unwrap();
    assert_eq!(selected.len(), 16);
    let expected: Vec<_> = (0..4)
        .flat_map(|y| {
            (0..4).map(move |x| {
                ((2 * y + 1) * area.height as usize / 8) * area.width as usize
                    + (2 * x + 1) * area.width as usize / 8
            })
        })
        .collect();
    assert_eq!(selected, expected);
}

#[test]
fn regular_windows_and_statistical_edits_reuse_original_source_positions() {
    let file = crate::dng_fixture::Fixture {
        side: 2048,
        ramp: true,
        ..Default::default()
    }
    .write();
    let scan = Scan::open(file.path(), None).unwrap();
    let input = AnalysisInput::capture(&scan, || false).unwrap().unwrap();
    let maximum = u64::from(scan.area.width.max(scan.area.height));
    let expected_size = [scan.area.width, scan.area.height]
        .map(|extent| ((2 * u64::from(extent) * 1024 + maximum) / (2 * maximum)) as u32);
    assert_eq!(input.size(), expected_size);
    let [width, height] = expected_size.map(|v| v as usize);
    assert!(input.retained_bytes() <= (12 * 1024 * 1024 + size_of::<AnalysisInput>()) as u64);
    for index in [0usize, width - 1, 515 * width + 391, width * height - 1] {
        let x = scan.area.x
            + (((2 * (index % width) + 1) as u64 * u64::from(scan.area.width)) / (2 * width) as u64)
                as u32;
        let y = scan.area.y
            + (((2 * (index / width) + 1) as u64 * u64::from(scan.area.height))
                / (2 * height) as u64) as u32;
        assert_eq!(input.position(index).unwrap(), [x, y]);
        let expected = scan
            .source()
            .camera_image()
            .unwrap()
            .pixel_samples(x, y)
            .unwrap();
        assert_eq!(input.pixel(index).unwrap(), expected);
    }
    let area = Rect {
        x: scan.area.x + 37,
        y: scan.area.y + 51,
        width: 1501,
        height: 1037,
    };
    let window = input.window(area, 512).unwrap();
    let size = window.size();
    assert!(size.iter().all(|&v| v > 0 && v <= 512));
    let mut previous = None;
    for local in 0..size[0] {
        let index = window.index(local);
        let [x, y] = input.position(index).unwrap();
        assert!(area.contains_region(Rect {
            x,
            y,
            width: 1,
            height: 1
        }));
        if let Some(previous) = previous {
            assert_eq!(index - previous, 2);
        }
        previous = Some(index);
    }
    let indices = input
        .select(area, 128, |p| p.x >= area.x + 400, &mut || false)
        .unwrap()
        .unwrap();
    assert!(!indices.is_empty() && indices.len() <= 128);
    assert!(
        indices
            .iter()
            .all(|&i| input.position(i).unwrap()[0] >= area.x + 400)
    );
}

#[test]
fn captured_sites_preserve_the_native_floor_and_do_not_read_excluded_pixels() {
    let file = crate::raw::fixture::bayer_dng_with_rgb([2; 3]);
    let mut scan = Scan::open(file.path(), None).unwrap();
    let pixel = scan
        .source
        .camera_image()
        .unwrap()
        .pixel(scan.area.x + 12, scan.area.y + 12)
        .unwrap()
        .min_element();
    Arc::make_mut(&mut scan.source).precision_floor = f64::from_bits(pixel.to_bits() - 1);
    let mut input = AnalysisInput::capture(&scan, || false).unwrap().unwrap();
    assert!(input.above_native_floor(0).unwrap());
    input.pixels[0][0] = f32::NAN;
    let indices = input
        .select(
            input.area(),
            8192,
            |p| p.x != input.area().x || p.y != input.area().y,
            &mut || false,
        )
        .unwrap()
        .unwrap();
    assert!(!indices.contains(&0));
    assert!(
        indices
            .iter()
            .all(|&i| input.above_native_floor(i).unwrap())
    );
    assert!(matches!(
        input.above_native_floor(0),
        Err(Error::NumericalOverflow)
    ));
    assert!(AnalysisInput::capture(&scan, || true).unwrap().is_none());
    assert!(
        input
            .select(input.area(), 16, |_| true, &mut || true)
            .unwrap()
            .is_none()
    );
}
