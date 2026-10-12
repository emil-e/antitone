use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub(crate) struct Read {
    pub path: PathBuf,
    pub bytes: u64,
    pub elapsed: Duration,
}

thread_local! {
    static READS: RefCell<Option<Vec<Read>>> = const { RefCell::new(None) };
}

pub(crate) fn start() -> Option<Instant> {
    READS.with(|reads| reads.borrow().as_ref().map(|_| Instant::now()))
}

pub(crate) fn finish(path: &Path, bytes: u64, started: Option<Instant>) {
    if let Some(started) = started {
        let elapsed = started.elapsed();
        READS.with(|reads| {
            reads.borrow_mut().as_mut().unwrap().push(Read {
                path: path.to_owned(),
                bytes,
                elapsed,
            });
        });
    }
}

pub(crate) fn capture<T>(work: impl FnOnce() -> T) -> (T, Vec<Read>) {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            READS.with(|reads| *reads.borrow_mut() = None);
        }
    }
    READS.with(|reads| {
        assert!(reads.borrow().is_none(), "fingerprint capture cannot nest");
        *reads.borrow_mut() = Some(Vec::new());
    });
    let _reset = Reset;
    let result = work();
    let reads = READS.with(|reads| reads.borrow_mut().take().unwrap());
    (result, reads)
}

#[test]
fn observer_counts_successful_reads_only_on_its_thread() {
    use super::Fingerprint;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source");
    std::fs::write(&path, vec![7; 70_001]).unwrap();
    assert!(start().is_none());
    let (fingerprint, reads) = capture(|| {
        std::thread::scope(|scope| {
            scope
                .spawn(|| Fingerprint::read(&path).unwrap())
                .join()
                .unwrap();
        });
        assert!(Fingerprint::read(&directory.path().join("missing")).is_err());
        Fingerprint::read(&path).unwrap()
    });
    assert_eq!(reads.len(), 1);
    assert_eq!(reads[0].path, path);
    assert_eq!(reads[0].bytes, 70_001);
    assert_eq!(fingerprint.bytes, reads[0].bytes);
    assert!(start().is_none());
    assert!(std::panic::catch_unwind(|| capture(|| panic!("capture cleanup"))).is_err());
    assert!(start().is_none());
}
