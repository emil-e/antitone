use super::*;
use crate::geometry::{Flips, QuarterTurns};

#[test]
fn margin_drag_uses_source_axes_through_all_turns_reflections_and_nonzero_origins() {
    let area = Rect {
        x: 12,
        y: 7,
        width: 864,
        height: 576,
    };
    let proposal = Crop {
        center: [450., 300.],
        size: [600, 400],
        angle: 0.035,
    };
    let crop = framing::inward_margin(proposal, area, 0.5).unwrap();
    for turns in [
        QuarterTurns::Zero,
        QuarterTurns::One,
        QuarterTurns::Two,
        QuarterTurns::Three,
    ] {
        for horizontal in [false, true] {
            for vertical in [false, true] {
                for photo_turns in [
                    QuarterTurns::Zero,
                    QuarterTurns::One,
                    QuarterTurns::Two,
                    QuarterTurns::Three,
                ] {
                    let composition = Composition::new(
                        area,
                        RollOrientation {
                            turns,
                            horizontal,
                            vertical,
                        },
                        PhotoOrientation {
                            turns: photo_turns,
                            flips: Flips {
                                horizontal: vertical,
                                vertical: horizontal,
                            },
                        },
                    )
                    .unwrap();
                    let layout = Viewport::default().layout_composition(
                        egui::Rect::from_min_size(egui::pos2(31., 42.), egui::vec2(800., 600.)),
                        composition,
                        2.,
                    );
                    let q = crop
                        .coverage()
                        .unwrap()
                        .corners()
                        .map(|p| layout.screen_frame_point(composition.source_to_frame(p)));
                    for (point, handle) in crop_editor::outline_handles(q) {
                        assert_eq!(hit(q, point), Some(handle));
                        let start = proposal
                            .local_point(composition.frame_to_source(layout.frame_point(point)));
                        let drag = MarginDrag {
                            source: SourceId(1),
                            proposal,
                            handle,
                            start,
                            percent: 0.5,
                        };
                        let next = std::array::from_fn(|i| start[i] - f64::from(handle[i]) * 4.);
                        let next = proposal.source_point(next);
                        let screen = layout.screen_frame_point(composition.source_to_frame(next));
                        assert_eq!(
                            drag.margin(composition.frame_to_source(layout.frame_point(screen))),
                            1.5
                        );
                        assert!(matches!(
                            cursor(q, handle),
                            egui::CursorIcon::ResizeHorizontal
                                | egui::CursorIcon::ResizeVertical
                                | egui::CursorIcon::ResizeNwSe
                                | egui::CursorIcon::ResizeNeSw
                        ));
                    }
                    let edge = q[0] + (q[3] - q[0]) * 0.25;
                    assert_eq!(
                        hit(q, edge),
                        Some([-1, 0]),
                        "whole edges are draggable, not just brackets"
                    );
                }
            }
        }
    }
}

#[test]
fn drag_clamps_the_shared_margin_and_outside_points_do_not_hit_an_edge() {
    let proposal = Crop {
        center: [500., 500.],
        size: [600, 400],
        angle: 0.,
    };
    let drag = MarginDrag {
        source: SourceId(1),
        proposal,
        handle: [-1, 0],
        start: [2., 200.],
        percent: 0.5,
    };
    assert_eq!(drag.margin(proposal.source_point([-100., 200.])), 0.);
    assert_eq!(
        drag.margin(proposal.source_point([500., 200.])),
        framing::MAX_MARGIN_PERCENT
    );
    let q = [[10., 10.], [110., 10.], [110., 90.], [10., 90.]].map(|p| egui::pos2(p[0], p[1]));
    assert_eq!(hit(q, egui::pos2(60., 50.)), None);
    assert_eq!(hit(q, egui::pos2(200., 50.)), None);
}

