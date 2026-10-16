use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use flate2::{Compression, Decompress, FlushDecompress, Status, write::ZlibEncoder};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::io::Write;

const ENCODING: &str = "zlib-rgb32le-base64-v1";
const MAX_COMPRESSED: usize = MAX_NODE_BYTES + 65536;
pub(crate) const MAX_EMBEDDED_TEXT_BYTES: usize = MAX_COMPRESSED.div_ceil(3) * 4;

fn input_range() -> String {
    "clip".into()
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Embedded {
    contract: String,
    name: String,
    origin: LookOrigin,
    size: u32,
    domain_min: [f32; 3],
    domain_max: [f32; 3],
    #[serde(default = "input_range")]
    input_range: String,
    encoding: String,
    content_id: String,
    samples: String,
}

impl Serialize for LookAsset {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        for value in self.lut.nodes().iter().flatten() {
            encoder
                .write_all(&value.to_le_bytes())
                .map_err(serde::ser::Error::custom)?;
        }
        let bytes = encoder.finish().map_err(serde::ser::Error::custom)?;
        let (domain_min, domain_max) = self.lut.domain();
        Embedded {
            contract: CONTRACT.into(),
            name: self.name.clone(),
            origin: self.origin.clone(),
            size: self.lut.size(),
            domain_min,
            domain_max,
            input_range: input_range(),
            encoding: ENCODING.into(),
            content_id: self.lut.id().to_string(),
            samples: STANDARD.encode(bytes),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for LookAsset {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let stored = Embedded::deserialize(deserializer)?;
        let load = || -> Result<LookAsset, Error> {
            require(
                stored.contract == CONTRACT
                    && stored.encoding == ENCODING
                    && stored.input_range == input_range(),
                "unsupported embedded LUT contract/encoding/input range policy",
            )?;
            require(
                (2..=MAX_GRID_SIZE).contains(&stored.size),
                "unsupported embedded LUT dimensions",
            )?;
            let expected = (stored.size as usize).pow(3) * 12;
            require(
                stored.samples.len() <= MAX_EMBEDDED_TEXT_BYTES,
                "embedded LUT exceeds the compressed budget",
            )?;
            let compressed = STANDARD.decode(&stored.samples).map_err(failure)?;
            require(
                compressed.len() <= MAX_COMPRESSED,
                "embedded LUT exceeds the compressed budget",
            )?;
            let mut decoder = Decompress::new(true);
            // decompress_vec never reallocates: a corrupt/hostile stream cannot
            // expand beyond the declared, independently bounded node payload.
            let mut bytes = Vec::with_capacity(expected + 1);
            let status = decoder
                .decompress_vec(&compressed, &mut bytes, FlushDecompress::Finish)
                .map_err(failure)?;
            require(
                status == Status::StreamEnd
                    && decoder.total_in() == compressed.len() as u64
                    && bytes.len() == expected,
                "incomplete, trailing or incorrectly sized embedded LUT payload",
            )?;
            let nodes = bytes
                .as_chunks::<12>()
                .0
                .iter()
                .map(|p| {
                    std::array::from_fn(|c| {
                        f32::from_le_bytes(p[c * 4..c * 4 + 4].try_into().unwrap())
                    })
                })
                .collect();
            let lut = Lut3D::new(stored.size, stored.domain_min, stored.domain_max, nodes)?;
            require(
                stored.content_id == lut.id().to_string(),
                "embedded LUT content identity mismatch",
            )?;
            LookAsset::new(stored.name.clone(), stored.origin.clone(), Arc::new(lut))
        };
        load().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset() -> LookAsset {
        LookAsset::new(
            "Saved look".into(),
            LookOrigin::Builtin {
                id: "no-longer-installed".into(),
                version: 73,
                generator: "historic-generator".into(),
                definition: "a".repeat(64),
            },
            Arc::new(super::super::tests::identity(3)),
        )
        .unwrap()
    }

    #[test]
    fn embedding_is_self_contained_and_preserves_exact_numeric_bits() {
        let original = asset();
        let bytes = serde_json::to_vec(&original).unwrap();
        let restored: LookAsset = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(restored, original);
        assert_eq!(
            restored
                .lut
                .nodes()
                .iter()
                .flatten()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>(),
            original
                .lut
                .nodes()
                .iter()
                .flatten()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>()
        );
        assert_eq!(restored.lut.id(), original.lut.id());
    }

    #[test]
    fn saved_boundary_policy_is_explicit_and_older_numeric_assets_keep_their_samples() {
        let original = asset();
        let mut value = serde_json::to_value(&original).unwrap();
        assert_eq!(value["input_range"], "clip");
        value.as_object_mut().unwrap().remove("input_range");
        let restored: LookAsset = serde_json::from_value(value).unwrap();
        assert_eq!(restored, original);
        let input = DVec3::new(-10., 0.4, 10.);
        let edge = DVec3::new(-0.5, 0.4, 1.);
        assert_eq!(
            restored.lut.sample_encoded(input).unwrap(),
            restored.lut.sample_encoded(edge).unwrap()
        );
    }

    #[test]
    fn invalid_metadata_corruption_truncation_and_oversized_inflation_reject() {
        let value = serde_json::to_value(asset()).unwrap();
        for (field, replacement) in [
            ("contract", serde_json::json!("rec709")),
            ("encoding", serde_json::json!("unknown")),
            ("input_range", serde_json::json!("extrapolate")),
            ("size", serde_json::json!(130)),
            ("content_id", serde_json::json!("0".repeat(64))),
            ("extra", serde_json::json!(true)),
            ("domain_min", serde_json::json!([1., 1., 1.])),
        ] {
            let mut changed = value.clone();
            changed[field] = replacement;
            assert!(
                serde_json::from_value::<LookAsset>(changed).is_err(),
                "{field}"
            );
        }
        let compressed = STANDARD.decode(value["samples"].as_str().unwrap()).unwrap();
        for bytes in [
            compressed[..compressed.len() - 1].to_vec(),
            [compressed.as_slice(), &[0]].concat(),
            vec![0, 1, 2, 3],
        ] {
            let mut changed = value.clone();
            changed["samples"] = serde_json::json!(STANDARD.encode(bytes));
            assert!(serde_json::from_value::<LookAsset>(changed).is_err());
        }
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&[0; 100_000]).unwrap();
        let mut changed = value;
        changed["samples"] = serde_json::json!(STANDARD.encode(encoder.finish().unwrap()));
        assert!(serde_json::from_value::<LookAsset>(changed).is_err());
    }
}
