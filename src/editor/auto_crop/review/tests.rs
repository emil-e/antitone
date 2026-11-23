use super::*;
use crate::project::{Development, DevelopmentMode, Source, SourceAnalysis};

fn add_photo(document: &mut Document, path: &str) -> SourceId {
    document
        .project
        .add_source(Source {
            id: SourceId(0),
            path: path.into(),
            capture_time: None,
            development_mode: DevelopmentMode::Unity,
            development: SourceDevelopment {
                decode: Development::new("fixture".into(), [1.; 4]),
                dimensions: [900, 600],
                active_area: [12, 7, 864, 576],
                camera_model: "fixture camera".into(),
                camera_to_ap1: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
            },
            repairs: Default::default(),
            adjustment_layers: None,
            orientation: Default::default(),
            crop: None,
        })
        .unwrap()
}

#[test]
fn review_candidate_preserves_order_and_only_crops_captured_targets() {
    let mut document = Document::default();
    let target = add_photo(&mut document, "target.ARW");
    let peer = add_photo(&mut document, "peer.ARW");
    document.project.analysis_regions.push(SourceAnalysis {
        source: target,
        areas: super::super::tests::measured_areas(),
    });
    let initialization = Initialization::default();
    let draft = Draft::capture(
        &Inputs::new(&document, None, Arc::default(), &initialization),
        vec![target],
        "Selection",
        framing::DEFAULT_MARGIN_PERCENT,
    );

    document.project.reorder_photos(vec![peer, target]).unwrap();
    let inputs = Inputs::new(&document, None, Arc::default(), &initialization);
    let candidate = draft.candidate(&inputs).unwrap();
    assert_eq!(candidate.order, [peer, target]);
    assert!(candidate.source(target).unwrap().crop.is_some());
    assert_eq!(candidate.source(peer).unwrap().crop, None);
    assert_eq!(document.project.source(target).unwrap().crop, None);

    let selected = SelectedPhoto {
        id: target,
        effective_crop: Some(Crop::from_region([200, 150, 100, 90].into())),
        loaded: None,
    };
    let inputs = Inputs::new(&document, Some(selected), Arc::default(), &initialization);
    assert!(
        draft.candidate(&inputs).is_err(),
        "a changed effective crop invalidates the proposal"
    );

    let foreign_file = FileKey::unversioned(std::path::Path::new("foreign.ARW"));
    let development = &document.project.source(target).unwrap().development;
    let selected = SelectedPhoto {
        id: target,
        effective_crop: None,
        loaded: Some(LoadedPhoto {
            file: &foreign_file,
            development,
            area: development.active_area.into(),
        }),
    };
    let inputs = Inputs::new(&document, Some(selected), Arc::default(), &initialization);
    assert!(
        draft.valid(&inputs),
        "the captured evidence itself is still current"
    );
    assert_eq!(draft.candidate(&inputs).unwrap_err().source, Some(target));

    document.project.remove_from_filmstrip(peer).unwrap();
    let inputs = Inputs::new(&document, None, Arc::default(), &initialization);
    assert!(
        draft.candidate(&inputs).is_err(),
        "even an untargeted photo is part of captured roll membership"
    );
}
