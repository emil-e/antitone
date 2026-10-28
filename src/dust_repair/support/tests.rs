use super::*;

#[test]
fn placement_matches_an_exhaustive_independent_overlap_check_on_cramped_sources() {
    let active = SourceRegion {
        x: 3,
        y: 5,
        width: 20,
        height: 19,
    };
    for seed in 0..64u32 {
        let size = [8 + seed % 9, 7 + seed / 9];
        let bits = (0..size[0] * size[1])
            .map(|i| i.wrapping_mul(2654435761).rotate_left(seed % 31) % 13 < 2 + seed % 11)
            .collect();
        let support = SupportMask::new([5, 7], size, bits).unwrap();
        let target = donor_target(&support, active).unwrap();
        let clean = |offset: [i32; 2]| {
            offset_target(&support, active, offset).is_ok()
                && support.selected_points().all(|[x, y]| {
                    (-2..=2).all(|dy| {
                        (-2..=2).all(|dx| {
                            let p = [x as i32 + offset[0] + dx, y as i32 + offset[1] + dy];
                            p[0] < 0 || p[1] < 0 || !support.contains(p.map(|v| v as u32))
                        })
                    })
                })
        };
        let exists = (active.y as i32 - target.y as i32
            ..=(active.y + active.height - target.y - target.height) as i32)
            .any(|dy| {
                (active.x as i32 - target.x as i32
                    ..=(active.x + active.width - target.x - target.width) as i32)
                    .any(|dx| clean([dx, dy]))
            });
        let placed = initial_offset(&support, active);
        assert_eq!(placed.is_some(), exists, "seed {seed}");
        if let Some(offset) = placed {
            assert!(clean(offset), "seed {seed}: {offset:?}");
        }
    }
}

#[test]
fn scanline_placement_finds_a_clean_gap_when_all_bbox_witnesses_are_outside_the_source() {
    let active = SourceRegion {
        x: 0,
        y: 0,
        width: 40,
        height: 40,
    };
    let mut bits = vec![false; 144];
    bits[0] = true;
    bits[143] = true;
    let support = SupportMask::new([14, 14], [12, 12], bits).unwrap();
    for offset in [[-14, 0], [14, 0], [0, -14], [0, 14]] {
        assert!(offset_target(&support, active, offset).is_err());
    }
    // The two marked samples are eleven pixels apart on both axes. A three-pixel
    // horizontal translation leaves each required two-pixel halo clear.
    assert!(offset_target(&support, active, [3, 0]).is_ok());
    let clean = |offset: [i32; 2]| {
        support.selected_points().all(|p| {
            support.selected_points().all(|q| {
                (p[0] as i32 + offset[0] - q[0] as i32)
                    .abs()
                    .max((p[1] as i32 + offset[1] - q[1] as i32).abs())
                    > 2
            })
        })
    };
    assert!(clean([3, 0]));
    let placed = initial_offset(&support, active).expect("a clean gap exists");
    assert!(offset_target(&support, active, placed).is_ok());
    assert!(clean(placed));
    let impossible = SupportMask::new([0, 0], [40, 40], vec![true; 1600]).unwrap();
    assert!(initial_offset(&impossible, active).is_none());
}

#[test]
fn admitted_large_solid_masks_get_a_disjoint_placement_without_nearest_ring_search() {
    let support = SupportMask::new([400, 400], [256, 256], vec![true; 256 * 256]).unwrap();
    let active = SourceRegion {
        x: 0,
        y: 0,
        width: 1200,
        height: 1200,
    };
    let offset = initial_offset(&support, active).unwrap();
    assert!(validate_offset(offset, &support, active).is_ok());
}

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
        .insert(mask(), RepairOrigin::Manual, [7, 0])
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
