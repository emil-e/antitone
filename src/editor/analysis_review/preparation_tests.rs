use super::*;
use crate::region_timing;
use std::sync::{Arc, atomic::AtomicUsize};

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
    document
        .project
        .set_input_floor(Some(crate::processing::CAMERA_INPUT_FLOOR));
    let selected = document.project.order[0];
    (
        analysis::Request {
            revision: 1,
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

#[test]
fn scratch_and_unavailable_storage_produce_identical_areas_without_additional_native_retention() {
    let (request, _directory, device, _queue) = request();
    let before = request.document.project.clone();
    let generation = AtomicU64::new(1);
    let (scheduler, count) = observed();
    let (cached, timings) =
        region_timing::record(|| prepare(&request, &generation, &scheduler, &device, |_, _| {}));
    let cached = cached.unwrap().unwrap();
    assert_eq!(count.load(Ordering::Relaxed), 2);
    assert_eq!(
        timings
            .samples
            .iter()
            .filter(|s| s.stage == "coarse_acquire")
            .map(|s| s.source)
            .collect::<Vec<_>>(),
        request
            .document
            .project
            .order
            .iter()
            .map(|id| Some(id.0))
            .collect::<Vec<_>>()
    );
    assert!(!timings.samples.iter().any(|s| s.stage == "fine_acquire"));
    let uncached = prepare_with_measurements(
        &request,
        &generation,
        &scheduler,
        &device,
        |_, _| {},
        &mut measurements::Measurements::unavailable(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(count.load(Ordering::Relaxed), 6);
    let overflow = prepare_with_measurements(
        &request,
        &generation,
        &scheduler,
        &device,
        |_, _| {},
        &mut measurements::Measurements::full(),
    )
    .unwrap()
    .unwrap();
    scheduler.close_and_wait().unwrap();
    assert_eq!(count.load(Ordering::Relaxed), 10);
    assert_eq!(cached.entries, overflow.entries);
    assert_eq!(cached.entries, uncached.entries);
    assert_eq!(cached.files, uncached.files);
    assert_eq!(request.document.project, before);
}

#[test]
fn cancellation_at_the_roll_barrier_neither_reads_scratch_nor_promotes_partial_areas() {
    let (request, _directory, device, _queue) = request();
    let before = request.document.project.clone();
    let generation = AtomicU64::new(1);
    let (scheduler, count) = observed();
    let (result, timings) = region_timing::record(|| {
        prepare(
            &request,
            &generation,
            &scheduler,
            &device,
            |index, total| {
                if index == total / 2 {
                    generation.store(2, Ordering::Relaxed);
                }
            },
        )
    });
    scheduler.close_and_wait().unwrap();
    assert!(result.unwrap().is_none());
    assert_eq!(count.load(Ordering::Relaxed), 2);
    assert_eq!(
        timings
            .samples
            .iter()
            .filter(|s| s.stage == "measurement_write")
            .count(),
        2
    );
    assert!(
        !timings
            .samples
            .iter()
            .any(|s| s.stage == "measurement_read")
    );
    assert_eq!(request.document.project, before);
}

#[test]
fn authored_areas_remain_authoritative_and_complete_edits_need_no_fine_capture() {
    let (mut request, _directory, device, _queue) = request();
    let mut areas = Areas::default();
    let opening = Coverage::rectangle(crate::geometry::SourceRegion {
        x: 16,
        y: 16,
        width: 480,
        height: 480,
    });
    let picture = Coverage::rectangle(crate::geometry::SourceRegion {
        x: 96,
        y: 96,
        width: 320,
        height: 320,
    });
    areas.edit_opening(opening);
    areas.edit_picture(picture);
    let first = SourceAnalysis {
        source: request.document.project.order[0],
        areas: areas.clone(),
    };
    areas = Areas {
        opening: Some(opening.corners().map(|p| p.0)),
        ..Default::default()
    };
    areas.edit_picture(picture);
    let second = SourceAnalysis {
        source: request.document.project.order[1],
        areas,
    };
    request.document.project.analysis_regions = vec![first.clone(), second.clone()];
    let before = request.document.project.clone();
    let (scheduler, count) = observed();
    let (result, timings) = region_timing::record(|| {
        prepare(&request, &AtomicU64::new(1), &scheduler, &device, |_, _| {})
    });
    let approved = result.unwrap().unwrap();
    assert_eq!(count.load(Ordering::Relaxed), 2);
    let reference = prepare_with_measurements(
        &request,
        &AtomicU64::new(1),
        &scheduler,
        &device,
        |_, _| {},
        &mut measurements::Measurements::unavailable(),
    )
    .unwrap()
    .unwrap();
    scheduler.close_and_wait().unwrap();
    assert_eq!(count.load(Ordering::Relaxed), 5);
    assert_eq!(approved.entries, reference.entries);
    assert_eq!(approved.entries[0], first);
    assert_eq!(approved.entries[1].areas.picture, second.areas.picture);
    assert!(approved.entries[1].areas.picture_edited);
    assert_eq!(
        timings
            .samples
            .iter()
            .filter(|s| s.stage == "measurement_write")
            .count(),
        1
    );
    assert_eq!(request.document.project, before);
}
