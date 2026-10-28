use super::*;
use crate::{
    crop::Crop,
    geometry::{Composition, FramePoint},
    grain::GrainParameters,
};

fn pipeline() -> PixelPipeline {
    PixelPipeline {
        stages: vec![
            PixelStage::Gaussian { sigma: 2. },
            PixelStage::Grain {
                parameters: GrainParameters {
                    grain_size: 2.,
                    amplitude: 0.055,
                    colour: 0.5,
                },
                seed: 17,
            },
            PixelStage::Gaussian { sigma: 7.5 },
        ],
    }
}

fn composition() -> Composition {
    Composition::new(
        [100, 80, 6000, 4000].into(),
        Default::default(),
        Default::default(),
    )
    .unwrap()
}

fn output() -> FrameRaster {
    // Frame coordinates exclude the nonzero source active-area origin.
    FrameRaster::new(
        composition(),
        FramePoint([900., 720.]),
        FramePoint([1412., 1232.]),
        [512; 2],
    )
    .unwrap()
}

#[test]
fn unequal_neighbourhoods_propagate_backwards_through_a_point_stage() {
    let plan = Plan::new(Request {
        output: output(),
        stages: &pipeline(),
        sampling: Sampling::Originals {
            reconstruction: SourceSupport([0; 2]),
        },
    })
    .unwrap();
    assert_eq!(
        plan.source_read(),
        SourceRead::Originals([962, 762, 588, 588].into())
    );
    let stages = plan.stages();
    assert_eq!(
        stages[2].output,
        ProcessingRaster::Originals([1000, 800, 512, 512].into())
    );
    assert_eq!(
        stages[2].input,
        ProcessingRaster::Originals([970, 770, 572, 572].into())
    );
    assert_eq!(stages[1].input, stages[1].output);
    assert_eq!(stages[1].support, RasterSupport([0; 2]));
    assert_eq!(stages[0].output, stages[1].input);
    assert_eq!(
        stages[0].input,
        ProcessingRaster::Originals([962, 762, 588, 588].into())
    );
    assert_eq!(
        plan.valid_output(),
        ValidOutput::Reconstruction {
            affine: [[38., 38.], [1., 0.], [0., 1.]],
        }
    );
    assert_eq!(
        plan.output_offset(),
        None,
        "originals reconstruct rather than slice a frame raster"
    );
}

#[test]
fn reconstruction_guard_acquires_real_cells_without_expanding_photographic_repair_influence() {
    let plan = Plan::new(Request {
        output: output(),
        stages: &pipeline(),
        sampling: Sampling::Originals {
            reconstruction: SourceSupport([1; 2]),
        },
    })
    .unwrap();
    assert_eq!(
        plan.source_read(),
        SourceRead::Originals([961, 761, 590, 590].into())
    );
    assert!(plan.original_repair_contributes([962, 800]).unwrap());
    assert!(!plan.original_repair_contributes([961, 800]).unwrap());
    assert!(!plan.original_repair_contributes([99, 800]).unwrap());
}

#[test]
fn filtered_rasters_keep_pitch_source_coordinates_and_valid_output_at_each_scale() {
    for (size, margin, input_size) in [([256; 2], 19, [294; 2]), ([1024; 2], 76, [1176; 2])] {
        let frame = FrameRaster {
            output: size,
            ..output()
        };
        let plan = Plan::new(Request {
            output: frame,
            stages: &pipeline(),
            sampling: Sampling::NeutralFootprints,
        })
        .unwrap();
        assert_eq!(plan.output_offset(), Some([margin; 2]));
        let ProcessingRaster::Frame(input) = plan.input() else {
            panic!("filtered raster")
        };
        assert_eq!(input.output, input_size);
        assert_eq!(plan.input().pitch(), ProcessingRaster::Frame(frame).pitch());
        for point in [[0.5, 0.5], [20.5, 13.5]] {
            assert_eq!(
                frame.source_point(point),
                input.source_point(point.map(|v| v + f64::from(margin)))
            );
        }
        assert_eq!(
            plan.source_read(),
            SourceRead::Originals([962, 762, 588, 588].into())
        );
    }
}

