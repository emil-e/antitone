use super::*;

#[test]
fn cold_fit_acquires_unresolved_interpretations_before_computation_and_needs_no_loader_afterward() {
    use std::sync::atomic::Ordering;
    let (mut request, _directory, device, queue) = request();
    let paths: Vec<_> = request
        .document
        .project
        .order
        .iter()
        .map(|id| request.document.source_path(*id).unwrap())
        .collect();
    let mut document = Document::default();
    document.project.input = InputSettings::Digital;
    document.import(&paths).unwrap();
    document.project.input = request.document.project.input;
    let before = document.project.clone();
    for id in &document.project.order {
        assert!(
            document
                .image_recipe(*id)
                .unwrap()
                .neutral
                .source_development
                .resolved()
                .is_err()
        );
    }
    assert!(document.scan_basis().is_none());
    request.document = document;
    approved(&mut request);
    let (loader, developments) = observed();
    let resources = Resources {
        sources: loader.provider(None),
        sampling_inputs: Default::default(),
        initialization: Arc::default(),
        device,
        queue,
        renderers: Arc::new(crate::renderer::RendererCache::new(Default::default())),
    };
    let fit = resources.fit(request.clone(), None).unwrap();
    assert_eq!(developments.load(Ordering::Relaxed), 2);
    assert_eq!(fit.request.document.project, before);
    for id in &before.order {
        assert_eq!(
            fit.request
                .document
                .image_recipe(*id)
                .unwrap()
                .neutral
                .source_development
                .resolved()
                .unwrap()
                .decode
                .white_balance,
            crate::project::UNITY_WB
        );
    }
    loader.close_and_wait().unwrap();
    assert!(
        resources.fit(request.clone(), None).is_err(),
        "without retained camera rasters the closed loader must prevent a fresh acquisition"
    );
    let outcome = pollster::block_on(fit.run(Cancellation::default(), |_, _| {}))
        .unwrap()
        .unwrap();
    assert_eq!(outcome.frames, 2);
    assert!(
        outcome
            .document
            .project
            .dmin()
            .unwrap()
            .iter()
            .all(|value| (value - 3000. / 4031.).abs() <= 2. / 65535.)
    );
    assert_eq!(
        outcome.document.project.sources[0].development_mode,
        crate::project::DevelopmentMode::AsShot
    );
    assert_eq!(developments.load(Ordering::Relaxed), 2);
    assert_eq!(request.document.project, before);
}

#[test]
fn acquired_rasters_establish_facts_before_matching_captured_region_evidence() {
    use std::sync::atomic::Ordering;
    let (mut request, _directory, device, queue) = request();
    let paths: Vec<_> = request
        .document
        .project
        .order
        .iter()
        .map(|id| request.document.source_path(*id).unwrap())
        .collect();
    let mut document = Document::default();
    document.project.input = InputSettings::Digital;
    document.import(&paths).unwrap();
    document.project.input = request.document.project.input;
    let before = document.project.clone();
    let initialization = Arc::new(crate::source::Initialization::default());
    let (loader, developments) = observed();
    let prepared = pollster::block_on(crate::editor::spatial::preparation::prepare(
        Arc::new(document.clone()),
        loader.provider(None),
        Default::default(),
        initialization.clone(),
        Cancellation::default(),
        |_, _| {},
    ))
    .unwrap()
    .unwrap();
    document.spatial_metadata = Arc::new(prepared.metadata);
    assert!(
        document
            .image_recipe(document.project.order[0])
            .unwrap()
            .neutral
            .source_development
            .resolved()
            .is_err(),
        "the snapshot must not already contain the independently acquired initialization facts"
    );
    request.document = document;
    approved(&mut request);
    loader.close_and_wait().unwrap();
    let resources = Resources {
        sources: loader.provider(None),
        sampling_inputs: prepared.sampling_inputs,
        initialization,
        device,
        queue,
        renderers: Arc::new(crate::renderer::RendererCache::new(Default::default())),
    };
    let fit = resources.fit(request.clone(), None).unwrap();
    assert_eq!(developments.load(Ordering::Relaxed), 2);
    let outcome = pollster::block_on(fit.run(Cancellation::default(), |_, _| {}))
        .unwrap()
        .unwrap();
    assert_eq!(outcome.frames, 2);
    assert_eq!(request.document.project, before);
    assert_eq!(developments.load(Ordering::Relaxed), 2);
}

