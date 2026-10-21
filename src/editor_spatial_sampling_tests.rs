use super::*;
use crate::{project::SourceAnalysis, scan_regions::Areas};

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
    editor.document.project.analysis_regions = editor
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
    let accepted = editor.document.project.clone();
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
            assert_eq!(editor.document.project, accepted);
        });
    });
    scheduler.close_and_wait().unwrap();
}
