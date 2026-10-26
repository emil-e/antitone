use super::exclusive_rename;

#[test]
fn publication_moves_complete_output_and_preserves_an_existing_destination() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("édition-写真.tmp");
    let destination = root.path().join("édition-写真.tiff");
    std::fs::write(&source, b"complete new output").unwrap();
    std::fs::write(&destination, b"keep old output").unwrap();
    let error = exclusive_rename(&source, &destination).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read(&source).unwrap(), b"complete new output");
    assert_eq!(std::fs::read(&destination).unwrap(), b"keep old output");

    std::fs::remove_file(&destination).unwrap();
    exclusive_rename(&source, &destination).unwrap();
    assert!(!source.exists());
    assert_eq!(std::fs::read(&destination).unwrap(), b"complete new output");

    std::fs::remove_file(&destination).unwrap();
    assert!(exclusive_rename(&source, &destination).is_err());
    assert!(
        !destination.exists(),
        "failed publication must not create an empty destination"
    );
}

#[test]
fn competing_publications_have_one_winner_and_leave_the_losers_source_intact() {
    let root = tempfile::tempdir().unwrap();
    let destination = root.path().join("result.tiff");
    let sources = [root.path().join("a.tmp"), root.path().join("b.tmp")];
    for (i, source) in sources.iter().enumerate() {
        std::fs::write(source, vec![i as u8; 1024 * 1024]).unwrap();
    }
    let barrier = std::sync::Barrier::new(2);
    let results = std::thread::scope(|scope| {
        let handles: Vec<_> = sources
            .iter()
            .map(|source| {
                let barrier = &barrier;
                let destination = &destination;
                scope.spawn(move || {
                    barrier.wait();
                    exclusive_rename(source, destination)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    let winner = results.iter().position(Result::is_ok).unwrap();
    let loser = 1 - winner;
    assert_eq!(
        results[loser].as_ref().unwrap_err().kind(),
        std::io::ErrorKind::AlreadyExists
    );
    assert!(!sources[winner].exists());
    assert_eq!(
        std::fs::read(&destination).unwrap(),
        vec![winner as u8; 1024 * 1024]
    );
    assert_eq!(
        std::fs::read(&sources[loser]).unwrap(),
        vec![loser as u8; 1024 * 1024]
    );
}

#[test]
fn embedded_nul_paths_fail_instead_of_publishing_to_a_truncated_name() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.tmp");
    let destination = root.path().join("output");
    std::fs::write(&source, b"source").unwrap();
    let invalid = root.path().join("output\0suffix");
    assert_eq!(
        exclusive_rename(&source, &invalid).unwrap_err().kind(),
        std::io::ErrorKind::InvalidInput
    );
    assert_eq!(std::fs::read(&source).unwrap(), b"source");
    assert!(!destination.exists());
}
