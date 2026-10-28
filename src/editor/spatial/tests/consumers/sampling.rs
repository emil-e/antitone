use super::*;
use crate::{coverage::Coverage, project::SourceAnalysis, scan_regions::Areas};

#[test]
fn film_edge_exclusion_survives_authored_review_and_failure_preserves_the_accepted_grade() {
    let (scheduler, _) = observed();
    let mut editor = fixture(scheduler.clone());
    let mut metadata = supplied(&mut editor);
    metadata.groups[0].sources[0].film = Some(Region {
        angle: 0.,
        skew: 0.,
        boundaries: [
            Boundary::Measured {
                plane: [1., 0., 400.],
            },
            Boundary::Unknown,
            Boundary::Unknown,
            Boundary::Unknown,
        ],
    });
    let inputs = editor.document.spatial_inputs();
    let scope = &mut editor.document;
    let pending = scope.spatial.begin().unwrap();
    assert!(
        scope
            .spatial
            .publish(pending, metadata.clone(), &scope.state.project, &inputs)
            .unwrap()
    );
    editor.document.state.project.analysis_regions = editor
        .document
        .project
        .order
        .iter()
        .map(|&source| {
            let mut areas = Areas::default();
            areas.edit_opening(Coverage::rectangle(analysis::rect([8, 8, 496, 496])));
            areas.edit_picture(Coverage::rectangle(analysis::rect([96, 96, 320, 320])));
            SourceAnalysis { source, areas }
        })
        .collect();
    editor.sync_document();
    let accepted = editor.document.state.project.clone();
    let dirty = editor.document.dirty();
    let undo = editor.history_label(history::Direction::Undo);
    with_application(editor, |app| {
        app.enter(|editor, cx| editor.request_analysis(cx));
        app.run_until(LIMIT, |editor| {
            editor.document.spatial_preparing.is_none()
                && editor.document.analysis_preparing.is_none()
        });
        app.read(|editor| {
            let review = editor
                .document
                .analysis_review
                .as_ref()
                .expect("interactive sampling review");
            assert_eq!(
                review.approved.entries, accepted.analysis_regions,
                "physical exclusion must not overwrite authored areas"
            );
        });
        app.enter(|editor, cx| editor.confirm_analysis_review(cx));
        app.run_until(LIMIT, |editor| !editor.busy);
        app.read(|editor| {
            assert!(
                editor.status.contains("only 40 of 40 picture samples are eligible; need 512"),
                "{}", editor.status
            );
            assert!(editor.document.analysis_review.is_none());
            assert_eq!(
                editor.document.state.project, accepted,
                "insufficient interior must not accept areas, skip a photograph or install a partial grade"
            );
            assert_eq!(editor.document.dirty(), dirty);
            assert_eq!(editor.history_label(history::Direction::Undo), undo);
            assert_eq!(*editor.document.spatial.ready(), metadata);
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn nominal_crop_applies_across_missing_film_and_analyze_keeps_that_framing() {
    let (scheduler, _) = observed();
    let mut editor = fixture(scheduler.clone());
    let mut metadata = supplied(&mut editor);
    let source = &mut metadata.groups[0].sources[0];
    source.observation.as_mut().unwrap().boundaries[0] = Boundary::Unknown;
    source.picture.as_mut().unwrap().boundaries[0] = Boundary::Inferred {
        plane: [1., 0., 96.],
    };
    source.film = Some(Region {
        angle: 0.,
        skew: 0.,
        boundaries: [
            Boundary::Measured {
                plane: [1., 0., 200.],
            },
            Boundary::Unknown,
            Boundary::Unknown,
            Boundary::Unknown,
        ],
    });
    metadata.groups[0].common_size = CommonSize::fit(&metadata.groups[0].sources);
    let inputs = editor.document.spatial_inputs();
    let scope = &mut editor.document;
    let pending = scope.spatial.begin().unwrap();
    assert!(
        scope
            .spatial
            .publish(pending, metadata.clone(), &scope.state.project, &inputs)
            .unwrap()
    );
    editor.document.state.project.analysis_regions = editor
        .document
        .project
        .order
        .iter()
        .map(|&source| {
            let mut areas = Areas::default();
            areas.edit_opening(Coverage::rectangle(analysis::rect([8, 8, 496, 496])));
            areas.edit_picture(Coverage::rectangle(analysis::rect([8, 8, 496, 496])));
            SourceAnalysis { source, areas }
        })
        .collect();
    editor.auto_crop_margin = 0.;
    editor.sync_document();
    let before = editor.document.state.project.clone();
    let target = before.order[0];
    with_application(editor, |app| {
        app.enter(|editor, cx| editor.request_auto_crop(auto_crop::Target::Roll, cx));
        app.run_until(LIMIT, |editor| editor.document.spatial_preparing.is_none());
        app.enter(|editor, _| editor.finish_auto_crop(true));
        let accepted = app.read(|editor| {
            let crop = editor
                .document
                .project
                .source(target)
                .unwrap()
                .crop
                .expect("nominal crop applied despite missing film");
            assert_eq!(
                crop.region([8, 8, 496, 496].into()).unwrap(),
                analysis::rect([96, 96, 320, 320])
            );
            assert_eq!(editor.document.state.project.dmin(), before.dmin());
            assert_eq!(*editor.document.spatial.ready(), metadata);
            editor.document.state.project.clone()
        });
        app.enter(|editor, cx| editor.request_analysis(cx));
        app.run_until(LIMIT, |editor| {
            editor.document.spatial_preparing.is_none()
                && editor.document.analysis_preparing.is_none()
        });
        app.enter(|editor, cx| editor.confirm_analysis_review(cx));
        app.run_until(LIMIT, |editor| !editor.busy);
        app.read(|editor| {
            assert!(editor.status.contains("Analyzed"), "{}", editor.status);
            assert!(editor.document.state.project.dmin().is_some());
            assert_eq!(
                editor.document.state.project.source(target).unwrap().crop,
                accepted.source(target).unwrap().crop,
                "analysis material support cannot trim the photographic crop"
            );
            assert_eq!(*editor.document.spatial.ready(), metadata);
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn analyze_uses_an_independent_shared_picture_inset_and_preserves_authored_sampling() {
    let (scheduler, _) = observed();
    let mut editor = fixture(scheduler.clone());
    let mut metadata = supplied(&mut editor);
    for row in &mut metadata.groups[0].sources {
        assert_eq!(row.input.development.active_area, [8, 8, 496, 496]);
        let picture = Region {
            angle: 0.,
            skew: 0.,
            boundaries: [
                [1., 0., 120.],
                [-1., 0., -392.],
                [0., 1., 128.],
                [0., -1., -384.],
            ]
            .map(|plane| Boundary::Measured { plane }),
        };
        row.observation = Some(picture.clone());
        row.picture = Some(picture);
    }
    metadata.groups[0].common_size = CommonSize::fit(&metadata.groups[0].sources);
    let inputs = editor.document.spatial_inputs();
    let scope = &mut editor.document;
    let request = scope.spatial.begin().unwrap();
    assert!(
        scope
            .spatial
            .publish(request, metadata.clone(), &scope.state.project, &inputs)
            .unwrap()
    );
    let old = [[160., 160.], [340., 160.], [340., 340.], [160., 340.]];
    let authored = [[170., 180.], [310., 180.], [310., 310.], [170., 310.]];
    editor.document.state.project.analysis_regions = editor
        .document
        .project
        .order
        .iter()
        .enumerate()
        .map(|(i, &source)| SourceAnalysis {
            source,
            areas: Areas {
                opening: Some([[8., 8.], [504., 8.], [504., 504.], [8., 504.]]),
                opening_shared: true,
                picture: Some(if i == 0 { old } else { authored }),
                picture_edited: i != 0,
                ..Default::default()
            },
        })
        .collect();
    editor.sync_document();
    let accepted = editor.document.state.project.clone();
    with_application(editor, |app| {
        app.enter(|editor, cx| editor.request_analysis(cx));
        app.run_until(LIMIT, |editor| {
            editor.document.spatial_preparing.is_none()
                && editor.document.analysis_preparing.is_none()
        });
        app.read(|editor| {
            let review = editor
                .document
                .analysis_review
                .as_ref()
                .expect("shared-prepared sampling review");
            // The declared source bounds are120..392 /128..384. Two evidence pixels
            // map to1.9375source pixels here, rounded conservatively to two per side.
            // No production geometry/inset function computes this expected quadrilateral.
            assert_eq!(
                review.approved.entries[0].areas.picture,
                Some([[122., 130.], [390., 130.], [390., 382.], [122., 382.]])
            );
            assert_eq!(review.approved.entries[1].areas.picture, Some(authored));
            assert!(review.approved.entries[1].areas.picture_edited);
            assert_eq!(*editor.document.spatial.ready(), metadata);
            assert_eq!(editor.document.state.project, accepted);
        });
    });
    scheduler.close_and_wait().unwrap();
}
