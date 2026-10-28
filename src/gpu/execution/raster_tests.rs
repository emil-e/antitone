use super::*;
use crate::{
    film_emulation::FilmEmulationPreset,
    gpu::{ProcessingParameters, SourceGeometry},
};

#[test]
fn resident_cells_keep_native_flags_and_reject_a_same_sized_but_relabelled_source_grid() {
    let (device, _queue, renderer) = crate::gpu::test_support::render_context();
    let mut source = ProcessingSource::new_continuous(
        &device,
        &[[12000., 16000., 18000., 0.]; 4],
        glam::DMat3::IDENTITY,
    )
    .unwrap();
    source
        .set_geometry(SourceGeometry::new([10, 20, 2, 2], [2, 2]).unwrap())
        .unwrap();
    let input = ProcessingInput::from(
        ProcessingParameters::interpret(
            crate::processing::InputSettings::Digital,
            Default::default(),
            glam::DMat3::IDENTITY,
        )
        .unwrap(),
    )
    .with_film_emulation(FilmEmulationPreset::Medium.settings())
    .with_frame_size([2000; 2])
    .with_film_seed(17);
    let plan = resident_plan(&source, &input).unwrap().unwrap();
    assert_eq!(
        Raster::planned(&plan).dimensions[2],
        0,
        "resident native flags do not require the footprint-coverage bit"
    );
    let unrelated = resident_plan(
        &{
            let mut other = ProcessingSource::new_continuous(
                &device,
                &[[12000., 16000., 18000., 0.]; 4],
                glam::DMat3::IDENTITY,
            )
            .unwrap();
            other
                .set_geometry(SourceGeometry::new([11, 20, 2, 2], [2, 2]).unwrap())
                .unwrap();
            other
        },
        &input,
    )
    .unwrap()
    .unwrap();
    let output = crate::gpu::raster::buffer(&device, [2, 2], "source-layout admission").unwrap();
    let mut encoder = device.create_command_encoder(&Default::default());
    assert!(
        Request::new(&source, &input, Some(&unrelated))
            .encode(&renderer, &device, &mut encoder, &output)
            .is_err(),
        "equal byte count cannot relabel the supplied source coordinates"
    );
}
