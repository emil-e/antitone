use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

pub(in crate::editor) fn request() -> (
    analysis::Request,
    tempfile::TempDir,
    wgpu::Device,
    wgpu::Queue,
) {
    let instance = crate::gpu::instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw");
    let paths: Vec<_> = ["roll-0.dng", "roll-1.dng"]
        .iter()
        .map(|name| {
            let path = directory.path().join(name);
            std::fs::copy(fixture.join(name), &path).unwrap();
            path
        })
        .collect();
    let mut document = Document::default();
    document.import(&paths).unwrap();
    document.project.set_dmin(Some([0.8; 3]));
    document.project.set_dmax(Some([0.02; 3]));
    let selected = document.project.order[0];
    (
        analysis::Request {
            document,
            selected,
            film: None,
            local_support: false,
            picture: None,
            areas: None,
        },
        directory,
        device,
        queue,
    )
}

pub(in crate::editor) fn observed() -> (source_loading::SourceLoader, Arc<AtomicUsize>) {
    let count = Arc::new(AtomicUsize::new(0));
    let observed = count.clone();
    (
        source_loading::SourceLoader::new_observed(move |_, phase, _| {
            if matches!(phase, source_loading::NativePhase::DevelopBegin) {
                observed.fetch_add(1, Ordering::Relaxed);
            }
            Ok(())
        }),
        count,
    )
}

pub(super) fn detect(
    request: &mut analysis::Request,
    scheduler: &source_loading::SourceLoader,
    _device: &wgpu::Device,
    _queue: &wgpu::Queue,
) {
    let metadata = futures_lite::future::block_on(crate::editor::spatial::preparation::prepare(
        Arc::new(request.document.clone()),
        scheduler.provider(None),
        Default::default(),
        Arc::default(),
        Default::default(),
        |_, _| {},
    ))
    .unwrap()
    .unwrap();
    request.document.spatial_metadata = Arc::new(metadata.metadata);
    assert!(!request.document.ready_spatial_metadata().groups.is_empty());
}

#[test]
fn region_acquisition_and_review_do_not_require_unrelated_look_device_admission() {
    use crate::looks::{LookAsset, LookOrigin, Lut3D};
    let (mut request, _directory, _, _) = request();
    let instance = crate::gpu::instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, _) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: wgpu::Limits {
            max_storage_buffer_binding_size: 1024,
            ..Default::default()
        },
        ..Default::default()
    }))
    .unwrap();
    request.document.project.output.lut = Some(Arc::new(
        LookAsset::new(
            "Valid look beyond this device's capacity".into(),
            LookOrigin::Imported,
            Arc::new(Lut3D::new(5, [0.; 3], [1.; 3], vec![[0.5; 3]; 125]).unwrap()),
        )
        .unwrap(),
    ));
    request.document.project.require_runtime().unwrap();
    assert!(
        crate::gpu::lut3d::Lut::require_limits(
            &device.limits(),
            request.document.project.output.lut.as_ref().unwrap().lut()
        )
        .is_err(),
        "the look must actually fail the unrelated GPU admission check"
    );
    let before = request.document.project.clone();
    let (loader, count) = observed();
    let prepared = pollster::block_on(crate::editor::spatial::preparation::prepare(
        Arc::new(request.document.clone()),
        loader.provider(None),
        Default::default(),
        Arc::default(),
        Default::default(),
        |_, _| {},
    ))
    .unwrap()
    .unwrap();
    request.document.spatial_metadata = Arc::new(prepared.metadata);
    assert!(prepared.sampling_inputs.retained_bytes() > 0);
    assert_eq!(count.load(Ordering::Relaxed), 2);
    loader.close_and_wait().unwrap();
    for id in &request.document.project.order {
        let physical = request
            .document
            .source_input(*id, &Default::default())
            .unwrap();
        let cached = prepared
            .sampling_inputs
            .get(*id, &physical.cached_key().unwrap())
            .unwrap();
        let camera = pollster::block_on(prepared.sampling_inputs.acquire(
            *id,
            &physical,
            loader.provider(None).as_ref(),
            &Cancellation::default(),
        ))
        .unwrap()
        .unwrap();
        assert!(
            Arc::ptr_eq(&camera.pixels(), &cached),
            "retained rasters must remain usable after native acquisition shuts down"
        );
    }
    let review = pollster::block_on(prepare_review(
        request.clone(),
        Cancellation::default(),
        |_, _| {},
    ))
    .unwrap()
    .unwrap();
    assert_eq!(review.entries.len(), 2);
    assert!(review.entries.iter().all(|row| row.areas.ready()));
    assert_eq!(request.document.project, before);
}

