use super::*;
use crate::{
    application::test_support::{TestApplication, with_application},
    frame_raster::FrameRaster,
    geometry::TextureMapping,
    project::SourceAnalysis,
    scan_regions::{Areas, PictureGeometry},
};

fn mapping(app: &TestApplication<'_, Editor>) -> (TextureMapping, FrameRaster, bool) {
    let context = egui::Context::default();
    let mut result = None;
    let mut output =
        context.run_ui(Default::default(), |ui| {
            app.enter(|editor, cx| {
                let ready = editor.document.visit.canvas.ready.as_ref().unwrap();
                let proxy = ready.proxy.clone();
                let composition = editor.canvas_composition(ready.area, None);
                let size = composition.size();
                let canvas = egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(size[0] as f32, size[1] as f32),
                );
                let geometry = editor.document.visit.canvas.viewport.layout_composition(
                    canvas,
                    composition,
                    1.,
                );
                let (_, paint) = editor
                    .canvas_image(ui, geometry, proxy, false, false, cx)
                    .unwrap();
                result = Some((
                    paint.texture_mapping(),
                    paint.view_frame().expect("canvas presents a RenderedView"),
                    editor.document.visit.working_view.displayed.is_some(),
                ));
            });
        });
    output.textures_delta.clear();
    result.unwrap()
}

fn assert_source_corners(uv: TextureMapping, frame: FrameRaster, crop: Crop) {
    let (s, c) = crop.angle.sin_cos();
    for corner in [[0., 0.], [1., 0.], [1., 1.], [0., 1.]] {
        let texture: [f64; 2] = std::array::from_fn(|axis| {
            f64::from(uv.origin[axis])
                + corner[0] * f64::from(uv.x_axis[axis])
                + corner[1] * f64::from(uv.y_axis[axis])
        });
        let actual = frame
            .source_point(std::array::from_fn(|axis| {
                texture[axis] * f64::from(frame.output[axis])
            }))
            .0;
        // Independent accepted-crop reference, not the composition/UV helper under test.
        let x = (corner[0] - 0.5) * f64::from(crop.size[0]);
        let y = (corner[1] - 0.5) * f64::from(crop.size[1]);
        let expected = [
            crop.center[0] + c * x - s * y,
            crop.center[1] + s * x + c * y,
        ];
        for axis in 0..2 {
            assert!(
                (actual[axis] - expected[axis]).abs() < 0.0002,
                "preview must show the accepted source crop: corner={corner:?}, actual={actual:?}, expected={expected:?}"
            );
        }
    }
}

fn provisional_then_physical(app: &mut TestApplication<'_, Editor>) {
    let crop = app.read(|editor| editor.document.visit.crop.unwrap());
    let admission = crate::background::testing::pause();
    let (uv, frame, physical) = mapping(app);
    assert!(
        !physical,
        "exercise the provisional renderer before original acquisition"
    );
    assert_source_corners(uv, frame, crop);
    app.pump();
    assert!(app.read(|editor| editor.document.visit.working.task.is_some()));
    drop(admission);
    app.run_until(Duration::from_secs(20), |editor| {
        editor.document.visit.working_view.support_ready()
    });
    let (uv, frame, physical) = mapping(app);
    assert!(
        physical,
        "compare the accepted original-derived working view too"
    );
    assert_source_corners(uv, frame, crop);
}

#[test]
fn proxy_and_working_image_keep_the_same_crop_after_apply_and_source_revisit() {
    for (angle, inspection) in [(0., true), (2.5, true), (0., false), (2.5, false)] {
        let mut editor = super::super::tests::authored_fixture(&["roll-0.dng", "roll-1.dng"]);
        editor.set_dmin(Some(DVec3::splat(3000. / 4031.)));
        editor.set_dmax(Some(DVec3::splat(128. / 4031.)));
        editor.document.source = inspection;
        editor.auto_crop_margin = 0.;
        // Declared frame measurements isolate the displayed geometry from detector discovery.
        editor.document.state.project.analysis_regions = editor
            .document
            .project
            .order
            .iter()
            .map(|&source| {
                let active = editor
                    .document
                    .project
                    .source(source)
                    .unwrap()
                    .development
                    .active_area
                    .into();
                let geometry = PictureGeometry {
                    size: [496, 496],
                    angle,
                    sides: [112., 368., 104., 296.].map(Some),
                };
                SourceAnalysis {
                    source,
                    areas: Areas {
                        opening: Some(Coverage::rectangle(active).corners().map(|p| p.0)),
                        picture: geometry.frame(active),
                        picture_geometry: Some(geometry),
                        picture_observed: [true; 4],
                        ..Default::default()
                    },
                }
            })
            .collect();
        editor.auto_crop(auto_crop::Target::Roll);
        assert!(editor.auto_crop_valid());
        editor.finish_auto_crop(true);
        assert!(
            editor.document.auto_crop_edit.is_none(),
            "{}",
            editor.status
        );
        assert!(editor.document.visit.crop.is_some());
        let first = editor
            .document
            .source_path(editor.document.state.project.order[0])
            .unwrap();
        let second = editor
            .document
            .source_path(editor.document.state.project.order[1])
            .unwrap();
        with_application(editor, |app| {
            provisional_then_physical(app);
            app.enter(|editor, cx| editor.load(second, cx));
            app.run_until(Duration::from_secs(20), |editor| !editor.busy);
            app.enter(|editor, cx| editor.load(first, cx));
            app.run_until(Duration::from_secs(20), |editor| !editor.busy);
            provisional_then_physical(app);
        });
    }
}
