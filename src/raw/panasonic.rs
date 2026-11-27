//! Native RW2 DistortionInfo, in the fixed-point layout documented by ExifTool and
//! the half-diagonal radial model used by darktable 5.8.
use super::{Metadata, Rect};
use crate::{
    Error,
    lens::{EmbeddedLensGeometry, EmbeddedLensProfile},
};

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(super) struct LensData {
    status: u32,
    words: [u16; 16],
    sensor_bounds: [u16; 4],
}

struct Distortion([f64; 3]);

impl LensData {
    fn distortion(&self) -> Result<Option<Distortion>, Error> {
        match self.status {
            0 => return Ok(None),
            1 => (),
            _ => return Err(invalid("malformed DistortionInfo tag")),
        }
        if self.words[7] & 0xf != 1 {
            return Ok(None);
        }
        let scale = 32768. + f64::from(self.words[5] as i16);
        if scale == 0. {
            return Err(invalid("invalid distortion scale"));
        }
        let coefficients = [8, 4, 11].map(|i| f64::from(self.words[i] as i16) / scale);
        if coefficients == [0.; 3] {
            return Ok(None);
        }
        Ok(Some(Distortion(coefficients)))
    }

    pub(super) fn geometry(
        &self,
        metadata: &Metadata,
    ) -> Result<Option<EmbeddedLensGeometry>, Error> {
        let Some(distortion) = self.distortion()? else {
            return Ok(None);
        };
        let [top, left, bottom, right] = self.sensor_bounds.map(u32::from);
        let active = Rect::from([
            left.checked_sub(metadata.left_margin)
                .ok_or_else(|| invalid("invalid sensor left edge"))?,
            top.checked_sub(metadata.top_margin)
                .ok_or_else(|| invalid("invalid sensor top edge"))?,
            right.saturating_sub(left),
            bottom.saturating_sub(top),
        ]);
        if !Rect::from([0, 0, metadata.width, metadata.height]).contains_region(active) {
            return Err(invalid("sensor bounds are outside the decoded raster"));
        }
        EmbeddedLensGeometry::with_framing(
            distortion.profile()?,
            active,
            metadata.native_image_area()?,
        )
        .map(Some)
    }
}

impl Distortion {
    fn forward(&self, radius: f64) -> f64 {
        let [a, b, c] = self.0;
        let square = radius * radius;
        radius * (1. + square * (a + square * (b + square * c)))
    }

    fn profile(&self) -> Result<EmbeddedLensProfile, Error> {
        let [a, b, c] = self.0;
        // Positive Bernstein coefficients bound dRu/dRd on the sensor's unit disc.
        // Reject a map whose monotonicity these bounds cannot establish.
        if [
            1. + a,
            1. + 2. * a + (5. / 3.) * b,
            1. + 3. * a + 5. * b + 7. * c,
        ]
        .into_iter()
        .any(|v| v <= 0.)
        {
            return Err(invalid("distortion is not safely invertible"));
        }
        let edge = self.forward(1.);
        EmbeddedLensProfile::from_inverse_distortion(513, |output| {
            // Extend the last scale past the sensor boundary for interpolation. The shared
            // valid rectangle uses that scale to keep output samples inside the supported disc.
            if output >= edge {
                return edge.recip();
            }
            let mut low = 0.;
            let mut high = 1.;
            for _ in 0..36 {
                let mid = (low + high) * 0.5;
                if self.forward(mid) < output {
                    low = mid;
                } else {
                    high = mid;
                }
            }
            (low + high) * 0.5 / output
        })
    }
}

