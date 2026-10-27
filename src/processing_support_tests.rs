use super::*;
use crate::{
    crop::Crop,
    geometry::Composition,
    grain::GrainParameters,
    pixel_pipeline::{PixelPipeline, PixelStage},
};

#[test]
fn ordered_support_adds_neighbourhoods_and_point_stages_add_none() {
    let stages = PixelPipeline {
        stages: vec![
            PixelStage::Gaussian { sigma: 1.5 },
            PixelStage::Grain {
                parameters: GrainParameters {
                    grain_size: 2.,
                    amplitude: 0.055,
                    colour: 0.5,
                },
                seed: 17,
            },
            PixelStage::Gaussian { sigma: 2.25 },
        ],
    };
    assert_eq!(stages.support([1., 1.]).unwrap(), Support([15, 15]));
    assert_eq!(stages.support([1.5, 3.]).unwrap(), Support([10, 5]));
}

#[test]
fn processing_support_extends_a_crop_without_reframing_or_changing_sampling_pitch() {
    let area = [100, 80, 6000, 6000].into();
    let composition = Composition::with_crop(
        area,
        Default::default(),
        Default::default(),
        Some(Crop {
            center: [1040., 1032.],
            size: [64, 64],
            angle: 0.17,
        }),
    )
    .unwrap();
    let output = FrameRaster::full(composition).unwrap();
    let input = Support([8, 9]).raster(output).unwrap();
    assert_eq!(output.composition, input.composition);
    assert_eq!(output.output, [64, 64]);
    assert_eq!(input.output, [80, 82]);
    for p in [[0.5, 0.5], [32.5, 17.5], [63.5, 63.5]] {
        let a = output.source_point(p).0;
        let b = input.source_point([p[0] + 8., p[1] + 9.]).0;
        assert!((a[0] - b[0]).abs() < 1e-10 && (a[1] - b[1]).abs() < 1e-10);
    }
    let out = output.bounds().unwrap();
    let required = input.bounds().unwrap();
    assert!(required.x < out.x && required.y < out.y);
}

#[test]
fn support_clips_only_at_real_source_edges() {
    let source: SourceRegion = [100, 80, 6000, 6000].into();
    assert_eq!(
        Support([8, 9]).expand([100, 80, 64, 64].into(), source),
        [100, 80, 72, 73].into()
    );
    assert_eq!(
        Support([8, 9]).expand([1008, 1000, 64, 64].into(), source),
        [1000, 991, 80, 82].into()
    );
}

#[test]
fn wholly_exterior_processing_windows_acquire_the_nearest_real_edge_cell() {
    let composition = Composition::new(
        [100, 80, 64, 64].into(),
        Default::default(),
        Default::default(),
    )
    .unwrap();
    for (min, max, expected) in [
        ([-8., -9.], [-3., -4.], [100, 80, 1, 1]),
        ([67., 68.], [72., 73.], [163, 143, 1, 1]),
    ] {
        let window = FrameRaster {
            composition,
            min: FramePoint(min),
            max: FramePoint(max),
            output: [5, 5],
        };
        assert_eq!(window.processing_bounds().unwrap(), expected.into());
        assert!(
            window.bounds().is_err(),
            "accepted composition still rejects exterior coverage"
        );
    }
}
