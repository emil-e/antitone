use super::*;
use crate::gpu::evaluation::CellRegion;

#[test]
fn repaired_previews_keep_the_neutral_dependency_when_pans_exclude_other_repairs() {
    // Constant CFA planes make constant-preserving healing, filtering and grading
    // an independent pan invariant; the regional identities still include real repairs.
    let authored = crate::dng_fixture::Fixture {
        side: 768,
        ..Default::default()
    }
    .write();
    for (capacity, pressure) in [
        (0, memory::Pressure::Warning),
        (128 << 20, memory::Pressure::Normal),
    ] {
        let mut editor =
            crate::editor::tests::authored_fixture(&[authored.path().to_str().unwrap()]);
        editor.set_dmin(Some(DVec3::splat(0.8)));
        editor.set_dmax(Some(DVec3::splat(0.2)));
        let id = editor.document.visit.current_source.unwrap();
        for x in [90, 500] {
            editor
                .document
                .state
                .project
                .source_mut(id)
                .unwrap()
                .repairs
                .insert(
                    crate::dust_repair::SupportMask::new([x, 70], [2, 2], vec![true; 4]).unwrap(),
                    crate::dust_repair::RepairOrigin::Manual,
                    [32, 32],
                )
                .unwrap();
        }
        let recipe = Arc::new(
            editor
                .view_recipe(
                    editor.input(),
                    editor.document.project_snapshot().roll,
                    editor.photo_settings(),
                )
                .unwrap(),
        );
        assert_eq!(recipe.repairs.len(), 2);
        let mut full = FrameRaster::full(recipe.composition).unwrap();
        full.output = [384, 256];
        let pane = |x| {
            FrameRaster::new(
                recipe.composition,
                FramePoint([x, 16.5]),
                FramePoint([x + 192., 144.5]),
                [96, 64],
            )
            .unwrap()
        };
        let left = pane(20.25);
        let right = pane(420.25);
        let services = editor.source_services.as_ref().unwrap();
        let device = services.device.clone();
        let queue = services.queue.clone();
        services.renderers.set_retention_budget(capacity);
        let renderer = services.renderers.get(&device, &queue).unwrap();
        renderer
            .evaluation()
            .materializations()
            .set_test_pressure(pressure);
        editor.document.visit.camera.pressure(pressure);
        let pixels = editor.source_context().unwrap();
        let scan = pixels.scan().unwrap();
        let (graph, _) = crate::gpu::photograph::PhotographGraph::preview(
            &device.limits(),
            &scan,
            None,
            &recipe,
            left,
            None,
        )
        .unwrap();
        assert_ne!(
            graph.neutral.identity(),
            graph
                .neutral
                .request([0, 0, 96, 64].into())
                .unwrap()
                .identity(),
            "this viewport must actually omit the other repair from its content description"
        );
        let mut constant: Option<[f32; 4]> = None;
        with_application(editor, |app| {
            for frame in [full, left, right, left, full] {
                let demand = Demand {
                    spec: SourceRequest::for_recipe(frame, &recipe)
                        .unwrap()
                        .with_identity(crate::gpu::source::identity::Identity::camera(
                            pixels.source.identity(),
                        )),
                    pixels: pixels.clone(),
                    capture: PhotographRequest::Recipe(recipe.clone()),
                };
                app.enter(|editor, cx| editor.request_working(demand.clone(), cx));
                app.run_until(Duration::from_secs(15), |editor| {
                    let view = &editor.document.visit.working_view;
                    assert!(
                        view.error.is_none(),
                        "repaired viewport acquisition failed: {:?}",
                        view.error
                    );
                    view.support_ready()
                });
                let acquired = app.read(|editor| {
                    editor
                        .document
                        .visit
                        .working_view
                        .frame
                        .as_ref()
                        .unwrap()
                        .clone()
                });
                assert_eq!(acquired.request.spec.frame(), frame);
                assert!(acquired.request.same_calculation(&demand));
                let produced = &acquired.processed[0];
                assert_eq!(
                    produced.definition.domain().finite().unwrap(),
                    CellRegion::new([0, 0], frame.output)
                );
                let actual = crate::gpu::evaluation::read_buffer(&device, &queue, &produced.pixels);
                assert_eq!(
                    actual.len(),
                    frame.output[0] as usize * frame.output[1] as usize
                );
                if let Some(expected) = constant {
                    // Full-source edge demosaicing is outside this interior-pan invariant.
                    if frame != full {
                        for pixel in actual {
                            for channel in 0..3 {
                                assert!(
                                    (pixel[channel] - expected[channel]).abs() < 2e-5,
                                    "constant interior changed after pan: {pixel:?} vs {expected:?}"
                                );
                            }
                        }
                    }
                } else {
                    let expected = actual[actual.len() / 2 + frame.output[0] as usize / 2];
                    assert!(expected[..3].iter().all(|v| v.is_finite()));
                    assert!(expected[..3].iter().any(|v| v.abs() > 1e-3));
                    constant = Some(expected);
                }
            }
        });
    }
}
