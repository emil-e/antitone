use super::*;
use crate::{
    geometry::{Composition, FramePoint},
    grain::GrainParameters,
};

fn stages(sigma: f64, amount: f64, seed: u64) -> PixelPipeline {
    PixelPipeline {
        stages: vec![
            PixelStage::Gaussian { sigma },
            PixelStage::Grain {
                parameters: GrainParameters {
                    grain_size: 2.,
                    amplitude: amount,
                    colour: 0.5,
                },
                seed,
            },
        ],
    }
}

fn frame() -> FrameRaster {
    let composition = Composition::new(
        [100, 80, 40, 30].into(),
        Default::default(),
        Default::default(),
    )
    .unwrap();
    FrameRaster::new(
        composition,
        FramePoint([5., 4.]),
        FramePoint([17., 10.]),
        [6, 3],
    )
    .unwrap()
}

#[test]
fn neutral_layout_reuse_ignores_downstream_values_but_preserves_actual_input_and_output_mapping() {
    let available = Plan::new(Request {
        output: frame(),
        stages: &stages(2., 0.05, 17),
        sampling: Sampling::NeutralFootprints,
    })
    .unwrap();
    let recoloured = available.for_pipeline(&stages(2., 0.15, 99)).unwrap();
    assert!(available.same_acquisition_layout(&recoloured));
    let demand = available.for_pipeline(&stages(1., 0.1, 99)).unwrap();
    assert!(available.can_supply(&demand));
    assert!(!demand.can_supply(&available));
    let reused = demand.using_input(&available).unwrap();
    assert_eq!(reused.input(), available.input());
    assert_eq!(reused.source_read(), available.source_read());
    assert_eq!(reused.valid_output(), available.valid_output());
    assert_eq!(
        reused.pipeline(),
        demand.pipeline(),
        "reusing input must not restore previous photographic stage values"
    );
    assert_eq!(reused.output_offset(), Some([4, 4]));
    assert_eq!(demand.output_offset(), Some([2, 2]));
    assert!(
        demand
            .using_input(&available.for_pipeline(&stages(0.5, 0.1, 99)).unwrap())
            .is_err()
    );
}

#[test]
fn acquisition_compatibility_requires_the_requested_grid_and_sample_representation() {
    let original = Plan::new(Request {
        output: frame(),
        stages: &stages(2., 0.05, 17),
        sampling: Sampling::NeutralFootprints,
    })
    .unwrap();
    let proxy = Plan::new(Request {
        output: frame(),
        stages: original.pipeline(),
        sampling: Sampling::CameraProxy {
            region: frame().composition.area(),
            size: [10, 10],
        },
    })
    .unwrap();
    assert!(!original.can_supply(&proxy));
    assert!(!original.same_acquisition_layout(&proxy));
    let moved = FrameRaster {
        min: FramePoint([6., 4.]),
        max: FramePoint([18., 10.]),
        ..frame()
    };
    assert!(!original.can_supply(&original.for_output(moved).unwrap()));
    let reduced = FrameRaster {
        output: [3, 3],
        ..frame()
    };
    assert!(!original.can_supply(&original.for_output(reduced).unwrap()));
}

#[test]
fn full_camera_cells_use_their_real_edge_boundary_without_viewport_padding() {
    let composition = Composition::new(
        [100, 80, 40, 30].into(),
        Default::default(),
        Default::default(),
    )
    .unwrap();
    let output = FrameRaster {
        output: [10, 10],
        ..FrameRaster::full(composition).unwrap()
    };
    let sampling = Sampling::CameraCells {
        region: composition.area(),
        size: [10, 10],
    };
    let plan = Plan::new(Request {
        output,
        stages: &stages(2., 0.05, 17),
        sampling,
    })
    .unwrap();
    assert_eq!(plan.input(), ProcessingRaster::Frame(output));
    assert_eq!(plan.output_offset(), Some([0, 0]));
    assert_eq!(
        plan.source_read(),
        SourceRead::CameraProxy {
            region: [100, 80, 40, 30].into(),
            size: [10, 10]
        }
    );
    assert!(plan.same_acquisition_layout(&plan.for_pipeline(&stages(3., 0.1, 99)).unwrap()));
    assert!(
        Plan::new(Request {
            output: frame(),
            stages: plan.pipeline(),
            sampling
        })
        .is_err(),
        "a cropped or resampled raster must not silently extend its own edges as source edges"
    );
    assert!(
        FootprintBatch::new(vec![std::sync::Arc::new(plan)], Default::default()).is_err(),
        "a sampled camera raster is not an original-source provider"
    );
}
