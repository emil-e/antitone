use super::*;
use std::path::{Path, PathBuf};

fn read(path: impl AsRef<Path>) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn floats(path: impl AsRef<Path>) -> Vec<f32> {
    let bytes = std::fs::read(path).unwrap();
    let (words, remainder) = bytes.as_chunks::<4>();
    assert!(remainder.is_empty(), "truncated float32 carrier");
    words.iter().copied().map(f32::from_le_bytes).collect()
}

fn write_vectors(path: impl AsRef<Path>, values: &[DVec3]) {
    std::fs::write(
        path,
        values
            .iter()
            .flat_map(|v| {
                v.to_array()
                    .into_iter()
                    .flat_map(|v| (v as f32).to_le_bytes())
            })
            .collect::<Vec<_>>(),
    )
    .unwrap();
}

#[test]
#[ignore = "Set ANTITONE_FRONTIER_ACCEPTANCE to an independently captured accepted-bake manifest"]
fn prepared_frontier_preserves_the_accepted_appearance() {
    let path = PathBuf::from(std::env::var_os("ANTITONE_FRONTIER_ACCEPTANCE").unwrap());
    let expected = read(path);
    let inputs: Vec<DVec3> = serde_json::from_value(expected["inputs"]["inputs"].clone()).unwrap();
    let output = std::env::var_os("ANTITONE_FRONTIER_OUTPUT").map(PathBuf::from);
    if let Some(path) = &output {
        assert!(!path.exists(), "acceptance output must be fresh");
        std::fs::create_dir_all(path).unwrap();
        write_vectors(path.join("inputs.f32"), &inputs);
    }
    let cancel = AtomicBool::new(false);
    let mut reports = Vec::new();
    for row in expected["groups"].as_array().unwrap() {
        let negative = match row["stock"].as_str().unwrap() {
            "none" => None,
            id => Some(spektrafilm::Negative::from_id(id).unwrap()),
        };
        let variant = match row["variant"].as_str().unwrap() {
            "A" => Variant::A,
            "B" => Variant::B,
            _ => panic!("unknown bank"),
        };
        let selection = Selection { negative, variant };
        let model = Model::new(selection, &cancel).unwrap();
        let actual = model.bake(65, &cancel).unwrap();
        let accepted: LookAsset =
            serde_json::from_value(read(row["asset"].as_str().unwrap())).unwrap();
        let label = selection.name();
        assert_eq!(actual.lut().domain(), accepted.lut().domain(), "{label}");
        let mismatch = actual
            .lut()
            .nodes()
            .iter()
            .zip(accepted.lut().nodes())
            .position(|(a, b)| a != b);
        assert!(
            mismatch.is_none(),
            "{label}/first changed node: {mismatch:?}"
        );
        assert_eq!(actual.id(), accepted.id(), "{label}/numeric identity");
        let native: Vec<_> = inputs.iter().map(|&p| model.ap1(p).unwrap()).collect();
        let native_expected = floats(row["native_carrier"].as_str().unwrap());
        assert_eq!(native_expected.len(), 3 * native.len());
        for (index, (&expected, actual)) in native_expected
            .iter()
            .zip(native.iter().flat_map(|p| p.to_array()))
            .enumerate()
        {
            assert_eq!(actual as f32, expected, "{label}/native/{index}");
        }
        let sampled: Vec<_> = inputs
            .iter()
            .map(|&p| actual.lut().apply(p).unwrap())
            .collect();
        let sampled_expected = floats(row["sampled_carrier"].as_str().unwrap());
        assert_eq!(sampled_expected.len(), 3 * sampled.len());
        for (index, (&expected, actual)) in sampled_expected
            .iter()
            .zip(sampled.iter().flat_map(|p| p.to_array()))
            .enumerate()
        {
            assert_eq!(actual as f32, expected, "{label}/creative65/{index}");
        }
        if let Some(path) = &output {
            let stem = format!(
                "{}-{}",
                row["stock"].as_str().unwrap(),
                row["variant"].as_str().unwrap()
            );
            let asset_path = path.join(format!("{stem}.json"));
            std::fs::write(&asset_path, serde_json::to_vec(&actual).unwrap()).unwrap();
            let native_path = path.join(format!("{stem}-native.f32"));
            write_vectors(&native_path, &native);
            write_vectors(path.join(format!("{stem}-sampled.f32")), &sampled);
            reports.push(serde_json::json!({"stock":row["stock"],"variant":row["variant"],"asset":asset_path,"numeric_id":actual.id().to_string(),"native_carrier":native_path,"expected":row}));
        }
        eprintln!(
            "{label}: {} controls and65cube nodes/identity exactly preserved",
            inputs.len()
        );
    }
    assert_eq!(expected["groups"].as_array().unwrap().len(), 34);
    if let Some(path) = &output {
        std::fs::write(
            path.join("manifest.json"),
            serde_json::to_vec_pretty(
                &serde_json::json!({"groups":reports,"inputs":expected["inputs"]}),
            )
            .unwrap(),
        )
        .unwrap();
    }
}
