use crate::{Error, lens::SonyLensProfile};

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

    /// Validates available coefficients without changing RAW-development behaviour.
    pub fn profile(&self) -> Result<Option<SonyLensProfile>, Error> {
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
        SonyLensProfile::from_components(distortion, ca, vignetting).map(Some)
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
