//! Cached geometry replay only: no RAW development, grade fitting, desktop or production wiring.
use crate::{
    auto_crop,
    project::{Development, SourceDevelopment, SourceId},
    scan_regions::{Evidence, FrameInput, prepare_spatial},
    spatial_metadata::{FileRevision, Input},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::Instant,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Protocol {
    version: u32,
    evidence_root: PathBuf,
    prior_trial: PathBuf,
    cases: Vec<String>,
}

fn read(path: &Path, revisions: &mut BTreeMap<PathBuf, FileRevision>) -> Vec<u8> {
    assert_ne!(
        path.extension()
            .and_then(|s| s.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("arw")
    );
    let before = FileRevision::read(path).unwrap();
    let data = std::fs::read(path).unwrap();
    assert_eq!(FileRevision::read(path).unwrap(), before);
    revisions.insert(path.to_path_buf(), before);
    data
}
fn json_read(path: &Path, revisions: &mut BTreeMap<PathBuf, FileRevision>) -> Value {
    serde_json::from_slice(&read(path, revisions)).unwrap()
}
fn quality(crop: crate::crop::Crop, active: crate::geometry::SourceRegion, marks: &Value) -> Value {
    if marks.is_null() {
        return Value::Null;
    }
    json!([0.,0.5].map(|margin| {
        let crop=auto_crop::inward_margin(crop,active,margin).unwrap();
        let score=|key:&str,wanted| {
            if marks[key].is_null() {Value::Null} else {
                let quads:Vec<crate::scan_regions::Quad>=serde_json::from_value(marks[key].clone()).unwrap();
                serde_json::to_value(auto_crop::diagnostics::score(crop,&quads,wanted).unwrap()).unwrap()
            }
        };
        json!({"margin_percent":margin,"crop":crop,"wanted_lost":score("wanted",true),"unwanted_admitted":score("unwanted",false),"dense_wanted_lost":score("dense_wanted",true)})
    }))
}

#[test]
#[ignore = "explicit frozen camera-code protocol and fresh private output directory"]
fn observe_spatial_metadata_cached_roll() {
    let started = Instant::now();
    let checkpoint = || {
        if started.elapsed().as_secs() < 120 {
            Ok(())
        } else {
            Err("spatial replay deadline".into())
        }
    };
    let mut revisions = BTreeMap::new();
    let protocol_path =
        std::env::var_os("ANTITONE_SPATIAL_PROTOCOL").expect("explicit spatial protocol");
    let protocol: Protocol =
        serde_json::from_slice(&read(Path::new(&protocol_path), &mut revisions)).unwrap();
    assert_eq!(protocol.version, 1);
    assert!((1..=64).contains(&protocol.cases.len()));
    assert_eq!(
        protocol.cases.iter().collect::<BTreeSet<_>>().len(),
        protocol.cases.len()
    );
    assert!(
        protocol
            .cases
            .iter()
            .all(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric()))
    );
    let requested = std::env::var("ANTITONE_SPATIAL_CASE").expect("explicit cached case");
    assert!(protocol.cases.contains(&requested));
    let output = PathBuf::from(std::env::var_os("ANTITONE_SPATIAL_OUTPUT").expect("fresh output"));
    assert!(output.is_absolute() && !output.exists());
    let rolls = json_read(
        &protocol.evidence_root.join("roll-input/rolls.json"),
        &mut revisions,
    );
    let roll = rolls["rolls"]
        .as_array()
        .unwrap()
        .iter()
        .find(|roll| {
            roll["photos"].as_array().unwrap().iter().any(|name| {
                Path::new(name.as_str().unwrap()).file_stem().unwrap() == requested.as_str()
            })
        })
        .unwrap();
    let photos = roll["photos"].as_array().unwrap();
    assert!((2..=40).contains(&photos.len()));
    let mut frames = vec![];
    let mut stems = vec![];
    let mut wb = None;
    for (index, name) in photos.iter().enumerate() {
        checkpoint().unwrap();
        let stem = Path::new(name.as_str().unwrap())
            .file_stem()
            .unwrap()
            .to_str()
            .unwrap();
        assert!(stem.bytes().all(|b| b.is_ascii_alphanumeric()));
        let meta = json_read(
            &protocol
                .evidence_root
                .join(format!("roll-input/proxies/{stem}.json")),
            &mut revisions,
        );
        let fixed: [f64; 4] = serde_json::from_value(meta["fixed_wb"].clone()).unwrap();
        if let Some(expected) = wb {
            assert_eq!(expected, fixed);
        } else {
            wb = Some(fixed);
        }
        let size: [u32; 2] = serde_json::from_value(meta["proxy_size"].clone()).unwrap();
        assert!(size.iter().all(|n| (16..=768).contains(n)));
        let path = protocol
            .evidence_root
            .join(format!("roll-input/proxies/{stem}.rgb16"));
        let bytes = read(&path, &mut revisions);
        assert_eq!(bytes.len(), size[0] as usize * size[1] as usize * 6);
        let rgb = bytes
            .as_chunks::<6>()
            .0
            .iter()
            .map(|b| std::array::from_fn(|i| u16::from_le_bytes([b[i * 2], b[i * 2 + 1]])))
            .collect::<Vec<_>>();
        let evidence = Evidence::observe(size, &rgb).unwrap();
        // This fixture identity describes old cached camera codes, not current SDK/Sony acceptance.
        let development = SourceDevelopment {
            lens_correction: None,
            dng_development: None,
            dimensions: serde_json::from_value(meta["developed_size"].clone()).unwrap(),
            active_area: serde_json::from_value(meta["active_area"].clone()).unwrap(),
            camera_model: meta["camera"].as_str().unwrap().into(),
            camera_to_ap1: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
            input_floor: crate::processing::CAMERA_INPUT_FLOOR,
        };
        frames.push(
            FrameInput::new(
                Input {
                    source: SourceId(index as u64 + 1),
                    revision: FileRevision::read(&path).unwrap(),
                    development,
                    correction: None,
                    code_scale: 1.,
                },
                size,
                rgb,
                evidence,
            )
            .unwrap(),
        );
        stems.push(stem.to_owned());
    }
    let metadata = prepare_spatial(
        frames,
        Development::new("frozen-RGB16-geometry-fixture".into(), wb.unwrap()),
        1.,
        checkpoint,
    )
    .unwrap();
    let mut cases = vec![];
    for (index, stem) in stems
        .iter()
        .enumerate()
        .filter(|(_, stem)| protocol.cases.contains(stem))
    {
        let source = metadata
            .groups
            .iter()
            .flat_map(|g| &g.sources)
            .find(|source| source.input.source == SourceId(index as u64 + 1))
            .unwrap();
        let crop = auto_crop::propose_spatial(source).ok();
        let prior_protocol = json_read(
            &protocol.prior_trial.join(format!("protocol-{stem}.json")),
            &mut revisions,
        );
        let prior = json_read(
            &protocol
                .prior_trial
                .join(format!("case-{stem}/report.json")),
            &mut revisions,
        );
        cases.push(json!({"source":stem,"metadata":source,"crop":crop,"quality":crop.map(|c|quality(c,source.input.development.active_area.into(),&prior_protocol["marks"][stem])),"gross_quality":crop.map(|c|quality(c,source.input.development.active_area.into(),&prior_protocol["gross_marks"][stem])),"prior_trial":prior["case"]["fixed_size_candidate"]["joint_fit_trial"]}));
    }
    for (path, revision) in &revisions {
        assert_eq!(&FileRevision::read(path).unwrap(), revision);
    }
    std::fs::create_dir(&output).unwrap();
    std::fs::write(output.join("report.json"),serde_json::to_vec_pretty(&json!({"roll":roll["name"],"metadata":metadata,"cases":cases,"inputs":revisions,"elapsed_seconds":started.elapsed().as_secs_f64(),"limits":"Consumed older camera-code inputs; runtime whole-group fit, not held-out accuracy or current-development/application acceptance. No acquisition."})).unwrap()).unwrap();
}
