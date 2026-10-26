use super::*;
use crate::{
    project::SourceAnalysis,
    scan_regions::{Areas, PictureGeometry},
};

pub(super) fn measured_areas() -> Areas {
    Areas {
        picture_geometry: Some(PictureGeometry {
            size: [768, 512],
            angle: 0.75,
            sides: [Some(80.), Some(680.), Some(60.), Some(450.)],
        }),
        picture_observed: [true; 4],
        opening: Some(
            Coverage::rectangle(Rect {
                x: 12,
                y: 7,
                width: 864,
                height: 576,
            })
            .corners()
            .map(|p| p.0),
        ),
        picture: Some(
            Coverage::rectangle(Rect {
                x: 100,
                y: 80,
                width: 650,
                height: 450,
            })
            .corners()
            .map(|p| p.0),
        ),
        ..Default::default()
    }
}

fn default_crop(active: Rect, areas: &Areas) -> Crop {
    framing::inward_margin(
        framing::propose(active, areas).unwrap(),
        active,
        framing::DEFAULT_MARGIN_PERCENT,
    )
    .unwrap()
}

pub(super) fn fixture() -> (Editor, tempfile::TempDir) {
    let mut editor = crate::editor::tests::fixture();
    crate::editor::tests::add_ready_fixture(&mut editor);
    let directory = tempfile::tempdir().unwrap();
    let id = editor.current_source.unwrap();
    let path = directory.path().join("photo-0.ARW");
    std::fs::write(&path, b"metadata-only source fixture").unwrap();
    editor.document.project.source_mut(id).unwrap().path = path.clone();
    let ready = editor.document.visit.ready.as_mut().unwrap();
    ready.path = path.clone();
    ready.record.path = path.clone();
    ready.file = thumbnails::FileKey::new(&path);
    for i in 1..4 {
        let mut s = editor.document.project.source(id).unwrap().clone();
        s.id = SourceId(0);
        s.path = directory.path().join(format!("photo-{i}.ARW"));
        s.adjustment_layers.as_mut().unwrap().layers[0]
            .adjustment
            .photo
            .exposure = i as f64 / 10.;
        std::fs::write(&s.path, b"metadata-only source fixture").unwrap();
        editor.document.project.add_source(s).unwrap();
    }
    editor.document.project.analysis_regions = editor
        .document
        .project
        .order
        .iter()
        .map(|&source| SourceAnalysis {
            source,
            areas: measured_areas(),
        })
        .collect();
    editor.sync_document();
    (editor, directory)
}

fn small() -> Crop {
    Crop::from_region(Rect {
        x: 200,
        y: 150,
        width: 100,
        height: 90,
    })
}

fn preview(editor: &Editor, id: SourceId) -> Option<Crop> {
    editor.auto_crop_preview(id).expect("fixed proposal member")
}

fn browse(editor: &mut Editor, index: usize, cx: &UiContext<'_, Editor>) {
    // Exercise production selection/load state, then deliver compatible metadata without RAW decoding.
    let mut ready = editor.document.visit.ready.take().unwrap();
    editor.select_roll_photo(index, egui::Modifiers::NONE, cx);
    let id = editor.current_source.unwrap();
    let source = editor.document.project.source(id).unwrap();
    ready.record = source.clone();
    ready.path = editor.document.source_path(id).unwrap();
    ready.file = thumbnails::FileKey::new(&ready.path);
    ready.area = source.development.active_area.into();
    editor.source_result(editor.document.visit.view, Ok(Box::new(ready)));
}

