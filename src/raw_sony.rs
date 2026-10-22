use crate::{Error, lens::EmbeddedLensProfile};
use glam::DVec2;

/// Sony's fixed-point lens tables: scales are signed offsets from unity.
const DISTORTION_UNIT: f64 = 1. / 16384.;
const CA_UNIT: f64 = 1. / 2097152.;

const DISTORTION: u32 = 1;
const CA: u32 = 2;
const DISTORTION_STATUS: u32 = 4;
const CA_STATUS: u32 = 8;
const VIGNETTING: u32 = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SonyLensDataStatus {
    Absent,
    Unavailable,
    Available { camera_setting: Option<u16> },
    Malformed,
}

/// Bounded metadata from the selected Sony RAW IFD; retained without applying corrections.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SonyLensData {
    present: u32,
    invalid: u32,
    distortion_length: u32,
    ca_length: u32,
    vignetting_length: u32,
    distortion_setting: u16,
    ca_setting: u16,
    distortion: [i16; 17],
    ca: [i16; 33],
    vignetting: [i16; 17],
}

impl SonyLensData {
    pub fn distortion_status(&self) -> SonyLensDataStatus {
        self.status(DISTORTION, DISTORTION_STATUS, self.distortion_setting)
    }

    pub fn ca_status(&self) -> SonyLensDataStatus {
        self.status(CA, CA_STATUS, self.ca_setting)
    }

    /// No setting tag is inferred for shading; 0x7032 contains the coefficients themselves.
    pub fn vignetting_status(&self) -> SonyLensDataStatus {
        self.status(VIGNETTING, 0, 0)
    }

    /// Decodes the available Sony tables into a vendor-neutral embedded profile.
    pub fn profile(&self) -> Result<Option<EmbeddedLensProfile>, Error> {
        let distortion = self.component(
            self.distortion_status(),
            &self.distortion,
            self.distortion_length,
        )?;
        let ca = self.component(self.ca_status(), &self.ca, self.ca_length)?;
        let vignetting = self.component(
            self.vignetting_status(),
            &self.vignetting,
            self.vignetting_length,
        )?;
        if distortion.is_none() && ca.is_none() && vignetting.is_none() {
            return Ok(None);
        }
        decode_profile(distortion, ca, vignetting).map(Some)
    }

    fn component<'a>(
        &self,
        status: SonyLensDataStatus,
        data: &'a [i16],
        length: u32,
    ) -> Result<Option<&'a [i16]>, Error> {
        match status {
            SonyLensDataStatus::Absent | SonyLensDataStatus::Unavailable => Ok(None),
            SonyLensDataStatus::Malformed => Err(Error::Raw("malformed Sony lens metadata".into())),
            SonyLensDataStatus::Available { .. } => data
                .get(..length as usize)
                .map(Some)
                .ok_or_else(|| Error::Raw("invalid Sony lens payload length".into())),
        }
    }

    fn status(&self, component: u32, setting_bit: u32, setting: u16) -> SonyLensDataStatus {
        if self.invalid & setting_bit != 0 {
            return SonyLensDataStatus::Malformed;
        }
        if self.present & setting_bit != 0 && setting == 255 {
            return SonyLensDataStatus::Unavailable;
        }
        if self.invalid & component != 0 {
            return SonyLensDataStatus::Malformed;
        }
        if self.present & component == 0 {
            return SonyLensDataStatus::Absent;
        }
        SonyLensDataStatus::Available {
            camera_setting: (self.present & setting_bit != 0).then_some(setting),
        }
    }
}

/// Decodes Sony's count-prefixed integer tables into normalized knots. Each argument carries
/// its leading signed count; `None` means the table is absent.
fn decode_profile(
    distortion: Option<&[i16]>,
    ca: Option<&[i16]>,
    vignetting: Option<&[i16]>,
) -> Result<EmbeddedLensProfile, Error> {
    let distortion: Vec<f64> = parameters(distortion, 1)?
        .iter()
        .map(|&v| 1. + f64::from(v) * DISTORTION_UNIT)
        .collect();
    let raw_ca = parameters(ca, 2)?;
    let ca: Vec<DVec2> = (0..raw_ca.len() / 2)
        .map(|i| {
            DVec2::new(
                1. + f64::from(raw_ca[i]) * CA_UNIT,
                1. + f64::from(raw_ca[i + raw_ca.len() / 2]) * CA_UNIT,
            )
        })
        .collect();
    let vignetting = parameters(vignetting, 1)?;
    // Sony writes zero coefficients for absent shading rather than omitting the table.
    let attenuation: Vec<f64> = if vignetting.iter().all(|&v| v == 0) {
        Vec::new()
    } else {
        vignetting
            .iter()
            .map(|&v| (0.5 - (f64::from(v) / 8192. - 1.).exp2()).exp2())
            .collect()
    };
    EmbeddedLensProfile::from_knots(&distortion, &ca, &attenuation)
}

