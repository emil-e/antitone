use super::tests::asset;
use super::*;
use crate::looks::Lut3D;

#[test]
fn exact_cached_spec_and_loaded_assets_reuse_samples_without_a_gpu_baker() {
    let directory = tempfile::tempdir().unwrap();
    let cache = Cache::at(directory.path().into());
    let selection = Selection {
        negative: None,
        paper: Default::default(),
    };
    let make = |size| {
        LookAsset::new(
            selection.name(),
            selection.origin(),
            Arc::new(
                Lut3D::new(
                    size,
                    [crate::looks::authoring::DOMAIN_MIN as f32; 3],
                    [crate::looks::authoring::domain_max() as f32; 3],
                    vec![[0.4; 3]; (size as usize).pow(3)],
                )
                .unwrap(),
            ),
        )
        .unwrap()
    };
    let other = make(3);
    cache.store_photochemical(selection, &other).unwrap();
    assert!(
        cache
            .load_photochemical(selection, DEFAULT_GRID)
            .unwrap()
            .is_none()
    );
    let expected = make(DEFAULT_GRID);
    cache.store_photochemical(selection, &expected).unwrap();
    let cancel = AtomicBool::new(false);
    let cached = prepare_asset(
        Source::Photochemical(selection.into()),
        &cancel,
        &wgpu::Limits::default(),
        Some(&cache),
    )
    .unwrap()
    .unwrap();
    assert_eq!(*cached, expected);
    assert_eq!(cache.load_photochemical(selection, 3).unwrap(), Some(other));
    let selected = asset(0.7);
    let loaded = prepare_asset(
        Source::Loaded(selected.clone()),
        &cancel,
        &wgpu::Limits::default(),
        None,
    )
    .unwrap()
    .unwrap();
    assert!(Arc::ptr_eq(loaded.lut(), selected.lut()));
    assert_eq!(loaded, selected);
}
