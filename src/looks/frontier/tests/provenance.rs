use super::*;
use crate::looks::{Lut3D, cache::Cache, material};

#[test]
fn renderer_revision_identifies_native_or_gamma_response_by_selection() {
    let portra = spektrafilm::Negative::from_id("kodak_portra_400");
    for (variant, native, gamma) in [
        (
            Variant::A,
            include_bytes!("../../../../assets/frontier/density-a.f32.z").as_slice(),
            include_bytes!("../../../../assets/frontier/density-negative-a.f32.z").as_slice(),
        ),
        (
            Variant::B,
            include_bytes!("../../../../assets/frontier/density-b.f32.z").as_slice(),
            include_bytes!("../../../../assets/frontier/density-negative-b.f32.z").as_slice(),
        ),
    ] {
        for (negative, bytes) in [(None, native), (portra, gamma)] {
            let LookOrigin::Renderer { revision, .. } = (Selection { negative, variant }).origin()
            else {
                unreachable!()
            };
            assert_eq!(revision, blake3::hash(bytes).to_hex().to_string());
        }
        assert_ne!(blake3::hash(native), blake3::hash(gamma));
    }
}

#[test]
fn captured_renderer_metadata_keeps_optional_negative_without_reinterpreting_unknown_stocks() {
    let cancel = AtomicBool::new(false);
    let selection = Selection {
        negative: None,
        variant: Variant::A,
    };
    let asset = Model::new(selection, &cancel)
        .unwrap()
        .bake(3, &cancel)
        .unwrap();
    let value = serde_json::to_value(&asset).unwrap();
    assert!(value["origin"].get("negative").is_none());
    let reopened: LookAsset = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(reopened, asset);
    assert_eq!(Selection::from_origin(reopened.origin()), Some(selection));

    let mut retired = value;
    retired["origin"]["negative"] = "retired_colour_stock".into();
    let captured: LookAsset = serde_json::from_value(retired.clone()).unwrap();
    assert_eq!(captured.id(), asset.id());
    assert_eq!(captured.lut().nodes(), asset.lut().nodes());
    assert_eq!(Selection::from_origin(captured.origin()), None);
    assert_eq!(material::Selection::from_origin(captured.origin()), None);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("captured.antitone");
    let mut project = crate::project::Project::default();
    project.output.film_and_paper = Some(Arc::new(captured.clone()));
    project.save(&path, crate::project::Save::New).unwrap();
    assert_eq!(
        crate::project::Project::load(&path)
            .unwrap()
            .output
            .film_and_paper
            .as_deref(),
        Some(&captured)
    );
    retired["origin"]["negative"] = "bad/stock".into();
    assert!(serde_json::from_value::<LookAsset>(retired).is_err());
}

#[test]
fn generated_cache_distinguishes_stock_variant_and_none_while_numeric_identity_stays_separate() {
    let directory = tempfile::tempdir().unwrap();
    let cache = Cache::at(directory.path().into());
    let portra = Selection {
        negative: spektrafilm::Negative::from_id("kodak_portra_400"),
        variant: Variant::A,
    };
    let variants = [
        portra,
        Selection {
            negative: spektrafilm::Negative::from_id("kodak_ektar_100"),
            ..portra
        },
        Selection {
            variant: Variant::B,
            ..portra
        },
        Selection {
            negative: None,
            ..portra
        },
    ];
    let lut = Arc::new(Lut3D::new(3, [-0.5; 3], [1.; 3], vec![[0.4; 3]; 27]).unwrap());
    let asset = LookAsset::new(portra.name(), portra.origin(), lut.clone()).unwrap();
    cache
        .store_photochemical(material::Selection::Frontier(portra), &asset)
        .unwrap();
    assert_eq!(
        cache
            .load_photochemical(material::Selection::Frontier(portra), 3)
            .unwrap(),
        Some(asset.clone())
    );
    for other in &variants[1..] {
        assert!(
            cache
                .load_photochemical(material::Selection::Frontier(*other), 3)
                .unwrap()
                .is_none()
        );
        assert!(
            cache
                .store_photochemical(material::Selection::Frontier(*other), &asset)
                .is_err()
        );
        let relabelled = LookAsset::new(other.name(), other.origin(), lut.clone()).unwrap();
        assert_eq!(asset.id(), relabelled.id());
        assert_ne!(asset.origin(), relabelled.origin());
    }
    assert!(
        cache
            .load_photochemical(material::Selection::Frontier(portra), 5)
            .unwrap()
            .is_none()
    );
}