/// Splits a table into its coefficient payload after validating Sony's count prefix.
fn parameters(values: Option<&[i16]>, channels: usize) -> Result<&[i16], Error> {
    let Some(values) = values else {
        return Ok(&[]);
    };
    let count = values.first().copied().unwrap_or(0);
    if count < (2 * channels) as i16
        || count > (16 * channels) as i16
        || !(count as usize).is_multiple_of(channels)
    {
        return Err(Error::Raw("invalid Sony lens coefficient count".into()));
    }
    let count = count as usize;
    if values.len() != count + 1 && values.len() != 16 * channels + 1 {
        return Err(Error::Raw("invalid Sony lens payload length".into()));
    }
    Ok(&values[1..=count])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lens::LensCorrections;
    use glam::DVec3;

    fn parameters(count: usize, value: i16) -> Vec<i16> {
        std::iter::once(count as i16)
            .chain(std::iter::repeat_n(value, count))
            .collect()
    }

    #[test]
    fn sony_tables_decode_through_known_binary_units() {
        let distortion = parameters(16, -1024);
        let ca: Vec<_> = std::iter::once(32)
            .chain(std::iter::repeat_n(16384, 16))
            .chain(std::iter::repeat_n(-32768, 16))
            .collect();
        let profile = decode_profile(Some(&distortion), Some(&ca), None).unwrap();
        assert!(profile.has_distortion() && profile.has_ca());
        let scale = 15. / 16.;
        for corrections in [
            LensCorrections {
                distortion: true,
                ca: false,
            },
            LensCorrections {
                distortion: false,
                ca: true,
            },
            LensCorrections::ALL,
        ] {
            let expected = if corrections.ca && corrections.distortion {
                DVec3::new(129. / 128., 1., 63. / 64.) * scale
            } else if corrections.ca {
                DVec3::new(129. / 128., 1., 63. / 64.)
            } else if corrections.distortion {
                DVec3::splat(scale)
            } else {
                DVec3::ONE
            };
            for radius in [0., 0.01, 0.5, 1.] {
                assert_eq!(
                    profile.inverse_scales(radius, corrections).unwrap(),
                    expected
                );
            }
        }
    }

    #[test]
    fn absent_zero_and_padded_tables_remain_distinct_from_malformed_payloads() {
        let absent = decode_profile(None, None, None).unwrap();
        assert!(!absent.has_distortion() && !absent.has_ca());
        assert_eq!(absent.knot_count(), 0);
        let zero = decode_profile(Some(&parameters(16, 0)), None, None).unwrap();
        assert!(!zero.has_distortion() && !zero.has_ca());
        assert_eq!(zero.knot_count(), 16);
        let mut padded = parameters(11, 256);
        padded.extend([i16::MAX; 5]);
        let lens = decode_profile(Some(&padded), None, None).unwrap();
        assert_eq!(lens.knot_count(), 11);
        assert_eq!(
            lens.inverse_scales(1., LensCorrections::ALL).unwrap(),
            DVec3::splat(65. / 64.)
        );
        let ca = parameters(22, 128);
        let ca_only = decode_profile(None, Some(&ca), None).unwrap();
        assert!(ca_only.has_ca() && !ca_only.has_distortion());
        assert_eq!(ca_only.knot_count(), 11);
    }

    #[test]
    fn invalid_counts_lengths_and_component_lattices_are_rejected() {
        for data in [vec![], vec![0], vec![-1], vec![1, 0], parameters(17, 0)] {
            assert!(decode_profile(Some(&data), None, None).is_err());
        }
        for data in [parameters(3, 0), parameters(33, 0), parameters(31, 0)] {
            assert!(decode_profile(None, Some(&data), None).is_err());
        }
        let mut truncated = parameters(16, 0);
        truncated.pop();
        assert!(decode_profile(Some(&truncated), None, None).is_err());
        assert!(decode_profile(Some(&parameters(16, 0)), Some(&parameters(22, 0)), None).is_err());
    }

    #[test]
    fn shading_decodes_the_full_signed_coefficient_domain() {
        for coefficient in [i16::MIN, -8192, 0, 8192, i16::MAX] {
            let profile = decode_profile(None, None, Some(&[2, coefficient, coefficient])).unwrap();
            let expected = 2f64.powf(2f64.powf(f64::from(coefficient) / 8192. - 1.) - 0.5);
            for r in [0., 0.25, 0.5, 1.] {
                assert!((profile.vignetting_gain(r).unwrap() - expected).abs() < 1e-12);
            }
            assert!(expected > 0. && expected < crate::processing::MAX_SOURCE_GAIN);
            assert!((profile.maximum_gain() - expected.max(1.)).abs() < 1e-12);
            assert_eq!(profile.has_vignetting(), coefficient != 0);
        }
        // The coefficient arrays are independently count-prefixed: shading is not a
        // geometric lattice shared with distortion.
        let profile =
            decode_profile(Some(&[2, -1024, -1024]), None, Some(&[3, 0, 8192, 16384])).unwrap();
        assert!(profile.has_distortion() && profile.has_vignetting());
        let mut padded = vec![11];
        padded.extend([0; 11]);
        padded.extend([i16::MAX; 5]);
        let zero = decode_profile(None, None, Some(&padded)).unwrap();
        assert!(!zero.has_vignetting());
        assert_eq!(zero.maximum_gain(), 1.);
        for invalid in [vec![], vec![0], vec![1, 0], vec![3, 0, 0], vec![17; 18]] {
            assert!(decode_profile(None, None, Some(&invalid)).is_err());
        }
    }
}
