use super::*;
use crate::{
    geometry::{Composition, FramePoint},
    grain::GrainParameters,
};

fn stages(sigma: f64, amount: f64, seed: u64) -> Assembly {
    {
        let mtf = crate::image_mtf::ImageMtfParameters { sigma };
        let grain = GrainParameters {
            grain_size: 2.,
            amplitude: amount,
            colour: 0.5,
        };
        let pitch = [Some(mtf.sigma / 0.6)]
            .into_iter()
            .flatten()
            .reduce(f64::min);
        crate::gpu::photograph::assembly::Assembly::authored(move |mut image, output| {
            image = mtf.compose(image)?;
            if let Some(output) = output {
                image = output.compose(image)?;
            }
            image = grain.compose(image, seed);
            Ok(image)
        })
        .with_view_pitch(pitch)
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
    let available = Plan::for_graph(
        Request {
            output: frame(),

            sampling: Sampling::NeutralFootprints,
        },
        |input| stages(2., 0.05, 17).sampling_image(input),
    )
    .unwrap();
    let recoloured = available.for_definition(&stages(2., 0.15, 99)).unwrap();
    assert!(available.same_acquisition_layout(&recoloured));
    let demand = available.for_definition(&stages(1., 0.1, 99)).unwrap();
    assert!(available.can_supply(&demand));
    assert!(!demand.can_supply(&available));
    let reused = demand.using_input(&available).unwrap();
    assert_eq!(reused.input(), available.input());
    assert_eq!(reused.source_read(), available.source_read());
    assert_eq!(reused.valid_output(), available.valid_output());
    assert_eq!(reused.output_offset(), Some([4, 4]));
    assert_eq!(demand.output_offset(), Some([2, 2]));
    assert!(
        demand
            .using_input(&available.for_definition(&stages(0.5, 0.1, 99)).unwrap())
            .is_err()
    );
}

#[test]
fn acquisition_compatibility_requires_the_requested_grid_and_sample_representation() {
    let original = Plan::for_graph(
        Request {
            output: frame(),

            sampling: Sampling::NeutralFootprints,
        },
        |input| stages(2., 0.05, 17).sampling_image(input),
    )
    .unwrap();
    let proxy = Plan::for_graph(
        Request {
            output: frame(),

            sampling: Sampling::CameraProxy {
                region: frame().composition.area(),
                size: [10, 10],
            },
        },
        |input| stages(2., 0.05, 17).sampling_image(input),
    )
    .unwrap();
    assert!(!original.can_supply(&proxy));
    assert!(!original.same_acquisition_layout(&proxy));
    let moved = FrameRaster {
        min: FramePoint([6., 4.]),
        max: FramePoint([18., 10.]),
        ..frame()
    };
    assert!(
        !original.can_supply(
            &Plan::for_graph(
                Request {
                    output: moved,
                    sampling: original.sampling()
                },
                |input| stages(2., 0.05, 17).sampling_image(input)
            )
            .unwrap()
        )
    );
    let reduced = FrameRaster {
        output: [3, 3],
        ..frame()
    };
    assert!(
        !original.can_supply(
            &Plan::for_graph(
                Request {
                    output: reduced,
                    sampling: original.sampling()
                },
                |input| stages(2., 0.05, 17).sampling_image(input)
            )
            .unwrap()
        )
    );
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
    let plan = Plan::for_graph(Request { output, sampling }, |input| {
        stages(2., 0.05, 17).sampling_image(input)
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
    assert!(plan.same_acquisition_layout(&plan.for_definition(&stages(3., 0.1, 99)).unwrap()));
    assert!(
        Plan::for_graph(
            Request {
                output: frame(),

                sampling
            },
            |input| stages(2., 0.05, 17).sampling_image(input)
        )
        .is_err(),
        "a cropped or resampled raster must not silently extend its own edges as source edges"
    );
}