#[test]
fn finished_shared_sampling_is_reused_without_changing_accepted_state() {
    let (mut request, _directory, device, queue) = request();
    let before = request.document.project.clone();
    let (scheduler, count) = observed();
    detect(&mut request, &scheduler, &device, &queue);
    assert_eq!(
        count.load(Ordering::Relaxed),
        2,
        "both authored sources must exercise detection"
    );
    let ready = request.document.spatial_metadata.clone();
    let first = assemble_review(&request, || false, |_, _| {})
        .unwrap()
        .unwrap();
    let second = assemble_review(&request, || false, |_, _| {})
        .unwrap()
        .unwrap();
    assert_eq!(first.entries, second.entries);
    assert_eq!(first.files, second.files);
    assert_eq!(first.entries.len(), 2);
    assert!(first.entries.iter().all(|entry| entry.areas.ready()));
    assert_eq!(count.load(Ordering::Relaxed), 2);
    assert_eq!(request.document.spatial_metadata, ready);
    assert_eq!(request.document.project, before);
    scheduler.close_and_wait().unwrap();
}

#[test]
fn cancelling_review_assembly_does_not_return_partial_areas_or_discard_detection() {
    let (mut request, _directory, device, queue) = request();
    let (scheduler, count) = observed();
    detect(&mut request, &scheduler, &device, &queue);
    let before = request.document.project.clone();
    let ready = request.document.spatial_metadata.clone();
    let cancelled = std::cell::Cell::new(false);
    let result = assemble_review(
        &request,
        || cancelled.get(),
        |index, _| {
            if index == 1 {
                cancelled.set(true);
            }
        },
    )
    .unwrap();
    assert!(result.is_none());
    assert_eq!(count.load(Ordering::Relaxed), 2);
    assert_eq!(request.document.project, before);
    assert_eq!(request.document.spatial_metadata, ready);
    scheduler.close_and_wait().unwrap();
}

#[test]
fn authored_windows_override_suggestions_without_changing_shared_results() {
    let (mut request, _directory, device, queue) = request();
    let (scheduler, count) = observed();
    detect(&mut request, &scheduler, &device, &queue);
    let opening = Coverage::rectangle(analysis::rect([16, 16, 480, 480]));
    let picture = Coverage::rectangle(analysis::rect([96, 96, 320, 320]));
    let mut first = Areas::default();
    first.edit_opening(opening);
    first.edit_picture(picture);
    let mut second = Areas {
        opening: Some(
            Coverage::rectangle(analysis::rect([8, 8, 496, 496]))
                .corners()
                .map(|p| p.0),
        ),
        ..Default::default()
    };
    second.edit_picture(picture);
    request.document.project.analysis_regions = vec![
        SourceAnalysis {
            source: request.document.project.order[0],
            areas: first.clone(),
        },
        SourceAnalysis {
            source: request.document.project.order[1],
            areas: second.clone(),
        },
    ];
    let before = request.document.project.clone();
    let ready = request.document.spatial_metadata.clone();
    let approved = assemble_review(&request, || false, |_, _| {})
        .unwrap()
        .unwrap();
    assert_eq!(approved.entries[0].areas, first);
    assert_eq!(approved.entries[1].areas.picture, second.picture);
    assert!(approved.entries[1].areas.picture_edited);
    let detected = ready.groups[0]
        .sources
        .iter()
        .find(|s| s.input.source == approved.entries[1].source)
        .unwrap();
    assert_eq!(
        approved.entries[1].areas.opening,
        detected.sampling_areas().opening
    );
    assert!(approved.entries[1].areas.opening.is_some());
    assert_eq!(count.load(Ordering::Relaxed), 2);
    assert_eq!(request.document.project, before);
    assert_eq!(request.document.spatial_metadata, ready);
    scheduler.close_and_wait().unwrap();
}

#[test]
fn unavailable_detected_components_are_reviewable_without_redetection() {
    let (mut request, _directory, device, queue) = request();
    let (scheduler, count) = observed();
    detect(&mut request, &scheduler, &device, &queue);
    let mut metadata = (*request.document.spatial_metadata).clone();
    let source = metadata.groups[0].sources[0].input.source;
    metadata.groups[0].sources[0].sampling = Default::default();
    metadata.groups[0].sources[0].picture = None;
    metadata.validate().unwrap();
    request.document.spatial_metadata = Arc::new(metadata.clone());
    let approved = assemble_review(&request, || false, |_, _| {})
        .unwrap()
        .unwrap();
    assert!(
        !approved
            .entries
            .iter()
            .find(|r| r.source == source)
            .unwrap()
            .areas
            .ready()
    );
    assert!(
        approved.entries.iter().any(|r| r.areas.ready()),
        "the positive source must remain usable"
    );
    assert_eq!(*request.document.spatial_metadata, metadata);
    assert_eq!(count.load(Ordering::Relaxed), 2);
    scheduler.close_and_wait().unwrap();
}

#[test]
fn missing_shared_detection_is_not_replaced_with_an_operation_local_detector() {
    let (request, _directory, _device, _queue) = request();
    assert!(assemble_review(&request, || false, |_, _| {}).is_err());
    assert!(request.document.spatial_metadata.groups.is_empty());
}