#[test]
fn batch_crop_closes_local_editing_and_preserves_masks_on_apply_and_cancel() {
    for apply in [false, true] {
        let (mut e, _directory) = fixture();
        e.tab = 1;
        e.source = false;
        let id = e.current_source.unwrap();
        let mut layers = crate::project::AdjustmentLayers::default();
        let local = layers
            .add(
                "Sky".into(),
                crate::adjustment_layers::AdjustmentLayer {
                    mask: crate::adjustment_layers::AdjustmentLayerCoverage::Linear {
                        start: crate::geometry::SourcePoint([100., 80.]),
                        end: crate::geometry::SourcePoint([650., 450.]),
                    },
                    photo: AdjustmentLayerSettings {
                        exposure: 0.25,
                        ..Default::default()
                    },
                    enabled: true,
                    invert: false,
                    strength: 1.,
                },
            )
            .unwrap();
        e.document.project.source_mut(id).unwrap().adjustment_layers = Some(layers.clone());
        e.refresh_adjustment_layer_selection();
        e.select_adjustment_layer(adjustment_layers::Target::Layer(local));
        assert!(e.mask_handles_available());
        let before = e.document.project.clone();
        e.auto_crop(Target::Roll);
        assert!(e.auto_crop_valid());
        assert!(e.document.visit.mask_edit.is_none());
        assert_eq!(
            e.adjustment_layer_target(),
            adjustment_layers::Target::Layer(local)
        );
        assert_eq!(e.document.project, before);

        // A late layer action cannot compete with the active crop draft.
        e.reveal_adjustment_layers();
        assert!(!e.mask_handles_available());
        assert!(!e.adjustment_layer_owns_delete());
        e.begin_linear_mask(None);
        e.begin_radial_mask(None);
        e.delete_selected_adjustment_layer();
        assert!(e.document.visit.mask_edit.is_none());
        assert!(!e.document.visit.adjustment_layers.confirming());
        assert_eq!(e.document.project, before);
        e.finish_adjustment_layer_interaction();

        e.finish_auto_crop(apply);
        assert!(e.document.auto_crop_edit.is_none(), "{}", e.status);
        assert_eq!(
            e.document.project.source(id).unwrap().adjustment_layers,
            Some(layers)
        );
        let mut expected = before;
        if apply {
            for source in &mut expected.sources {
                source.crop = Some(default_crop(
                    source.development.active_area.into(),
                    &measured_areas(),
                ));
            }
        }
        assert_eq!(e.document.project, expected);
    }
}

#[test]
fn partial_proposals_keep_source_reasons_without_warning_about_uncaptured_photos() {
    let (e, _directory) = fixture();
    crate::application::test_support::with_application(e, |app| {
        app.enter(|e, cx| {
            let ids = e.document.project.order.clone();
            e.select_roll_photo(1, egui::Modifiers::COMMAND, cx);
            e.select_roll_photo(2, egui::Modifiers::COMMAND, cx);
            e.document
                .project
                .analysis_regions
                .retain(|r| r.source != ids[1]);
            e.document.project.analysis_regions[1]
                .areas
                .picture_observed[0] = false;
            e.document.project.analysis_regions[1].areas.picture_weak[0] = true;
            let before = e.document.project.clone();
            e.auto_crop(Target::Selected);
            assert!(e.auto_crop_valid());
            assert!(e.source_tool_warning(ids[0]).is_none());
            assert!(
                e.source_tool_warning(ids[1])
                    .unwrap()
                    .contains("no retained frame evidence")
            );
            assert!(
                e.source_tool_warning(ids[2])
                    .unwrap()
                    .contains("four measured picture edges")
            );
            assert!(
                e.source_tool_warning(ids[3]).is_none(),
                "not captured is not a failed proposal"
            );
            browse(e, 1, cx);
            let reason = e.source_tool_warning(ids[1]).unwrap().to_owned();
            e.select_active_photo_only();
            e.adjust_auto_crop_margin(2.);
            assert_eq!(e.source_tool_warning(ids[1]), Some(reason.as_str()));
            assert!(e.source_tool_warning(ids[3]).is_none());
            assert_eq!(e.document.project, before);
            e.finish_auto_crop(false);
            assert!(ids.iter().all(|&id| e.source_tool_warning(id).is_none()));
            assert_eq!(e.document.project, before);
        });
    });
}

