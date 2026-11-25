//! Separate mathematical-definition capture from bounded compiler-expression construction.
use std::{cell::RefCell, thread::LocalKey};
type Counter = RefCell<Option<(usize, usize)>>;
thread_local! {
    static DEFINITIONS: Counter=const{RefCell::new(None)};
    static CALCULATIONS: Counter=const{RefCell::new(None)};
}
fn increment(key: &'static LocalKey<Counter>) {
    key.with_borrow_mut(|observation| {
        if let Some((count, limit)) = observation {
            assert!(
                *count < *limit,
                "description expanded past the observed allocation bound"
            );
            *count += 1;
        }
    });
}
pub(super) fn created() {
    increment(&DEFINITIONS)
}
pub(super) fn calculated() {
    increment(&CALCULATIONS)
}
fn observe_in<T>(
    key: &'static LocalKey<Counter>,
    limit: usize,
    work: impl FnOnce() -> T,
) -> (T, usize) {
    struct Reset(&'static LocalKey<Counter>);
    impl Drop for Reset {
        fn drop(&mut self) {
            self.0.with_borrow_mut(|value| *value = None);
        }
    }
    key.with_borrow_mut(|value| {
        assert!(value.is_none());
        *value = Some((0, limit));
    });
    let reset = Reset(key);
    let result = work();
    let count = key.with_borrow(|value| value.unwrap().0);
    drop(reset);
    (result, count)
}
pub(crate) fn observe<T>(limit: usize, work: impl FnOnce() -> T) -> (T, usize) {
    observe_in(&DEFINITIONS, limit, work)
}
pub(crate) fn observe_calculations<T>(limit: usize, work: impl FnOnce() -> T) -> (T, usize) {
    observe_in(&CALCULATIONS, limit, work)
}
