use super::*;

fn mask() -> SupportMask {
    SupportMask::new(
        [12, 20],
        [8, 2],
        (0..16)
            .map(|index| [0, 1, 5, 8, 9, 15].contains(&index))
            .collect(),
    )
    .unwrap()
}

#[test]
fn cloned_repairs_share_immutable_runs_and_replacing_support_preserves_the_snapshot() {
    let mut repairs = RepairSet::default();
    let id = repairs
        .insert(mask(), RepairOrigin::Manual, DonorChoice::Automatic)
        .unwrap();
    let snapshot = repairs.clone();
    assert!(Arc::ptr_eq(
        &repairs.repairs()[0].support.runs,
        &snapshot.repairs()[0].support.runs
    ));
    let replacement = SupportMask::new([14, 20], [2, 1], vec![true; 2]).unwrap();
    repairs.set_support(id, replacement).unwrap();
    assert!(!Arc::ptr_eq(
        &repairs.repairs()[0].support.runs,
        &snapshot.repairs()[0].support.runs
    ));
    assert_eq!(
        snapshot.repairs()[0]
            .support
            .selected_points()
            .collect::<Vec<_>>(),
        [[12, 20], [13, 20], [17, 20], [12, 21], [13, 21], [19, 21]]
    );
    assert_eq!(
        repairs.repairs()[0]
            .support
            .selected_points()
            .collect::<Vec<_>>(),
        [[14, 20], [15, 20]]
    );
}

#[test]
fn each_logical_owner_is_charged_for_shared_run_payload() {
    let support = mask();
    let cloned = support.clone();
    // Four authored [u32; 2] runs occupy 32 bytes, independently of Arc bookkeeping.
    let expected = std::mem::size_of::<SupportMask>() + 32;
    assert_eq!(support.retained_bytes(), expected);
    assert_eq!(cloned.retained_bytes(), expected);
    assert_eq!(support.marked_pixels(), 6);
}

#[cfg(feature = "project")]
#[test]
fn shared_runs_keep_the_project_json_contract_and_value_equality() {
    let support = mask();
    let expected = serde_json::json!({
        "origin": [12, 20],
        "size": [8, 2],
        "runs": [[0, 2], [5, 1], [8, 2], [15, 1]]
    });
    assert_eq!(serde_json::to_value(&support).unwrap(), expected);
    let restored: SupportMask = serde_json::from_value(expected).unwrap();
    restored.validate().unwrap();
    assert_eq!(restored, support);
    assert!(!Arc::ptr_eq(&restored.runs, &support.runs));
}
