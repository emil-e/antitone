//! Opt-in application-path coverage using private, read-only X-Trans RAFs.
#[test]
#[ignore = "requires ANTITONE_XTRANS_SAMPLES (path-separated RAF files) and a GPU"]
fn raf_import_reopen_view_and_bounded_exports() {
    let paths: Vec<_> = std::env::split_paths(
        &std::env::var_os("ANTITONE_XTRANS_SAMPLES").expect("set ANTITONE_XTRANS_SAMPLES"),
    )
    .collect();
    assert!(!paths.is_empty());
    for path in &paths {
        let header = crate::raw::RawFile::open(path).unwrap().metadata;
        assert_eq!(
            header.filters,
            9,
            "{} must actually be X-Trans",
            path.display()
        );
    }
    super::raw_formats::import_reopen_view_and_bounded_exports(&paths);
}