fn invalid(reason: &str) -> Error {
    Error::Raw(format!("Panasonic RW2 lens correction: {reason}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        lens::LensCorrections,
        raw::{RawFile, panasonic_fixture},
    };
    use glam::DVec2;

    fn payload() -> LensData {
        let mut words = [0; 16];
        words[7] = 0xa001;
        words[5] = (-16384_i16) as u16;
        words[8] = (-2048_i16) as u16;
        words[4] = 1024;
        words[11] = 512;
        LensData {
            status: 1,
            words,
            sensor_bounds: [0; 4],
        }
    }

    #[test]
    fn distortion_payload_inverts_the_documented_signed_polynomial() {
        let profile = payload().distortion().unwrap().unwrap().profile().unwrap();
        for source in [0.125_f64, 0.5, 0.875, 0.975] {
            let destination =
                source - source.powi(3) / 8. + source.powi(5) / 16. + source.powi(7) / 32.;
            let scales = profile
                .inverse_scales(destination, LensCorrections::ALL)
                .unwrap();
            for scale in scales.to_array() {
                assert!(
                    (destination * scale - source).abs() < 5e-7,
                    "{source}: {scale}"
                );
            }
        }
        assert!(!profile.has_vignetting());
        assert!(!profile.has_ca());
    }

    #[test]
    fn absent_and_disabled_payloads_do_not_enable_distortion() {
        let mut lens = payload();
        lens.words[7] &= !0xf;
        assert!(lens.distortion().unwrap().is_none());
        lens.status = 0;
        lens.words[7] = 1;
        assert!(lens.distortion().unwrap().is_none());
        lens.status = 1;
        lens.words = [0; 16];
        lens.words[7] = 1;
        assert!(lens.distortion().unwrap().is_none());
    }

    #[test]
    fn malformed_or_folding_distortion_is_rejected() {
        let mut lens = payload();
        lens.status = 2;
        assert!(lens.distortion().is_err());
        lens.status = 1;
        lens.words[5] = i16::MIN as u16;
        assert!(lens.distortion().is_err());
        assert!(Distortion([-0.5, 0., 0.]).profile().is_err());
    }

    #[test]
    fn rw2_header_distinguishes_absent_disabled_and_bad_tag() {
        let mut off = panasonic_fixture::distortion();
        off[14] = 0;
        for payload in [None, Some(off)] {
            let file = panasonic_fixture::source(payload, true);
            let raw = RawFile::open(file.path()).unwrap();
            assert!(raw.metadata.lens_geometry().unwrap().is_none());
            assert_eq!(
                raw.metadata.image_area().unwrap(),
                Rect::from([20, 4, 88, 88])
            );
        }
        let bad = panasonic_fixture::source(Some(vec![0; 31]), true);
        assert!(
            RawFile::open(bad.path())
                .err()
                .unwrap()
                .to_string()
                .contains("malformed DistortionInfo")
        );
        assert!(RawFile::open_uncorrected(bad.path()).is_ok());
    }

    #[test]
    fn rw2_aspect_crop_keeps_full_sensor_calibration_and_samples_the_warped_pixels() {
        let square = panasonic_fixture::source(Some(panasonic_fixture::distortion()), true);
        let full = panasonic_fixture::source(Some(panasonic_fixture::distortion()), false);
        let mut corrected = RawFile::open(square.path()).unwrap();
        let full = RawFile::open(full.path())
            .unwrap()
            .metadata
            .lens_geometry()
            .unwrap()
            .unwrap();
        let geometry = corrected.metadata.lens_geometry().unwrap().unwrap();
        assert_eq!(full.original_area(), Rect::from([8, 4, 112, 88]));
        assert_eq!(geometry.original_area(), full.original_area());
        // The minimum forward scale is 103/108 at Rd² = 2/3; both
        // half-extents need a three-pixel inset to guarantee valid support.
        assert_eq!(geometry.size(), [82, 82]);
        assert_eq!(full.size(), [106, 82]);

        let mut uncorrected = RawFile::open_uncorrected(square.path()).unwrap();
        let input = uncorrected.develop_camera([1.; 4]).unwrap();
        let output = corrected.develop_camera([1.; 4]).unwrap();
        assert_eq!([output.width, output.height], geometry.size());
        assert!(output.is_continuous());
        // Independently solve Ru = Rd - Rd³/8 + Rd⁵/16 + Rd⁷/32 at these sites.
        // The output's (0,0) is (23,7) in the undistorted sensor frame.
        for (x, y, sx, sy) in [
            (5, 10, 26.45473766, 15.67238024),
            (2, 75, 23.17156752, 83.63846547),
            (77, 20, 101.37258946, 26.22909359),
        ] {
            let target = DVec2::new(sx, sy);
            assert_eq!(
                geometry.sample_positions(x, y).unwrap(),
                full.sample_positions(x + 12, y).unwrap()
            );
            for actual in geometry.sample_positions(x, y).unwrap() {
                assert!((actual - target).length() < 0.0001, "{actual} != {target}");
            }
            let (ix, iy) = (sx.floor() as u32, sy.floor() as u32);
            let (fx, fy) = (sx.fract(), sy.fract());
            let actual = output.pixel_samples(x, y).unwrap();
            let mut expected = [0.; 3];
            for (dx, wx) in [(0, 1. - fx), (1, fx)] {
                for (dy, wy) in [(0, 1. - fy), (1, fy)] {
                    let sample = input.pixel_samples(ix + dx, iy + dy).unwrap();
                    for channel in 0..3 {
                        expected[channel] += f64::from(sample[channel]) * wx * wy;
                    }
                }
            }
            assert_ne!(input.pixel_samples(x + 23, y + 7).unwrap(), actual);
            for channel in 0..3 {
                assert!((f64::from(actual[channel]) - expected[channel]).abs() < 0.05);
            }
        }
    }

    #[test]
    #[ignore = "requires enabled Panasonic RW2 files in ANTITONE_RW2_DISTORTION_SAMPLES (path-separated)"]
    fn real_rw2_warp_develops_in_the_advertised_source_geometry() {
        let samples =
            std::env::var_os("ANTITONE_RW2_DISTORTION_SAMPLES").expect("set sample paths");
        let paths: Vec<_> = std::env::split_paths(&samples).collect();
        assert!(!paths.is_empty());
        for path in paths {
            let mut raw =
                RawFile::open(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            let crop = raw.metadata.native_image_area().unwrap();
            let geometry = raw
                .metadata
                .lens_geometry()
                .unwrap()
                .expect("sample must have enabled distortion");
            let [width, height] = geometry.size();
            assert!(width <= crop.width && height <= crop.height);
            assert_eq!(
                raw.metadata.image_area().unwrap(),
                Rect::from([0, 0, width, height])
            );
            let center = geometry.sample_positions(width / 2, height / 2).unwrap()[0];
            let area = geometry.original_area();
            assert!(area.contains_region(crop));
            eprintln!(
                "{}: sensor {area:?}, framing {crop:?}, corrected {width}x{height}, centre {center}",
                path.display()
            );
            let image = raw.develop_camera([1.; 4]).unwrap();
            assert_eq!([image.width, image.height], geometry.size());
            assert!(image.is_continuous());
            let sample = image.pixel_samples(width / 2, height / 2).unwrap();
            assert!(
                sample[..3].iter().any(|v| *v > 0.) && sample[..3].iter().all(|v| v.is_finite())
            );
        }
    }
}
