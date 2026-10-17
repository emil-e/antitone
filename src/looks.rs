//! Scene-referred creative LUTs. Source provenance does not change execution.
use crate::{Error, photographic};
use glam::DVec3;
use std::sync::Arc;

#[cfg(feature = "project")]
#[path = "looks_builtin.rs"]
pub mod builtin;

#[cfg(feature = "project")]
#[path = "looks_cache.rs"]
pub mod cache;

#[cfg(feature = "project")]
#[path = "looks_storage.rs"]
mod storage;
#[cfg(feature = "project")]
pub(crate) use storage::MAX_EMBEDDED_TEXT_BYTES;

pub const CONTRACT: &str = "acescct-ap1-d60-tetrahedral-v1";
pub const MAX_GRID_SIZE: u32 = 129;
pub const MAX_IMPORT_BYTES: usize = 128 * 1024 * 1024;
pub const MAX_NODE_BYTES: usize =
    MAX_GRID_SIZE as usize * MAX_GRID_SIZE as usize * MAX_GRID_SIZE as usize * 12;

pub(crate) fn failure(message: impl std::fmt::Display) -> Error {
    Error::Rendering(format!("Creative look: {message}"))
}

fn require(condition: bool, message: &str) -> Result<(), Error> {
    if condition {
        Ok(())
    } else {
        Err(failure(message))
    }
}

/// Identity includes every value affecting interpolation, not labels or source names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LookId(pub(crate) [u8; 32]);

impl LookId {
    pub fn bytes(self) -> [u8; 32] {
        self.0
    }
}

impl std::fmt::Display for LookId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct Lut3D {
    size: u32,
    min: [f32; 3],
    max: [f32; 3],
    nodes: Arc<[[f32; 3]]>,
    id: LookId,
}

impl PartialEq for Lut3D {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Lut3D {
    pub fn new(
        size: u32,
        min: [f32; 3],
        max: [f32; 3],
        nodes: Vec<[f32; 3]>,
    ) -> Result<Self, Error> {
        require(
            (2..=MAX_GRID_SIZE).contains(&size),
            "unsupported 3D LUT size (expected 2–129)",
        )?;
        require(
            nodes.len() == (size as usize).pow(3),
            "3D LUT node count differs from its dimensions",
        )?;
        for c in 0..3 {
            let span = max[c] - min[c];
            require(
                min[c].is_finite()
                    && max[c].is_finite()
                    && span.is_finite()
                    && span > 0.
                    && (1. / span).is_finite(),
                "invalid or unsupported LUT input domain",
            )?;
        }
        require(
            nodes
                .iter()
                .flatten()
                .all(|v| v.is_finite() && (photographic::decode(f64::from(*v)) as f32).is_finite()),
            "LUT output cannot be decoded to finite scene-linear float32",
        )?;
        let mut hash = blake3::Hasher::new();
        hash.update(CONTRACT.as_bytes());
        hash.update(&size.to_le_bytes());
        for v in min.iter().chain(max.iter()).chain(nodes.iter().flatten()) {
            hash.update(&v.to_le_bytes());
        }
        let id = LookId(*hash.finalize().as_bytes());
        Ok(Self {
            size,
            min,
            max,
            nodes: nodes.into(),
            id,
        })
    }

    pub fn id(&self) -> LookId {
        self.id
    }

    pub fn size(&self) -> u32 {
        self.size
    }

    pub fn domain(&self) -> ([f32; 3], [f32; 3]) {
        (self.min, self.max)
    }

    pub fn nodes(&self) -> &[[f32; 3]] {
        &self.nodes
    }

    pub fn byte_len(&self) -> usize {
        self.nodes.len() * 12
    }

    /// Double-precision tetrahedral reference with component-wise input clipping.
    pub fn sample_encoded(&self, input: DVec3) -> Result<DVec3, Error> {
        let input = input.to_array();
        let mut cell = [0; 3];
        let mut fraction = [0.; 3];
        for c in 0..3 {
            let lo = f64::from(self.min[c]);
            let hi = f64::from(self.max[c]);
            require(input[c].is_finite(), "nonfinite ACEScct look input")?;
            let position = (input[c].clamp(lo, hi) - lo) / (hi - lo) * f64::from(self.size - 1);
            cell[c] = (position as u32).min(self.size - 2);
            fraction[c] = position - f64::from(cell[c]);
        }
        let at = |p: [u32; 3]| {
            DVec3::from_array(
                self.nodes[((p[2] * self.size + p[1]) * self.size + p[0]) as usize].map(f64::from),
            )
        };
        let mut order = [0, 1, 2];
        order.sort_by(|&a, &b| fraction[b].total_cmp(&fraction[a]));
        let [a, b, c] = order.map(|i| fraction[i]);
        let mut out = at(cell) * (1. - a);
        cell[order[0]] += 1;
        out += at(cell) * (a - b);
        cell[order[1]] += 1;
        out += at(cell) * (b - c);
        cell[order[2]] += 1;
        Ok(out + at(cell) * c)
    }

