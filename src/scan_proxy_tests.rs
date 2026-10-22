use super::*;

fn authored_scan() -> Scan {
    Scan::open(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng"),
        None,
    )
    .unwrap()
}

#[test]
fn minified_camera_proxy_averages_a_bounded_grid_in_continuous_camera_units() {
    let scan = authored_scan();
    let proxy = CameraProxy::sample(&scan, 16).unwrap();
    let [width, height] = proxy.dimensions();
    let area = scan.area;
    let image = scan.source.camera_image().unwrap();
    assert!(area.width / width >= 4 && area.height / height >= 4);
    let mut distinguishes_point_sampling = false;
    for y in 0..height {
        for x in 0..width {
            let mut sum = [0.; 3];
            // Four stratified centres per axis, expressed directly as rational
            // source coordinates rather than through the production raster helper.
            for sy in 0..4 {
                for sx in 0..4 {
                    let px = area.x + ((8 * x + 2 * sx + 1) * area.width) / (8 * width);
                    let py = area.y + ((8 * y + 2 * sy + 1) * area.height) / (8 * height);
                    let p = image.pixel_samples(px, py).unwrap();
                    for c in 0..3 {
                        sum[c] += f64::from(p[c]);
                    }
                }
            }
            let expected = sum.map(|value| (value / 16.) as f32);
            let actual = proxy.pixels[(y * width + x) as usize];
            assert_eq!(actual[..3], expected, "sampled camera codes at [{x}, {y}]");
            let point = image
                .pixel_samples(
                    area.x + (2 * x + 1) * area.width / (2 * width),
                    area.y + (2 * y + 1) * area.height / (2 * height),
                )
                .unwrap();
            distinguishes_point_sampling |= (0..3).any(|c| (expected[c] - point[c]).abs() > 2.);
        }
    }
    assert!(
        distinguishes_point_sampling,
        "the fixture must expose a regression to a single sample, beyond code rounding"
    );
    assert_eq!(proxy.matrix, scan.camera_to_ap1);
}

#[test]
fn native_sized_camera_proxy_preserves_samples_without_upscaling() {
    let scan = authored_scan();
    let proxy = CameraProxy::from_scan(&scan).unwrap();
    assert_eq!(proxy.dimensions(), [scan.area.width, scan.area.height]);
    let image = scan.source.camera_image().unwrap();
    for y in 0..proxy.height {
        for x in 0..proxy.width {
            let expected = image
                .pixel_samples(scan.area.x + x, scan.area.y + y)
                .unwrap();
            let actual = proxy.pixels[(y * proxy.width + x) as usize];
            assert_eq!(actual[..3], expected[..3]);
        }
    }
}
