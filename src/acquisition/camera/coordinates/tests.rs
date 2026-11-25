use super::*;

#[cfg(all(feature = "raw", feature = "export"))]
#[test]
fn original_pairs_keep_real_rectangles_exact_slots_and_admit_each_member() {
    for size in [[1, 1], [127, 65], [128, 128]] {
        let origins = [[10, 20], [(1 << 24) - size[0], (1 << 24) - size[1]]];
        let pair = SourceGeometry::original_pair(origins, size).unwrap();
        let count = size[0] * size[1];
        assert_eq!(pair.pixels(), 2 * count);
        assert_eq!(pair.size(), [size[0], 2 * size[1]]);
        let regions: Vec<_> = pair.regions().collect();
        assert_eq!(regions.len(), 2);
        for index in 0..2 * count {
            let member = (index / count) as usize;
            let i = index % count;
            assert_eq!(
                pair.pixel(index).unwrap(),
                [
                    origins[member][0] + i % size[0],
                    origins[member][1] + i / size[0]
                ]
            );
        }
        assert!(pair.pixel(2 * count).is_err());
        let uniform = SourceCoordinates::new(Some(pair));
        assert_eq!(std::mem::size_of_val(&uniform), 48);
        let words: &[u32] = bytemuck::cast_slice(bytemuck::bytes_of(&uniform));
        assert_eq!(&words[4..8], &[size[0], 2 * size[1], size[0], size[1]]);
        assert_eq!(
            &words[8..12],
            &[origins[0][0], origins[0][1], origins[1][0], origins[1][1]]
        );
    }
    for origins in [
        [[0, 0], [1 << 24, 0]],
        [[1 << 24, 0], [0, 0]],
        [[0, 0], [0, (1 << 24) - 127]],
    ] {
        assert!(SourceGeometry::original_pair(origins, [128, 128]).is_err());
    }
    assert!(SourceGeometry::original_pair([[0, 0]; 2], [0, 128]).is_err());
    assert!(SourceGeometry::original_pair([[0, 0]; 2], [65536, 65536]).is_err());
    let single = SourceGeometry::new([10, 20, 513, 65], [128, 32]).unwrap();
    assert!(!single.is_pair());
    assert_eq!(single.regions().count(), 1);
    let uniform = SourceCoordinates::new(Some(single));
    let words: &[u32] = bytemuck::cast_slice(bytemuck::bytes_of(&uniform));
    assert_eq!(&words[6..8], &[0, 0]);
    assert_eq!(&words[8..12], &[0; 4]);
}

#[test]
fn source_geometry_uses_exact_nearest_centres_and_rejects_overflow() {
    let geometry = SourceGeometry::new([12, 14, 8640, 5760], [257, 173]).unwrap();
    for i in [0, 1, 256, 257, 44460] {
        let [x, y] = geometry.pixel(i).unwrap();
        assert_eq!(
            x,
            12 + ((2 * u64::from(i % 257) + 1) * 8640 / (2 * 257)) as u32
        );
        assert_eq!(
            y,
            14 + ((2 * u64::from(i / 257) + 1) * 5760 / (2 * 173)) as u32
        );
    }
    assert!(geometry.pixel(44461).is_err());
    for (area, size) in [
        ([17, 29, 23, 19], [1, 1]),
        ([17, 29, 23, 19], [7, 1]),
        ([17, 29, 23, 19], [1, 5]),
        ([16_777_190, 16_777_180, 23, 19], [7, 5]),
    ] {
        let geometry = SourceGeometry::new(area, size).unwrap();
        for index in 0..size[0] * size[1] {
            let pixel = geometry.pixel(index).unwrap();
            for axis in 0..2 {
                let coordinate = if axis == 0 {
                    index % size[0]
                } else {
                    index / size[0]
                };
                let expected = u64::from(area[axis])
                    + (2 * u64::from(coordinate) + 1) * u64::from(area[axis + 2])
                        / (2 * u64::from(size[axis]));
                assert_eq!(u64::from(pixel[axis]), expected);
            }
        }
    }
    for (a, s) in [
        ([0, 0, 0, 100], [1, 1]),
        ([u32::MAX, 0, 2, 2], [1, 1]),
        ([0, 0, 8704, 6144], [u32::MAX, 1]),
        ([0, 0, 100, 100], [0, 2]),
    ] {
        assert!(SourceGeometry::new(a, s).is_err());
    }
}
