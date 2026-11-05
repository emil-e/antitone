use super::*;
use wgpu::util::DeviceExt;

fn rejected_sample(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    look: &crate::looks::LookAsset,
) -> Arc<crate::gpu::lut3d::Check> {
    let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("nonfinite Look input fixture"),
        contents: bytemuck::cast_slice(&[[f32::NAN, 0.18, 0.18, 8.]]),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 16,
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    });
    let kernel = pollster::block_on(crate::gpu::lut3d::Kernel::new(device)).unwrap();
    let dispatch = kernel
        .bind(
            device,
            &input,
            &output,
            crate::gpu::lut3d::Lut::new(device, look.lut()).unwrap(),
            1,
            false,
        )
        .unwrap();
    let mut encoder = device.create_command_encoder(&Default::default());
    kernel.encode(&mut encoder, &dispatch);
    let submission = queue.submit([encoder.finish()]);
    dispatch.check.begin(|| {});
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(Duration::from_secs(30)),
        })
        .unwrap();
    assert!(
        dispatch
            .check
            .result()
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("nonfinite")
    );
    dispatch.check
}

fn reject_displayed(
    h: &mut Harness<'_, Editor>,
    check: &Arc<crate::gpu::lut3d::Check>,
    id: crate::looks::LookId,
) {
    paint_until(h, |editor| {
        editor
            .canvas_frame
            .displayed
            .as_ref()
            .is_some_and(|painted| painted.view.look_id() == Some(id) && !painted.view.in_flight())
    });
    h.read(|editor| {
        editor
            .canvas_frame
            .displayed
            .as_ref()
            .unwrap()
            .view
            .image
            .set_look_check(Some(check.clone()))
    });
    h.step();
    h.read(|editor| assert!(editor.look_rendering_error().is_some()));
    assert!(h.query_by_label("Look rendering failed").is_some());
}

#[test]
fn look_errors_follow_the_painted_request_not_the_retained_photograph_or_active_tool() {
    fixture_editor_gpu(|mut h, _, device, queue| {
        let asset = constant_asset(0.4);
        // Supply a real GPU rejection at the image's readout boundary. This tests error
        // ownership, not admission of corrupt camera samples through the application.
        let rejection = rejected_sample(&device, &queue, &asset);
        let label = asset.name().to_owned();
        let id = asset.lut().id();
        h.update(|editor| editor.look.loaded.push(asset));
        choose(&mut h, &label);
        wait_for(&mut h, |editor| !editor.interaction_blocked());
        reject_displayed(&mut h, &rejection, id);
        let failed = h.read(|editor| {
            let view = editor.canvas_frame.displayed.as_ref().unwrap().view.clone();
            assert_eq!(view.look_id(), Some(id));
            assert!(view.image.look_failure(id).is_some());
            view
        });
        assert!(h.query_by_label("Look rendering failed").is_some());

        h.enter(|editor, cx| editor.begin_flat_edit(fixture("bare-light.dng"), None, cx));
        h.step();
        wait_for(&mut h, |editor| !editor.interaction_blocked());
        paint_until(&mut h, |editor| {
            editor
                .canvas_frame
                .displayed
                .as_ref()
                .is_some_and(|painted| painted.view.spec.is_inspection())
        });
        h.read(|editor| {
            assert!(editor.document.flat_edit().is_some());
            assert!(
                !editor.document.source,
                "ordinary photo preference is not the reference view"
            );
            assert_eq!(
                editor
                    .document
                    .visit
                    .working_view
                    .displayed
                    .as_ref()
                    .unwrap()
                    .look_id(),
                Some(id)
            );
            assert!(
                failed.image.look_failure(id).is_some(),
                "the unrelated failure is still retained"
            );
            assert!(
                editor
                    .canvas_frame
                    .displayed
                    .as_ref()
                    .unwrap()
                    .view
                    .look_id()
                    .is_none()
            );
            assert!(editor.look_rendering_error().is_none());
        });
        assert!(h.query_by_label("Look rendering failed").is_none());
        h.get_by_label("Cancel").click();
        h.step();
        reject_displayed(&mut h, &rejection, id);

        stage(&mut h, "Photo");
        h.get_by_label("Dust repair").click();
        h.step();
        h.read(|editor| assert!(editor.dusting()));
        reject_displayed(&mut h, &rejection, id);
        h.read(|editor| {
            assert_eq!(
                editor
                    .canvas_frame
                    .displayed
                    .as_ref()
                    .unwrap()
                    .view
                    .look_id(),
                Some(id)
            )
        });
        h.get_by_label("Done").click();
        h.step();
        h.enter(|editor, cx| editor.request_analysis(cx));
        h.step();
        wait_for(&mut h, |editor| editor.document.analysis_review().is_some());
        paint_until(&mut h, |editor| {
            editor
                .canvas_frame
                .displayed
                .as_ref()
                .is_some_and(|painted| painted.view.spec.is_inspection())
        });
        h.read(|editor| {
            assert!(
                editor
                    .canvas_frame
                    .displayed
                    .as_ref()
                    .unwrap()
                    .view
                    .look_id()
                    .is_none()
            );
            assert!(editor.look_rendering_error().is_none());
        });
        assert!(h.query_by_label("Look rendering failed").is_none());
        h.get_by_label("Cancel").click();
        h.step();
        reject_displayed(&mut h, &rejection, id);
    });
}
