use super::*;
use crate::{
    processing::InputSettings,
    project::{Save, UNITY_WB},
    raw::{RawFile, Rect, panasonic_fixture},
};

#[cfg(target_os = "macos")]
#[test]
fn corrected_rw2_import_reopen_view_and_bounded_exports() {
    let file = panasonic_fixture::source(Some(panasonic_fixture::distortion()), true);
    let raw = RawFile::open(file.path()).unwrap();
    assert!(raw.metadata.lens_geometry().unwrap().is_some());
    super::raw_formats::import_reopen_view_and_bounded_exports(&[file.path().to_owned()]);
}

#[test]
fn switching_input_stage_preserves_corrected_rw2_geometry() {
    let fixture = panasonic_fixture::source(Some(panasonic_fixture::distortion()), true);
    let path = fixture.path();
    let raw = RawFile::open(path).unwrap();
    let corrected_area = Rect::from([0, 0, 82, 82]);
    let mut document = Document::default();
    document.import(&[path.to_owned()]).unwrap();
    let id = document.project.order[0];
    assert_eq!(
        document.project.source(id).unwrap().development.active_area,
        [0, 0, 82, 82]
    );
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("corrected.antitone");
    document.save(&project, Save::New).unwrap();

    let mut opened = Document::open(&project).unwrap();
    let loader = source_loading::SourceLoader::new();
    let initialization = crate::source::Initialization::default();
    for (input, wb) in [
        (InputSettings::default(), UNITY_WB),
        (InputSettings::Digital, raw.metadata.as_shot_wb()),
    ] {
        opened.project.input = input;
        let source = opened.source_input(id, &initialization).unwrap();
        let acquired =
            pollster::block_on(source.source(&loader, crate::source::Priority::Foreground))
                .unwrap();
        let source = acquired.prepared.source;
        assert_eq!(source.area(), corrected_area);
        assert_eq!(source.white_balance(), wb);
        assert!(source.camera_image().unwrap().is_continuous());
    }
    loader.close_and_wait().unwrap();
}
