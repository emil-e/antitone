use super::*;

#[test]
fn rotated_source_basis_preserves_coverage_through_discrete_actions_and_rasters() {
    use crate::frame_raster::FrameRaster;
    let area = SourceRegion {
        x: 12,
        y: 7,
        width: 51,
        height: 33,
    };
    for angle in [-89_f64, -45., -13.7, 0., 19.3, 44., 89.] {
        let crop = Crop {
            center: [34.25, 22.75],
            size: [31, 17],
            angle: angle.to_radians(),
        }
        .constrained(area)
        .unwrap();
        // Zero-angle compatibility retains exact integer edges.
        crop.region(area).unwrap();
        for turns in [
            QuarterTurns::Zero,
            QuarterTurns::One,
            QuarterTurns::Two,
            QuarterTurns::Three,
        ] {
            for horizontal in [false, true] {
                for vertical in [false, true] {
                    let composition = Composition::with_crop(
                        area,
                        RollOrientation {
                            turns,
                            horizontal,
                            vertical,
                        },
                        PhotoOrientation::default(),
                        Some(crop),
                    )
                    .unwrap();
                    let raster = FrameRaster::full(composition).unwrap();
                    let coverage = crop.coverage().unwrap();
                    assert!(
                        (coverage.area() - f64::from(crop.size[0] * crop.size[1])).abs() < 1e-10
                    );
                    assert!(
                        (coverage.pixels(area).unwrap().map(|(_, w)| w).sum::<f64>()
                            - coverage.area())
                        .abs()
                            < 1e-10
                    );
                    for point in [SourcePoint(crop.center), SourcePoint([14.25, 11.5])] {
                        let roundtrip =
                            composition.frame_to_source(composition.source_to_frame(point));
                        for axis in 0..2 {
                            assert!((roundtrip.0[axis] - point.0[axis]).abs() < 1e-11);
                        }
                    }
                    let mut total = 0.;
                    for y in 0..raster.output[1] {
                        for x in 0..raster.output[0] {
                            let footprint = raster.footprint([x, y]).unwrap();
                            assert!(
                                (footprint.pixels(area).unwrap().map(|(_, w)| w).sum::<f64>() - 1.)
                                    .abs()
                                    < 1e-10
                            );
                            total += footprint.area();
                        }
                    }
                    assert!((total - coverage.area()).abs() < 1e-8);
                }
            }
        }
    }
}
const AREA: SourceRegion = SourceRegion {
    x: 12,
    y: 7,
    width: 5,
    height: 3,
};
fn flips() -> [Flips; 4] {
    [
        Flips::default(),
        Flips {
            horizontal: true,
            vertical: false,
        },
        Flips {
            horizontal: false,
            vertical: true,
        },
        Flips {
            horizontal: true,
            vertical: true,
        },
    ]
}
fn rolls() -> impl Iterator<Item = RollOrientation> {
    [
        QuarterTurns::Zero,
        QuarterTurns::One,
        QuarterTurns::Two,
        QuarterTurns::Three,
    ]
    .into_iter()
    .flat_map(|turns| {
        flips().map(|flips| RollOrientation {
            turns,
            ..flips.into()
        })
    })
}
fn fixture(roll: RollOrientation, photo: PhotoOrientation) -> Vec<Vec<[u32; 2]>> {
    fixture_region(AREA, roll, photo)
}
fn fixture_region(
    area: SourceRegion,
    roll: RollOrientation,
    photo: PhotoOrientation,
) -> Vec<Vec<[u32; 2]>> {
    let mut image: Vec<Vec<_>> = (0..area.height)
        .map(|y| (0..area.width).map(|x| [area.x + x, area.y + y]).collect())
        .collect();
    let reflect = |image: &mut Vec<Vec<[u32; 2]>>, flips: Flips| {
        if flips.horizontal {
            for row in image.iter_mut() {
                row.reverse();
            }
        }
        if flips.vertical {
            image.reverse();
        }
    };
    for (turns, flips) in [
        (
            roll.turns,
            Flips {
                horizontal: roll.horizontal,
                vertical: roll.vertical,
            },
        ),
        (photo.turns, photo.flips),
    ] {
        let turns = match turns {
            QuarterTurns::Zero => 0,
            QuarterTurns::One => 1,
            QuarterTurns::Two => 2,
            QuarterTurns::Three => 3,
        };
        for _ in 0..turns {
            image = (0..image[0].len())
                .map(|x| image.iter().rev().map(|row| row[x]).collect())
                .collect();
        }
        reflect(&mut image, flips);
    }
    image
}
#[test]
fn every_discrete_composition_matches_independent_pixel_permutation_and_roundtrips() {
    for roll in rolls() {
        for turns in [
            QuarterTurns::Zero,
            QuarterTurns::One,
            QuarterTurns::Two,
            QuarterTurns::Three,
        ] {
            for photo_flips in flips() {
                let photo = PhotoOrientation {
                    turns,
                    flips: photo_flips,
                };
                let composition = Composition::new(AREA, roll, photo).unwrap();
                let expected = fixture(roll, photo);
                assert_eq!(
                    composition.size(),
                    [expected[0].len() as u32, expected.len() as u32]
                );
                for (y, row) in expected.iter().enumerate() {
                    for (x, &source) in row.iter().enumerate() {
                        assert_eq!(
                            composition.source_pixel([x as u32, y as u32]).unwrap(),
                            source
                        );
                        let p = SourcePoint(source.map(|v| f64::from(v) + 0.5));
                        let frame = composition.source_to_frame(p);
                        assert_eq!(frame, FramePoint([x as f64 + 0.5, y as f64 + 0.5]));
                        assert_eq!(composition.frame_to_source(frame), p);
                    }
                }
                let outside = SourcePoint([-3.25, 100.5]);
                assert_eq!(
                    composition.frame_to_source(composition.source_to_frame(outside)),
                    outside
                );
                assert!(composition.source_pixel(composition.size()).is_err());
            }
        }
    }
}
#[test]
fn displayed_turn_actions_rotate_flipped_photos_and_are_reversible() {
    for roll in rolls() {
        for photo_flips in flips() {
            let original = PhotoOrientation {
                turns: QuarterTurns::One,
                flips: photo_flips,
            };
            let mut turned = original;
            turned.turn_clockwise();
            let old = fixture(roll, original);
            let expected: Vec<Vec<_>> = (0..old[0].len())
                .map(|x| old.iter().rev().map(|row| row[x]).collect())
                .collect();
            assert_eq!(fixture(roll, turned), expected);
            turned.turn_counterclockwise();
            assert_eq!(turned, original);
            for _ in 0..4 {
                turned.turn_clockwise();
            }
            assert_eq!(turned, original);
        }
    }
}
#[test]
fn roll_turn_actions_rotate_the_scan_baseline_and_are_reversible() {
    for roll in rolls() {
        let original = fixture(roll, PhotoOrientation::default());
        let expected: Vec<Vec<_>> = (0..original[0].len())
            .map(|x| original.iter().rev().map(|row| row[x]).collect())
            .collect();
        let mut turned = roll;
        turned.turn_clockwise();
        assert_eq!(fixture(turned, PhotoOrientation::default()), expected);
        turned.turn_counterclockwise();
        assert_eq!(turned, roll);
        for _ in 0..4 {
            turned.turn_clockwise();
        }
        assert_eq!(turned, roll);
        for _ in 0..4 {
            turned.turn_counterclockwise();
        }
        assert_eq!(turned, roll);
    }
}
#[test]
fn texture_mapping_uses_the_same_source_coordinates_as_export() {
    for roll in rolls() {
        for turns in [
            QuarterTurns::Zero,
            QuarterTurns::One,
            QuarterTurns::Two,
            QuarterTurns::Three,
        ] {
            let composition = Composition::new(
                AREA,
                roll,
                PhotoOrientation {
                    turns,
                    flips: Flips::default(),
                },
            )
            .unwrap();
            let size = composition.size();
            let mapping = composition.texture_mapping(
                FramePoint([0., 0.]),
                FramePoint(size.map(f64::from)),
                AREA,
            );
            for y in 0..size[1] {
                for x in 0..size[0] {
                    let uv = [
                        (x as f32 + 0.5) / size[0] as f32,
                        (y as f32 + 0.5) / size[1] as f32,
                    ];
                    let sampled: [u32; 2] = std::array::from_fn(|axis| {
                        let position = mapping.origin[axis]
                            + uv[0] * mapping.x_axis[axis]
                            + uv[1] * mapping.y_axis[axis];
                        let origin = [AREA.x, AREA.y];
                        let extent = [AREA.width, AREA.height];
                        origin[axis] + (position * extent[axis] as f32).floor() as u32
                    });
                    assert_eq!(sampled, composition.source_pixel([x, y]).unwrap());
                }
            }
        }
    }
}
#[test]
fn source_raster_matches_nearest_centres_for_sparse_and_full_pixel_grids() {
    for size in [[5, 3], [2, 1], [9, 7]] {
        let raster = SourceRaster::new(AREA, size).unwrap();
        for y in 0..size[1] {
            for x in 0..size[0] {
                let pixel = [x, y];
                let expected = [
                    AREA.x + ((2 * x + 1) * AREA.width) / (2 * size[0]),
                    AREA.y + ((2 * y + 1) * AREA.height) / (2 * size[1]),
                ];
                assert_eq!(raster.source_pixel(pixel).unwrap(), expected);
                let point = RasterPoint([x as f64 + 0.5, y as f64 + 0.5]);
                let back = raster.raster_point(raster.source_point(point));
                for axis in 0..2 {
                    assert!((back.0[axis] - point.0[axis]).abs() < 1e-12);
                }
            }
        }
    }
    let big = SourceRegion {
        x: 0,
        y: 0,
        width: u32::MAX,
        height: 1,
    };
    let raster = SourceRaster::new(big, [u32::MAX, 1]).unwrap();
    assert_eq!(
        raster.source_pixel([u32::MAX - 1, 0]).unwrap(),
        [u32::MAX - 1, 0]
    );
    assert!(raster.source_pixel([u32::MAX, 0]).is_err());
}
#[test]
fn crops_preserve_selected_content_through_every_orientation_and_texture_adapter() {
    let region = SourceRegion {
        x: 13,
        y: 8,
        width: 3,
        height: 2,
    };
    let crop = Crop::from_region(region);
    for roll in rolls() {
        for turns in [
            QuarterTurns::Zero,
            QuarterTurns::One,
            QuarterTurns::Two,
            QuarterTurns::Three,
        ] {
            for reflected in flips() {
                let photo = PhotoOrientation {
                    turns,
                    flips: reflected,
                };
                let composition = Composition::with_crop(AREA, roll, photo, Some(crop)).unwrap();
                let expected = fixture_region(region, roll, photo);
                assert_eq!(composition.area(), AREA);
                assert_eq!(composition.region(), region);
                assert_eq!(
                    composition.size(),
                    [expected[0].len() as u32, expected.len() as u32]
                );
                let mapping = composition.texture_mapping(
                    FramePoint([0., 0.]),
                    FramePoint(composition.size().map(f64::from)),
                    AREA,
                );
                for (y, row) in expected.iter().enumerate() {
                    for (x, &pixel) in row.iter().enumerate() {
                        assert_eq!(
                            composition.source_pixel([x as u32, y as u32]).unwrap(),
                            pixel
                        );
                        let uv = [
                            (x as f32 + 0.5) / composition.size()[0] as f32,
                            (y as f32 + 0.5) / composition.size()[1] as f32,
                        ];
                        let point = [
                            mapping.origin[0]
                                + uv[0] * mapping.x_axis[0]
                                + uv[1] * mapping.y_axis[0],
                            mapping.origin[1]
                                + uv[0] * mapping.x_axis[1]
                                + uv[1] * mapping.y_axis[1],
                        ];
                        assert_eq!(
                            [
                                AREA.x + (point[0] * AREA.width as f32).floor() as u32,
                                AREA.y + (point[1] * AREA.height as f32).floor() as u32
                            ],
                            pixel
                        );
                    }
                }
                assert_eq!(
                    composition.crop_from_frame(
                        FramePoint([0., 0.]),
                        FramePoint(composition.size().map(f64::from))
                    ),
                    crop
                );
                let outside = SourcePoint([-33.25, 120.5]);
                assert_eq!(
                    composition.frame_to_source(composition.source_to_frame(outside)),
                    outside
                );
            }
        }
    }
}
#[test]
fn invalid_regions_and_empty_rasters_are_rejected() {
    assert!(SourceRaster::new(AREA, [0, 1]).is_err());
    assert!(
        Composition::new(
            SourceRegion { width: 0, ..AREA },
            RollOrientation::default(),
            PhotoOrientation::default()
        )
        .is_err()
    );
    assert!(
        SourceRegion {
            x: u32::MAX,
            ..AREA
        }
        .validate()
        .is_err()
    );
    assert!(AREA.contains_region(SourceRegion {
        x: 13,
        y: 8,
        width: 2,
        height: 1
    }));
    assert!(!AREA.contains_region(SourceRegion {
        x: 13,
        y: 8,
        width: u32::MAX,
        height: 1
    }));
}
