//! Nikon Z FX NEFInfo lens decoding, checked against camera JPEGs and NX Studio.
use super::{Metadata, Rect};
use crate::{
    Error,
    lens::{EmbeddedLensGeometry, EmbeddedLensProfile},
};
use glam::DVec2;

#[repr(C)]
pub(super) struct NefInfo {
    pub(super) size: u32,
    pub(super) bytes: [u8; 4096],
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Optics {
    distortion: Option<Distortion>,
    vignette: Option<Vignette>,
    center_shift: DVec2,
}

impl Optics {
    pub(super) fn read(blob: &[u8]) -> Option<Self> {
        let tiff = blob.strip_prefix(b"Nikon\0")?.get(4..)?;
        // The nested stream is little-endian even when the outer TIFF is not.
        if !tiff.starts_with(b"II\x2a\0") {
            return None;
        }
        let directory = word(tiff, 4)? as usize;
        if directory < 8 {
            return None;
        }
        let count = u16::from_le_bytes(bytes(tiff, directory)?) as usize;
        let table = directory.checked_add(2)?;
        let end = table.checked_add(count * 12)?.checked_add(4)?;
        tiff.get(..end)?;
        let (entries, _) = tiff[table..end - 4].as_chunks::<12>();
        let record = |tag: u16| {
            let mut matches = entries
                .iter()
                .filter(|entry| entry[..2] == tag.to_le_bytes());
            let entry = matches.next()?;
            if matches.next().is_some() || u16::from_le_bytes(bytes(entry, 2)?) != 7 {
                return None;
            }
            let offset = word(entry, 8)? as usize;
            let length = word(entry, 4)? as usize;
            if offset < end {
                return None;
            }
            tiff.get(offset..offset.checked_add(length)?)
        };
        let center = record(8)?;
        if center.len() != 24 || !center.starts_with(b"0100") {
            return None;
        }
        let center_shift = DVec2::new(rational(center, 4)?, rational(center, 12)?);
        let distortion = record(5).and_then(Distortion::read);
        let vignette = record(6).and_then(Vignette::read);
        (distortion.is_some() || vignette.is_some()).then_some(Self {
            distortion,
            vignette,
            center_shift,
        })
    }

    pub(super) fn geometry(
        self,
        metadata: &Metadata,
    ) -> Result<Option<EmbeddedLensGeometry>, Error> {
        // Unknown capture modes cannot reuse the tested FX calibration just because they
        // carry the same radius tag (DX files in particular do).
        if !supported_active(
            &metadata.model(),
            [metadata.inset_width, metadata.inset_height],
        ) || metadata.inset_left == u32::from(u16::MAX)
            || metadata.inset_top == u32::from(u16::MAX)
        {
            return Ok(None);
        }
        Ok(self.warp(metadata.native_image_area()?))
    }

    fn warp(self, area: Rect) -> Option<EmbeddedLensGeometry> {
        if self.center_shift.x.abs() > f64::from(area.width) * 0.05
            || self.center_shift.y.abs() > f64::from(area.height) * 0.05
        {
            return None;
        }
        const KNOTS: usize = 513;
        // Invalid components are independent: a folded warp does not discard valid shading,
        // and invalid shading does not change the distortion's framing.
        EmbeddedLensGeometry::with_shifted_profile(area, self.center_shift, |radius_ratio| {
            let mut profile = self
                .distortion
                .and_then(|distortion| {
                    EmbeddedLensProfile::from_inverse_distortion(KNOTS, |r| {
                        distortion.scale(r * radius_ratio)
                    })
                    .ok()
                })
                .unwrap_or_default();
            if let Some(vignette) = self.vignette {
                let _ = profile.set_radial_gain(KNOTS, |r| vignette.gain(r * radius_ratio));
            }
            profile
        })
        .ok()
        .flatten()
    }
}

#[derive(Debug, Clone, Copy)]
struct Distortion {
    low_to_high: [f64; 3],
    zoom: f64,
}

impl Distortion {
    fn read(block: &[u8]) -> Option<Self> {
        if block.len() != 84
            || !block.starts_with(b"0100")
            || !matches!(block[4], 1 | 3)
            || (rational(block, 8)? - 21.6).abs() > 1e-4
            || word(block, 16)? != 4
        {
            return None;
        }
        let low_to_high = [
            rational(block, 36)?,
            rational(block, 28)?,
            rational(block, 20)?,
        ];
        let zoom = rational(block, 64)?;
        if low_to_high.iter().any(|&c| c.abs() > 1.)
            || zoom.abs() > 0.5
            || (low_to_high == [0.; 3] && zoom == 0.)
        {
            return None;
        }
        Some(Self { low_to_high, zoom })
    }

