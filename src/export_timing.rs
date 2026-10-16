//! Explicit diagnostic recording only; normal builds expand markers to the original expression.
macro_rules! timed {
    ($stage:ident, $body:expr) => {{
        #[cfg(test)]
        {
            $crate::export_timing::measure($crate::export_timing::Stage::$stage, || $body)
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

    #[derive(Clone, Copy, Debug)]
    #[repr(usize)]
    pub(crate) enum Stage {
        Setup,
        CoveragePlan,
        TileAccess,
        SampleFill,
        WeightedMean,
        TargetEncode,
        FrameAllocation,
        FormatPack,
        FileEncode,
        FileFlush,
        DestinationFinalize,
        SourcePrepare,
    }

    impl Stage {
        const ALL: [Self; 12] = [
            Self::Setup,
            Self::CoveragePlan,
            Self::TileAccess,
            Self::SampleFill,
            Self::WeightedMean,
            Self::TargetEncode,
            Self::FrameAllocation,
            Self::FormatPack,
            Self::FileEncode,
            Self::FileFlush,
            Self::DestinationFinalize,
            Self::SourcePrepare,
        ];
    }

    #[derive(Default)]
    pub(crate) struct Timings {
        elapsed: [Duration; 12],
        calls: [u64; 12],
    }

    impl Timings {
        pub(crate) fn report(&self, total: Duration) {
            for stage in Stage::ALL {
                let i = stage as usize;
                println!(
                    "{} phase={stage:?} seconds={:.6} calls={}",
                    if i >= 11 {
                        "export_detail"
                    } else {
                        "export_stage"
                    },
                    self.elapsed[i].as_secs_f64(),
                    self.calls[i]
                );
            }
            let sum: Duration = self.elapsed[..11].iter().sum();
            println!(
                "export_stage accounted_seconds={:.6} residual_seconds={:.6} overlap_seconds={:.6} total_seconds={:.6}",
                sum.as_secs_f64(),
                total.saturating_sub(sum).as_secs_f64(),
                sum.saturating_sub(total).as_secs_f64(),
                total.as_secs_f64()
            );
        }
    }

    thread_local! {
        static ACTIVE: RefCell<Option<Timings>> = const { RefCell::new(None) };
    }

    pub(crate) fn measure<T>(stage: Stage, run: impl FnOnce() -> T) -> T {
        if !ACTIVE.with(|active| active.borrow().is_some()) {
            return run();
        }
        let start = Instant::now();
        let result = run();
        let elapsed = start.elapsed();
        ACTIVE.with(|active| {
            let mut active = active.borrow_mut();
            let timings = active.as_mut().unwrap();
            timings.elapsed[stage as usize] += elapsed;
            timings.calls[stage as usize] += 1;
        });
        result
    }

    pub(crate) fn record<T>(run: impl FnOnce() -> T) -> (T, Timings) {
        ACTIVE.with(|active| {
            let mut active = active.borrow_mut();
            assert!(
                active.is_none(),
                "export timing recordings must not overlap on one thread"
            );
            *active = Some(Timings::default());
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
        (result, timings)
    }

    #[test]
    fn recording_is_explicit_single_execution_and_resets_after_error_or_panic() {
        let mut calls = 0;
        super::timed!(CoveragePlan, {
            calls += 1;
        });
        assert_eq!(calls, 1);
        let start = Instant::now();
        let (result, timings) = record(|| {
            super::timed!(CoveragePlan, {
                calls += 1;
                Err::<(), _>("stop")
            })
        });
        timings.report(start.elapsed());
        assert_eq!(result, Err("stop"));
        assert_eq!(calls, 2);
        assert_eq!(timings.calls[Stage::CoveragePlan as usize], 1);
        assert_eq!(timings.calls.iter().sum::<u64>(), 1);
        assert!(ACTIVE.with(|a| a.borrow().is_none()));
        assert!(std::panic::catch_unwind(|| record(|| panic!("diagnostic unwind"))).is_err());
        assert!(ACTIVE.with(|a| a.borrow().is_none()));
        let (_, empty) = record(|| ());
        assert_eq!(empty.calls.iter().sum::<u64>(), 0);
    }
}
