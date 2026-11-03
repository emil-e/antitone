use super::*;
use crate::looks::{
    Lut3D,
    spektrafilm::{Negative, Paper, Selection},
};
use std::sync::Arc;

#[test]
fn independently_sourced_paper_reuses_its_exact_cached_asset_without_aliasing_colour_media() {
    let directory = tempfile::tempdir().unwrap();
    let cache = Cache::at(directory.path().into());
    let paper = crate::looks::monochrome::Paper::default();
    let selection = crate::looks::material::Selection::Monochrome(paper.into());
    let asset = selection
        .bake(3, &std::sync::atomic::AtomicBool::new(false))
        .unwrap();
    assert!(cache.load_photochemical(selection, 3).unwrap().is_none());
    cache.store_photochemical(selection, &asset).unwrap();
    assert_eq!(
        cache.load_photochemical(selection, 3).unwrap(),
        Some(asset.clone())
    );
    let colour = Selection {
        negative: None,
        paper: Paper::default(),
    };
    assert!(cache.load_photochemical(colour, 3).unwrap().is_none());
    assert!(cache.store_photochemical(colour, &asset).is_err());
    assert!(cache.load_photochemical(selection, 5).unwrap().is_none());
}

#[test]
fn pair_and_grid_key_the_cache_while_projects_preserve_numeric_historic_assets() {
    let directory = tempfile::tempdir().unwrap();
    let cache = Cache::at(directory.path().into());
    let paper_only = Selection {
        negative: None,
        paper: Paper::default(),
    };
    let portra = Selection {
        negative: Negative::from_id("kodak_portra_400"),
        paper: Paper::default(),
    };
    let other_paper = Selection {
        paper: Paper::from_id("kodak_2383").unwrap(),
        ..portra
    };
    let lut = Arc::new(Lut3D::new(3, [-0.5; 3], [1.; 3], vec![[0.4; 3]; 27]).unwrap());
    let asset = LookAsset::new("Saved name".into(), portra.origin(), lut.clone()).unwrap();
    cache.store_photochemical(portra, &asset).unwrap();
    assert_eq!(
        cache.load_photochemical(portra, 3).unwrap().unwrap().lut(),
        &lut
    );
    assert!(cache.load_photochemical(paper_only, 3).unwrap().is_none());
    assert!(cache.load_photochemical(other_paper, 3).unwrap().is_none());
    assert!(cache.load_photochemical(portra, 5).unwrap().is_none());
    assert!(cache.store_photochemical(other_paper, &asset).is_err());
    let historic = LookAsset::new(
        "Historical source".into(),
        LookOrigin::Photochemical {
            negative: Some("retired_stock".into()),
            paper: "retired_paper".into(),
            revision: "a".repeat(40),
            generator: "historic".into(),
            attribution: crate::looks::spektrafilm::ATTRIBUTION.into(),
        },
        lut,
    )
    .unwrap();
    let encoded = serde_json::to_vec(&historic).unwrap();
    let text = std::str::from_utf8(&encoded).unwrap();
    assert!(text.contains("Andrea Volpato") && text.contains("CC BY-SA 4.0"));
    let reopened: LookAsset = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(reopened, historic);
    let mut missing_notice: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    missing_notice["origin"]
        .as_object_mut()
        .unwrap()
        .remove("attribution");
    assert!(serde_json::from_value::<LookAsset>(missing_notice).is_err());
    assert_eq!(Selection::from_origin(reopened.origin()), None);
    std::fs::remove_dir_all(directory.path()).unwrap();
    assert_eq!(
        serde_json::from_slice::<LookAsset>(&encoded).unwrap(),
        historic
    );
}
