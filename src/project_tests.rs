use super::*;
use std::io::Write;

#[test]
fn only_unsupported_publication_errors_select_the_exclusive_copy_fallback() {
    // exFAT answers a no-replace rename with ENOTSUP; anything else — above all
    // EEXIST for an occupied destination — must keep failing the save directly
    // rather than retrying publication through a different mechanism.
    let unsupported = [libc::ENOTSUP, libc::EOPNOTSUPP];
    for errno in unsupported {
        assert!(publication_unsupported(&std::io::Error::from_raw_os_error(
            errno
        )));
    }
    let supported = [libc::EEXIST, libc::EACCES, libc::EISDIR, libc::EROFS];
    for errno in supported {
        assert!(!publication_unsupported(
            &std::io::Error::from_raw_os_error(errno)
        ));
    }
    assert!(!publication_unsupported(&std::io::Error::other(
        "no raw errno"
    )));
}

#[test]
fn exclusive_copy_publishes_complete_content_and_never_replaces_an_existing_file() {
    let directory = tempfile::tempdir().unwrap();
    let completed = |name: &str| {
        let mut temporary = tempfile::NamedTempFile::new_in(directory.path()).unwrap();
        temporary.write_all(name.as_bytes()).unwrap();
        temporary.as_file().sync_all().unwrap();
        temporary
    };

    let path = directory.path().join("published");
    copy_exclusive(&completed("first save"), &path).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"first save");

    // Publication via copy must leave the same private mode as publication
    // via rename, which keeps the temporary's 0o600.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    // A second save to the occupied destination fails and leaves it untouched,
    // like the no-replace rename it substitutes for.
    assert!(copy_exclusive(&completed("second save"), &path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"first save");
}