#[test]
fn all_failed_attempts_stay_selected_with_reasons_but_cannot_apply_or_edit_crops() {
    let (mut e, _directory) = fixture();
    e.document.project.analysis_regions.clear();
    let before = e.document.project.clone();
    let serialized = serde_json::to_string(&before).unwrap();
    e.auto_crop(Target::Roll);
    assert!(e.document.auto_crop_edit.is_some() && e.auto_crop_context());
    assert!(!e.auto_crop_valid() && !e.auto_crop_editable());
    for &id in &before.order {
        assert!(
            e.source_tool_warning(id)
                .unwrap()
                .contains("No crop proposal")
        );
    }
    e.adjust_auto_crop_margin(2.);
    e.finish_auto_crop(true);
    assert!(e.document.auto_crop_edit.is_some());
    assert_eq!(
        serde_json::to_string(&e.document.project).unwrap(),
        serialized
    );
    assert!(e.document.analysis_preparing.is_none() && e.document.analysis_pending.is_none());
    e.finish_auto_crop(false);
    assert!(e.document.auto_crop_edit.is_none());
    assert_eq!(e.document.project, before);
}

#[test]
fn scopes_capture_drafts_and_apply_only_their_fixed_successful_sources() {
    let (e, _directory) = fixture();
    crate::application::test_support::with_application(e, |app| {
        app.enter(|e, cx| {
            let ids = e.document.project.order.clone();
            e.crop = Some(small());
            e.sync_document();
            let mut expected = e.document.project.clone();
            let active = e.document.visit.ready.as_ref().unwrap().area;
            let proposed = default_crop(active, &measured_areas());
            assert!(proposed.angle.abs() > 0.);
            e.begin_crop();
            e.auto_crop(Target::Photo);
            assert_eq!(e.document.project, expected);
            assert_eq!(e.crop, Some(small()));
            assert_eq!(e.document.visit.crop_edit.as_ref().unwrap().draft, proposed);
            assert!(e.document.auto_crop_edit.is_none());
            e.finish_crop(true);
            e.sync_document();
            expected.source_mut(ids[0]).unwrap().crop = Some(proposed);
            assert_eq!(e.document.project, expected);
            assert_eq!(e.crop, Some(proposed));
            assert!(e.document.auto_crop_edit.is_none());
            e.select_roll_photo(2, egui::Modifiers::COMMAND, cx);
            e.auto_crop(Target::Selected);
            e.select_roll_photo(3, egui::Modifiers::COMMAND, cx);
            e.adjust_auto_crop_margin(1.25);
            let margin = framing::inward_margin(
                framing::propose(active, &measured_areas()).unwrap(),
                active,
                1.25,
            )
            .unwrap();
            assert_eq!(e.document.project, expected);
            assert!(e.auto_crop_preview(ids[1]).is_none() && e.auto_crop_preview(ids[3]).is_none());
            e.finish_auto_crop(true);
            for &id in &[ids[0], ids[2]] {
                expected.source_mut(id).unwrap().crop = Some(margin);
            }
            assert_eq!(e.document.project, expected);
            e.auto_crop(Target::Roll);
            assert!(ids.iter().all(|&id| e.auto_crop_preview(id).is_some()));
            assert_eq!(e.document.project, expected);
            e.finish_auto_crop(true);
            for id in ids {
                expected.source_mut(id).unwrap().crop = Some(margin);
            }
            assert_eq!(e.document.project, expected);
            assert_eq!(e.current_source, Some(expected.order[0]));
            assert!(
                e.document.analysis_preparing.is_none()
                    && e.document.analysis_pending.is_none()
                    && !e.document.visit.source.pending()
                    && e.document.visit.grade.task.is_none(),
                "no hidden Analyze, RAW reload or grading work"
            );
        });
    });
}

