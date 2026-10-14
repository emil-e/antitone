use super::*;
fn monitor() -> Monitor {
    let read = Reading {
        footprint: 100,
        resident: 100,
        metal: Some(10),
        swapouts: 42,
        pressure: 1,
        free: 4 << 30,
    };
    Monitor {
        baseline: 100,
        swapouts: 42,
        status: Mutex::new(Status {
            latest: read,
            sampled: Instant::now(),
            peak_footprint: 100,
            peak_resident: 100,
            peak_metal: 10,
            samples: 1,
            stopped: None,
        }),
    }
}
#[test]
fn stops_latch_on_alarm_unknown_counter_change_growth_and_stale_sampling() {
    for cause in ["pressure", "counter", "growth", "unknown", "stale"] {
        let monitor = monitor();
        let mut read = monitor.status.lock().unwrap().latest;
        match cause {
            "pressure" => read.pressure = 2,
            "counter" => read.swapouts += 1,
            "growth" => read.footprint = monitor.baseline + GROWTH_LIMIT,
            "unknown" => monitor.record(Err("unavailable".into())),
            "stale" => {
                monitor.status.lock().unwrap().sampled = Instant::now() - Duration::from_secs(3)
            }
            _ => unreachable!(),
        }
        if !matches!(cause, "unknown" | "stale") {
            monitor.record(Ok(read));
        }
        assert!(monitor.check().is_err());
        let healthy = Reading {
            pressure: 1,
            swapouts: 42,
            footprint: 100,
            ..read
        };
        monitor.record(Ok(healthy));
        assert!(
            monitor.check().is_err(),
            "a later healthy sample must not erase the stop"
        );
    }
}
