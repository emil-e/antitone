use super::*;
use crate::{
    crop::Crop,
    geometry::{Composition, FramePoint},
    pixel_pipeline::PixelStage,
};

fn plan(frame: FrameRaster, sigma: f64) -> Arc<Plan> {
    Arc::new(
        Plan::new(Request {
            output: frame,
            stages: &PixelPipeline {
                stages: vec![PixelStage::Gaussian { sigma }],
            },
            sampling: Sampling::NeutralFootprints,
        })
        .unwrap(),
    )
}

fn contains(region: SourceRegion, point: [u32; 2]) -> bool {
    point[0] >= region.x
        && point[0] < region.x + region.width
        && point[1] >= region.y
        && point[1] < region.y + region.height
}

fn verify_coverage(plans: &[Arc<Plan>], batch: &FootprintBatch, exterior: bool) {
    let units: Vec<_> = batch.units().collect::<Result<_, _>>().unwrap();
    for unit in &units {
        assert!(
            unit.read.width <= batch.budget.source_tile[0]
                && unit.read.height <= batch.budget.source_tile[1]
        );
        for submission in &unit.submissions {
            assert!(!submission.is_empty());
            assert!(
                submission
                    .iter()
                    .map(|work| u64::from(work.window.1[0]) * u64::from(work.window.1[1]))
                    .sum::<u64>()
                    <= batch.budget.destination_pixels
            );
        }
    }
    for (destination, plan) in plans.iter().enumerate() {
        let frame = plan.processing_frame().unwrap();
        let area = frame.composition.area();
        for y in 0..frame.output[1] {
            for x in 0..frame.output[0] {
                let needed: Vec<[u32; 2]> = if exterior {
                    // Axis-aligned forward footprints enumerate original cells, clamping
                    // exterior positions to the real edge independently of inverse windows.
                    let low = frame.source_point([f64::from(x), f64::from(y)]).0;
                    let high = frame.source_point([f64::from(x + 1), f64::from(y + 1)]).0;
                    (low[1].floor() as i64..high[1].ceil() as i64)
                        .flat_map(|sy| {
                            (low[0].floor() as i64..high[0].ceil() as i64).map(move |sx| {
                                [
                                    sx.clamp(i64::from(area.x), i64::from(area.x + area.width - 1))
                                        as u32,
                                    sy.clamp(i64::from(area.y), i64::from(area.y + area.height - 1))
                                        as u32,
                                ]
                            })
                        })
                        .collect()
                } else {
                    frame
                        .footprint([x, y])
                        .unwrap()
                        .pixels(area)
                        .unwrap()
                        .map(|(point, _)| point)
                        .collect()
                };
                assert!(!needed.is_empty());
                for point in needed {
                    let unit = units
                        .iter()
                        .find(|unit| contains(unit.read, point))
                        .expect("every required real cell is read");
                    assert!(
                        unit.submissions.iter().flatten().any(|work| {
                            let (origin, size) = work.window;
                            work.destination == destination
                                && x >= origin[0]
                                && x < origin[0] + size[0]
                                && y >= origin[1]
                                && y < origin[1] + size[1]
                        }),
                        "source {point:?} must contribute to destination {destination} pixel ({x},{y})"
                    );
                }
            }
        }
    }
}

#[test]
fn paired_footprints_share_original_reads_and_bound_magnified_destination_work() {
    let composition = Composition::new(
        [100, 80, 24, 18].into(),
        Default::default(),
        Default::default(),
    )
    .unwrap();
    let detail = FrameRaster::new(
        composition,
        FramePoint([5., 4.]),
        FramePoint([13., 8.]),
        [16, 8],
    )
    .unwrap();
    let underlay = FrameRaster {
        output: [6, 6],
        ..FrameRaster::full(composition).unwrap()
    };
    let plans = vec![plan(detail, 0.5), plan(underlay, 0.5)];
    let batch = FootprintBatch::new(
        plans.clone(),
        FootprintBudget {
            source_tile: [7, 5],
            destination_tile: [3, 2],
            destination_pixels: 6,
        },
    )
    .unwrap();
    let units: Vec<_> = batch.units().collect::<Result<_, _>>().unwrap();
    assert_eq!(units.first().unwrap().read, [100, 80, 7, 5].into());
    assert_eq!(units.last().unwrap().read, [121, 95, 3, 3].into());
    assert_eq!(
        units
            .iter()
            .map(|unit| u64::from(unit.read.width) * u64::from(unit.read.height))
            .sum::<u64>(),
        432,
        "paired outputs traverse each original cell once"
    );
    assert!(
        units.iter().any(|unit| unit.submissions.len() > 1),
        "magnification must exercise bounded destination submissions"
    );
    verify_coverage(&plans, &batch, true);
}