#[test]
fn abstentions_preserve_previous_crops_without_preventing_other_proposals() {
    let (mut e, directory) = fixture();
    let ids = e.document.project.order.clone();
    for &id in &ids {
        e.document.project.source_mut(id).unwrap().crop = Some(small());
    }
    e.crop = Some(small());
    e.document.project.analysis_regions[1]
        .areas
        .picture_observed[0] = false;
    e.document.project.analysis_regions[1].areas.picture_weak[0] = true;
    std::fs::remove_file(directory.path().join("photo-2.ARW")).unwrap();
    e.document
        .project
        .analysis_regions
        .retain(|r| r.source != ids[3]);
    let before = e.document.project.clone();
    e.auto_crop(Target::Roll);
    assert!(e.status.contains("1 proposed, 3 skipped"), "{}", e.status);
    assert_eq!(e.document.project, before);
    assert!(ids[1..].iter().all(|&id| e.auto_crop_preview(id).is_none()));
    let draft = preview(&e, ids[0]);
    e.finish_auto_crop(true);
    let mut expected = before;
    expected.source_mut(ids[0]).unwrap().crop = draft;
    assert_eq!(e.document.project, expected);
    assert!(e.document.auto_crop_edit.is_none(), "{}", e.status);
}

#[test]
fn loaded_file_freshness_survives_thumbnail_seed_consumption() {
    let (mut e, directory) = fixture();
    let before = e.document.project.clone();
    e.document.visit.ready.as_mut().unwrap().thumbnail.take();
    e.begin_crop();
    std::fs::write(
        directory.path().join("photo-0.ARW"),
        b"different RAW revision",
    )
    .unwrap();
    e.auto_crop(Target::Photo);
    assert!(e.document.auto_crop_edit.is_none());
    assert!(e.status.contains("reopen"));
    assert_eq!(e.document.project, before);
    e.document.visit.ready.as_mut().unwrap().file =
        thumbnails::FileKey::new(&directory.path().join("photo-0.ARW"));
    e.auto_crop(Target::Photo);
    assert!(e.document.visit.crop_edit.is_some() && e.document.auto_crop_edit.is_none());
    assert_eq!(
        e.document.visit.crop_edit.as_ref().unwrap().draft,
        default_crop(
            e.document.visit.ready.as_ref().unwrap().area,
            &measured_areas()
        )
    );
    e.finish_crop(false);
    assert_eq!(e.document.project, before);
}

#[test]
fn margin_uses_originals_and_target_identity_across_browsing_and_reselection() {
    let (e, _directory) = fixture();
    crate::application::test_support::with_application(e, |app| {
        app.enter(|e, cx| {
            let ids = e.document.project.order.clone();
            e.source = true;
            e.select_roll_photo(2, egui::Modifiers::COMMAND, cx);
            let before = e.document.project.clone();
            e.auto_crop(Target::Selected);
            let proposals = e
                .document
                .auto_crop_edit
                .as_ref()
                .unwrap()
                .entries
                .iter()
                .map(|r| r.proposal)
                .collect::<Vec<_>>();
            browse(e, 1, cx);
            assert!(e.source && e.auto_crop_context());
            assert!(e.auto_crop_preview(ids[1]).is_none());
            e.select_active_photo_only();
            let viewport = e.viewport;
            for percent in [2., 1.25, 0., 5., 0.] {
                e.adjust_auto_crop_margin(percent);
                assert_eq!(e.auto_crop_margin, percent);
                assert_eq!(e.document.project, before);
                assert_eq!(e.viewport, viewport);
                for (&id, &original) in [ids[0], ids[2]].iter().zip(&proposals) {
                    let active = e
                        .document
                        .project
                        .source(id)
                        .unwrap()
                        .development
                        .active_area
                        .into();
                    assert_eq!(
                        preview(e, id),
                        Some(framing::inward_margin(original, active, percent).unwrap())
                    );
                }
                assert!(
                    e.auto_crop_preview(ids[1]).is_none() && e.auto_crop_preview(ids[3]).is_none()
                );
            }
            assert!(
                e.document
                    .auto_crop_edit
                    .as_ref()
                    .unwrap()
                    .entries
                    .iter()
                    .map(|r| r.proposal)
                    .eq(proposals)
            );
            e.finish_auto_crop(false);
            assert_eq!(e.current_source, Some(ids[1]));
            assert_eq!(e.document.project, before);
            assert!(e.source);
            assert_eq!(
                e.auto_crop_margin, 0.,
                "transient preference is reused, not persisted crop data"
            );
        });
    });
}

