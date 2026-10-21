use super::*;
use crate::looks::{LookOrigin, Lut3D};
use std::time::Instant;
fn asset() -> Arc<LookAsset> {
    let mut nodes = Vec::new();
    for b in 0..2 {
        for g in 0..2 {
            for r in 0..2 {
                nodes.push([r as f32, g as f32, b as f32]);
            }
        }
    }
    Arc::new(
        LookAsset::new(
            "Embedded test look".into(),
            LookOrigin::Imported,
            Arc::new(Lut3D::new(2, [0.; 3], [1.; 3], nodes).unwrap()),
        )
        .unwrap(),
    )
}
fn request(editor: &mut Editor) -> (u64, Source, Arc<AtomicBool>) {
    let pending = editor.document.look.as_mut().unwrap();
    (
        pending.id,
        pending.source.take().unwrap(),
        pending.cancel.clone(),
    )
}

#[test]
fn historic_builtin_sources_use_catalog_labels_but_imports_keep_their_names() {
    let data = asset();
    for (builtin, name, old_name) in [
        (Builtin::PortraEndura, "Portrait Paper", "Portra Endura"),
        (Builtin::ProImageII, "Colour Paper", "Pro Image II"),
        (Builtin::Kodak2383, "Cinema Print", "Kodak 2383"),
        (
            Builtin::IlfordFilter2,
            "Monochrome Paper",
            "Ilford RC Deluxe · Filter 2",
        ),
    ] {
        let legacy = LookAsset::new(
            old_name.into(),
            LookOrigin::Builtin {
                id: builtin.id().into(),
                version: 73,
                generator: "historic-generator".into(),
                definition: "a".repeat(64),
            },
            data.lut().clone(),
        )
        .unwrap();
        assert_eq!(asset_name(&legacy), name);
        assert_eq!(asset_builtin(&legacy), Some(builtin));
        let note = source_note(builtin);
        assert!(note.contains(builtin.source_product()));
        assert!(note.contains("Independent creative interpretation"));
        assert!(note.contains("Not affiliated with or endorsed"));
    }
    let imported = LookAsset::new(
        "Kodak 2383".into(),
        LookOrigin::Imported,
        data.lut().clone(),
    )
    .unwrap();
    assert_eq!(asset_name(&imported), "Kodak 2383");
    assert_eq!(asset_builtin(&imported), None);
    let unknown = LookAsset::new(
        "Saved historic look".into(),
        LookOrigin::Builtin {
            id: "unknown-source".into(),
            version: 1,
            generator: "historic-generator".into(),
            definition: "a".repeat(64),
        },
        data.lut().clone(),
    )
    .unwrap();
    assert_eq!(asset_name(&unknown), "Saved historic look");
    assert_eq!(asset_builtin(&unknown), None);
}
#[test]
fn loaded_luts_remain_available_after_switching_and_duplicate_loads_reuse_one_entry() {
    let mut editor = super::super::tests::fixture();
    let first = asset();
    let mut nodes = first.lut().nodes().to_vec();
    nodes[0][0] += 0.01;
    let second = Arc::new(
        LookAsset::new(
            first.name().into(),
            LookOrigin::Imported,
            Arc::new(Lut3D::new(2, [0.; 3], [1.; 3], nodes).unwrap()),
        )
        .unwrap(),
    );
    for chosen in [first.clone(), second.clone(), first.clone()] {
        editor.begin_look(Source::Loaded(chosen.clone()));
        let (id, _, _) = request(&mut editor);
        editor.receive_look(id, Ok(Some(chosen)));
    }
    assert_eq!(editor.loaded_looks.len(), 2);
    assert_eq!(
        loaded_names(&editor.loaded_looks),
        ["Embedded test look", "Embedded test look (2)"]
    );
    let names = loaded_names(&editor.loaded_looks);
    assert_eq!(editor.loaded_looks[0].name(), first.name());
    assert_ne!(first.lut().id(), second.lut().id());
    editor.clear_look();
    assert_eq!(editor.loaded_looks.len(), 2);
    assert!(editor.document.project.output.look.is_none());
    let mut novel_nodes = second.lut().nodes().to_vec();
    novel_nodes[0][0] += 0.01;
    let novel = Arc::new(
        LookAsset::new(
            "Unaccepted LUT".into(),
            LookOrigin::Imported,
            Arc::new(Lut3D::new(2, [0.; 3], [1.; 3], novel_nodes).unwrap()),
        )
        .unwrap(),
    );
    for rejected in [false, true] {
        editor.begin_look(Source::Loaded(novel.clone()));
        let (id, _, _) = request(&mut editor);
        if rejected {
            editor.receive_look(id, Err("invalid LUT".into()));
        } else {
            editor.cancel_look();
            editor.receive_look(id, Ok(Some(novel.clone())));
        }
        assert_eq!(editor.loaded_looks.len(), 2);
        assert!(editor.document.project.output.look.is_none());
    }
    let reserved =
        Arc::new(LookAsset::new("None".into(), LookOrigin::Imported, first.lut().clone()).unwrap());
    let collision = Arc::new(
        LookAsset::new(
            "None (2)".into(),
            LookOrigin::Imported,
            second.lut().clone(),
        )
        .unwrap(),
    );
    assert_eq!(
        loaded_names(&[reserved.clone(), collision, reserved]),
        ["None (2)", "None (2) (2)", "None (3)"]
    );
    assert_eq!(loaded_names(&editor.loaded_looks), names);
}
#[test]
fn current_success_installs_one_project_wide_embedded_asset_and_stale_replies_do_not() {
    let mut editor = super::super::tests::fixture();
    let original = editor.document.project.clone();
    editor.begin_look(Source::Builtin(Builtin::PortraEndura));
    let (old, _, cancel) = request(&mut editor);
    assert!(editor.busy);
    assert_eq!(editor.document.project, original);
    editor.begin_look(Source::Builtin(Builtin::Kodak2383));
    let (current, _, _) = request(&mut editor);
    assert!(cancel.load(Ordering::Relaxed));
    editor.receive_look(old, Ok(Some(asset())));
    assert!(editor.busy);
    assert!(editor.document.project.output.look.is_none());
    assert!(editor.loaded_looks.is_empty());
    let chosen = asset();
    editor.receive_look(current, Ok(Some(chosen.clone())));
    assert!(!editor.busy);
    assert_eq!(editor.document.project.output.look, Some(chosen));
    assert!(editor.document.look.is_none());
    assert_eq!(editor.loaded_looks.len(), 1);
    let document = serde_json::to_vec(&editor.document.project).unwrap();
    let reopened: crate::project::Project = serde_json::from_slice(&document).unwrap();
    assert_eq!(reopened.output.look, editor.document.project.output.look);
}
#[test]
fn failure_cancel_and_document_retirement_preserve_prior_appearance() {
    let mut editor = super::super::tests::fixture();
    editor.document.project.output.look = Some(asset());
    let original = editor.document.project.clone();
    editor.begin_look(Source::Imported("missing.cube".into()));
    let (id, _, _) = request(&mut editor);
    editor.receive_look(id, Err("invalid LUT".into()));
    assert_eq!(editor.document.project, original);
    assert!(!editor.busy);
    assert_eq!(editor.failure.as_ref().unwrap().kind, FailureKind::Look);
    editor.failure = None;
    editor.begin_look(Source::Builtin(Builtin::ProImageII));
    let (id, _, cancel) = request(&mut editor);
    editor.cancel_task(TaskRequest::Look(id));
    assert!(cancel.load(Ordering::Relaxed));
    assert!(editor.busy);
    editor.receive_look(id, Ok(Some(asset())));
    assert_eq!(editor.document.project, original);
    assert!(!editor.busy);
    editor.begin_look(Source::Builtin(Builtin::IlfordFilter2));
    let (id, _, cancel) = request(&mut editor);
    editor.retire_look();
    assert!(cancel.load(Ordering::Relaxed));
    editor.receive_look(id, Ok(Some(asset())));
    assert_eq!(editor.document.project, original);
    editor.clear_look();
    assert!(editor.document.project.output.look.is_none());
    assert!(!editor.busy);
}
#[test]
fn successful_resolution_is_reused_without_requiring_a_prepared_generator() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let gate = crate::gpu::SubmissionGate::default();
        let renderers = crate::renderer::RendererCache::new(gate.clone());
        let start = Instant::now();
        renderers.get(&device, &queue).unwrap();
        assert!(renderers.is_initialized());
        let renderer_setup = start.elapsed();
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
        let first = resolve(
            Source::Builtin(Builtin::Kodak2383),
            &cancel,
            &renderers,
            &device,
            &queue,
            &gate,
            Some(&cache),
        )
        .unwrap()
        .unwrap();
        let cold = start.elapsed();
        let fresh = crate::renderer::RendererCache::new(gate.clone());
        assert!(!fresh.is_initialized());
        let start = Instant::now();
        let second = resolve(
            Source::Builtin(Builtin::Kodak2383),
            &cancel,
            &fresh,
            &device,
            &queue,
            &gate,
            Some(&cache),
        )
        .unwrap()
        .unwrap();
        let warm = start.elapsed();
        assert!(
            !fresh.is_initialized(),
            "a cache hit must not initialize the generator"
        );
        let loaded = asset();
        let restored = resolve(
            Source::Loaded(loaded.clone()),
            &cancel,
            &fresh,
            &device,
            &queue,
            &gate,
            None,
        )
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
            "look resolver Kodak2383/{DEFAULT_GRID}³ renderer-setup={renderer_setup:?} cold-generate+validate+persist={cold:?} warm-load+validate={warm:?}; excludes presentation/upload"
        );
    });
}

#[test]
fn positive_input_adopts_look_but_source_inspection_cannot() {
    let look = asset();
    let source =
        ProcessingInput::from(ProcessingParameters::source()).with_look(Some(look.clone()));
    assert!(source.look().is_none());
    let calibration = Calibration {
        dmin: DVec3::splat(0.7),
        dmax: DVec3::splat(0.003),
        input_floor: crate::processing::CAMERA_INPUT_FLOOR,
    };
    let input = ProcessingInput::from(
        ProcessingParameters::positive(
            calibration,
            RollSettings::new_grade(),
            PhotoSettings::default(),
            glam::DMat3::IDENTITY,
        )
        .unwrap(),
    );
    let looked = input.clone().with_look(Some(look));
    assert_ne!(input.signature(), looked.signature());
    assert!(input.signature().same_photographic(&looked.signature()));
}