#[test]
fn rotated_forward_coverage_is_not_lost_by_inverse_source_windows() {
    let composition = Composition::with_crop(
        [100, 80, 400, 300].into(),
        Default::default(),
        Default::default(),
        Some(Crop {
            center: [300., 230.],
            size: [64, 48],
            angle: 0.31,
        }),
    )
    .unwrap();
    let frame = FrameRaster::new(
        composition,
        FramePoint([15., 14.]),
        FramePoint([25., 20.]),
        [4, 3],
    )
    .unwrap();
    let plans = vec![plan(frame, 3.)];
    let batch = FootprintBatch::new(
        plans.clone(),
        FootprintBudget {
            source_tile: [7, 5],
            destination_tile: [3, 2],
            destination_pixels: 6,
        },
    )
    .unwrap();
    verify_coverage(&plans, &batch, false);
}

#[test]
fn wholly_exterior_footprints_still_receive_the_real_edge_cell() {
    let composition = Composition::new(
        [100, 80, 24, 18].into(),
        Default::default(),
        Default::default(),
    )
    .unwrap();
    let frame = FrameRaster {
        composition,
        min: FramePoint([-8., -9.]),
        max: FramePoint([-4., -3.]),
        output: [4, 6],
    };
    let plans = vec![plan(frame, 0.)];
    let batch = FootprintBatch::new(
        plans.clone(),
        FootprintBudget {
            source_tile: [7, 5],
            destination_tile: [3, 2],
            destination_pixels: 6,
        },
    )
    .unwrap();
    assert_eq!(batch.bounds, [100, 80, 1, 1].into());
    verify_coverage(&plans, &batch, true);
}

#[test]
fn disjoint_outputs_leave_empty_source_units_and_damage_windows_preserve_the_acquired_grid() {
    let composition = Composition::new(
        [100, 80, 90, 60].into(),
        Default::default(),
        Default::default(),
    )
    .unwrap();
    let a = FrameRaster::new(
        composition,
        FramePoint([0., 0.]),
        FramePoint([4., 4.]),
        [4, 4],
    )
    .unwrap();
    let b = FrameRaster::new(
        composition,
        FramePoint([86., 56.]),
        FramePoint([90., 60.]),
        [4, 4],
    )
    .unwrap();
    let plans = vec![plan(a, 0.), plan(b, 0.)];
    let batch = FootprintBatch::new(
        plans.clone(),
        FootprintBudget {
            source_tile: [8, 8],
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        batch
            .units()
            .any(|unit| unit.unwrap().submissions.is_empty()),
        "gaps must not require source upload/filter submissions"
    );
    verify_coverage(&plans, &batch, true);
    let padded = plan(a, 1.);
    let frame = padded.processing_frame().unwrap();
    let changed: SourceRegion = [100, 80, 1, 1].into();
    let windows = padded
        .affected_windows([changed, changed], Default::default())
        .unwrap();
    assert_eq!(
        windows.len(),
        1,
        "duplicate damage must not repeat a reconstruction window"
    );
    let region = padded.window_frame(([1, 2], [3, 4])).unwrap();
    assert_eq!(
        region.source_point([0.5, 1.5]),
        frame.source_point([1.5, 3.5])
    );
    assert!(
        padded
            .affected_windows([[180, 130, 1, 1].into()], Default::default())
            .unwrap()
            .is_empty()
    );
    assert!(padded.window_frame(([0, 0], [0, 1])).is_err());
}

#[test]
fn invalid_window_budgets_cannot_stall_submission_packing() {
    let composition = Composition::new(
        [0, 0, 16, 16].into(),
        Default::default(),
        Default::default(),
    )
    .unwrap();
    let plans = vec![plan(FrameRaster::full(composition).unwrap(), 1.)];
    for budget in [
        FootprintBudget {
            source_tile: [0, 8],
            ..Default::default()
        },
        FootprintBudget {
            destination_tile: [3, 0],
            ..Default::default()
        },
        FootprintBudget {
            destination_tile: [3, 2],
            destination_pixels: 5,
            ..Default::default()
        },
    ] {
        assert!(FootprintBatch::new(plans.clone(), budget).is_err());
    }
}
