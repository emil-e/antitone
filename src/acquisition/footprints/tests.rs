use super::*;
use crate::geometry::{Composition, FramePoint, SourcePoint};

fn plan() -> Plan {
    let composition = Composition::new(
        [100, 80, 90, 60].into(),
        Default::default(),
        Default::default(),
    )
    .unwrap();
    let frame = FrameRaster {
        composition,
        min: FramePoint([-4., -4.]),
        max: FramePoint([8., 8.]),
        output: [12, 12],
    };
    Plan::new(Request {
        output: frame,
        stages: &PixelPipeline::default(),
        sampling: Sampling::NeutralFootprints,
    })
    .unwrap()
}

#[test]
fn damage_windows_preserve_the_acquired_grid_and_do_not_repeat_duplicate_support() {
    let padded = plan();
    let frame = padded.processing_frame().unwrap();
    let changed: SourceRegion = [100, 80, 1, 1].into();
    let windows = padded
        .affected_windows([changed, changed], Default::default())
        .unwrap();
    assert_eq!(windows.len(), 1);
    let region = padded.window_frame(([1, 2], [3, 4])).unwrap();
    assert_eq!(region.source_point([0.5, 1.5]), SourcePoint([97.5, 79.5]));
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
fn empty_window_tiles_reject_instead_of_stalling_traversal() {
    for destination_tile in [[0, 8], [3, 0]] {
        assert!(
            plan()
                .affected_windows([[100, 80, 1, 1].into()], WindowBudget { destination_tile })
                .is_err()
        );
    }
}
