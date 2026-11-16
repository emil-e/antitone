//! Paper-independent colour-negative exposure and development.
use super::{EPSILON, Negative, data, interpolate_exposure_to_density, matrix};
use crate::{
    Error,
    looks::{failure, lookup::cubic_2d},
};
use glam::DVec3;
use std::sync::{
    OnceLock,
    atomic::{AtomicBool, Ordering},
};

pub(in crate::looks) struct Formation {
    pub(super) profile: &'static data::Film,
    tc: Vec<DVec3>,
}

impl Formation {
    pub(in crate::looks) fn new(negative: Negative, cancel: &AtomicBool) -> Result<Self, Error> {
        let catalog = data::catalog();
        let profile = &catalog.negative[negative.0];
        let spectra = data::spectra()?;
        let n = catalog.spectra_size;
        let mut raw = Vec::with_capacity(n * n);
        for (i, light) in spectra.chunks_exact(catalog.wavelengths).enumerate() {
            if i % n == 0 && cancel.load(Ordering::Relaxed) {
                return Err(failure("look preparation cancelled"));
            }
            raw.push(
                light
                    .iter()
                    .zip(&profile.sensitivity)
                    .fold(DVec3::ZERO, |sum, (value, sensitivity)| {
                        sum + *sensitivity * *value
                    }),
            );
        }
        let coordinates = data::coordinates(&profile.illuminant)?;
        let tc = coordinates
            .into_iter()
            .map(|p| super::grid::bilinear(&raw, n, p))
            .collect();
        Ok(Self { profile, tc })
    }

    pub(super) fn expose(&self, rgb: DVec3) -> DVec3 {
        let xyz = matrix(self.profile.matrix) * rgb;
        let brightness = xyz.element_sum();
        let denominator = brightness.max(EPSILON);
        let x = xyz.x / denominator;
        let y = xyz.y / denominator;
        let coordinates = [
            ((1. - x) * (1. - x)).clamp(0., 1.),
            (y / (1. - x).max(EPSILON)).clamp(0., 1.),
        ];
        let raw = cubic_2d(&self.tc, data::catalog().spectra_size, coordinates) * brightness;
        raw.map(|value| (value.max(0.) + EPSILON).log10())
    }

    pub(in crate::looks) fn density(&self, rgb: DVec3) -> DVec3 {
        let film = self.profile;
        let h = self.expose(rgb);
        let initial = interpolate_exposure_to_density(
            h,
            &film.log_exposure,
            &film.density_curves,
            film.gamma,
        );
        // Upstream DIR uses donor rows and receiver columns.
        let corrected = h - matrix(film.dir_matrix).transpose() * initial;
        interpolate_exposure_to_density(
            corrected,
            &film.log_exposure,
            &film.pre_dir_curves,
            film.gamma,
        )
    }

    /// Coefficients are already in the profile's dye basis, not raw Status-A density.
    pub(in crate::looks) fn spectrum(&self) -> impl Iterator<Item = (usize, DVec3, f64)> + '_ {
        self.profile
            .support
            .iter()
            .enumerate()
            .filter(|(_, supported)| **supported)
            .map(|(index, _)| (index, self.profile.dyes[index], self.profile.base[index]))
    }
}

pub(in crate::looks) fn fingerprint(negative: Negative) -> &'static str {
    static VALUES: OnceLock<Vec<String>> = OnceLock::new();
    &VALUES.get_or_init(|| {
        data::catalog()
            .negative
            .iter()
            .map(|profile| {
                let mut hash = blake3::Hasher::new();
                for source in [
                    include_bytes!("../spektrafilm.rs").as_slice(),
                    include_bytes!("negative.rs").as_slice(),
                    include_bytes!("data.rs").as_slice(),
                    include_bytes!("grid.rs").as_slice(),
                    include_bytes!("../lookup.rs").as_slice(),
                ] {
                    hash.update(source);
                }
                hash.update(
                    &serde_json::to_vec(profile).expect("finite embedded negative profile"),
                );
                for name in [
                    "irradiance_xy_tc.f16.z",
                    "input_gamut_D55.f64.z",
                    "input_gamut_T.f64.z",
                ] {
                    let (_, bytes) = data::FILES
                        .iter()
                        .find(|(file, _)| *file == name)
                        .expect("embedded exposure data");
                    hash.update(name.as_bytes());
                    hash.update(bytes);
                }
                hash.finalize().to_hex().to_string()
            })
            .collect()
    })[negative.0]
}