    fn scale(self, radius: f64) -> f64 {
        let [a, b, c] = self.low_to_high;
        let r2 = radius * radius;
        (1. + r2 * (a + r2 * (b + r2 * c))) / (1. + self.zoom)
    }
}

#[derive(Debug, Clone, Copy)]
struct Vignette {
    low_to_high: [f64; 4],
    strength: f64,
}

impl Vignette {
    fn read(block: &[u8]) -> Option<Self> {
        // Byte 4 is 1 for enabled, 2 for camera Off. Admit the measured single-polynomial layout.
        if block.len() != 116
            || !block.starts_with(b"0100\x01\x01\0\0")
            || (rational(block, 8)? - 21.6).abs() > 1e-4
            || word(block, 16)? != 8
        {
            return None;
        }
        for offset in [28, 44, 60, 76] {
            if rational(block, offset)? != 0. {
                return None;
            }
        }
        let low_to_high = [
            rational(block, 68)?,
            rational(block, 52)?,
            rational(block, 36)?,
            rational(block, 20)?,
        ];
        let strength = rational(block, 96)?;
        if strength <= 0.
            || strength > 1.
            || low_to_high == [0.; 4]
            || low_to_high.iter().any(|&c| c.abs() > 8.)
        {
            return None;
        }
        Some(Self {
            low_to_high,
            strength,
        })
    }

    fn gain(self, radius: f64) -> f64 {
        let [a, b, c, d] = self.low_to_high;
        let r2 = radius * radius;
        1. + self.strength * r2 * (a + r2 * (b + r2 * (c + r2 * d)))
    }
}

fn supported_active(model: &str, size: [u32; 2]) -> bool {
    matches!(
        (model, size),
        ("Z 6" | "Z 6_2", [6048, 4024])
            | ("Z f", [6048, 4032])
            | ("Z 7" | "Z 8" | "Z 9", [8256, 5504])
    )
}

fn bytes<const N: usize>(data: &[u8], at: usize) -> Option<[u8; N]> {
    data.get(at..at.checked_add(N)?)?.try_into().ok()
}

fn word(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes(data, at)?))
}

