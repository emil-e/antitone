//! Shared preparation boundary: provider models become self-contained numeric LUTs.
use super::{LookAsset, LookOrigin};
use crate::Error;
use glam::DVec3;
use std::sync::atomic::AtomicBool;

/// Sample a provider's scene-linear AP1 input / ACEScct-encoded AP1 output function.
/// Source assets and selection remain provider-owned; only this LUT is saved and executed.
pub fn bake(
    name: String,
    origin: LookOrigin,
    size: u32,
    cancel: &AtomicBool,
    evaluate_encoded_ap1: &(impl Fn(DVec3) -> Result<DVec3, Error> + Sync),
) -> Result<LookAsset, Error> {
    let lut = super::sampling::bake(size, cancel, evaluate_encoded_ap1)?;
    LookAsset::new(name, origin, lut)
}

pub const DEFAULT_GRID: u32 = 65;
pub const REFERENCE: [f64; 9] = [
    100., 0.694, 0.306, 0.2175, 0.7435, 0.1405, 0.053, 0.3127, 0.3290,
];
pub const DOMAIN_MIN: f64 = -0.5;
pub fn domain_max() -> f64 {
    DOMAIN_MIN + (crate::photographic::encode(0.18) - DOMAIN_MIN) * (64. / 38.)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        looks::{LookId, material},
        photographic,
    };
    use std::sync::Arc;

    #[test]
    fn an_unregistered_provider_prepares_and_reopens_without_a_runtime_definition() {
        let origin = LookOrigin::Renderer {
            renderer: "authored_test_provider".into(),
            revision: "a".repeat(64),
            generator: "test-affine-v1".into(),
            attribution: "Authored test function".into(),
        };
        assert_eq!(material::Selection::from_origin(&origin), None);
        let response = |rgb: DVec3| {
            let q = rgb.map(photographic::encode);
            Ok(DVec3::new(
                0.5 * q.y + 0.1,
                0.25 * q.z - 0.02,
                0.75 * q.x + 0.03,
            ))
        };
        let asset = bake(
            "Independent provider".into(),
            origin,
            5,
            &AtomicBool::new(false),
            &response,
        )
        .unwrap();
        let id: LookId = asset.id();
        let mut project = crate::project::Project::default();
        project.output.film_and_paper = Some(Arc::new(asset));
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("provider.antitone");
        project.save(&path, crate::project::Save::New).unwrap();
        let reopened = crate::project::Project::load(&path)
            .unwrap()
            .output
            .film_and_paper
            .unwrap();
        assert_eq!(reopened.id(), id);
        for q in [
            DVec3::new(-0.3, 0.5, 0.8),
            DVec3::new(0.7, -0.1, 0.2),
            DVec3::splat(0.3),
        ] {
            let expected = DVec3::new(0.5 * q.y + 0.1, 0.25 * q.z - 0.02, 0.75 * q.x + 0.03);
            assert!(
                (reopened.lut().sample_encoded(q).unwrap() - expected)
                    .abs()
                    .max_element()
                    < 1e-7
            );
        }
        let imported = LookAsset::new(
            "Same numbers".into(),
            LookOrigin::Imported,
            reopened.lut().clone(),
        )
        .unwrap();
        assert_eq!(imported.id(), id);
        assert_eq!(imported.lut().nodes(), reopened.lut().nodes());
    }
}
