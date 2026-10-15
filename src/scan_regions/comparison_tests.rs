//! Opt-in comparisons read the frozen local baseline; outputs belong in a fresh .cache directory.
use super::*;
use std::path::{Path, PathBuf};

fn pixels(path: &Path) -> Vec<[u16; 3]> {
    std::fs::read(path)
        .unwrap()
        .as_chunks::<6>()
        .0
        .iter()
        .map(|v| std::array::from_fn(|c| u16::from_le_bytes([v[2 * c], v[2 * c + 1]])))
        .collect()
}
fn json(path: &Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}
fn populations(c: &Capture, a: &Areas) -> [usize; 2] {
    let active = c.area;
    let budget = 8192usize;
    let columns = ((budget as f64 * f64::from(active.width) / f64::from(active.height)).sqrt()
        as usize)
        .max(1)
        .min(budget)
        .min((active.width / 8) as usize);
    let rows = (budget / columns).min((active.height / 8) as usize);
    let raster = crate::geometry::SourceRaster::new(active, [columns as u32, rows as u32]).unwrap();
    let mut counts = [0; 2];
    for y in 0..rows {
        for x in 0..columns {
            let p = raster.source_pixel([x as u32, y as u32]).unwrap();
            let f = SourceRegion {
                x: p[0] - 4,
                y: p[1] - 4,
                width: 8,
                height: 8,
            };
            for (picture, count) in counts.iter_mut().enumerate() {
                *count += usize::from(a.contains_footprint(f, active, picture == 1));
            }
        }
    }
    counts
}

#[test]
#[ignore = "Explicit read-only private-corpus shared/local spatial comparison, not photographic acceptance"]
fn compare_shared_holder_with_frozen_local_baseline() {
    let root = PathBuf::from(
        std::env::var_os("ANTITONE_REGION_EVIDENCE").expect("ANTITONE_REGION_EVIDENCE"),
    );
    let out = PathBuf::from(
        std::env::var_os("ANTITONE_REGION_OUTPUT").expect("fresh ANTITONE_REGION_OUTPUT"),
    );
    assert!(
        !out.exists(),
        "choose a fresh output directory; do not overwrite retained reports"
    );
    let parent = out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    assert!(
        !parent
            .canonicalize()
            .expect("existing output parent")
            .starts_with(root.canonicalize().unwrap()),
        "outputs must be outside frozen evidence"
    );
    std::fs::create_dir(&out).unwrap();
    let manifest = json(&root.join("roll-input/rolls.json"));
    let filter = std::env::var("ANTITONE_REGION_ROLL").ok();
    let mut reports = vec![];
    let mut inadequate = vec![];
    for roll in manifest["rolls"].as_array().unwrap() {
        let roll_name = roll["name"].as_str().unwrap();
        if filter.as_ref().is_some_and(|f| f != roll_name) {
            continue;
        }
        let mut captures = vec![];
        let mut names = vec![];
        for photo in roll["photos"].as_array().unwrap() {
            let name = Path::new(photo.as_str().unwrap())
                .file_stem()
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned();
            let m = json(&root.join(format!("roll-input/proxies/{name}.json")));
            let area: [u32; 4] = serde_json::from_value(m["active_area"].clone()).unwrap();
            let size = serde_json::from_value(m["proxy_size"].clone()).unwrap();
            let rgb = pixels(&root.join(format!("roll-input/proxies/{name}.rgb16")));
            let evidence = Evidence::observe(size, &rgb).unwrap();
            captures.push(Capture::new(area.into(), size, rgb, evidence).unwrap());
            names.push(name);
        }
        let model = RollModel::fit(&captures).unwrap();
        let mut shared = 0;
        let mut exceptions = 0;
        let mut min_picture = usize::MAX;
        let mut film_total = 0;
        let mut local = vec![];
        let mut prepared = vec![];
        for (i, c) in captures.iter().enumerate() {
            let name = &names[i];
            let fine = root.join(format!("rectangle-measurements/{name}.json"));
            let (size, rgb) = if fine.exists() {
                (
                    serde_json::from_value(json(&fine)["proxy_size"].clone()).unwrap(),
                    pixels(&root.join(format!("rectangle-measurements/{name}.rgb16"))),
                )
            } else {
                (c.size, c.rgb.clone())
            };
            local.push(model.local_areas(i, c, size, &rgb).unwrap());
            prepared.push(model.areas(i, c, size, &rgb).unwrap());
        }
        model.refine_areas(&mut prepared).unwrap();
        for (((c, name), old), new) in captures.iter().zip(&names).zip(&local).zip(&prepared) {
            let before = populations(c, old);
            let after = populations(c, new);
            shared += usize::from(new.opening_shared);
            exceptions += usize::from(new.opening_exception);
            min_picture = min_picture.min(after[1]);
            film_total += after[0];
            if !new.ready() || after[1] < 512 {
                inadequate.push(format!("{roll_name}/{name}: {after:?}"));
            }
            let report = serde_json::json!({"roll":roll_name,"source":name,"local":old,"shared":new,"local_centres":before,"shared_centres":after});
            std::fs::write(
                out.join(format!("{name}.json")),
                serde_json::to_vec_pretty(&report).unwrap(),
            )
            .unwrap();
            #[cfg(feature = "editor")]
            if std::env::var("ANTITONE_REGION_IMAGES")
                .is_ok_and(|v| v.split(',').any(|s| s == name))
            {
                image(c, old, new, &out.join(format!("{name}.png")));
            }
        }
        let expected_shared = if roll_name == "04495-04532" {
            0
        } else {
            captures.len()
        };
        assert_eq!(
            shared, expected_shared,
            "{roll_name}: shared-support baseline changed; inspect before accepting a new baseline"
        );
        assert_eq!(
            exceptions, 0,
            "{roll_name}: previously coherent roll acquired photo exceptions"
        );
        let common: Vec<_> = prepared
            .iter()
            .filter(|a| a.opening_shared && !a.opening_exception)
            .collect();
        if let Some(first) = common.first() {
            assert!(
                common.iter().all(|a| a.opening == first.opening),
                "{roll_name}: normal photos do not share one opening"
            );
        }
        if film_total < 2048 {
            inadequate.push(format!("{roll_name}: pooled film {film_total}"));
        }
        let report = serde_json::json!({"roll":roll_name,"photos":captures.len(),"shared":shared,"exceptions":exceptions,"min_picture_centres":min_picture,"film_centres":film_total});
        eprintln!("{report}");
        reports.push(report);
    }
    assert!(!reports.is_empty(), "no matching rolls");
    std::fs::write(
        out.join("summary.json"),
        serde_json::to_vec_pretty(&serde_json::json!({"rolls":reports,"inadequate":inadequate}))
            .unwrap(),
    )
    .unwrap();
    assert!(
        inadequate.is_empty(),
        "spatial support inadequate (before numerical gates): {inadequate:?}"
    );
}