#[test]
fn only_actual_source_edges_clip_neighbourhoods_and_exterior_reads_use_nearest_cells() {
    let composition = Composition::new(
        [100, 80, 64, 48].into(),
        Default::default(),
        Default::default(),
    )
    .unwrap();
    let empty = PixelPipeline::default();
    for (min, max, expected) in [
        ([-8., -9.], [4., 5.], [100, 80, 4, 5]),
        ([-8., -9.], [-3., -4.], [100, 80, 1, 1]),
        ([67., 52.], [72., 57.], [163, 127, 1, 1]),
    ] {
        // Internal processing windows may lie outside accepted composition.
        let frame = FrameRaster {
            composition,
            min: FramePoint(min),
            max: FramePoint(max),
            output: [5; 2],
        };
        for sampling in [
            Sampling::NeutralFootprints,
            Sampling::Originals {
                reconstruction: SourceSupport([0; 2]),
            },
        ] {
            let plan = Plan::new(Request {
                output: frame,
                stages: &empty,
                sampling,
            })
            .unwrap();
            assert_eq!(plan.source_read(), SourceRead::Originals(expected.into()));
            if matches!(sampling, Sampling::Originals { .. }) {
                assert!(
                    plan.original_repair_contributes([expected[0], expected[1]])
                        .unwrap(),
                    "the nearest real cell also supplies exterior output"
                );
                assert!(!plan.original_repair_contributes([130, 100]).unwrap());
            }
        }
    }
    let plan = Plan::new(Request {
        output: FrameRaster::full(composition).unwrap(),
        stages: &pipeline(),
        sampling: Sampling::Originals {
            reconstruction: SourceSupport([1; 2]),
        },
    })
    .unwrap();
    assert_eq!(
        plan.source_read(),
        SourceRead::Originals([100, 80, 64, 48].into())
    );
}

#[test]
fn rotated_dependencies_preserve_footprints_and_do_not_select_bounding_box_repairs() {
    let composition = Composition::with_crop(
        [100, 80, 400, 300].into(),
        Default::default(),
        Default::default(),
        Some(Crop {
            center: [300., 230.],
            size: [40, 20],
            angle: std::f64::consts::FRAC_PI_4,
        }),
    )
    .unwrap();
    let frame = FrameRaster {
        output: [20, 10],
        ..FrameRaster::full(composition).unwrap()
    };
    let plan = Plan::new(Request {
        output: frame,
        stages: &pipeline(),
        sampling: Sampling::NeutralFootprints,
    })
    .unwrap();
    assert_eq!(plan.output_offset(), Some([19; 2]));
    let ProcessingRaster::Frame(input) = plan.input() else {
        panic!("frame")
    };
    for p in [[0.5, 0.5], [19.5, 9.5]] {
        let expected = frame.source_point(p).0;
        let actual = input.source_point(p.map(|v| v + 19.)).0;
        for i in 0..2 {
            assert!((expected[i] - actual[i]).abs() < 1e-10);
        }
    }
    let native = Plan::new(Request {
        output: frame,
        stages: &PixelPipeline::default(),
        sampling: Sampling::Originals {
            reconstruction: SourceSupport([1; 2]),
        },
    })
    .unwrap();
    assert!(native.original_repair_contributes([300, 230]).unwrap());
    assert!(
        !native.original_repair_contributes([279, 209]).unwrap(),
        "bounding rectangle corner has no composition coverage"
    );
}

#[test]
fn proxy_read_declares_its_representation_separately_from_original_reads_and_publication() {
    let pipeline = pipeline();
    let frame = output();
    let sampling = Sampling::CameraProxy {
        region: frame.composition.area(),
        size: [600, 400],
    };
    let plan = Plan::new(Request {
        output: frame,
        stages: &pipeline,
        sampling,
    })
    .unwrap();
    assert_eq!(
        plan.source_read(),
        SourceRead::CameraProxy {
            region: [100, 80, 6000, 4000].into(),
            size: [600, 400]
        }
    );
    assert_eq!(plan.output_offset(), Some([38; 2]));
    assert_eq!(plan.pipeline(), &pipeline);
    assert!(plan.original_repair_contributes([1000, 800]).is_err());
    assert!(
        Plan::new(Request {
            output: frame,
            stages: &pipeline,
            sampling: Sampling::CameraProxy {
                region: [0, 0, 6000, 4000].into(),
                size: [600, 400]
            }
        })
        .is_err()
    );
}