#[test]
fn cancel_restores_entry_view_and_never_installs_preview_crops() {
    let (mut e, _directory) = fixture();
    e.crop = Some(small());
    e.sync_document();
    e.viewport.zoom = Some(1.5);
    let viewport = e.viewport;
    let before = e.document.project.clone();
    let serialized = serde_json::to_string(&before).unwrap();
    e.auto_crop(Target::Roll);
    e.adjust_auto_crop_margin(3.5);
    assert_eq!(
        serde_json::to_string(&e.document.project).unwrap(),
        serialized
    );
    assert_eq!(e.crop, Some(small()));
    assert_ne!(preview(&e, before.order[0]), e.crop);
    let composition = Composition::with_crop(
        e.document.visit.ready.as_ref().unwrap().area,
        e.roll_orientation,
        e.photo_orientation,
        e.crop,
    )
    .unwrap();
    assert_eq!(
        composition.crop(),
        small(),
        "accepted output authority is unchanged"
    );
    e.finish_auto_crop(false);
    assert_eq!(e.document.project, before);
    assert_eq!(e.crop, Some(small()));
    assert_eq!(e.viewport, viewport);
    assert!(e.document.auto_crop_edit.is_none() && !e.auto_crop_context());
}

#[test]
fn apply_is_atomic_when_any_source_file_changes_after_the_draft() {
    let (mut e, directory) = fixture();
    let before = e.document.project.clone();
    e.auto_crop(Target::Roll);
    e.adjust_auto_crop_margin(2.);
    // Source zero passes validation and is written only to a local candidate before source one fails.
    std::fs::write(directory.path().join("photo-1.ARW"), b"changed source").unwrap();
    e.finish_auto_crop(true);
    assert!(e.status.contains("not applied"), "{}", e.status);
    assert!(
        e.source_tool_warning(before.order[1])
            .unwrap()
            .contains("source changed")
    );
    assert!(e.source_tool_warning(before.order[0]).is_none());
    assert!(!e.auto_crop_valid());
    assert_eq!(e.document.project, before);
    assert_eq!(e.crop, None);
    assert!(e.document.auto_crop_edit.is_some());
    e.finish_auto_crop(false);
    assert_eq!(e.document.project, before);
}

#[test]
fn stale_metadata_evidence_authored_crops_order_and_document_cannot_be_promoted() {
    for case in 0..8 {
        let (mut e, directory) = fixture();
        let ids = e.document.project.order.clone();
        e.auto_crop(Target::Roll);
        match case {
            0 => {
                e.document.project.analysis_regions[1]
                    .areas
                    .picture_geometry
                    .as_mut()
                    .unwrap()
                    .angle += 0.5
            }
            1 => e.document.project.analysis_regions[1]
                .areas
                .picture_constraints
                .push([1., 0., -40.]),
            2 => {
                e.document.project.analysis_regions[1]
                    .areas
                    .picture_observed[0] = false
            }
            3 => {
                e.document
                    .project
                    .source_mut(ids[1])
                    .unwrap()
                    .development
                    .active_area[2] -= 1
            }
            4 => e.document.project.source_mut(ids[1]).unwrap().crop = Some(small()),
            5 => e.document.project.order.swap(1, 2),
            6 => {
                let replacement = directory.path().join("replacement.ARW");
                std::fs::write(&replacement, b"metadata-only source fixture").unwrap();
                e.document.project.source_mut(ids[1]).unwrap().path = replacement;
            }
            7 => *e.document = document::Document::default(),
            _ => unreachable!(),
        }
        let before = e.document.project.clone();
        e.adjust_auto_crop_margin(3.);
        e.finish_auto_crop(true);
        assert_eq!(e.document.project, before, "case {case}");
        assert!(
            e.document.auto_crop_edit.is_some(),
            "case {case}: {}",
            e.status
        );
        assert!(
            e.status.contains("not applied"),
            "case {case}: {}",
            e.status
        );
        e.finish_auto_crop(false);
        assert_eq!(e.document.project, before);
    }
}