#[test]
fn shared_outline_shades_outside_but_not_inside_rotated_and_reflected_frames() {
    let canvas = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(500., 400.));
    for angle in [-0.2_f32, 0., 0.3] {
        for reflected in [false, true] {
            let (s, c) = angle.sin_cos();
            let mut q = [[-100., -70.], [100., -70.], [100., 70.], [-100., 70.]]
                .map(|[x, y]| egui::pos2(250. + c * x - s * y, 200. + s * x + c * y));
            if reflected {
                q.reverse();
            }
            let context = egui::Context::default();
            let mut output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(canvas),
                    ..Default::default()
                },
                |ui| crop_editor::paint_outline(ui, canvas, q, false, 64),
            );
            output.textures_delta.clear();
            let mesh = output
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::Shape::Mesh(m)
                        if m.vertices
                            .iter()
                            .all(|v| v.color == egui::Color32::from_black_alpha(64)) =>
                    {
                        Some(m)
                    }
                    _ => None,
                })
                .unwrap();
            let covered = |p: egui::Pos2| {
                mesh.indices.as_chunks::<3>().0.iter().any(|t| {
                    let pts = std::array::from_fn::<_, 3, _>(|i| mesh.vertices[t[i] as usize].pos);
                    let values = std::array::from_fn::<_, 3, _>(|i| {
                        (pts[(i + 1) % 3] - pts[i]).rot90().dot(p - pts[i])
                    });
                    values.iter().all(|v| *v >= -1e-4) || values.iter().all(|v| *v <= 1e-4)
                })
            };
            assert!(!covered(egui::pos2(250., 200.)));
            assert!(covered(egui::pos2(20., 20.)));
            assert!(covered(egui::pos2(480., 380.)));
        }
    }
}

#[test]
fn context_changes_only_the_view_and_margin_changes_reuse_the_uncropped_image_authority() {
    let (mut editor, _directory) = super::super::tests::fixture();
    let project = editor.document.state.project.clone();
    editor.auto_crop(Target::Roll);
    assert!(editor.auto_crop_context());
    let area = editor.document.visit.canvas.ready.as_ref().unwrap().area;
    let composition = Composition::new(
        area,
        editor.document.roll_orientation,
        editor.document.visit.photo_orientation,
    )
    .unwrap();
    let layout = editor.document.visit.canvas.viewport.layout_composition(
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800., 600.)),
        composition,
        1.,
    );
    editor.document.visit.canvas.last_layout = Some((layout, 1.));
    let viewport = editor.document.visit.canvas.viewport;
    let revision = editor.document.visit.canvas.grade.original.revision();
    let outline = editor.auto_crop_outline(layout).unwrap();
    editor.adjust_auto_crop_margin(2.);
    assert_ne!(editor.auto_crop_outline(layout).unwrap(), outline);
    assert_eq!(editor.document.visit.canvas.viewport, viewport);
    assert_eq!(
        editor.document.visit.canvas.grade.original.revision(),
        revision,
        "the full-scan reference remains valid when only an overlay moves"
    );
    assert!(editor.document.visit.canvas.last_layout.is_some());
    editor
        .document
        .auto_crop_edit
        .as_mut()
        .unwrap()
        .show_outline = false;
    assert!(
        editor.auto_crop_context(),
        "artwork visibility is not a second tool lifecycle"
    );
    assert_eq!(
        editor.document.visit.canvas.grade.original.revision(),
        revision
    );
    editor.begin_crop();
    assert!(editor.document.visit.crop_edit.is_none() && editor.auto_crop_context());
    editor.finish_auto_crop(false);
    assert!(!editor.auto_crop_context());
    assert_eq!(editor.document.state.project, project);
    assert_ne!(
        editor.document.visit.canvas.grade.original.revision(),
        revision,
        "leaving the tool changes the view domain"
    );
    editor.begin_crop();
    assert!(editor.document.visit.crop_edit.is_some());
    editor.finish_crop(false);
    assert_eq!(editor.document.state.project, project);
}
