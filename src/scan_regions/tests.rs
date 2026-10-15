use super::*;
#[test]
fn authored_opening_and_picture_keep_distinct_full_footprint_membership() {
    let active = SourceRegion {
        x: 12,
        y: 20,
        width: 192,
        height: 128,
    };
    let mut areas = Areas::default();
    areas.edit_opening(Coverage::rectangle(SourceRegion {
        x: 22,
        y: 30,
        width: 172,
        height: 108,
    }));
    areas.edit_picture(Coverage::rectangle(SourceRegion {
        x: 32,
        y: 40,
        width: 152,
        height: 88,
    }));
    assert!(areas.contains_footprint(
        SourceRegion {
            x: 24,
            y: 32,
            width: 8,
            height: 8
        },
        active,
        false
    ));
    assert!(!areas.contains_footprint(
        SourceRegion {
            x: 24,
            y: 32,
            width: 8,
            height: 8
        },
        active,
        true
    ));
    assert!(!areas.contains_footprint(
        SourceRegion {
            x: 18,
            y: 32,
            width: 8,
            height: 8
        },
        active,
        false
    ));
    assert!(areas.contains_footprint(
        SourceRegion {
            x: 40,
            y: 50,
            width: 8,
            height: 8
        },
        active,
        true
    ));
    areas.validate(active).unwrap();
}
#[test]
fn authored_geometry_replaces_automatic_masks_and_half_planes() {
    let mut a = Areas {
        material: Some(Material {
            size: [16, 16],
            xs: vec![0, 15],
            ys: vec![0, 15],
            labels: "HFLF".into(),
        }),
        picture_constraints: vec![[1., 0., 100.]],
        ..Areas::default()
    };
    let q = Coverage::rectangle(SourceRegion {
        x: 0,
        y: 0,
        width: 16,
        height: 16,
    });
    a.edit_opening(q);
    a.edit_picture(q);
    assert!(a.material.is_none() && a.picture_constraints.is_empty());
    assert!(a.opening_edited && a.picture_edited);
}
#[test]
fn material_rejects_malformed_grids_and_checks_interior_cells_not_just_corners() {
    let mut m = Material {
        size: [24, 24],
        xs: vec![0, 12, 23],
        ys: vec![0, 12, 23],
        labels: "FFFFHFFFF".into(),
    };
    m.validate().unwrap();
    m.labels.push('F');
    assert!(m.validate().is_err());
    m.labels.pop();
    let active = SourceRegion {
        x: 0,
        y: 0,
        width: 24,
        height: 24,
    };
    let q = Coverage::rectangle(active).corners().map(|p| p.0);
    let a = Areas {
        opening: Some(q),
        picture: Some(q),
        material: Some(m),
        ..Areas::default()
    };
    assert!(!a.contains_footprint(
        SourceRegion {
            x: 2,
            y: 2,
            width: 20,
            height: 20
        },
        active,
        true
    ));
}

#[cfg(feature = "raw")]
#[path = "comparison_tests.rs"]
mod comparison;
#[path = "opening_tests.rs"]
mod opening_tests;
#[path = "rectangle_tests.rs"]
mod rectangles_tests;

