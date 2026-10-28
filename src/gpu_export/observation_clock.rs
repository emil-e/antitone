//! Opt-in GPU timing for the shared engine; never changes processing or admission.
use crate::gpu::timestamp_ledger::Timebase;
use std::{cell::RefCell, sync::Arc};

thread_local! {
    static CLOCK: RefCell<Option<Arc<Timebase>>> = const { RefCell::new(None) };
}

pub(crate) fn timebase() -> Option<Arc<Timebase>> {
    CLOCK.with(|clock| clock.borrow().clone())
}

#[cfg(feature = "editor")]
pub(super) fn with_clock<T>(clock: Option<Arc<Timebase>>, run: impl FnOnce() -> T) -> T {
    CLOCK.with(|active| {
        assert!(active.borrow().is_none());
        *active.borrow_mut() = clock;
    });
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            CLOCK.with(|active| {
                active.borrow_mut().take();
            });
        }
    }
    let _reset = Reset;
    run()
}

#[cfg(feature = "editor")]
mod tests;