#[test]
fn development_change_and_invalid_margins_do_not_mutate_the_draft_or_document() {
    let (mut e, _directory) = fixture();
    e.auto_crop(Target::Roll);
    let before = e.document.project.clone();
    let id = before.order[0];
    let draft = preview(&e, id);
    for percent in [f64::NAN, -0.1, 5.01] {
        e.adjust_auto_crop_margin(percent);
        assert_eq!(preview(&e, id), draft);
        assert_eq!(e.auto_crop_margin, framing::DEFAULT_MARGIN_PERCENT);
        assert_eq!(e.document.project, before);
    }
    e.document.project.development_mut().unwrap().fixed_wb[0] *= 1.01;
    let changed = e.document.project.clone();
    e.adjust_auto_crop_margin(2.);
    e.finish_auto_crop(true);
    assert_eq!(e.document.project, changed);
    assert!(e.document.auto_crop_edit.is_some());
    e.finish_auto_crop(false);
    assert_eq!(e.document.project, changed);
}

#[test]
fn a_whole_source_proposal_is_an_explicit_draft_reset_not_an_absent_override() {
    let (mut e, _directory) = fixture();
    let id = e.current_source.unwrap();
    let active = e.document.visit.ready.as_ref().unwrap().area;
    let areas = &mut e.document.project.analysis_regions[0].areas;
    areas.picture_geometry = Some(PictureGeometry {
        size: [768, 512],
        angle: 0.,
        sides: [Some(0.), Some(768.), Some(0.), Some(512.)],
    });
    areas.picture = Some(Coverage::rectangle(active).corners().map(|p| p.0));
    e.crop = Some(small());
    e.sync_document();
    e.auto_crop_margin = 0.;
    let before = e.document.project.clone();
    e.select_active_photo_only();
    e.auto_crop(Target::Selected);
    assert_eq!(e.auto_crop_preview(id), Some(None));
    assert_eq!(e.auto_crop_preview(id).unwrap_or(e.crop), None);
    assert_eq!(e.document.project, before);
    e.finish_auto_crop(true);
    assert_eq!(e.crop, None);
    assert_eq!(e.document.project.source(id).unwrap().crop, None);
}

#[test]
fn draft_tool_blocks_competing_tools_document_commands_export_and_removal() {
    let (e, directory) = fixture();
    crate::application::test_support::with_application(e, |app| {
        app.enter(|e, cx| {
            let before = e.document.project.clone();
            e.auto_crop(Target::Roll);
            e.begin_crop();
            e.begin_pick(Pick::Dmin, cx);
            e.begin_flat_edit(
                directory.path().join("flat.ARW"),
                None,
                &UiContext::render_only(),
            );
            e.begin_dust_cleanup();
            e.request_analysis(cx);
            e.request_removal();
            for action in [
                FileAction::New,
                FileAction::Save,
                FileAction::Add,
                FileAction::Remove,
            ] {
                e.file_action(action, cx);
            }
            e.export(
                directory.path().join("draft.exr"),
                OutputFormat::Exr(ExrPrecision::Full),
                Overwrite::Refuse,
                cx,
            );
            assert!(
                e.document.visit.crop_edit.is_none()
                    && e.pick.is_none()
                    && e.flat_edit.is_none()
                    && !e.document.dust.cleanup
                    && e.removal_pending.is_none()
            );
            assert!(e.document.auto_crop_edit.is_some());
            assert!(e.export_operation.pending.is_none());
            assert_eq!(e.document.project, before);
            assert!(
                e.document.analysis_preparing.is_none()
                    && e.document.analysis_pending.is_none()
                    && e.document_pending.is_none()
                    && e.export_operation.pending.is_none()
            );
            e.start_document(Preparation::New, cx);
            assert!(e.document.auto_crop_edit.is_none());
            assert_eq!(e.document.project, before);
            assert!(e.busy && e.document_pending.is_some());
        });
    });
}