    pub fn apply(&self, linear_ap1: DVec3) -> Result<DVec3, Error> {
        require(linear_ap1.is_finite(), "nonfinite scene-linear look input")?;
        // A finite negative AP1 value can overflow ACEScct's linear branch.
        // Clip the encoded result before admission; nonfinite source values still reject.
        let encoded = linear_ap1.map(photographic::encode).clamp(
            DVec3::from_array(self.min.map(f64::from)),
            DVec3::from_array(self.max.map(f64::from)),
        );
        let output = self.sample_encoded(encoded)?.map(photographic::decode);
        require(output.is_finite(), "nonfinite scene-linear look output")?;
        Ok(output)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "project", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "project",
    serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)
)]
pub enum LookOrigin {
    Imported,
    Builtin {
        id: String,
        version: u32,
        generator: String,
        definition: String,
    },
}

impl LookOrigin {
    fn validate(&self) -> Result<(), Error> {
        if let Self::Builtin {
            id,
            version,
            generator,
            definition,
        } = self
        {
            require(
                !id.is_empty()
                    && id.len() <= 128
                    && id
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
                "invalid built-in look identity",
            )?;
            require(
                *version > 0
                    && !generator.is_empty()
                    && generator.len() <= 128
                    && !generator.chars().any(char::is_control),
                "invalid built-in look version/generator",
            )?;
            require(
                definition.len() == 64 && definition.bytes().all(|b| b.is_ascii_hexdigit()),
                "invalid built-in definition digest",
            )?;
        }
        Ok(())
    }
}

/// A project owns the resolved samples, even when their authoring definition is unavailable.
#[derive(Debug, Clone, PartialEq)]
pub struct LookAsset {
    name: String,
    origin: LookOrigin,
    lut: Arc<Lut3D>,
}

impl LookAsset {
    pub fn new(name: String, origin: LookOrigin, lut: Arc<Lut3D>) -> Result<Self, Error> {
        require(
            !name.trim().is_empty() && name.len() <= 512 && !name.chars().any(char::is_control),
            "invalid look name",
        )?;
        origin.validate()?;
        Ok(Self { name, origin, lut })
    }

    /// Explicit asset payload, conservatively charging shared LUT samples without deduplication.
    pub fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.name.capacity()
            + std::mem::size_of::<Lut3D>()
            + self.lut.byte_len()
            + match &self.origin {
                LookOrigin::Imported => 0,
                LookOrigin::Builtin {
                    id,
                    generator,
                    definition,
                    ..
                } => id.capacity() + generator.capacity() + definition.capacity(),
            }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn origin(&self) -> &LookOrigin {
        &self.origin
    }

    pub fn lut(&self) -> &Arc<Lut3D> {
        &self.lut
    }

