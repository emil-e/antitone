use super::*;

#[test]
fn healing_feedback_distinguishes_success_failure_and_cancellation_without_poisoning_composition() {
    use crate::dust_repair::{Repair, RepairId, RepairOrigin, SupportMask};
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let renderer = pollster::block_on(GpuRenderer::new_with_cache_policy(
        &device,
        &queue,
        evaluation::CachePolicy { generated_bytes: 0 },
    ))
    .unwrap();
    let scan = Scan::open(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng"),
        Some([1.; 4]),
    )
    .unwrap();
    let recipe = recipe(&scan, InputSettings::Digital);
    let mut original = Source::from_recipe(
        prepared_field(&device, &scan),
        &renderer,
        &queue,
        &scan,
        &recipe,
    )
    .unwrap();
    let repair = |id, origin| Repair {
        id: RepairId(id),
        enabled: true,
        origin: RepairOrigin::Manual,
        support: SupportMask::new(origin, [3, 3], vec![true; 9]).unwrap(),
        offset: [30, 0],
    };
    let failed = repair(1, [scan.area.x, scan.area.y]);
    let healthy = repair(2, [scan.area.x + 50, scan.area.y + 50]);
    assert_eq!(
        original.repair_succeeded(&healthy, &|| false).unwrap(),
        Some(true)
    );
    assert_eq!(
        original.repair_succeeded(&failed, &|| false).unwrap(),
        Some(false)
    );
    assert_eq!(original.repair_succeeded(&healthy, &|| true).unwrap(), None);

    let region = SourceRegion::from([scan.area.x, scan.area.y, 3, 3]);
    let expected = original.read(region, None, &|| false).unwrap().unwrap();
    let image =
        crate::gpu::repairs::definition::compose(original.image().clone(), &[failed, healthy])
            .unwrap();
    let mut repaired = Source::from_image(&renderer, &queue, image).unwrap();
    assert_eq!(
        repaired.read(region, None, &|| false).unwrap().unwrap(),
        expected
    );
    assert_eq!(renderer.evaluation().retained_bytes(), 0);
}