#[test]
fn photo_initialization_stays_in_manual_crop_and_keeps_its_cancel_baseline() {
    let (mut e, _directory) = fixture();
    e.crop = Some(small());
    e.sync_document();
    e.viewport.zoom = Some(1.5);
    let viewport = e.viewport;
    let before = e.document.project.clone();
    let active = e.document.visit.ready.as_ref().unwrap().area;
    let expected = default_crop(active, &measured_areas());
    e.begin_crop();
    e.document.visit.crop_edit.as_mut().unwrap().draft = Crop::from_region(active);
    e.auto_crop(Target::Photo);
    assert!(e.document.auto_crop_edit.is_none());
    assert_eq!(e.document.visit.crop_edit.as_ref().unwrap().draft, expected);
    assert_eq!(e.crop, Some(small()));
    assert_eq!(e.document.project, before);
    e.finish_crop(false);
    assert_eq!(e.viewport, viewport);
    assert_eq!(e.document.project, before);
    e.begin_crop();
    e.auto_crop(Target::Photo);
    e.finish_crop(true);
    e.sync_document();
    let mut accepted = before;
    accepted.source_mut(e.current_source.unwrap()).unwrap().crop = Some(expected);
    assert_eq!(e.document.project, accepted);
    assert!(e.document.analysis_preparing.is_none() && e.document.analysis_pending.is_none());
}

#[test]
fn unavailable_photo_proposal_does_not_replace_a_manual_draft() {
    let (mut e, directory) = fixture();
    e.begin_crop();
    e.document.visit.crop_edit.as_mut().unwrap().draft = small();
    let before = e.document.project.clone();
    let viewport = e.viewport;
    std::fs::write(directory.path().join("photo-0.ARW"), b"changed").unwrap();
    e.auto_crop(Target::Photo);
    assert_eq!(e.document.visit.crop_edit.as_ref().unwrap().draft, small());
    assert!(e.document.auto_crop_edit.is_none());
    assert_eq!(e.document.project, before);
    assert_eq!(e.viewport, viewport);
    assert!(e.status.contains("Crop unchanged"));
    assert!(
        e.source_tool_warning(e.current_source.unwrap())
            .unwrap()
            .contains("Automatic crop unavailable")
    );
    let path = directory.path().join("photo-0.ARW");
    e.document.visit.ready.as_mut().unwrap().file = thumbnails::FileKey::new(&path);
    e.auto_crop(Target::Photo);
    assert!(
        e.source_tool_warning(e.current_source.unwrap()).is_none(),
        "successful retry retires the cue"
    );
    e.finish_crop(false);
    assert!(e.source_tool_warning(e.current_source.unwrap()).is_none());
}

#[test]
fn entry_respects_other_active_tools_and_foreground_work() {
    for case in 0..6 {
        let (mut e, _directory) = fixture();
        let before = e.document.project.clone();
        match case {
            0 => e.busy = true,
            1 => e.pick = Some(Pick::Dmin),
            2 => e.begin_crop(),
            3 => e.begin_dust_cleanup(),
            4 => e.document_pending = Some(99),
            5 => e.removal_pending = Some(vec![e.current_source.unwrap()]),
            _ => unreachable!(),
        }
        e.auto_crop(Target::Roll);
        assert!(e.document.auto_crop_edit.is_none(), "case {case}");
        assert_eq!(e.document.project, before);
    }
}
