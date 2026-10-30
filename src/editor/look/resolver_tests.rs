use super::tests::asset;
use super::*;
use crate::looks::Lut3D;
use std::time::Instant;

#[test]
fn cached_and_loaded_assets_do_not_initialize_the_baker() {
    pollster::block_on(async {
        let instance = crate::gpu::instance();
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let gate = crate::gpu::SubmissionGate::default();
        let baker = crate::looks::bake::Baker::new(device.clone(), queue.clone(), gate.clone());
        assert!(!baker.is_initialized());
        let directory = tempfile::tempdir().unwrap();
        let cache = Cache::at(directory.path().into());
        // A valid cached asset at another grid must not satisfy the requested bake spec.
        let other_grid = if DEFAULT_GRID == 65 { 97 } else { 65 };
        let other = LookAsset::new(
            Builtin::Kodak2383.name().into(),
            Builtin::Kodak2383.origin(),
            Arc::new(
                Lut3D::new(
                    other_grid,
                    [crate::looks::builtin::DOMAIN_MIN as f32; 3],
                    [crate::looks::builtin::domain_max() as f32; 3],
                    vec![[0.4; 3]; (other_grid as usize).pow(3)],
                )
                .unwrap(),
            ),
        )
        .unwrap();
        cache.store(Builtin::Kodak2383, &other).unwrap();
        let cancel = AtomicBool::new(false);
        let start = Instant::now();
        let first = prepare_asset(
            Source::Builtin(Builtin::Kodak2383),
            &cancel,
            &baker,
            Some(&cache),
        )
        .unwrap()
        .unwrap();
        let cold = start.elapsed();
        let fresh = crate::looks::bake::Baker::new(device.clone(), queue.clone(), gate.clone());
        assert!(!fresh.is_initialized());
        let start = Instant::now();
        let second = prepare_asset(
            Source::Builtin(Builtin::Kodak2383),
            &cancel,
            &fresh,
            Some(&cache),
        )
        .unwrap()
        .unwrap();
        let warm = start.elapsed();
        assert!(
            !fresh.is_initialized(),
            "a cache hit must not initialize the generator"
        );
        let loaded = asset(0.4);
        let restored = prepare_asset(Source::Loaded(loaded.clone()), &cancel, &fresh, None)
            .unwrap()
            .unwrap();
        assert_eq!(restored, loaded);
        assert!(Arc::ptr_eq(restored.lut(), loaded.lut()));
        assert!(!fresh.is_initialized(), "loaded assets need no generator");
        assert_eq!(first.lut().size(), DEFAULT_GRID);
        assert_eq!(first, second);
        assert_eq!(
            cache.load(Builtin::Kodak2383, other_grid).unwrap(),
            Some(other)
        );
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 2);
        println!(
            "look resolver Kodak2383/{DEFAULT_GRID}³ cold-generate+validate+persist={cold:?} warm-load+validate={warm:?}; excludes presentation/upload"
        );
    });
}
