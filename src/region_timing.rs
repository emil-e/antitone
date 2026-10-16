//! Explicit test-only preparation timings; ordinary builds evaluate only the marked expression.
macro_rules! timed {
    ($stage:literal, $body:expr) => {{
        #[cfg(test)]
        {
            $crate::region_timing::measure($stage, || $body)
        }
        #[cfg(not(test))]
        {
            $body
        }
    }};
}
pub(crate) use timed;

#[cfg(test)]
pub(crate) use recording::*;
#[cfg(test)]
mod recording {
    use std::{
        cell::RefCell,
        time::{Duration, Instant},
    };

    #[derive(Debug, serde::Serialize)]
    pub(crate) struct Sample {
        pub stage: &'static str,
        pub start_seconds: f64,
        pub seconds: f64,
        pub exclusive_seconds: f64,
    }
    pub(crate) struct Timings {
        pub samples: Vec<Sample>,
        origin: Instant,
        children: Vec<Duration>,
    }
    thread_local! { static ACTIVE: RefCell<Option<Timings>> = const { RefCell::new(None) }; }

    struct Scope {
        stage: &'static str,
        start: Instant,
    }
    impl Drop for Scope {
        fn drop(&mut self) {
            let elapsed = self.start.elapsed();
            ACTIVE.with(|active| {
                let mut active = active.borrow_mut();
                let timings = active.as_mut().expect("active timing scope");
                let children = timings.children.pop().expect("balanced timing scope");
                if let Some(parent) = timings.children.last_mut() {
                    *parent += elapsed;
                }
                timings.samples.push(Sample {
                    stage: self.stage,
                    start_seconds: self.start.duration_since(timings.origin).as_secs_f64(),
                    seconds: elapsed.as_secs_f64(),
                    exclusive_seconds: elapsed.saturating_sub(children).as_secs_f64(),
                });
            });
        }
    }
    pub(crate) fn measure<T>(stage: &'static str, run: impl FnOnce() -> T) -> T {
        let scope = ACTIVE.with(|active| {
            let mut active = active.borrow_mut();
            active.as_mut().map(|timings| {
                timings.children.push(Duration::ZERO);
                Scope {
                    stage,
                    start: Instant::now(),
                }
            })
        });
        let result = run();
        drop(scope);
        result
    }
    pub(crate) fn record<T>(run: impl FnOnce() -> T) -> (T, Timings) {
        ACTIVE.with(|active| {
            let mut active = active.borrow_mut();
            assert!(
                active.is_none(),
                "region recordings must not overlap on one thread"
            );
            *active = Some(Timings {
                samples: vec![],
                origin: Instant::now(),
                children: vec![],
            });
        });
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                ACTIVE.with(|active| {
                    active.borrow_mut().take();
                });
            }
        }
        let _reset = Reset;
        let result = run();
        let timings = ACTIVE.with(|active| active.borrow_mut().take().unwrap());
        assert!(timings.children.is_empty());
        (result, timings)
    }

    #[test]
    fn explicit_recording_executes_once_accounts_for_children_and_resets_after_unwind() {
        let mut calls = 0;
        super::timed!("inactive", {
            calls += 1;
        });
        let (result, timings) = record(|| {
            super::timed!("outer", {
                super::timed!("inner", {
                    calls += 1;
                });
                Err::<(), _>("stop")
            })
        });
        assert_eq!(calls, 2);
        assert_eq!(result, Err("stop"));
        assert_eq!(timings.samples.len(), 2);
        let inner = &timings.samples[0];
        let outer = &timings.samples[1];
        assert_eq!(inner.stage, "inner");
        assert_eq!(outer.stage, "outer");
        assert!(outer.seconds >= inner.seconds);
        assert!((outer.exclusive_seconds + inner.seconds - outer.seconds).abs() < 1e-9);
        assert!(
            std::panic::catch_unwind(|| record(|| super::timed!("panic", {
                panic!("diagnostic unwind")
            })))
            .is_err()
        );
        let (_, empty) = record(|| ());
        assert!(empty.samples.is_empty());
    }
}