    /// `.cube` carries no reliable colour-space declaration. Import explicitly means CONTRACT.
    /// 1D shapers, display/colour conversions and unknown directives are not inferred or repaired.
    pub fn import_cube(text: &str, fallback_name: &str) -> Result<Self, Error> {
        require(
            text.len() <= MAX_IMPORT_BYTES,
            "LUT file exceeds the import budget",
        )?;
        let mut size = None;
        let mut min = None;
        let mut max = None;
        let mut title = None;
        let mut nodes = Vec::new();
        for (index, raw) in text.lines().enumerate() {
            let line = raw.trim().trim_start_matches('\u{feff}');
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let fail = |message: &str| failure(format!("line {}: {message}", index + 1));
            if let Some(value) = line
                .strip_prefix("TITLE ")
                .or_else(|| line.strip_prefix("TITLE\t"))
            {
                require(
                    title.is_none() && nodes.is_empty(),
                    "duplicate or misplaced TITLE",
                )?;
                title = Some(parse_title(value).map_err(|e| fail(&e.to_string()))?);
                continue;
            }
            let content = line.split('#').next().unwrap().trim();
            let fields = content.split_whitespace().take(5).collect::<Vec<_>>();
            require(fields.len() <= 4, "too many fields in LUT line")?;
            if fields.is_empty() {
                continue;
            }
            match fields[0] {
                "LUT_3D_SIZE" => {
                    require(
                        size.is_none() && nodes.is_empty() && fields.len() == 2,
                        "duplicate or malformed LUT_3D_SIZE",
                    )?;
                    let n = fields[1]
                        .parse::<u32>()
                        .map_err(|_| fail("invalid grid size"))?;
                    require(
                        (2..=MAX_GRID_SIZE).contains(&n),
                        "unsupported 3D LUT size (expected 2–129)",
                    )?;
                    size = Some(n);
                }
                "DOMAIN_MIN" | "DOMAIN_MAX" => {
                    require(nodes.is_empty(), "domain must precede LUT data")?;
                    let entry = if fields[0] == "DOMAIN_MIN" {
                        &mut min
                    } else {
                        &mut max
                    };
                    require(
                        entry.is_none() && fields.len() == 4,
                        "duplicate or malformed input domain",
                    )?;
                    *entry = Some(
                        parse_triplet(&fields[1..]).map_err(|_| fail("invalid domain values"))?,
                    );
                }
                _ => {
                    let n = size.ok_or_else(|| fail("expected LUT_3D_SIZE before data"))?;
                    require(nodes.len() < (n as usize).pow(3), "too many LUT nodes")?;
                    nodes.push(parse_triplet(&fields).map_err(|_| {
                        fail("expected three finite values; unsupported directives/shapers reject")
                    })?);
                }
            }
        }
        let size = size.ok_or_else(|| failure("missing LUT_3D_SIZE"))?;
        let lut = Lut3D::new(size, min.unwrap_or([0.; 3]), max.unwrap_or([1.; 3]), nodes)?;
        Self::new(
            title.unwrap_or_else(|| fallback_name.to_owned()),
            LookOrigin::Imported,
            Arc::new(lut),
        )
    }
}
fn parse_title(value: &str) -> Result<String, Error> {
    let value = value.trim();
    let content = value
        .strip_prefix('"')
        .ok_or_else(|| failure("TITLE must be quoted"))?;
    let mut title = String::new();
    let mut escaped = false;
    for (i, c) in content.char_indices() {
        if escaped {
            title.push(c);
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == '"' {
            let suffix = content[i + 1..].trim();
            require(
                suffix.is_empty() || suffix.starts_with('#'),
                "invalid TITLE suffix",
            )?;
            return Ok(title);
        } else {
            title.push(c);
        }
        require(title.len() <= 256, "TITLE exceeds the look-name budget")?;
    }
    Err(failure("unterminated TITLE"))
}
fn parse_triplet(fields: &[&str]) -> Result<[f32; 3], Error> {
    require(fields.len() == 3, "expected three LUT values")?;
    let mut result = [0.; 3];
    for (out, field) in result.iter_mut().zip(fields) {
        *out = field
            .parse::<f32>()
            .map_err(|_| failure("invalid LUT number"))?;
        require(out.is_finite(), "nonfinite LUT number")?;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn identity(n: u32) -> Lut3D {
        let mut nodes = Vec::new();
        for b in 0..n {
            for g in 0..n {
                for r in 0..n {
                    nodes.push([r, g, b].map(|v| -0.5 + v as f32 / (n - 1) as f32 * 1.5));
                }
            }
        }
        Lut3D::new(n, [-0.5; 3], [1.; 3], nodes).unwrap()
    }

    #[test]
    fn tetrahedral_identity_covers_signed_values_all_tetrahedra_and_endpoints() {
        let lut = identity(3);
        for r in -2..=4 {
            for g in -2..=4 {
                for b in -2..=4 {
                    let input = DVec3::new(r as f64, g as f64, b as f64) * 0.25;
                    assert!(
                        (lut.sample_encoded(input).unwrap() - input)
                            .abs()
                            .max_element()
                            < 1e-14
                    );
                }
            }
        }
        let scene = DVec3::new(-0.01, 0.18, 20.);
        assert!((lut.apply(scene).unwrap() - scene).abs().max_element() < 1e-12);
    }
    #[test]
    fn tetrahedral_corner_products_form_hinges_in_all_six_orders_and_on_ties() {
        let mut nodes = Vec::new();
        for b in 0..2 {
            for g in 0..2 {
                for r in 0..2 {
                    nodes.push([(r * g) as f32, (g * b) as f32, (b * r) as f32]);
                }
            }
        }
        let lut = Lut3D::new(2, [0.; 3], [1.; 3], nodes).unwrap();
        // On each tetrahedron the corner products extend as min of each axis pair;
        // trilinear interpolation would give products such as 0.2*0.5 instead.
        for point in [
            [0.2, 0.5, 0.8],
            [0.2, 0.8, 0.5],
            [0.5, 0.2, 0.8],
            [0.5, 0.8, 0.2],
            [0.8, 0.2, 0.5],
            [0.8, 0.5, 0.2],
            [0.2, 0.2, 0.8],
            [0.2, 0.8, 0.2],
            [0.8, 0.2, 0.2],
            [0.5; 3],
            [0.; 3],
            [1.; 3],
            [0., 0.5, 1.],
        ] {
            let [r, g, b]: [f64; 3] = point;
            let expected = DVec3::new(r.min(g), g.min(b), b.min(r));
            assert!(
                (lut.sample_encoded(DVec3::from_array(point)).unwrap() - expected)
                    .abs()
                    .max_element()
                    < 1e-14
            );
        }
    }

    #[test]
    fn finite_inputs_clip_to_boundary_without_regrading_in_range_channels() {
        let lut = identity(3);
        let encoded = DVec3::new(-0.6, 0.3, 1.5);
        let expected = DVec3::new(-0.5, encoded.y, 1.);
        assert!(
            (lut.sample_encoded(encoded).unwrap() - expected)
                .abs()
                .max_element()
                < 1e-14
        );
        let extreme = lut.apply(DVec3::new(-f64::MAX, f64::MAX, 0.18)).unwrap();
        let expected = DVec3::new(photographic::decode(-0.5), photographic::decode(1.), 0.18);
        assert!((extreme - expected).abs().max_element() < 1e-12);
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(lut.sample_encoded(DVec3::splat(value)).is_err());
            assert!(lut.apply(DVec3::splat(value)).is_err());
        }
    }

    #[test]
    fn oversized_lines_and_titles_reject_before_unbounded_token_or_name_collection() {
        let line = "0 ".repeat(1 << 19);
        let error = LookAsset::import_cube(&format!("LUT_3D_SIZE 2\n{line}"), "test").unwrap_err();
        assert!(error.to_string().contains("too many fields"));
        let error = LookAsset::import_cube(&format!("TITLE \"{}\"\n", "x".repeat(1024)), "test")
            .unwrap_err();
        assert!(error.to_string().contains("name budget"));
    }

    #[test]
    fn malformed_assets_reject_before_use() {
        assert!(Lut3D::new(1, [0.; 3], [1.; 3], vec![]).is_err());
        assert!(Lut3D::new(130, [0.; 3], [1.; 3], vec![]).is_err());
        assert!(Lut3D::new(2, [0.; 3], [0.; 3], vec![[0.; 3]; 8]).is_err());
        assert!(Lut3D::new(2, [0.; 3], [1.; 3], vec![[0.; 3]; 7]).is_err());
        assert!(Lut3D::new(2, [0.; 3], [1.; 3], vec![[f32::NAN; 3]; 8]).is_err());
        assert!(Lut3D::new(2, [0.; 3], [1.; 3], vec![[100.; 3]; 8]).is_err());
    }

    #[test]
    fn imports_preserve_order_domain_dimension_and_negative_outputs() {
        let text = "TITLE \"My # Look\" # comment\nLUT_3D_SIZE 2\nDOMAIN_MIN -0.5 -0.5 -0.5\nDOMAIN_MAX 1 1 1\n-0.5 -0.5 -0.5\n1 -0.5 -0.5\n-0.5 1 -0.5\n1 1 -0.5\n-0.5 -0.5 1\n1 -0.5 1\n-0.5 1 1\n1 1 1\n";
        let asset = LookAsset::import_cube(text, "fallback").unwrap();
        assert_eq!(asset.name(), "My # Look");
        assert_eq!(
            parse_title("\"A # B\" # comment \"quoted\"").unwrap(),
            "A # B"
        );
        assert_eq!(parse_title("\"A \\\"B\\\"\"").unwrap(), "A \"B\"");
        assert!(parse_title("\"unterminated").is_err());
        assert_eq!(*asset.lut(), Arc::new(identity(2)));
        for bad in [
            text.replace("LUT_3D_SIZE 2", "LUT_1D_SIZE 2"),
            text.replace("DOMAIN_MAX 1 1 1", "DOMAIN_MAX 0 0 0\nDOMAIN_MAX 1 1 1"),
            text.replace("-0.5 -0.5 -0.5\n1", "NaN -0.5 -0.5\n1"),
            format!("{text}0 0 0\n"),
            text.replace("1 1 1\n", ""),
        ] {
            assert!(LookAsset::import_cube(&bad, "fallback").is_err());
        }
    }

    #[test]
    fn numeric_identity_excludes_name_origin_but_includes_all_execution_values() {
        let a = identity(2);
        let mut data = a.nodes().to_vec();
        data[0][0] += 0.001;
        let b = Lut3D::new(2, a.min, a.max, data).unwrap();
        assert_ne!(a.id(), b.id());
        assert_ne!(a.id(), identity(3).id());
        let asset =
            LookAsset::new("one".into(), LookOrigin::Imported, Arc::new(a.clone())).unwrap();
        let other = LookAsset::new("two".into(), LookOrigin::Imported, Arc::new(a)).unwrap();
        assert_eq!(asset.lut().id(), other.lut().id());
        assert_ne!(asset, other);
    }
}
