use super::*;

#[test]
fn review_candidate_preserves_order_and_only_crops_captured_targets() {
    let (editor, _directory) = super::super::tests::fixture();
    let mut document = editor.document.capture();
    let spatial = editor.document.spatial.ready();
    let mut order = document.project.order.clone();
    let (target, peer) = (order[0], order[1]);
    let initialization = Initialization::default();
    let draft = Draft::capture(
        &Inputs::new(&document, None, spatial.clone(), &initialization),
        vec![target],
        "Selection",
        framing::DEFAULT_MARGIN_PERCENT,
    );

    order.swap(0, 1);
    document.project.reorder_photos(order.clone()).unwrap();
    let inputs = Inputs::new(&document, None, spatial.clone(), &initialization);
    let candidate = draft.candidate(&inputs).unwrap();
    assert_eq!(candidate.order, order);
    assert!(candidate.source(target).unwrap().crop.is_some());
    assert_eq!(candidate.source(peer).unwrap().crop, None);
    assert_eq!(document.project.source(target).unwrap().crop, None);

    let selected = SelectedPhoto {
        id: target,
        effective_crop: Some(Crop::from_region([200, 150, 100, 90].into())),
        loaded: None,
    };
    let inputs = Inputs::new(&document, Some(selected), spatial.clone(), &initialization);
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
    let inputs = Inputs::new(&document, Some(selected), spatial.clone(), &initialization);
    assert!(
        draft.valid(&inputs),
        "the captured evidence itself is still current"
    );
    assert_eq!(draft.candidate(&inputs).unwrap_err().source, Some(target));

    document.project.remove_from_filmstrip(peer).unwrap();
    let inputs = Inputs::new(&document, None, spatial, &initialization);
    assert!(
        draft.candidate(&inputs).is_err(),
        "even an untargeted photo is part of captured roll membership"
    );
}
