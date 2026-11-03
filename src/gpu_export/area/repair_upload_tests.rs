use super::*;

#[test]
fn logical_repairs_upload_only_the_requested_support_without_retaining_gpu_buffers() {
    let (device, _, _) = super::super::tests::context();
    let area = [0, 0, 128, 128].into();
    let repair = crate::dust_repair::Repair {
        id: crate::dust_repair::RepairId(1),
        enabled: true,
        origin: crate::dust_repair::RepairOrigin::Manual,
        support: crate::dust_repair::SupportMask::new([16, 24], [16, 16], vec![true; 256]).unwrap(),
        offset: [48, 0],
    };
    let evaluated =
        Arc::new(crate::dust_repair::evaluate(&repair, area, |_| Ok([0.25, 0.5, 0.75])).unwrap());
    let snapshot = crate::gpu::repairs::Snapshot::logical(vec![evaluated]).unwrap();
    snapshot.require_area(area).unwrap();
    let retained = snapshot.byte_len();
    let patch = &snapshot.0[0];
    // Each sparse GPU record contains a 16-byte address and a 16-byte AP1 value.
    let full = patch.buffer_for(&device, area).unwrap().unwrap();
    assert_eq!(full.size(), 256 * 32);
    let part = patch
        .buffer_for(&device, [20, 28, 4, 6].into())
        .unwrap()
        .unwrap();
    assert_eq!(part.size(), 24 * 32);
    assert!(
        patch
            .buffer_for(&device, [100, 100, 8, 8].into())
            .unwrap()
            .is_none()
    );
    assert_eq!(snapshot.byte_len(), retained);
}
