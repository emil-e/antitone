use super::*;

#[test]
#[ignore = "requires an enabled FX Nikon Z NEF in ANTITONE_NIKON_Z_EDITOR_SAMPLE and a GPU"]
fn nikon_z_corrected_import_reopen_view_and_bounded_exports() {
    let path =
        PathBuf::from(std::env::var_os("ANTITONE_NIKON_Z_EDITOR_SAMPLE").expect("set sample path"));
    let raw = crate::raw::RawFile::open(&path).unwrap();
    assert!(raw.metadata.lens_geometry().unwrap().is_some());
    super::raw_formats::import_reopen_view_and_bounded_exports(&[path]);
}
