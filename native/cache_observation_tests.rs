use super::*;

fn monitor() -> Monitor {
    let read = Reading {
        footprint: 100,
        resident: 100,
        metal: Some(10),
        swapouts: 42,
        pressure: 1,
        free: 4 << 30,
        speculative_free: 0,
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
#[ignore = "read-only process/system preflight observation; no RAW or export work"]
fn resource_preflight_only() {
    with_monitor(|monitor, _, _| monitor.check()).unwrap();
}

#[test]
fn preflight_reports_pressure_and_free_memory_without_relaxing_either_requirement() {
    let mut read = monitor().status.lock().unwrap().latest;
    assert!(preflight(read).is_ok());
    read.free = GROWTH_LIMIT - 1;
    read.speculative_free = GROWTH_LIMIT;
    let error = preflight(read).unwrap_err();
    assert!(error.contains("pressure=1"));
    assert!(error.contains(&format!("reported_free_bytes={}", GROWTH_LIMIT - 1)));
    assert!(error.contains(&format!("speculative_free_bytes={GROWTH_LIMIT}")));
    read.free = GROWTH_LIMIT;
    assert!(preflight(read).is_ok());
    read.pressure = 2;
    assert!(preflight(read).unwrap_err().contains("pressure=2"));
    read.pressure = 4;
    assert!(preflight(read).is_err());
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