#[cfg(feature = "raw")]
#[test]
#[ignore = "Explicit retained private camera-code evidence comparison; no RAWs are modified"]
fn retained_rectangle_baseline_parity() {
    use std::path::{Path, PathBuf};
    let root = PathBuf::from(
        std::env::var_os("ANTITONE_REGION_EVIDENCE").expect("ANTITONE_REGION_EVIDENCE"),
    );
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("roll-input/rolls.json")).unwrap())
            .unwrap();
    let filter = std::env::var("ANTITONE_REGION_ROLL").ok();
    fn pixels(path: &Path) -> Vec<[u16; 3]> {
        std::fs::read(path)
            .unwrap()
            .as_chunks::<6>()
            .0
            .iter()
            .map(|v| std::array::from_fn(|c| u16::from_le_bytes([v[2 * c], v[2 * c + 1]])))
            .collect()
    }
    let native = std::env::var_os("ANTITONE_REGION_NATIVE").is_some();
    let mut checked = 0;
    let mut errors = vec![];
    for roll in manifest["rolls"].as_array().unwrap() {
        let name = roll["name"].as_str().unwrap();
        if filter.as_ref().is_some_and(|f| f != name) {
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
            let m: serde_json::Value = serde_json::from_slice(
                &std::fs::read(root.join(format!("roll-input/proxies/{name}.json"))).unwrap(),
            )
            .unwrap();
            let area: [u32; 4] = serde_json::from_value(m["active_area"].clone()).unwrap();
            let size = serde_json::from_value(m["proxy_size"].clone()).unwrap();
            let rgb = pixels(&root.join(format!("roll-input/proxies/{name}.rgb16")));
            let evidence: Evidence = serde_json::from_slice(
                &std::fs::read(root.join(format!("roll-evidence/{name}.json"))).unwrap(),
            )
            .unwrap();
            let evidence = if native {
                Evidence::observe(size, &rgb).unwrap()
            } else {
                evidence
            };
            captures.push(Capture::new(area.into(), size, rgb, evidence).unwrap());
            names.push(name);
        }
        let model = RollModel::fit(&captures).unwrap();
        let tsv = std::fs::read_to_string(
            root.join(format!("rectangle-populations-associated/{name}.tsv")),
        )
        .unwrap();
        let mut frame = String::new();
        let mut expected =
            std::collections::BTreeMap::<String, std::collections::BTreeMap<[u32; 2], u8>>::new();
        for line in tsv.lines() {
            let fields: Vec<_> = line.split('\t').collect();
            if fields[0] == "FRAME" {
                frame = Path::new(fields[1])
                    .file_stem()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_owned();
                expected.insert(frame.clone(), Default::default());
            } else {
                expected.get_mut(&frame).unwrap().insert(
                    [fields[1].parse().unwrap(), fields[2].parse().unwrap()],
                    fields[3].parse().unwrap(),
                );
            }
        }
        for (i, (capture, name)) in captures.iter().zip(&names).enumerate() {
            let fine = root.join(format!("rectangle-measurements/{name}.json"));
            let (size, rgb) = if fine.exists() {
                let m: serde_json::Value =
                    serde_json::from_slice(&std::fs::read(fine).unwrap()).unwrap();
                (
                    serde_json::from_value(m["proxy_size"].clone()).unwrap(),
                    pixels(&root.join(format!("rectangle-measurements/{name}.rgb16"))),
                )
            } else {
                (capture.size, capture.rgb.clone())
            };
            let a = model.local_areas(i, capture, size, &rgb).unwrap();
            let old: serde_json::Value = serde_json::from_slice(
                &std::fs::read(root.join(format!(
                    "rectangle-populations-associated/regions/{name}.json"
                )))
                .unwrap(),
            )
            .unwrap();
            let r = &old["rectangle_support"];
            for (key, q) in [("opening", a.opening), ("picture", a.picture)] {
                let expected: Option<Quad> =
                    serde_json::from_value(r[key]["polygon"].clone()).unwrap();
                let expected = expected.map(|q| q.map(|p| capture.source_point(p)));
                let delta = match (q, expected) {
                    (Some(a), Some(b)) => a
                        .iter()
                        .flatten()
                        .zip(b.iter().flatten())
                        .map(|(a, b)| (a - b).abs())
                        .fold(0., f64::max),
                    (None, None) => 0.,
                    _ => f64::INFINITY,
                };
                if delta > 1e-7 {
                    errors.push(format!("{name} {key}: {delta}"));
                }
            }
            let active = capture.area;
            let budget = 8192usize;
            let columns = ((budget as f64 * f64::from(active.width) / f64::from(active.height))
                .sqrt() as usize)
                .max(1)
                .min(budget)
                .min((active.width / 8) as usize);
            let rows = (budget / columns).min((active.height / 8) as usize);
            let raster =
                crate::geometry::SourceRaster::new(active, [columns as u32, rows as u32]).unwrap();
            let mut actual = std::collections::BTreeMap::new();
            for y in 0..rows {
                for x in 0..columns {
                    let p = raster.source_pixel([x as u32, y as u32]).unwrap();
                    let footprint = SourceRegion {
                        x: p[0] - 4,
                        y: p[1] - 4,
                        width: 8,
                        height: 8,
                    };
                    let flags = u8::from(a.contains_footprint(footprint, active, false))
                        + 2 * u8::from(a.contains_footprint(footprint, active, true));
                    if flags > 0 {
                        actual.insert(p, flags);
                    }
                }
            }
            if actual != expected[name] {
                errors.push(format!("{name}: original-centre membership differs"));
            }
            checked += 1;
        }
        eprintln!(
            "{name}: checked {} photos; {} differences",
            captures.len(),
            errors.len()
        );
    }
    eprintln!("{}", errors.join("\n"));
    assert!(checked > 0);
    assert!(
        errors.is_empty(),
        "{checked} photos, {} baseline differences",
        errors.len()
    );
}
