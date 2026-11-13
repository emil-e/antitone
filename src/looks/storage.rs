use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use flate2::{Compression, Decompress, FlushDecompress, Status, write::ZlibEncoder};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::io::{BufWriter, Write};

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

// Coalesce scalar writes: a large LUT otherwise calls the compressor millions of times.
fn write_samples(output: impl Write, nodes: &[[f32; 3]]) -> std::io::Result<()> {
    let mut output = BufWriter::new(output);
    for value in nodes.iter().flatten() {
        output.write_all(&value.to_le_bytes())?;
    }
    output.into_inner().map_err(|error| error.into_error())?;
    Ok(())
}

impl Serialize for LookAsset {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let lut = self.lut();
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        write_samples(&mut encoder, lut.nodes()).map_err(serde::ser::Error::custom)?;
        let bytes = encoder.finish().map_err(serde::ser::Error::custom)?;
        let (domain_min, domain_max) = lut.domain();
        Embedded {
            contract: CONTRACT.into(),
            name: self.name.clone(),
            origin: self.origin.clone(),
            size: lut.size(),
            domain_min,
            domain_max,
            input_range: input_range(),
            encoding: ENCODING.into(),
            content_id: lut.id().to_string(),
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
            LookOrigin::Photochemical {
                negative: Some("no_longer_installed".into()),
                paper: "historic_medium".into(),
                revision: "a".repeat(40),
                generator: "historic-generator".into(),
                attribution: "Retained source attribution".into(),
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
                .lut()
                .nodes()
                .iter()
                .flatten()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>(),
            original
                .lut()
                .nodes()
                .iter()
                .flatten()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>()
        );
        assert_eq!(restored.id(), original.id());
    }

    #[test]
    fn embedding_flushes_multiple_buffers_and_the_tail_without_changing_sample_bits() {
        let size = 17u32;
        let bits = [
            0,
            0x8000_0000,
            0x3e80_0001,
            0xbf00_0001,
            0x3f80_0001,
            1,
            0x3f00_0011,
        ];
        let nodes: Vec<[f32; 3]> = (0..size.pow(3) as usize)
            .map(|i| std::array::from_fn(|c| f32::from_bits(bits[(i * 3 + c) % bits.len()])))
            .collect();
        let expected: Vec<u8> = nodes
            .iter()
            .flatten()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        #[derive(Default)]
        struct Sink {
            bytes: Vec<u8>,
            writes: usize,
        }
        impl Write for Sink {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.writes += 1;
                self.bytes.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut sink = Sink::default();
        write_samples(&mut sink, &nodes).unwrap();
        assert_eq!(sink.bytes, expected);
        assert!(
            sink.writes < nodes.len(),
            "sample writes must coalesce across nodes, not call compression for each float"
        );
        let original = LookAsset::new(
            "Buffered samples".into(),
            LookOrigin::Imported,
            Arc::new(Lut3D::new(size, [-0.5; 3], [1.; 3], nodes).unwrap()),
        )
        .unwrap();
        let bytes = serde_json::to_vec(&original).unwrap();
        let stored: Embedded = serde_json::from_slice(&bytes).unwrap();
        let compressed = STANDARD.decode(stored.samples).unwrap();
        let mut decoder = flate2::read::ZlibDecoder::new(compressed.as_slice());
        let mut payload = Vec::new();
        std::io::Read::read_to_end(&mut decoder, &mut payload).unwrap();
        assert_eq!(
            payload, expected,
            "stored format is exact RGB float32 little-endian, including the final partial buffer"
        );
        let restored: LookAsset = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(restored.id(), original.id());
        assert_eq!(
            restored
                .lut()
                .nodes()
                .iter()
                .flatten()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>(),
            original
                .lut()
                .nodes()
                .iter()
                .flatten()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>(),
        );
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
            restored.lut().sample_encoded(input).unwrap(),
            restored.lut().sample_encoded(edge).unwrap()
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
