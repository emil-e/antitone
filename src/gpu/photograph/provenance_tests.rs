use super::*;
use crate::{
    dust_repair::{Repair, RepairId, RepairOrigin, SupportMask},
    gpu::test_inputs::AuthoredPhotograph,
};

#[test]
fn branched_neutral_graphs_keep_correction_and_charge_each_resident_buffer_once() {
    let (device, _, _) = crate::gpu::test_support::render_context();
    let area: crate::geometry::SourceRegion = [0, 0, 64, 64].into();
    let values = vec![[20000., 25000., 30000., 0.]; 64 * 64];
    let source = || {
        ProcessingSource::new_continuous(
            &device,
            &values,
            glam::DMat3::IDENTITY,
            Some(SourceGeometry::original_tile([0, 0, 64, 64], [64; 2]).unwrap()),
        )
        .unwrap()
    };
    let camera = source();
    let field = Arc::new(
        GpuFlatField::new(
            &device,
            Arc::new(
                crate::flat_field::FlatField::from_nodes(
                    [0, 0, 64, 64],
                    [2, 2],
                    vec![glam::DVec3::splat(1.2); 4],
                )
                .unwrap(),
            ),
        )
        .unwrap(),
    );
    let mut input = AuthoredPhotograph::new(
        &device.limits(),
        crate::processing::InputSettings::Digital,
        Default::default(),
        crate::adjustment_layers::AdjustmentLayerSnapshot::new(Vec::new()).unwrap(),
        glam::DMat3::IDENTITY,
        area,
    )
    .unwrap()
    .with_flat_field(Some(field.clone()));
    input.repairs = vec![Repair {
        id: RepairId(1),
        enabled: true,
        origin: RepairOrigin::Manual,
        support: SupportMask::new([20, 20], [3, 3], vec![true; 9]).unwrap(),
        offset: [16, 0],
    }];
    let graph = |camera: &ProcessingSource| {
        PhotographGraph::from_resident_with_stages(
            &device.limits(),
            camera,
            &input,
            crate::gpu::raster::Raster::region(area, [64; 2]),
            &Default::default(),
        )
        .unwrap()
    };
    let first = graph(&camera);
    assert_eq!(first.neutral.inputs().len(), 2);
    assert!(PhotographGraph::has_correction(
        &first.neutral,
        Some(&field)
    ));
    assert!(!PhotographGraph::has_correction(&first.neutral, None));
    assert_eq!(
        PhotographGraph::input_transform(&first.neutral)
            .unwrap()
            .as_ref(),
        input.source_transform()
    );
    let alias = graph(&camera);
    let support = SupportMask::new([30, 30], [2, 2], vec![true; 4]).unwrap();
    let shared = crate::gpu::repairs::definition::compose_images(
        first.neutral.clone(),
        vec![(support.clone(), alias.unrepaired)],
    )
    .unwrap();
    assert!(
        shared
            .ancestors()
            .filter(|node| (node.operation().as_ref() as &dyn std::any::Any).is::<ResidentCamera>())
            .count()
            > 1,
        "the charge control must include distinct nodes over one allocation"
    );
    assert_eq!(
        PhotographGraph::resident_camera_bytes(&shared),
        camera.input.size()
    );
    let second = source();
    let mixed = crate::gpu::repairs::definition::compose_images(
        first.neutral.clone(),
        vec![(support, graph(&second).unrepaired)],
    )
    .unwrap();
    assert_eq!(
        PhotographGraph::resident_camera_bytes(&mixed),
        camera.input.size() + second.input.size()
    );
    let arguments = NeutralInput {
        interpretation: crate::gpu::interpretation::SourceInterpretation::Photograph(
            *input.source_transform().unwrap(),
        ),
        field: Some(field.clone()),
        repairs: input.repairs.clone(),
        replacements: Vec::new(),
    };
    let rebound = arguments.recompose_resident(&shared).unwrap();
    assert_eq!(rebound.domain(), shared.domain());
    assert_eq!(
        PhotographGraph::input_transform(&rebound).unwrap().as_ref(),
        input.source_transform()
    );
    assert_eq!(
        PhotographGraph::resident_camera_bytes(&rebound),
        camera.input.size()
    );
    assert!(matches!(
        arguments.recompose_resident(&mixed),
        Err(Error::InvalidParameter(
            "calculation mixes resident camera arguments"
        ))
    ));

    let other_field = Arc::new(
        GpuFlatField::new(
            &device,
            Arc::new(
                crate::flat_field::FlatField::from_nodes(
                    [0, 0, 64, 64],
                    [2, 2],
                    vec![glam::DVec3::splat(1.3); 4],
                )
                .unwrap(),
            ),
        )
        .unwrap(),
    );
    let different = input.clone().with_flat_field(Some(other_field.clone()));
    let other = PhotographGraph::from_resident_with_stages(
        &device.limits(),
        &camera,
        &different,
        crate::gpu::raster::Raster::region(area, [64; 2]),
        &Default::default(),
    )
    .unwrap();
    let conflicting = crate::gpu::repairs::definition::compose_images(
        shared,
        vec![(
            SupportMask::new([30, 30], [2, 2], vec![true; 4]).unwrap(),
            other.unrepaired,
        )],
    )
    .unwrap();
    assert!(
        !PhotographGraph::has_correction(&conflicting, Some(&field)),
        "a matching first camera cannot hide a differently corrected branch"
    );
    assert!(!PhotographGraph::has_correction(
        &conflicting,
        Some(&other_field)
    ));
}
