use super::*;
use crate::gpu::interpretation::InspectionView;
use crate::gpu::photograph::request::PhotographRequest;

#[test]
fn same_proxy_inspection_reuses_unchanged_correction_but_refreshes_changed_project_gains() {
    pollster::block_on(async {
        let (mut editor, device, queue) = crate::editor::processing::tests::fixture().await;
        let proxy = editor
            .document
            .visit
            .canvas
            .ready
            .as_ref()
            .unwrap()
            .proxy
            .clone();
        let layout = editor.document.visit.canvas.last_layout.unwrap().0;
        let context = egui::Context::default();
        let capture =
            PhotographRequest::Inspection(InspectionView::new(glam::DMat3::IDENTITY, 0.).unwrap());
        let repairs = Default::default();
        editor
            .preview_paint(&proxy, layout, (&capture, &repairs), false, false, &context)
            .unwrap();
        let original = editor
            .active_canvas()
            .preview_rendered
            .as_ref()
            .unwrap()
            .2
            .clone();
        original.ensure_submitted(&device, &queue, &editor.submissions, &context);
        let original_pixels =
            crate::viewer::readback::image_pixels(&device, &queue, &original.image);
        editor
            .preview_paint(&proxy, layout, (&capture, &repairs), false, false, &context)
            .unwrap();
        assert!(
            Arc::ptr_eq(
                &original,
                &editor.active_canvas().preview_rendered.as_ref().unwrap().2
            ),
            "unchanged inspection must reuse the current rendered value"
        );

        let id = editor.document.visit.current_source.unwrap();
        let source = editor.document.state.project.source(id).unwrap().clone();
        let map = Arc::new(
            crate::flat_field::FlatField::from_nodes(
                source.development.active_area,
                [2, 2],
                vec![glam::DVec3::new(0.8, 1.1, 1.2); 4],
            )
            .unwrap(),
        );
        editor
            .document
            .state
            .project
            .set_flat_field(Some(crate::project::FlatFieldSettings::new(
                source.path.clone(),
                map,
            )))
            .unwrap();
        editor.document.correction = Correction::prepare(&device, &editor.document, None)
            .unwrap()
            .into();
        editor
            .preview_paint(&proxy, layout, (&capture, &repairs), false, false, &context)
            .unwrap();
        let corrected = editor
            .active_canvas()
            .preview_rendered
            .as_ref()
            .unwrap()
            .2
            .clone();
        assert!(!Arc::ptr_eq(&original, &corrected));
        corrected.ensure_submitted(&device, &queue, &editor.submissions, &context);
        let corrected_pixels =
            crate::viewer::readback::image_pixels(&device, &queue, &corrected.image);
        assert_eq!(original_pixels.len(), corrected_pixels.len());
        assert!(
            original_pixels
                .iter()
                .zip(&corrected_pixels)
                .any(|(a, b)| (0..3).any(|channel| (a[channel] - b[channel]).abs() > 1e-4)),
            "a correction edit must refresh pixels even when proxy, inspection transform and geometry stay the same"
        );
        assert_eq!(
            original_pixels,
            crate::viewer::readback::image_pixels(&device, &queue, &original.image),
            "the prior rendered value remains valid for its held consumer"
        );
        editor
            .preview_paint(&proxy, layout, (&capture, &repairs), false, false, &context)
            .unwrap();
        assert!(Arc::ptr_eq(
            &corrected,
            &editor.active_canvas().preview_rendered.as_ref().unwrap().2
        ));

        editor.document.state.project.set_flat_field(None).unwrap();
        editor.document.correction = None.into();
        editor
            .preview_paint(&proxy, layout, (&capture, &repairs), false, false, &context)
            .unwrap();
        let cleared = editor
            .active_canvas()
            .preview_rendered
            .as_ref()
            .unwrap()
            .2
            .clone();
        cleared.ensure_submitted(&device, &queue, &editor.submissions, &context);
        assert_eq!(
            original_pixels,
            crate::viewer::readback::image_pixels(&device, &queue, &cleared.image)
        );
        assert!(proxy.same_source(&editor.document.visit.canvas.ready.as_ref().unwrap().proxy));
    });
}
