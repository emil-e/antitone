use super::eligibility::EligibilityMask;
use crate::{Error, lens::SonyLensGeometry};

pub(super) struct CorrectedImage {
    pub pixels: Vec<[f32; 3]>,
    pub size: [u32; 2],
    pub eligibility: EligibilityMask,
}

impl CorrectedImage {
    pub(super) fn retained_bytes(&self) -> usize {
        self.pixels.capacity() * size_of::<[f32; 3]>() + self.eligibility.retained_bytes()
    }
}

pub(super) fn reconstruct(
    input: &[u16],
    input_size: [u32; 2],
    geometry: &SonyLensGeometry,
    support: &EligibilityMask,
    threads: usize,
) -> Result<CorrectedImage, Error> {
    if support.size() != input_size {
        return Err(Error::Raw(
            "capture support differs from native image".into(),
        ));
    }
    let size = geometry.size();
    let count = (size[0] as usize)
        .checked_mul(size[1] as usize)
        .filter(|&n| n <= isize::MAX as usize / size_of::<[f32; 3]>())
        .ok_or_else(|| Error::Raw("corrected Sony image exceeds addressable storage".into()))?;
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(count)
        .map_err(|_| Error::Raw("cannot allocate corrected Sony image".into()))?;
    pixels.resize(count, [0.; 3]);
    let mut eligibility = EligibilityMask::new(size)?;
    let row_words = eligibility.row_words();
    let rows = (size[1] as usize).div_ceil(threads.clamp(1, 8));
    std::thread::scope(|scope| -> Result<(), Error> {
        let mut jobs = Vec::new();
        for (chunk, (output, flags)) in pixels
            .chunks_mut(rows * size[0] as usize)
            .zip(eligibility.words_mut().chunks_mut(rows * row_words))
            .enumerate()
        {
            jobs.push(scope.spawn(move || -> Result<(), Error> {
                let start_y = chunk * rows;
                for (i, pixel) in output.iter_mut().enumerate() {
                    let x = (i % size[0] as usize) as u32;
                    let local_y = i / size[0] as usize;
                    let y = (start_y + local_y) as u32;
                    let mut capture = 0;
                    for (c, point) in geometry.sample_positions(x, y)?.into_iter().enumerate() {
                        let left = point.x.floor() as u32;
                        let top = point.y.floor() as u32;
                        let right = point.x.ceil() as u32;
                        let bottom = point.y.ceil() as u32;
                        let mut sample = |x: u32, y: u32| {
                            let value =
                                input[(y as usize * input_size[0] as usize + x as usize) * 4 + c];
                            // Native WB/scaling can clip below the original CFA white threshold.
                            // Record that loss before shading brightens or interpolation hides it.
                            capture |= u8::from(value == u16::MAX);
                            f64::from(value) * geometry.source_gain(x, y)
                        };
                        let u = point.x - left as f64;
                        let v = point.y - top as f64;
                        let a = sample(left, top) * (1. - u) + sample(right, top) * u;
                        let b = sample(left, bottom) * (1. - u) + sample(right, bottom) * u;
                        pixel[c] = (a * (1. - v) + b * v) as f32;
                        // ceil equals floor at an integer coordinate: zero-weight neighbours add no flags.
                        capture |= support.at(left, top)
                            | support.at(right, top)
                            | support.at(left, bottom)
                            | support.at(right, bottom);
                    }
                    flags[local_y * row_words + x as usize / 32] |=
                        u64::from(capture) << (2 * (x % 32));
                }
                Ok(())
            }));
        }
        for job in jobs {
            job.join()
                .map_err(|_| Error::Raw("Sony correction worker panicked".into()))??;
        }
        Ok(())
    })?;
    Ok(CorrectedImage {
        pixels,
        size,
        eligibility,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        lens::SonyLensProfile,
        raw::{CaptureFlags, Rect},
    };

    fn source_mask(size: [u32; 2], sites: &[[u32; 3]]) -> EligibilityMask {
        let mut mask = EligibilityMask::new(size).unwrap();
        let stride = mask.row_words();
        for &[x, y, flags] in sites {
            mask.words_mut()[y as usize * stride + x as usize / 32] |=
                u64::from(flags) << (2 * (x % 32));
        }
        mask
    }

    #[test]
    fn precomputed_flags_match_the_original_channel_support_union_and_ahd_halo() {
        let size = [64, 64];
        let raw = source_mask(size, &[[5, 6, 1], [27, 35, 2], [48, 39, 3]]);
        let support = raw.ahd_support().unwrap();
        let profile = SonyLensProfile::from_parameters(
            Some(&[2, 1024, 1024]),
            Some(&[4, 16384, 16384, -32768, -32768]),
        )
        .unwrap();
        let geometry = SonyLensGeometry::new(profile, Rect::from([7, 11, 40, 28])).unwrap();
        let image = reconstruct(&vec![100; 64 * 64 * 4], size, &geometry, &support, 8).unwrap();
        let mut region_flags = 0;
        for y in 0..image.size[1] {
            for x in 0..image.size[0] {
                let mut expected = 0;
                for p in geometry.sample_positions(x, y).unwrap() {
                    for yy in
                        (p.y.floor() as u32).saturating_sub(5)..(p.y.ceil() as u32 + 6).min(size[1])
                    {
                        for xx in (p.x.floor() as u32).saturating_sub(5)
                            ..(p.x.ceil() as u32 + 6).min(size[0])
                        {
                            expected |= raw.at(xx, yy);
                        }
                    }
                }
                assert_eq!(image.eligibility.at(x, y), expected);
                region_flags |= expected;
            }
        }
        assert_eq!(
            image
                .eligibility
                .flags(Rect::from([0, 0, image.size[0], image.size[1]]))
                .unwrap(),
            CaptureFlags {
                at_white: region_flags & 1 != 0,
                below_black: region_flags & 2 != 0
            }
        );
    }

    #[test]
    fn shading_headroom_is_not_clipping_but_native_clipping_survives_attenuation() {
        let size = [64, 64];
        let support = source_mask(size, &[]);
        let geometry = |coefficient| {
            SonyLensGeometry::new(
                SonyLensProfile::from_components(None, None, Some(&[2, coefficient, coefficient]))
                    .unwrap(),
                Rect::from([0, 0, 64, 64]),
            )
            .unwrap()
        };
        let image = reconstruct(
            &vec![50000; 64 * 64 * 4],
            size,
            &geometry(8192),
            &support,
            8,
        )
        .unwrap();
        assert!(image.pixels.iter().all(|p| p[0] > 65535.));
        assert_eq!(image.eligibility.at(5, 5), 0);
        let mut input = vec![10000; 64 * 64 * 4];
        input[(5 * 64 + 5) * 4] = u16::MAX;
        let image = reconstruct(&input, size, &geometry(-8192), &support, 8).unwrap();
        assert!(image.pixels[5 * 64 + 5][0] < 65535.);
        assert_eq!(image.eligibility.at(5, 5), 1);
        assert_eq!(image.eligibility.at(4, 5), 0);
    }

    #[test]
    fn zero_weight_neighbours_do_not_invalidate_an_identity_sample() {
        let size = [64, 64];
        let support = source_mask(size, &[[21, 12, 1]]).ahd_support().unwrap();
        let geometry = SonyLensGeometry::new(
            SonyLensProfile::from_parameters(Some(&[2, 0, 0]), None).unwrap(),
            Rect::from([0, 0, 40, 28]),
        )
        .unwrap();
        let image = reconstruct(&vec![100; 64 * 64 * 4], size, &geometry, &support, 2).unwrap();
        assert_eq!(image.eligibility.at(15, 12), 0);
        assert_eq!(image.eligibility.at(16, 12), 1);
    }
}