fn rational(data: &[u8], at: usize) -> Option<f64> {
    let numerator = i32::from_le_bytes(bytes(data, at)?);
    let denominator = i32::from_le_bytes(bytes(data, at.checked_add(4)?)?);
    (denominator > 0).then(|| f64::from(numerator) / f64::from(denominator))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raw::RawFile;

    fn ratio(block: &mut [u8], at: usize, n: i32, d: i32) {
        block[at..at + 4].copy_from_slice(&n.to_le_bytes());
        block[at + 4..at + 8].copy_from_slice(&d.to_le_bytes());
    }

    fn vignette() -> [u8; 116] {
        let mut block = [0; 116];
        block[..8].copy_from_slice(b"0100\x01\x01\0\0");
        ratio(&mut block, 8, 108, 5);
        block[16] = 8;
        for (i, (n, d)) in [(1, 16), (1, 4), (-1, 8), (1, 2)].into_iter().enumerate() {
            ratio(&mut block, 20 + i * 16, n, d);
            ratio(&mut block, 28 + i * 16, 0, 1);
        }
        ratio(&mut block, 96, 1, 2);
        block
    }

    fn fixture(mode: u8, vignette: Option<&[u8]>) -> Vec<u8> {
        let mut distortion = [0; 84];
        distortion[..4].copy_from_slice(b"0100");
        distortion[4] = mode;
        ratio(&mut distortion, 8, 108, 5);
        distortion[16] = 4;
        ratio(&mut distortion, 20, -1, 32);
        ratio(&mut distortion, 28, 1, 16);
        ratio(&mut distortion, 36, -1, 8);
        ratio(&mut distortion, 64, 1, 16);
        let mut center = [0; 24];
        center[..4].copy_from_slice(b"0100");
        ratio(&mut center, 4, 4, 1);
        ratio(&mut center, 12, -3, 1);

        let mut nef = b"Nikon\0\x01\x02\0\0II\x2a\0\x08\0\0\0".to_vec();
        let mut records: Vec<(u16, &[u8])> = vec![(5, &distortion), (8, &center)];
        if let Some(block) = vignette {
            records.push((6, block));
        }
        nef.extend((records.len() as u16).to_le_bytes());
        let mut offset = 14 + records.len() as u32 * 12;
        for &(tag, block) in &records {
            let size = block.len() as u32;
            nef.extend(tag.to_le_bytes());
            nef.extend(7_u16.to_le_bytes());
            nef.extend(size.to_le_bytes());
            nef.extend(offset.to_le_bytes());
            offset += size;
        }
        nef.extend([0; 4]);
        for (_, block) in records {
            nef.extend(block);
        }
        nef
    }

    #[test]
    fn nikon_nef_info_honors_modes_and_refuses_truncation_and_invalid_offsets() {
        for mode in [1, 3] {
            assert!(Optics::read(&fixture(mode, None)).is_some());
        }
        for mode in [0, 2, 4] {
            assert!(Optics::read(&fixture(mode, None)).is_none());
        }
        let data = fixture(3, None);
        for n in 0..data.len() {
            assert!(Optics::read(&data[..n]).is_none(), "accepted length {n}");
        }
        let mut corrupt = data.clone();
        corrupt[40..44].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(Optics::read(&corrupt).is_none());
        let mut invalid_ratio = data;
        invalid_ratio[48 + 40..48 + 44].fill(0);
        assert!(Optics::read(&invalid_ratio).is_none());
    }

    #[test]
    fn nikon_distortion_uses_camera_zoom_and_sensor_shift_without_moving_the_frame() {
        let geometry = Optics::read(&fixture(3, None))
            .unwrap()
            .warp(Rect::from([8, 4, 128, 96]))
            .unwrap();
        assert_eq!(geometry.size(), [128, 96]);
        // The active half-diagonal is 80px; the shifted centre is (75.5, 48.5).
        let center = DVec2::new(75.5, 48.5);
        for [x, y] in [[0, 0], [22, 8], [68, 45], [127, 95], [0, 95]] {
            let offset = DVec2::new(f64::from(x) + 8., f64::from(y) + 4.) - center;
            let r = offset.length() / 80.;
            let factor = (1. - r.powi(2) / 8. + r.powi(4) / 16. - r.powi(6) / 32.) * (16. / 17.);
            let expected = center + offset * factor;
            for actual in geometry.sample_positions(x, y).unwrap() {
                assert!(
                    (actual - expected).length() < 0.001,
                    "{actual} != {expected}"
                );
            }
        }
    }

    #[test]
    fn nikon_expanding_map_trims_against_the_farthest_edges_and_folds_are_skipped() {
        let area = Rect::from([8, 4, 128, 96]);
        let mut optics = Optics {
            distortion: Some(Distortion {
                low_to_high: [0.; 3],
                zoom: -0.25,
            }),
            vignette: None,
            center_shift: DVec2::new(4., -3.),
        };
        let geometry = optics.warp(area).unwrap();
        // A 4/3 expansion needs ceil(67.5/4), ceil(50.5/4) on each side.
        assert_eq!(geometry.size(), [94, 70]);
        for [x, y] in [[0, 0], [93, 0], [0, 69], [93, 69]] {
            for point in geometry.sample_positions(x, y).unwrap() {
                assert!(point.cmpge(DVec2::new(8., 4.)).all());
                assert!(point.cmple(DVec2::new(135., 99.)).all());
            }
        }
        optics.distortion.as_mut().unwrap().low_to_high[0] = -0.75;
        assert!(optics.warp(area).is_none());
        optics.vignette = Vignette::read(&vignette());
        let shading_only = optics.warp(area).unwrap();
        assert_eq!(shading_only.size(), [area.width, area.height]);
        assert!(shading_only.source_gain(area.x, area.y) > 1.);
        assert!(!supported_active("Z 50", [5568, 3712]));
        assert!(!supported_active("Z 8", [5392, 3592]));
    }

    #[test]
    fn nikon_shading_honors_camera_off_and_rejects_unsupported_records_without_losing_distortion() {
        let mut disabled = vignette();
        disabled[4] = 2;
        let mut off = disabled;
        ratio(&mut off, 96, 0, 1024);
        let mut enabled_zero = vignette();
        ratio(&mut enabled_zero, 96, 0, 1024);
        let mut unsupported_pair = vignette();
        ratio(&mut unsupported_pair, 60, 1, 16);
        let mut malformed = vignette();
        ratio(&mut malformed, 36, 1, 0);
        let area = Rect::from([8, 4, 128, 96]);
        for block in [
            &off[..],
            &disabled,
            &enabled_zero,
            &unsupported_pair,
            &malformed,
            &off[..115],
        ] {
            assert!(Optics::read(&fixture(0, Some(block))).is_none());
            let optics = Optics::read(&fixture(3, Some(block))).unwrap();
            assert!(optics.vignette.is_none());
            let geometry = optics.warp(area).unwrap();
            assert_eq!(geometry.source_gain(8, 4), 1.);
            assert_ne!(
                geometry.sample_positions(0, 0).unwrap()[0],
                DVec2::new(8., 4.)
            );
        }

        let mut nonpositive_gain = vignette();
        ratio(&mut nonpositive_gain, 68, -8, 1);
        let optics = Optics::read(&fixture(3, Some(&nonpositive_gain))).unwrap();
        let geometry = optics.warp(area).unwrap();
        assert_eq!(geometry.source_gain(8, 4), 1.);
        assert_ne!(
            geometry.sample_positions(0, 0).unwrap()[0],
            DVec2::new(8., 4.)
        );
    }

    #[test]
    fn nikon_shading_uses_the_shifted_source_radius_and_all_four_recorded_terms() {
        let area = Rect::from([8, 4, 128, 96]);
        let block = vignette();
        let optics = Optics::read(&fixture(0, Some(&block))).unwrap();
        let geometry = optics.warp(area).unwrap();
        let warped = Optics::read(&fixture(3, Some(&block)))
            .unwrap()
            .warp(area)
            .unwrap();
        assert_eq!(geometry.size(), [128, 96]);
        // The upper-left sample is outside the centred half-diagonal after the shift.
        for [x, y] in [[8, 4], [22, 8], [76, 49], [130, 96]] {
            let r = (x as f64 - 75.5).hypot(y as f64 - 48.5) / 80.;
            let expected = 1. + r.powi(2) / 4. - r.powi(4) / 16. + r.powi(6) / 8. + r.powi(8) / 32.;
            let actual = geometry.source_gain(x, y);
            assert!((expected - actual).abs() < 3e-6, "{expected} != {actual}");
            assert_eq!(actual, warped.source_gain(x, y));
        }
    }

    #[test]
    #[ignore = "requires Nikon NEFs with inactive/unsupported profiles in ANTITONE_NIKON_UNCORRECTED_SAMPLES (path-separated)"]
    fn real_nikon_unsupported_profiles_keep_native_framing() {
        let paths =
            std::env::var_os("ANTITONE_NIKON_UNCORRECTED_SAMPLES").expect("set sample paths");
        let paths: Vec<_> = std::env::split_paths(&paths).collect();
        assert!(!paths.is_empty());
        for path in paths {
            let raw = RawFile::open(&path).unwrap();
            assert!(
                raw.metadata.lens_geometry().unwrap().is_none(),
                "{}",
                path.display()
            );
            assert_eq!(
                raw.metadata.image_area().unwrap(),
                raw.metadata.native_image_area().unwrap()
            );
        }
    }

    #[test]
    #[ignore = "requires z7.NEF, z9.NEF and zf.NEF from the NX Studio comparison in ANTITONE_NIKON_NX_SAMPLES"]
    fn real_nikon_linear_development_matches_nx_studio_shading() {
        let root = std::path::PathBuf::from(
            std::env::var_os("ANTITONE_NIKON_NX_SAMPLES")
                .expect("set NX comparison sample directory"),
        );
        // Off/recorded-strength NX Studio exports: independently exposure-calibrated
        // median correction, measured in the radial bin about 0.775 on the source raster.
        for (name, expected_ev) in [("z7.NEF", 0.2166), ("z9.NEF", 0.3327), ("zf.NEF", 0.0835)] {
            let path = root.join(name);
            let mut raw = RawFile::open(&path).unwrap();
            let geometry = raw.metadata.lens_geometry().unwrap().unwrap();
            let area = geometry.original_area();
            let half = DVec2::new(area.width as f64, area.height as f64) / 2.;
            let center = DVec2::new(area.x as f64, area.y as f64) + half - DVec2::splat(0.5)
                + raw.metadata.nikon_optics.unwrap().center_shift;
            let corrected = raw.develop_camera([1.; 4]).unwrap();
            let mut baseline = RawFile::open_uncorrected(&path).unwrap();
            let baseline = baseline.develop_camera([1.; 4]).unwrap();
            let mut measured = Vec::new();
            for j in 0..80 {
                for i in 0..80 {
                    let x = corrected.width * i / 80;
                    let y = corrected.height * j / 80;
                    let point = geometry.sample_positions(x, y).unwrap()[1];
                    let radius = point.distance(center) / half.length();
                    if (radius - 0.775).abs() > 0.004 {
                        continue;
                    }
                    let mut reference = 0.;
                    for sy in [point.y.floor() as u32, point.y.floor() as u32 + 1] {
                        for sx in [point.x.floor() as u32, point.x.floor() as u32 + 1] {
                            let weight = (1. - (point.x - f64::from(sx)).abs())
                                * (1. - (point.y - f64::from(sy)).abs());
                            if weight > 0. {
                                reference +=
                                    weight * f64::from(baseline.pixel_samples(sx, sy).unwrap()[1]);
                            }
                        }
                    }
                    if reference > 64. {
                        let gain = f64::from(corrected.pixel_samples(x, y).unwrap()[1]) / reference;
                        measured.push(gain.log2());
                    }
                }
            }
            assert!(measured.len() > 20, "too few viable samples in {name}");
            measured.sort_by(f64::total_cmp);
            let median = measured[measured.len() / 2];
            eprintln!("{name}: measured {median:.4} EV, NX Studio median {expected_ev:.4} EV");
            assert!((median - expected_ev).abs() < 0.015, "{name}: {median} EV");
        }
    }

    #[test]
    #[ignore = "requires Nikon FX NEFs with enabled distortion and shading in ANTITONE_NIKON_Z_SAMPLES (path-separated)"]
    fn real_nikon_z_selected_raw_profile_is_applied_in_development() {
        let paths = std::env::var_os("ANTITONE_NIKON_Z_SAMPLES").expect("set sample paths");
        let paths: Vec<_> = std::env::split_paths(&paths).collect();
        assert!(!paths.is_empty());
        for path in paths {
            let mut raw = RawFile::open(&path).unwrap();
            let baseline = RawFile::open_uncorrected(&path).unwrap();
            let geometry = raw
                .metadata
                .lens_geometry()
                .unwrap()
                .unwrap_or_else(|| panic!("missing correction: {}", path.display()));
            let active = baseline.metadata.native_image_area().unwrap();
            assert_eq!(geometry.original_area(), active);
            assert!(
                geometry.source_gain(active.x, active.y) > 1.,
                "select nontrivial corner shading"
            );
            let [width, height] = geometry.size();
            assert!(width <= active.width && height <= active.height);
            let point = [width / 4, height / 4];
            let [r, g, b] = geometry.sample_positions(point[0], point[1]).unwrap();
            assert_eq!(r, g);
            assert_eq!(g, b);
            let unwarped = DVec2::new(
                (active.x + point[0] + (active.width - width) / 2) as f64,
                (active.y + point[1] + (active.height - height) / 2) as f64,
            );
            assert!(
                (g - unwarped).length() > 1.,
                "select a nontrivial correction"
            );
            let image = raw.develop_camera([1.; 4]).unwrap();
            assert_eq!([image.width, image.height], [width, height]);
            assert!(image.is_continuous());
            eprintln!(
                "{}: {active:?} -> {width}x{height}, quarter-frame displacement {:.1}px",
                path.display(),
                (g - unwarped).length()
            );
        }
    }
}
