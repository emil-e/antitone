use super::{CaptureFlags, Rect};
use crate::Error;

/// Two capture-limit flags per pixel; word-aligned rows permit independent reconstruction jobs.
pub(super) struct EligibilityMask {
    size: [u32; 2],
    row_words: usize,
    words: Vec<u64>,
}

impl EligibilityMask {
    pub(super) fn new(size: [u32; 2]) -> Result<Self, Error> {
        let row_words = size[0].div_ceil(32) as usize;
        let count = row_words
            .checked_mul(size[1] as usize)
            .filter(|&n| {
                size[0] != 0 && size[1] != 0 && n <= isize::MAX as usize / size_of::<u64>()
            })
            .ok_or_else(|| Error::Raw("invalid capture mask dimensions".into()))?;
        let mut words = Vec::new();
        words
            .try_reserve_exact(count)
            .map_err(|_| Error::Raw("cannot allocate capture mask".into()))?;
        words.resize(count, 0);
        Ok(Self {
            size,
            row_words,
            words,
        })
    }

    pub(super) fn size(&self) -> [u32; 2] {
        self.size
    }
    pub(super) fn row_words(&self) -> usize {
        self.row_words
    }
    pub(super) fn words_mut(&mut self) -> &mut [u64] {
        &mut self.words
    }
    pub(super) fn retained_bytes(&self) -> usize {
        self.words.capacity() * size_of::<u64>()
    }

    pub(super) fn at(&self, x: u32, y: u32) -> u8 {
        ((self.words[y as usize * self.row_words + x as usize / 32] >> (2 * (x % 32))) & 3) as u8
    }

    /// AHD reads two CFA steps for green, one for neighbouring RGB and two for homogeneity.
    /// Dilate once by that conservative five-site dependency, clipped to the native image.
    pub(super) fn ahd_support(&self) -> Result<Self, Error> {
        let mut horizontal = Self::new(self.size)?;
        for (source, target) in self
            .words
            .chunks(self.row_words)
            .zip(horizontal.words.chunks_mut(self.row_words))
        {
            for (i, target) in target.iter_mut().enumerate() {
                *target = source[i];
                for distance in 1..=5 {
                    let shift = 2 * distance;
                    *target |= source[i] << shift | source[i] >> shift;
                    if i > 0 {
                        *target |= source[i - 1] >> (64 - shift);
                    }
                    if i + 1 < source.len() {
                        *target |= source[i + 1] << (64 - shift);
                    }
                }
            }
        }
        let mut result = Self::new(self.size)?;
        for (y, target) in result.words.chunks_mut(self.row_words).enumerate() {
            for source_y in y.saturating_sub(5)..(y + 6).min(self.size[1] as usize) {
                let start = source_y * self.row_words;
                for (out, &value) in target
                    .iter_mut()
                    .zip(&horizontal.words[start..start + self.row_words])
                {
                    *out |= value;
                }
            }
        }
        Ok(result)
    }

    pub(super) fn flags(&self, area: Rect) -> Result<CaptureFlags, Error> {
        if area.width == 0
            || area.height == 0
            || !Rect::from([0, 0, self.size[0], self.size[1]]).contains_region(area)
        {
            return Err(Error::Raw(
                "capture mask region outside developed source".into(),
            ));
        }
        let start = area.x as usize;
        let end = start + area.width as usize;
        let mut combined = 0u64;
        for y in area.y..area.y + area.height {
            for word in start / 32..end.div_ceil(32) {
                let low = 2 * start.saturating_sub(word * 32);
                let high = 2 * (end - word * 32).min(32);
                combined |= self.words[y as usize * self.row_words + word]
                    & (u64::MAX << low)
                    & (u64::MAX >> (64 - high));
            }
            if combined & 0x5555_5555_5555_5555 != 0 && combined & 0xaaaa_aaaa_aaaa_aaaa != 0 {
                break;
            }
        }
        Ok(CaptureFlags {
            at_white: combined & 0x5555_5555_5555_5555 != 0,
            below_black: combined & 0xaaaa_aaaa_aaaa_aaaa != 0,
            ..CaptureFlags::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(mask: &mut EligibilityMask, x: u32, y: u32, flags: u8) {
        mask.words[y as usize * mask.row_words + x as usize / 32] |=
            u64::from(flags) << (2 * (x % 32));
    }

    #[test]
    fn packed_dilation_matches_clipped_neighbourhoods_across_word_and_image_edges() {
        for width in [1, 7, 31, 32, 33, 65, 173] {
            let mut raw = EligibilityMask::new([width, 17]).unwrap();
            for [x, y, flag] in [[0, 0, 1], [width - 1, 16, 2], [width / 2, 8, 3]] {
                set(&mut raw, x, y, flag as u8);
            }
            let support = raw.ahd_support().unwrap();
            for y in 0u32..17 {
                for x in 0..width {
                    let mut expected = 0;
                    for yy in y.saturating_sub(5)..(y + 6).min(17) {
                        for xx in x.saturating_sub(5)..(x + 6).min(width) {
                            expected |= raw.at(xx, yy);
                        }
                    }
                    assert_eq!(support.at(x, y), expected, "width={width}, point=[{x},{y}]");
                }
            }
        }
    }

    #[test]
    fn queries_or_flags_without_padding_leaks_or_changing_the_mask() {
        let mut mask = EligibilityMask::new([65, 3]).unwrap();
        set(&mut mask, 31, 0, 1);
        set(&mut mask, 32, 1, 2);
        set(&mut mask, 64, 2, 3);
        assert_eq!(
            mask.flags(Rect::from([0, 0, 31, 3])).unwrap(),
            CaptureFlags::default()
        );
        assert_eq!(
            mask.flags(Rect::from([31, 0, 1, 1])).unwrap(),
            CaptureFlags {
                at_white: true,
                below_black: false,
                ..CaptureFlags::default()
            }
        );
        assert_eq!(
            mask.flags(Rect::from([31, 0, 2, 2])).unwrap(),
            CaptureFlags {
                at_white: true,
                below_black: true,
                ..CaptureFlags::default()
            }
        );
        assert_eq!(
            mask.flags(Rect::from([64, 0, 1, 2])).unwrap(),
            CaptureFlags::default()
        );
        assert!(mask.flags(Rect::from([65, 0, 1, 1])).is_err());
        assert!(mask.flags(Rect::from([0, 0, 0, 1])).is_err());
    }
}