#[cfg(feature = "editor")]
fn image(c: &Capture, old: &Areas, new: &Areas, path: &Path) {
    let mut image = image::RgbImage::new(c.size[0], c.size[1]);
    for (pixel, rgb) in image.pixels_mut().zip(&c.rgb) {
        *pixel =
            image::Rgb(rgb.map(|v| (255. * (f64::from(v) / 65535.).powf(1. / 2.2)).round() as u8));
    }
    for (quad, color) in [
        (old.opening, [240, 80, 80]),
        (new.picture, [60, 200, 255]),
        (new.opening, [255, 200, 40]),
    ] {
        let Some(q) = quad else {
            continue;
        };
        let q = q.map(|p| {
            [
                (p[0] - f64::from(c.area.x)) * f64::from(c.size[0]) / f64::from(c.area.width),
                (p[1] - f64::from(c.area.y)) * f64::from(c.size[1]) / f64::from(c.area.height),
            ]
        });
        for edge in 0..4 {
            let a = q[edge];
            let b = q[(edge + 1) % 4];
            let steps = (a[0] - b[0]).abs().max((a[1] - b[1]).abs()).ceil().max(1.) as usize;
            for i in 0..=steps {
                let [x, y] = std::array::from_fn(|axis| {
                    (a[axis] + (b[axis] - a[axis]) * i as f64 / steps as f64).floor() as i64
                });
                if x >= 0 && y >= 0 && x < i64::from(c.size[0]) && y < i64::from(c.size[1]) {
                    image.put_pixel(x as u32, y as u32, image::Rgb(color));
                }
            }
        }
    }
    image.save(path).unwrap();
}