#[test]
fn analyze_consumes_unity_interpretations_from_mixed_registration_modes_with_and_without_correction()
 {
    for corrected in [false, true] {
        let (mut request, directory, device, queue) = request();
        let paths: Vec<_> = request
            .document
            .project
            .order
            .iter()
            .map(|id| request.document.source_path(*id).unwrap())
            .collect();
        let mut bytes = std::fs::read(&paths[1]).unwrap();
        let ifd = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
        let count = u16::from_le_bytes(bytes[ifd..ifd + 2].try_into().unwrap()) as usize;
        let neutral = (0..count)
            .map(|index| ifd + 2 + index * 12)
            .find(|entry| {
                u16::from_le_bytes(bytes[*entry..*entry + 2].try_into().unwrap()) == 50728
            })
            .expect("authored AsShotNeutral tag");
        let offset =
            u32::from_le_bytes(bytes[neutral + 8..neutral + 12].try_into().unwrap()) as usize;
        for (index, gain) in [2u32, 1, 3].into_iter().enumerate() {
            bytes[offset + index * 8..offset + index * 8 + 4].copy_from_slice(&1u32.to_le_bytes());
            bytes[offset + index * 8 + 4..offset + index * 8 + 8]
                .copy_from_slice(&gain.to_le_bytes());
        }
        std::fs::write(&paths[1], bytes).unwrap();
        let mut document = Document::default();
        document.import(&paths[..1]).unwrap();
        document.project.input = InputSettings::Digital;
        document.project.scan = None;
        document.import(&paths[1..]).unwrap();
        assert_eq!(
            document.project.sources[0].development_mode,
            crate::project::DevelopmentMode::Unity
        );
        assert_eq!(
            document.project.sources[1].development_mode,
            crate::project::DevelopmentMode::AsShot
        );
        assert_ne!(
            document.project.sources[1].development.decode.white_balance,
            crate::project::UNITY_WB
        );
        document.project.input = request.document.project.input;
        let mut scope: DocumentScope = document.into();
        let scheduler = source_loading::SourceLoader::new();
        let sources = scheduler.provider(None);
        let mut sampling_inputs = SamplingInputs::default();
        for id in &scope.project.order {
            let acquired = pollster::block_on(
                scope
                    .source_input(*id)
                    .unwrap()
                    .source(sources.as_ref(), source_loading::Priority::Foreground),
            )
            .unwrap();
            assert_eq!(
                acquired.record.development.decode.white_balance,
                crate::project::UNITY_WB
            );
            let input = Arc::new(
                crate::scan::AnalysisInput::capture(&acquired.prepared.scan(), || false)
                    .unwrap()
                    .unwrap(),
            );
            sampling_inputs.insert(
                *id,
                scope
                    .state
                    .source_key_with(*id, &scope.initialization)
                    .unwrap(),
                input,
            );
        }
        scope.state.capture_initialization(&scope.initialization);
        if corrected {
            let basis = scope.capture().scan_basis().unwrap();
            scope.project.scan = Some(crate::project::ScanSetup {
                basis,
                flat_field: None,
            });
            let map = Arc::new(
                crate::flat_field::FlatField::from_nodes(
                    [0, 0, 512, 512],
                    [2, 2],
                    vec![glam::DVec3::splat(1.05); 4],
                )
                .unwrap(),
            );
            scope
                .project
                .set_flat_field(Some(crate::project::FlatFieldSettings::new(
                    "reference.dng",
                    map,
                )))
                .unwrap();
        }
        request.document = scope.capture();
        request.selected = scope.project.order[0];
        approved(&mut request);
        let resources = Resources {
            sources,
            sampling_inputs,
            initialization: scope.initialization.clone(),
            device,
            queue,
            renderers: Arc::new(crate::renderer::RendererCache::new(Default::default())),
        };
        let correction = input(&resources, &request);
        let before = request.document.project.clone();
        let outcome = pollster::block_on(run(
            request.clone(),
            correction,
            resources,
            Cancellation::default(),
            |_, _| {},
        ))
        .unwrap()
        .unwrap();
        assert_eq!(outcome.frames, 2);
        let gain = if corrected { 1.05 } else { 1. };
        assert!(
            outcome
                .document
                .project
                .dmin()
                .unwrap()
                .iter()
                .all(|value| (value - (3000. / 4031.) * gain).abs() <= (2. / 65535.) * gain)
        );
        assert_eq!(request.document.project, before);
        for (after, before) in outcome.document.project.sources.iter().zip(&before.sources) {
            assert_eq!(
                (
                    after.id,
                    &after.path,
                    &after.development,
                    after.development_mode
                ),
                (
                    before.id,
                    &before.path,
                    &before.development,
                    before.development_mode
                )
            );
        }
        assert_eq!(
            outcome.document.project.analysis_regions,
            request.areas.as_ref().unwrap().entries
        );
        scheduler.close_and_wait().unwrap();
        drop(directory);
    }
}
