use crate::{Error, lens::EmbeddedLensGeometry};

pub(super) struct CorrectedImage {
    pub pixels: Vec<[f32; 3]>,
    pub size: [u32; 2],
}

impl CorrectedImage {
    pub(super) fn retained_bytes(&self) -> usize {
        self.pixels.capacity() * size_of::<[f32; 3]>()
    }
}

pub(super) fn reconstruct(
    input: &super::Developed<'_>,
    geometry: &EmbeddedLensGeometry,
    threads: usize,
) -> Result<CorrectedImage, Error> {
    let size = geometry.size();
    // Workers borrow plain immutable slices, not the thread-local native owner.
    let codes = input.pixels;
    let continuous = input.continuous;
    let width = input.width;
    let sample_scale = input.sample_scale;
    let count = (size[0] as usize)
        .checked_mul(size[1] as usize)
        .filter(|&n| n <= isize::MAX as usize / size_of::<[f32; 3]>())
        .ok_or_else(|| Error::Raw("corrected image exceeds addressable storage".into()))?;
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(count)
        .map_err(|_| Error::Raw("cannot allocate the corrected image".into()))?;
    pixels.resize(count, [0.; 3]);
    let rows = (size[1] as usize).div_ceil(threads.clamp(1, 8));
    std::thread::scope(|scope| -> Result<(), Error> {
        let mut jobs = Vec::new();
        for (chunk, output) in pixels.chunks_mut(rows * size[0] as usize).enumerate() {
            jobs.push(scope.spawn(move || -> Result<(), Error> {
                let start_y = chunk * rows;
                for (i, pixel) in output.iter_mut().enumerate() {
                    let x = (i % size[0] as usize) as u32;
                    let y = (start_y + i / size[0] as usize) as u32;
                    for (c, point) in geometry.sample_positions(x, y)?.into_iter().enumerate() {
                        let left = point.x.floor() as u32;
                        let top = point.y.floor() as u32;
                        let right = point.x.ceil() as u32;
                        let bottom = point.y.ceil() as u32;
                        let sample = |x: u32, y: u32| {
                            let index = y as usize * width as usize + x as usize;
                            let value = match continuous {
                                Some(pixels) => pixels[index][c],
                                None => f32::from(codes[index * 4 + c]),
                            };
                            f64::from(value * sample_scale) * geometry.source_gain(x, y)
                        };
                        let u = point.x - left as f64;
                        let v = point.y - top as f64;
                        let a = sample(left, top) * (1. - u) + sample(right, top) * u;
                        let b = sample(left, bottom) * (1. - u) + sample(right, bottom) * u;
                        pixel[c] = (a * (1. - v) + b * v) as f32;
                    }
                }
                Ok(())
            }));
        }
        for job in jobs {
            job.join()
                .map_err(|_| Error::Raw("lens correction worker panicked".into()))??;
        }
        Ok(())
    })?;
    Ok(CorrectedImage { pixels, size })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{lens::EmbeddedLensProfile, raw::Rect};
    use glam::DVec2;

    fn reconstruct(
        input: &[u16],
        input_size: [u32; 2],
        geometry: &EmbeddedLensGeometry,
        threads: usize,
    ) -> Result<CorrectedImage, Error> {
        let image = super::super::Developed {
            pixels: input,
            continuous: None,
            sample_scale: 1.,
            width: input_size[0],
            height: input_size[1],
            maximum: 65535,
            data_maximum: 65535,
            warnings: 0,
            pre_mul: [1.; 4],
            _owner: std::marker::PhantomData,
        };
        super::reconstruct(&image, geometry, threads)
    }

    #[test]
    fn reconstruction_preserves_constant_channels_across_geometry_and_worker_partitions() {
        let profile = EmbeddedLensProfile::from_knots(
            &[1.0625, 1.0625],
            &[DVec2::new(1. + 16384. / 2097152., 1. - 32768. / 2097152.); 2],
            &[],
        )
        .unwrap();
        let geometry = EmbeddedLensGeometry::new(profile, Rect::from([7, 11, 40, 28])).unwrap();
        let input: Vec<_> = (0..64 * 64).flat_map(|_| [100, 200, 300, 9999]).collect();
        for threads in [1, 2, 8] {
            let image = reconstruct(&input, [64, 64], &geometry, threads).unwrap();
            assert_eq!(image.size, geometry.size());
            assert_eq!(
                image.pixels.len(),
                image.size[0] as usize * image.size[1] as usize
            );
            assert!(image.pixels.iter().all(|&p| p == [100., 200., 300.]));
        }
    }

    #[test]
    fn shading_preserves_headroom_and_scales_native_white_without_clamping() {
        let geometry = |attenuation: f64| {
            EmbeddedLensGeometry::new(
                EmbeddedLensProfile::from_knots(&[], &[], &[attenuation; 2]).unwrap(),
                Rect::from([0, 0, 64, 64]),
            )
            .unwrap()
        };
        let image = reconstruct(
            &vec![50000; 64 * 64 * 4],
            [64, 64],
            &geometry((-0.5f64).exp2()),
            8,
        )
        .unwrap();
        let expected = (50000. * 2f64.sqrt()) as f32;
        assert!(
            image
                .pixels
                .iter()
                .flatten()
                .all(|&v| (v - expected).abs() < 0.01)
        );

        let image = reconstruct(
            &vec![u16::MAX; 64 * 64 * 4],
            [64, 64],
            &geometry((0.25f64).exp2()),
            8,
        )
        .unwrap();
        let expected = (65535. / 2f64.powf(0.25)) as f32;
        assert!(
            image
                .pixels
                .iter()
                .flatten()
                .all(|&v| (v - expected).abs() < 0.01)
        );
    }

    #[test]
    fn continuous_repaired_light_retains_fractions_and_overrange_through_optics() {
        let pixels = vec![[70000.25, 120000.5, 65535.75]; 64 * 64];
        let image = super::super::Developed {
            pixels: &[],
            continuous: Some(&pixels),
            sample_scale: 1.,
            width: 64,
            height: 64,
            maximum: 65535,
            data_maximum: 65535,
            warnings: 0,
            pre_mul: [1.; 4],
            _owner: std::marker::PhantomData,
        };
        let geometry = EmbeddedLensGeometry::new(
            EmbeddedLensProfile::from_knots(&[1.02; 2], &[DVec2::new(1.01, 0.99); 2], &[0.5; 2])
                .unwrap(),
            Rect::from([0, 0, 64, 64]),
        )
        .unwrap();
        for threads in [1, 8] {
            let output = super::reconstruct(&image, &geometry, threads).unwrap();
            assert!(
                output
                    .pixels
                    .iter()
                    .all(|p| *p == [140000.5, 240001., 131071.5]),
                "optics must consume repaired floats, not stale RGB16 codes or a rounded reconstruction"
            );
        }
    }

    #[test]
    fn identity_reconstruction_keeps_isolated_native_white_at_its_original_site() {
        let geometry = EmbeddedLensGeometry::new(
            EmbeddedLensProfile::from_knots(&[1., 1.], &[], &[]).unwrap(),
            Rect::from([0, 0, 64, 64]),
        )
        .unwrap();
        let mut input = vec![10000; 64 * 64 * 4];
        input[(5 * 64 + 5) * 4] = u16::MAX;
        let image = reconstruct(&input, [64, 64], &geometry, 2).unwrap();
        for y in 0..64 {
            for x in 0..64 {
                let expected = if [x, y] == [5, 5] {
                    [65535., 10000., 10000.]
                } else {
                    [10000.; 3]
                };
                assert_eq!(image.pixels[y * 64 + x], expected);
            }
        }
    }
}
