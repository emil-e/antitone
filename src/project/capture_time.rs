use serde::{Deserialize, Serialize};

/// The camera's recorded local date and time at second precision, without a time zone.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "u64", into = "u64")]
pub struct CaptureTime(u64);

impl CaptureTime {
    #[cfg(feature = "raw")]
    pub(crate) fn from_exif(bytes: &[u8; 19]) -> Option<Self> {
        let mut packed = 0;
        for (index, &byte) in bytes.iter().enumerate() {
            match index {
                4 | 7 | 13 | 16 if byte == b':' => (),
                10 if byte == b' ' => (),
                4 | 7 | 10 | 13 | 16 => return None,
                _ if byte.is_ascii_digit() => packed = packed * 10 + u64::from(byte - b'0'),
                _ => return None,
            }
        }
        Self::try_from(packed).ok()
    }
}

impl TryFrom<u64> for CaptureTime {
    type Error = &'static str;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        // YYYYMMDDhhmmss sorts chronologically without interpreting the camera's local time.
        let part = |divisor| (value / divisor) % 100;
        let (year, month, day) = (value / 10_000_000_000, part(100_000_000), part(1_000_000));
        let last_day = match month {
            2 if year.is_multiple_of(4)
                && (!year.is_multiple_of(100) || year.is_multiple_of(400)) =>
            {
                29
            }
            2 => 28,
            4 | 6 | 9 | 11 => 30,
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            _ => 0,
        };
        if !(1..=9999).contains(&year)
            || !(1..=last_day).contains(&day)
            || part(10_000) >= 24
            || part(100) >= 60
            || part(1) >= 60
        {
            return Err("invalid capture time");
        }
        Ok(Self(value))
    }
}

impl From<CaptureTime> for u64 {
    fn from(time: CaptureTime) -> Self {
        time.0
    }
}
